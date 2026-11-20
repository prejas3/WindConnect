//! CCS/1 M:WN-48 the PC's output volume for the phone (volume/1, PROTOCOL.md §9.9).
//! FLOW:Core Audio (the default render device's IAudioEndpointVolume) -> its change callback -> [`PcVolume`] events -> the
//! session loop sends `Level`; the phone's `Set` -> [`PcVolume::set`] -> one COM thread. A new default device (IMMNotificationClient)
//! re-attaches. RULE:no polling; a `Set` changes only what it names (a level alone never unmutes).
use tokio::sync::mpsc::UnboundedSender;

/// The PC's volume, 0..=100, and muted.
pub type Level = (u8, bool);

enum Job {
    Set(Option<u8>, Option<bool>),
    /// The default device changed (or the first attach).
    Attach,
}

#[cfg(debug_assertions)]
static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Debug: what the COM thread did ("set <level?> <muted?>", "level <n> <muted>"), numbers only, for check-call-volume-android.cjs.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_volume_log() -> Vec<String> {
    LOG.lock().unwrap().clone()
}

fn log(_line: impl FnOnce() -> String) {
    #[cfg(debug_assertions)]
    LOG.lock().unwrap().push(_line());
}

pub struct PcVolume {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl PcVolume {
    /// Starts the COM thread; every level (the first one, every change from anywhere) goes to `levels`.
    pub fn start(levels: UnboundedSender<Level>) -> Self {
        let (jobs, rx) = std::sync::mpsc::channel();
        let again = jobs.clone();
        let _ = std::thread::Builder::new()
            .name("pc-volume".into())
            .spawn(move || platform::run(rx, again, levels));
        let _ = jobs.send(Job::Attach);
        Self { jobs }
    }

    pub fn set(&self, level: Option<u8>, muted: Option<bool>) {
        let _ = self.jobs.send(Job::Set(level.map(|l| l.min(100)), muted));
    }
}

impl Clone for PcVolume {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
        }
    }
}

/// The phone's volume as the panel shows it; `controllable`: the phone takes `volume/1` (its "Volume from the computer").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PhoneVolume {
    pub level: u8,
    pub muted: bool,
    pub controllable: bool,
}

fn accepts_volume(session: &wind_core::session::Session) -> bool {
    session
        .peer_capabilities()
        .iter()
        .any(|c| c.name() == "volume" && c.version() == 1)
}

/// CCS/1 FLOW:WN-48 the PC's level -> the phone, while the phone takes `volume/1` and "Volume from the phone" is on; again when
/// the phone starts taking it; at most every [`wind_core::volume::PERIOD`]. Ends with the session (`tick` closes).
pub async fn send_levels(
    session: wind_core::session::Session,
    mut level: tokio::sync::watch::Receiver<Option<Level>>,
    mut tick: tokio::sync::watch::Receiver<u64>,
    mut enabled: tokio::sync::watch::Receiver<bool>,
) {
    use wind_core::volume::{self, Message, PERIOD};
    let mut sent: Option<Level> = None;
    loop {
        let accepted = accepts_volume(&session);
        let now = *level.borrow_and_update();
        let on = *enabled.borrow_and_update();
        tick.borrow_and_update();
        if !accepted || !on {
            sent = None;
        }
        if let (true, true, Some((l, m))) = (accepted, on, now) {
            if sent != now {
                match volume::send(&session, &Message::Level { level: l, muted: m }).await {
                    Ok(true) => sent = now,
                    Ok(false) => {}
                    Err(_) => return,
                }
                tokio::time::sleep(PERIOD).await;
                continue;
            }
        }
        tokio::select! {
            changed = level.changed() => if changed.is_err() { return },
            changed = tick.changed() => if changed.is_err() { return },
            changed = enabled.changed() => if changed.is_err() { return },
        }
    }
}

/// The phone's `Level`s (to the panel) and `Set`s (to Core Audio while "Volume from the phone" is on): of what is queued, the
/// latest of each kind; `Set`s at most every PERIOD.
pub async fn serve_phone(
    mut queue: tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
    volume: PcVolume,
    levels: UnboundedSender<(u8, bool)>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use wind_core::volume::{Message, PERIOD};
    while let Some(first) = queue.recv().await {
        let (mut level, mut set) = (None, None);
        let mut take = |m: &wind_core::session::ChannelMessage| match Message::decode(&m.message) {
            Some(Message::Level { level: l, muted }) => level = Some((l, muted)),
            Some(Message::Set { level: l, muted }) => set = Some((l, muted)),
            None => {}
        };
        take(&first);
        while let Ok(next) = queue.try_recv() {
            take(&next);
        }
        if let Some(l) = level {
            let _ = levels.send(l);
        }
        if let Some((l, m)) = set {
            if enabled.load(std::sync::atomic::Ordering::SeqCst) {
                volume.set(l, m);
            }
            tokio::time::sleep(PERIOD).await;
        }
    }
}

/// The panel's changes to the phone's volume: the latest wins, at most every PERIOD.
pub async fn send_sets(
    session: wind_core::session::Session,
    mut sets: tokio::sync::mpsc::Receiver<(Option<u8>, Option<bool>)>,
) {
    use wind_core::volume::{self, Message, PERIOD};
    while let Some((level, muted)) = volume::latest(&mut sets).await {
        if volume::send(&session, &Message::Set { level, muted })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(PERIOD).await;
    }
}

/// Core Audio's scalar 0.0..=1.0 as 0..=100.
pub fn from_scalar(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

pub fn to_scalar(level: u8) -> f32 {
    f32::from(level.min(100)) / 100.0
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::core::{implement, Result, PCWSTR};
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::{
        eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
        DEVICE_STATE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    #[implement(IAudioEndpointVolumeCallback)]
    struct Changes(UnboundedSender<Level>);

    impl IAudioEndpointVolumeCallback_Impl for Changes_Impl {
        fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
            // SAFETY: Core Audio passes a valid struct for the call's duration.
            if let Some(d) = unsafe { data.as_ref() } {
                let level = (from_scalar(d.fMasterVolume), d.bMuted.as_bool());
                log(|| format!("level {} {}", level.0, level.1));
                let _ = self.0.send(level);
            }
            Ok(())
        }
    }

    #[implement(IMMNotificationClient)]
    struct Devices(std::sync::mpsc::Sender<Job>);

    impl IMMNotificationClient_Impl for Devices_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }
        fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
            if flow == eRender && role == eConsole {
                let _ = self.0.send(Job::Attach);
            }
            Ok(())
        }
        fn OnPropertyValueChanged(
            &self,
            _: &PCWSTR,
            _: &windows::Win32::Foundation::PROPERTYKEY,
        ) -> Result<()> {
            Ok(())
        }
    }

    fn read(endpoint: &IAudioEndpointVolume) -> Option<Level> {
        unsafe {
            let scalar = endpoint.GetMasterVolumeLevelScalar().ok()?;
            let muted = endpoint.GetMute().ok()?.as_bool();
            Some((from_scalar(scalar), muted))
        }
    }

    pub(super) fn run(
        jobs: std::sync::mpsc::Receiver<Job>,
        again: std::sync::mpsc::Sender<Job>,
        levels: UnboundedSender<Level>,
    ) {
        // SAFETY: this thread's own apartment.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let Ok(enumerator): Result<IMMDeviceEnumerator> =
            (unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) })
        else {
            return;
        };
        let devices: IMMNotificationClient = Devices(again).into();
        let _ = unsafe { enumerator.RegisterEndpointNotificationCallback(&devices) };
        let callback: IAudioEndpointVolumeCallback = Changes(levels.clone()).into();
        let mut endpoint: Option<IAudioEndpointVolume> = None;
        for job in jobs {
            match job {
                Job::Attach => {
                    if let Some(old) = endpoint.take() {
                        let _ = unsafe { old.UnregisterControlChangeNotify(&callback) };
                    }
                    endpoint = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                        .and_then(|d| unsafe {
                            d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                        })
                        .ok();
                    if let Some(e) = &endpoint {
                        let _ = unsafe { e.RegisterControlChangeNotify(&callback) };
                        if let Some(l) = read(e) {
                            let _ = levels.send(l);
                        }
                    }
                }
                Job::Set(level, muted) => {
                    log(|| format!("set {level:?} {muted:?} attached={}", endpoint.is_some()));
                    let Some(e) = &endpoint else { continue };
                    // Only what the Set names; the change callback reports the result.
                    unsafe {
                        if let Some(l) = level {
                            let _ = e.SetMasterVolumeLevelScalar(to_scalar(l), std::ptr::null());
                        }
                        if let Some(m) = muted {
                            let _ = e.SetMute(m, std::ptr::null());
                        }
                    }
                }
            }
        }
        if let Some(e) = endpoint {
            let _ = unsafe { e.UnregisterControlChangeNotify(&callback) };
        }
        let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&devices) };
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn run(
        jobs: std::sync::mpsc::Receiver<Job>,
        _: std::sync::mpsc::Sender<Job>,
        _: UnboundedSender<Level>,
    ) {
        for _ in jobs {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads this PC's real default device (manual: `cargo test pc_volume -- --ignored`); changes nothing.
    #[test]
    #[ignore]
    fn reads_the_default_device() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _volume = PcVolume::start(tx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Ok(level) = rx.try_recv() {
                assert!(level.0 <= 100);
                println!("default device: {}% muted={}", level.0, level.1);
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no level from Core Audio"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn scalar_and_percent() {
        assert_eq!(from_scalar(0.0), 0);
        assert_eq!(from_scalar(1.0), 100);
        assert_eq!(from_scalar(0.404), 40);
        assert_eq!(from_scalar(2.0), 100);
        assert_eq!(from_scalar(-1.0), 0);
        for l in 0..=100u8 {
            assert_eq!(from_scalar(to_scalar(l)), l);
        }
    }
}
