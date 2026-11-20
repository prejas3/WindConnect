//! CCS/1 M:WN-47 the PC's player (Windows SMTC) for the phone (PROTOCOL.md §9.6): Now/Art/Gone out on `media`, Command in
//! on `media-control`. RULE:event-driven (SMTC session events), only while a phone session is live and the setting
//! "Share playback with the phone" is on; the first Playing session, else the system's current one.
//! SEC:debug builds honour WINDCONNECT_TEST_MEDIA_APP (one AUMID only) so a test never touches the person's own players.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use wind_core::media::{allow, Action, Now};
use wind_core::session::Session;

/// Windows' 100 ns ticks.
pub fn ticks_to_ms(ticks: i64) -> u64 {
    (ticks.max(0) / 10_000) as u64
}
pub fn ms_to_ticks(ms: u64) -> i64 {
    (ms as i64).saturating_mul(10_000)
}

/// The first Playing session (status 4), else the system's current one when it is among `statuses` (the sessions we may
/// follow), else the first of them (§9.6, as on the phone).
pub fn pick(statuses: &[i32], current: Option<usize>) -> Option<usize> {
    statuses
        .iter()
        .position(|s| *s == 4)
        .or(current.filter(|i| *i < statuses.len()))
        .or((!statuses.is_empty()).then_some(0))
}

/// SMTC controls to §9.6 bits.
pub fn actions(
    play: bool,
    pause: bool,
    toggle: bool,
    next: bool,
    previous: bool,
    seek: bool,
) -> u64 {
    let mut bits = 0;
    if play || toggle {
        bits |= allow::PLAY;
    }
    if pause || toggle {
        bits |= allow::PAUSE;
    }
    if next {
        bits |= allow::NEXT;
    }
    if previous {
        bits |= allow::PREVIOUS;
    }
    if seek {
        bits |= allow::SEEK;
    }
    bits
}

/// The position now from a timeline read at `updated` (both 100 ns ticks since 1601), moving only while playing.
pub fn position_now(position: i64, updated: i64, now: i64, playing: bool, rate: f64) -> i64 {
    if !playing || updated <= 0 {
        return position;
    }
    position + ((now - updated).max(0) as f64 * rate.max(0.0)) as i64
}

/// A source the test filter lets through (always true in release).
pub fn allowed_source(aumid: &str) -> bool {
    #[cfg(debug_assertions)]
    if let Ok(only) = std::env::var("WINDCONNECT_TEST_MEDIA_APP") {
        return aumid == only;
    }
    let _ = aumid;
    true
}

/// A readable name for an AUMID: "Spotify.exe" -> "Spotify", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" ->
/// "ZuneMusic"; at most 64 bytes.
pub fn app_name(aumid: &str) -> String {
    let tail = aumid.rsplit(['!', '\\', '/']).next().unwrap_or(aumid);
    let tail = tail
        .strip_suffix(".exe")
        .or_else(|| tail.strip_suffix(".EXE"))
        .unwrap_or(tail);
    let tail = tail
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(tail);
    let mut name: String = tail.chars().filter(|c| !c.is_control()).collect();
    while name.len() > wind_core::media::MAX_APP {
        name.pop();
    }
    if name.is_empty() {
        crate::i18n::tr("device.computer").into()
    } else {
        name
    }
}

/// At most `max` UTF-8 bytes, cut on a character boundary.
pub fn bounded(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

enum Msg {
    Refresh,
    /// The phone's capabilities changed: send the current state again.
    Resend,
    Command(Action),
    Stop,
}

/// The PC's player followed for one phone session.
pub struct PcMedia {
    tx: mpsc::Sender<Msg>,
    /// What the last sent Now allowed: commands outside it are dropped (§9.6, rule 4).
    allowed: Arc<AtomicU64>,
}

impl PcMedia {
    /// Starts following SMTC for `session`; sends through `runtime`. None when SMTC isn't available.
    pub fn start(session: Session, runtime: tokio::runtime::Handle) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let allowed = Arc::new(AtomicU64::new(0));
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker_tx = tx.clone();
        let worker_allowed = allowed.clone();
        std::thread::Builder::new()
            .name("WindConnect SMTC".into())
            .spawn(move || platform::run(session, runtime, rx, worker_tx, worker_allowed, ready_tx))
            .ok()?;
        ready_rx.recv().ok().filter(|ok| *ok)?;
        Some(Self { tx, allowed })
    }

    /// The phone changed what it accepts (e.g. "Show the computer's player" back on): resend.
    pub fn resend(&self) {
        let _ = self.tx.send(Msg::Resend);
    }

    /// A command from the phone (already decoded); dropped when not allowed.
    pub fn command(&self, action: Action) {
        if action.allowed_by(self.allowed.load(Ordering::SeqCst)) {
            let _ = self.tx.send(Msg::Command(action));
        }
    }
}

impl Drop for PcMedia {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapEncoder, BitmapInterpolationMode};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as SmtcSession,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
    };
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

    fn now_ticks() -> i64 {
        // 100 ns since 1601, like WinRT DateTime.
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64 / 100)
            .unwrap_or(0);
        unix + 116_444_736_000_000_000
    }

    struct Followed {
        session: SmtcSession,
        id: windows::core::HSTRING,
        tokens: [i64; 3],
    }

    pub(super) fn run(
        phone: Session,
        runtime: tokio::runtime::Handle,
        rx: mpsc::Receiver<Msg>,
        tx: mpsc::Sender<Msg>,
        allowed: Arc<AtomicU64>,
        ready: mpsc::Sender<bool>,
    ) {
        // An MTA for this thread: SMTC's change events from other processes are delivered into it.
        // SAFETY: once, on this thread, before any WinRT call here.
        let _ = unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
        };
        let manager = match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => m,
            Err(_) => {
                let _ = ready.send(false);
                return;
            }
        };
        let refresh = |tx: mpsc::Sender<Msg>| {
            move || {
                let _ = tx.send(Msg::Refresh);
            }
        };
        let r1 = refresh(tx.clone());
        let r2 = refresh(tx.clone());
        let t1 = manager
            .SessionsChanged(&TypedEventHandler::new(move |_, _| {
                r1();
                Ok(())
            }))
            .ok();
        let t2 = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                r2();
                Ok(())
            }))
            .ok();
        let _ = ready.send(true);
        let sender = wind_core::media::Sender::default();
        let mut followed: Option<Followed> = None;
        let _ = tx.send(Msg::Refresh);
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Stop => break,
                Msg::Resend => {
                    sender.reset();
                    let _ = tx.send(Msg::Refresh);
                }
                Msg::Command(action) => {
                    if let Some(f) = &followed {
                        let s = &f.session;
                        let _ = match action {
                            Action::Play => s.TryPlayAsync().and_then(|o| o.get()),
                            Action::Pause => s.TryPauseAsync().and_then(|o| o.get()),
                            Action::Toggle => s.TryTogglePlayPauseAsync().and_then(|o| o.get()),
                            Action::Next => s.TrySkipNextAsync().and_then(|o| o.get()),
                            Action::Previous => s.TrySkipPreviousAsync().and_then(|o| o.get()),
                            Action::Seek(ms) => s
                                .TryChangePlaybackPositionAsync(ms_to_ticks(ms))
                                .and_then(|o| o.get()),
                        };
                        // The person's own action: read the result once, in case the player's event is late.
                        let _ = tx.send(Msg::Refresh);
                    }
                }
                Msg::Refresh => {
                    let chosen = choose(&manager);
                    // The same player even through a new wrapper object: compare its source.
                    let same = matches!((&followed, &chosen), (Some(f), Some(c)) if c.SourceAppUserModelId().is_ok_and(|id| id == f.id));
                    if !same {
                        if let Some(f) = followed.take() {
                            unfollow(&f);
                        }
                        followed = chosen.map(|s| follow(s, &tx));
                    }
                    match followed.as_ref().and_then(|f| read(&f.session)) {
                        Some((now, art)) => {
                            allowed.store(now.actions, Ordering::SeqCst);
                            let _ = runtime.block_on(sender.now(&phone, now, art.as_deref()));
                        }
                        None => {
                            allowed.store(0, Ordering::SeqCst);
                            let _ = runtime.block_on(sender.gone(&phone));
                        }
                    }
                }
            }
        }
        if let Some(f) = followed.take() {
            unfollow(&f);
        }
        if let Some(t) = t1 {
            let _ = manager.RemoveSessionsChanged(t);
        }
        if let Some(t) = t2 {
            let _ = manager.RemoveCurrentSessionChanged(t);
        }
        let _ = runtime.block_on(sender.gone(&phone));
    }

    fn choose(manager: &Manager) -> Option<SmtcSession> {
        let sessions: Vec<SmtcSession> = manager
            .GetSessions()
            .ok()?
            .into_iter()
            .filter(|s| {
                s.SourceAppUserModelId()
                    .is_ok_and(|id| allowed_source(&id.to_string_lossy()))
            })
            .collect();
        let statuses: Vec<i32> = sessions
            .iter()
            .map(|s| {
                s.GetPlaybackInfo()
                    .and_then(|p| p.PlaybackStatus())
                    .map(|s| s.0)
                    .unwrap_or(0)
            })
            .collect();
        let current = manager
            .GetCurrentSession()
            .ok()
            .and_then(|c| sessions.iter().position(|s| *s == c));
        pick(&statuses, current).map(|i| sessions[i].clone())
    }

    fn follow(session: SmtcSession, tx: &mpsc::Sender<Msg>) -> Followed {
        let (t1, t2, t3) = (tx.clone(), tx.clone(), tx.clone());
        let tokens = [
            session
                .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = t1.send(Msg::Refresh);
                    Ok(())
                }))
                .unwrap_or(0),
            session
                .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = t2.send(Msg::Refresh);
                    Ok(())
                }))
                .unwrap_or(0),
            session
                .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = t3.send(Msg::Refresh);
                    Ok(())
                }))
                .unwrap_or(0),
        ];
        let id = session.SourceAppUserModelId().unwrap_or_default();
        Followed {
            session,
            id,
            tokens,
        }
    }

    fn unfollow(f: &Followed) {
        let _ = f.session.RemoveMediaPropertiesChanged(f.tokens[0]);
        let _ = f.session.RemovePlaybackInfoChanged(f.tokens[1]);
        let _ = f.session.RemoveTimelinePropertiesChanged(f.tokens[2]);
    }

    /// The session as §9.6 `Now`, plus its art (JPEG ≤ 256 px ≤ 96 KiB); None when it has stopped or closed.
    fn read(s: &SmtcSession) -> Option<(Now, Option<Vec<u8>>)> {
        let info = s.GetPlaybackInfo().ok()?;
        let status = info.PlaybackStatus().ok()?.0;
        if !(status == 4 || status == 5) {
            return None;
        }
        let playing = status == 4;
        let props = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()).ok()?;
        let controls = info.Controls().ok()?;
        let c = |r: windows::core::Result<bool>| r.unwrap_or(false);
        let rate = info.PlaybackRate().and_then(|r| r.Value()).unwrap_or(1.0);
        let timeline = s.GetTimelineProperties().ok();
        let (position, duration) = match &timeline {
            Some(t) => {
                let start = t.StartTime().map(|d| d.Duration).unwrap_or(0);
                let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
                let at = t.Position().map(|d| d.Duration).unwrap_or(0);
                let updated = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                let now = position_now(at, updated, now_ticks(), playing, rate);
                let duration = (end > start).then(|| ticks_to_ms(end - start));
                (
                    ticks_to_ms(now - start).min(duration.unwrap_or(u64::MAX)),
                    duration,
                )
            }
            None => (0, None),
        };
        let text = |r: windows::core::Result<windows::core::HSTRING>| {
            r.ok()
                .map(|h| bounded(&h.to_string_lossy(), wind_core::media::MAX_TEXT))
                .filter(|t| !t.is_empty())
        };
        let now = Now {
            title: text(props.Title()).unwrap_or_default(),
            artist: text(props.Artist()).or_else(|| text(props.AlbumArtist())),
            album: text(props.AlbumTitle()),
            app: app_name(
                &s.SourceAppUserModelId()
                    .map(|h| h.to_string_lossy())
                    .unwrap_or_default(),
            ),
            duration_ms: duration,
            position_ms: position,
            playing,
            speed_permille: (rate * 1000.0).round().max(0.0) as u64,
            actions: actions(
                c(controls.IsPlayEnabled()),
                c(controls.IsPauseEnabled()),
                c(controls.IsPlayPauseToggleEnabled()),
                c(controls.IsNextEnabled()),
                c(controls.IsPreviousEnabled()),
                c(controls.IsPlaybackPositionEnabled()),
            ),
            art: None,
        };
        let art = props.Thumbnail().ok().and_then(|t| thumbnail(&t).ok());
        Some((now, art))
    }

    /// Re-encodes the session's thumbnail as JPEG ≤ 256 px; refused over 96 KiB.
    fn thumbnail(
        reference: &windows::Storage::Streams::IRandomAccessStreamReference,
    ) -> windows::core::Result<Vec<u8>> {
        let stream = reference.OpenReadAsync()?.get()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
        let (w, h) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
        let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
        let out = InMemoryRandomAccessStream::new()?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::JpegEncoderId()?, &out)?.get()?;
        let converted = windows::Graphics::Imaging::SoftwareBitmap::Convert(
            &bitmap,
            windows::Graphics::Imaging::BitmapPixelFormat::Bgra8,
        )?;
        encoder.SetSoftwareBitmap(&converted)?;
        let side = w.max(h).max(1);
        if side > 256 {
            let t = encoder.BitmapTransform()?;
            t.SetScaledWidth((w * 256 / side).max(1))?;
            t.SetScaledHeight((h * 256 / side).max(1))?;
            t.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
        }
        encoder.FlushAsync()?.get()?;
        let size = out.Size()? as u32;
        if size as usize > wind_core::media::MAX_ART {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8007_000E_u32 as i32,
            )));
        }
        let reader = DataReader::CreateDataReader(&out.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn run(
        _: Session,
        _: tokio::runtime::Handle,
        _: mpsc::Receiver<Msg>,
        _: mpsc::Sender<Msg>,
        _: Arc<AtomicU64>,
        ready: mpsc::Sender<bool>,
    ) {
        let _ = ready.send(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_first_else_current() {
        assert_eq!(pick(&[5, 4], Some(0)), Some(1));
        assert_eq!(pick(&[5, 5], Some(1)), Some(1));
        assert_eq!(
            pick(&[5], None),
            Some(0),
            "paused, the current one elsewhere: still ours"
        );
        assert_eq!(
            pick(&[5, 5], Some(7)),
            Some(0),
            "a current index out of range"
        );
        assert_eq!(pick(&[], Some(0)), None);
    }

    #[test]
    fn controls_and_time() {
        assert_eq!(
            actions(false, false, true, false, false, false),
            allow::PLAY | allow::PAUSE
        );
        assert_eq!(actions(true, true, false, true, true, true), 31);
        assert_eq!(ticks_to_ms(ms_to_ticks(61_000)), 61_000);
        assert_eq!(ticks_to_ms(-5), 0);
        // 2 s after the update while playing at 1.5×: 3 s further.
        assert_eq!(
            position_now(10_000_000, 1_000, 1_000 + 20_000_000, true, 1.5),
            40_000_000
        );
        assert_eq!(
            position_now(10_000_000, 1_000, 1_000 + 20_000_000, false, 1.0),
            10_000_000
        );
    }

    #[test]
    fn names_and_bounds() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(app_name("C:\\Program Files\\VideoLAN\\VLC\\vlc.exe"), "vlc");
        assert_eq!(app_name(""), "Komputer");
        let polish = "ż".repeat(200);
        let cut = bounded(&polish, 256);
        assert!(cut.len() <= 256 && cut.chars().all(|c| c == 'ż'));
    }
}
