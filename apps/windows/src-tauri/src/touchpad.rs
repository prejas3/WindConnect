//! CCS/1 M:touchpad | IO:admitted input/2 -> one-frame coalescing -> SendInput.
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use wind_core::{
    input::{InputEvent, Interpolator},
    protocol::{Button, Channel, Gesture},
    session::Session,
};

#[derive(Default, Clone, PartialEq, serde::Serialize)]
pub struct View {
    pub allowed: bool,
    pub active: bool,
    pub message: String,
}
pub struct Control {
    view: Arc<Mutex<View>>,
    enabled: mpsc::Sender<(bool, tokio::sync::oneshot::Sender<()>)>,
    stopped: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Control {
    pub fn start(
        session: Session,
        input: mpsc::Receiver<InputEvent>,
        name: String,
        allowed: bool,
        changed: mpsc::UnboundedSender<()>,
    ) -> Self {
        Self::start_with(session, input, name, allowed, changed, Native::default())
    }
    fn start_with(
        session: Session,
        mut input: mpsc::Receiver<InputEvent>,
        name: String,
        allowed: bool,
        changed: mpsc::UnboundedSender<()>,
        mut native: impl Injector,
    ) -> Self {
        let view = Arc::new(Mutex::new(View {
            allowed,
            ..View::default()
        }));
        let (enabled, mut on) = mpsc::channel::<(bool, tokio::sync::oneshot::Sender<()>)>(8);
        let (stopped, mut stop) = watch::channel(false);
        let state = view.clone();
        let task = tokio::spawn(async move {
            let mut smooth = Interpolator::default();
            let mut deadline = None;
            let mut refused = false;
            let mut accepting = allowed;
            let mut withdrawal_pending = false;
            let mut live = false;
            let mut published = state.lock().unwrap().clone();
            loop {
                let events = tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    enabled_change = on.recv() => {
                        let Some((enabled,ack))=enabled_change else {break};
                        accepting=enabled;live=false;
                        refused = false;
                        native.release(); smooth.clear(); deadline = None;
                        { let mut v = state.lock().unwrap(); v.allowed = enabled; v.active = false; v.message.clear(); }
                        if !enabled {withdrawal_pending=true;}
                        let change=session.set_accepted(Channel::Input, enabled);
                        tokio::pin!(change);
                        // CCS/1 RULE:drain without injection while withdrawing, so a full platform queue cannot deadlock the input actor.
                        loop {
                            tokio::select! {
                                result = &mut change => {
                                    if matches!(result, Ok(false)) && !enabled { withdrawal_pending = false; }
                                    break;
                                }
                                event = input.recv(), if !enabled => {
                                    if matches!(event, Some(InputEvent::Withdrawn)) { withdrawal_pending = false; }
                                    if event.is_none() { break; }
                                }
                            }
                        }
                        let _ = changed.send(());
                        let _ = ack.send(());
                        continue;
                    }
                    _ = session.closed() => break,
                    _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {
                        deadline = None; smooth.flush()
                    }
                    event = input.recv() => {
                        let Some(event) = event else { break };
                        if event==InputEvent::Withdrawn {withdrawal_pending=false;continue;}
                        if !accepting || refused || withdrawal_pending { continue; }
                        match event {
                            InputEvent::Begin(_) => { native.release();live=true; state.lock().unwrap().active = true; }
                            InputEvent::End(_) => { native.release();live=false; let mut v=state.lock().unwrap();v.active = false;v.message.clear(); }
                            _ => {}
                        }
                        if !live && !matches!(event,InputEvent::End(_)){continue;}
                        let events = smooth.push(event);
                        if smooth.pending() && deadline.is_none() { deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_micros(8_333)); }
                        events
                    }
                };
                for event in events {
                    if native.apply(event).is_err() {
                        native.release();
                        smooth.clear();
                        deadline = None;
                        {
                            let mut v = state.lock().unwrap();
                            v.active = false;
                            v.allowed = false;
                            v.message="Touchpad wyłączony: pulpit zablokowany lub Windows odmówił sterowania. Włącz ponownie na zwykłym pulpicie.".into();
                        }
                        refused = true;
                        live = false;
                        withdrawal_pending = true;
                        let withdraw = session.set_accepted(Channel::Input, false);
                        tokio::pin!(withdraw);
                        loop {
                            tokio::select! {
                                result = &mut withdraw => {
                                    if matches!(result, Ok(false)) {withdrawal_pending=false;}
                                    break;
                                }
                                event = input.recv() => {
                                    if matches!(event, Some(InputEvent::Withdrawn)) {withdrawal_pending=false;}
                                    if event.is_none() {break;}
                                }
                            }
                        }
                        // CCS/1 SEC:do not retry after a desktop check or injection failure; UIPI is not diagnosed separately.
                        let _ = changed.send(());
                        break;
                    }
                }
                let mut v = state.lock().unwrap();
                if v.active {
                    v.message = format!("Telefon steruje kursorem · {name}");
                }
                if *v != published {
                    published = v.clone();
                    let _ = changed.send(());
                }
            }
            native.release();
            state.lock().unwrap().active = false;
        });
        Self {
            view,
            enabled,
            stopped,
            task,
        }
    }
    pub fn view(&self) -> View {
        self.view.lock().unwrap().clone()
    }
    pub async fn set(&self, value: bool) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.enabled.send((value, tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
    pub async fn stop(self) {
        self.stopped.send_replace(true);
        let _ = self.task.await;
    }
}

trait Injector: Send + 'static {
    fn apply(&mut self, event: InputEvent) -> Result<(), ()>;
    fn release(&mut self);
}
impl Injector for Native {
    fn apply(&mut self, event: InputEvent) -> Result<(), ()> {
        Native::apply(self, event)
    }
    fn release(&mut self) {
        Native::release(self)
    }
}

#[derive(Default)]
struct Native {
    pressed: Vec<Button>,
    remainder: [i64; 4],
}
impl Native {
    fn release(&mut self) {
        for button in std::mem::take(&mut self.pressed) {
            let _ = send(&[mouse(button_flags(button, false), 0, 0, 0)]);
        }
        self.remainder = [0; 4];
    }
    fn scaled(&mut self, slot: usize, value: i64, divisor: i64, bound: i64) -> i32 {
        let total = self.remainder[slot].saturating_add(value);
        let delta = (total / divisor).clamp(-bound, bound);
        self.remainder[slot] = if (total / divisor).abs() > bound {
            0
        } else {
            total % divisor
        };
        delta as i32
    }
    fn apply(&mut self, event: InputEvent) -> Result<(), ()> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        if matches!(
            event,
            InputEvent::Begin(_) | InputEvent::End(_) | InputEvent::Withdrawn
        ) {
            self.release();
            return Ok(());
        }
        if !ordinary_desktop() {
            return Err(());
        }
        let events = match event {
            InputEvent::Pointer { dx, dy, .. } => {
                let x = self.scaled(0, dx, 100, 4096);
                let y = self.scaled(1, dy, 100, 4096);
                vec![mouse(MOUSEEVENTF_MOVE, x, y, 0)]
            }
            InputEvent::Scroll { dx, dy, .. } => {
                let x = self.scaled(2, dx, 100, 1200);
                let y = self.scaled(3, dy, 100, 1200);
                let mut v = Vec::new();
                if x != 0 {
                    v.push(mouse(MOUSEEVENTF_HWHEEL, 0, 0, x));
                }
                if y != 0 {
                    v.push(mouse(MOUSEEVENTF_WHEEL, 0, 0, -y));
                }
                v
            }
            InputEvent::Button { button, down } => {
                if down && !self.pressed.contains(&button) {
                    self.pressed.push(button);
                }
                vec![mouse(button_flags(button, down), 0, 0, 0)]
            }
            InputEvent::Gesture(g) => chord(g)
                .into_iter()
                .map(|(key, up)| keyboard(key, up))
                .collect(),
            _ => Vec::new(),
        };
        if send(&events).is_err() {
            self.release();
            return Err(());
        }
        if let InputEvent::Button {
            button,
            down: false,
        } = event
        {
            self.pressed.retain(|b| *b != button);
        }
        Ok(())
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.release();
    }
}
fn chord(g: Gesture) -> Vec<(u16, bool)> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let keys: &[u16] = match g {
        Gesture::TaskView => &[VK_LWIN, VK_TAB],
        Gesture::ShowDesktop => &[VK_LWIN, 0x44],
        Gesture::PreviousApp => &[VK_MENU, VK_SHIFT, VK_TAB],
        Gesture::NextApp => &[VK_MENU, VK_TAB],
        Gesture::Other(_) => &[],
    };
    keys.iter()
        .map(|k| (*k, false))
        .chain(keys.iter().rev().map(|k| (*k, true)))
        .collect()
}
fn button_flags(button: Button, down: bool) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    match (button, down) {
        (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (Button::Left, false) => MOUSEEVENTF_LEFTUP,
        (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
        (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
        (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        _ => 0,
    }
}
fn mouse(
    flags: u32,
    x: i32,
    y: i32,
    data: i32,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
fn keyboard(key: u16, up: bool) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
fn send(events: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> Result<(), ()> {
    inject(events, |events| unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
        ) as usize
    })
}
fn inject(
    events: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
    mut write: impl FnMut(&[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> usize,
) -> Result<(), ()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    if events.is_empty() {
        return Ok(());
    }
    let inserted = write(events);
    if inserted == events.len() {
        Ok(())
    } else {
        // CCS/1 RULE:only the inserted prefix owns keys; undo remaining downs in reverse order, once.
        let mut held = Vec::new();
        for input in events.iter().take(inserted) {
            if input.r#type == INPUT_KEYBOARD {
                // SAFETY:the input type selects the keyboard union member.
                let key = unsafe { input.Anonymous.ki };
                if key.dwFlags & KEYEVENTF_KEYUP != 0 {
                    held.retain(|k| *k != key.wVk);
                } else if !held.contains(&key.wVk) {
                    held.push(key.wVk);
                }
            }
        }
        let mut releases = Vec::new();
        for key in held.into_iter().rev() {
            if key == VK_LWIN || key == VK_RWIN {
                // CCS/1 WHY:reserved VK 0xFF down/up prevents a lone Win release from opening Start.
                releases.push(keyboard(0xff, false));
                releases.push(keyboard(0xff, true));
            }
            releases.push(keyboard(key, true));
        }
        if !releases.is_empty() {
            write(&releases);
        }
        Err(())
    }
}
fn ordinary_desktop() -> bool {
    use windows_sys::Win32::System::StationsAndDesktops::*;
    unsafe {
        let desktop = OpenInputDesktop(0, false as i32, DESKTOP_READOBJECTS);
        if desktop.is_null() {
            return false;
        }
        let mut name = [0u16; 256];
        let mut needed = 0;
        let ok = GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            512,
            &mut needed,
        ) != 0;
        CloseDesktop(desktop);
        ok && String::from_utf16_lossy(&name[..name.iter().position(|v| *v == 0).unwrap_or(256)])
            .eq_ignore_ascii_case("Default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_off_on_and_injection_refusal_release_without_replay() {
        use wind_core::{
            identity::DeviceIdentity,
            protocol::DeviceKind,
            session::{LocalInfo, Router},
            transport::{Endpoint, PinnedKeys},
        };
        struct Recording(Arc<Mutex<Vec<Option<InputEvent>>>>);
        impl Injector for Recording {
            fn apply(&mut self, event: InputEvent) -> Result<(), ()> {
                self.0.lock().unwrap().push(Some(event));
                if matches!(event, InputEvent::Gesture(_)) {
                    Err(())
                } else {
                    Ok(())
                }
            }
            fn release(&mut self) {
                self.0.lock().unwrap().push(None);
            }
        }
        let a = DeviceIdentity::generate().unwrap();
        let b = DeviceIdentity::generate().unwrap();
        let pa = Arc::new(PinnedKeys::new());
        let pb = Arc::new(PinnedKeys::new());
        pa.pin(b.public_key());
        pb.pin(a.public_key());
        let ea = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &a, pa).unwrap();
        let eb = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &b, pb).unwrap();
        let info = || LocalInfo {
            name: "test".into(),
            kind: DeviceKind::Computer,
            app: None,
        };
        let mut router = Router::new();
        let rx = router.route_input_version(2, 2);
        let accepting = eb.clone();
        let remote_info = info();
        let server = tokio::spawn(async move {
            Session::accept(
                accepting.accept().await.unwrap().unwrap(),
                remote_info,
                router,
            )
            .await
            .unwrap()
            .0
        });
        let (phone, _) = Session::dial(
            &ea,
            eb.local_addr().unwrap(),
            b.public_key(),
            info(),
            Router::new(),
        )
        .await
        .unwrap();
        let pc = server.await.unwrap();
        let input = phone.input().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let (changed, _) = mpsc::unbounded_channel();
        let control = Control::start_with(
            pc,
            rx,
            "S25".into(),
            true,
            changed,
            Recording(output.clone()),
        );
        input.button(Button::Left, true).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !output.lock().unwrap().contains(&Some(InputEvent::Button {
                button: Button::Left,
                down: true,
            })) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(control.set(false), control.set(true));
        })
        .await
        .unwrap();
        assert!(!control.view().active);
        assert!(control.view().allowed);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while input.announcement().is_none_or(|a| a.enabled == 0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        output.lock().unwrap().clear();
        input.gesture(Gesture::TaskView).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while control.view().allowed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!control.view().active);
        assert!(control.view().message.contains("wyłączony"));
        assert!(output.lock().unwrap().contains(&None));
        control.stop().await;
        phone
            .connection()
            .close(wind_core::protocol::ErrorCode::GoingAway, b"");
        ea.close();
        eb.close();
    }
    #[test]
    fn partial_chord_releases_every_key_in_a_recording_injector() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        for (gesture, keys) in [
            (Gesture::TaskView, vec![VK_LWIN, VK_TAB]),
            (Gesture::ShowDesktop, vec![VK_LWIN, 0x44]),
            (Gesture::PreviousApp, vec![VK_MENU, VK_SHIFT, VK_TAB]),
            (Gesture::NextApp, vec![VK_MENU, VK_TAB]),
        ] {
            let inputs: Vec<_> = chord(gesture)
                .into_iter()
                .map(|(k, up)| keyboard(k, up))
                .collect();
            for partial in 0..=inputs.len() {
                let mut batches = Vec::new();
                let result = inject(&inputs, |batch| {
                    batches.push(
                        batch
                            .iter()
                            .map(|i| unsafe {
                                (
                                    i.Anonymous.ki.wVk,
                                    i.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0,
                                )
                            })
                            .collect::<Vec<_>>(),
                    );
                    if batches.len() == 1 {
                        partial
                    } else {
                        batch.len()
                    }
                });
                assert_eq!(result.is_ok(), partial == inputs.len());
                // All downs precede reverse-order ups: only these initial keys remain held.
                let held = partial.min(inputs.len() - partial);
                let mut expected = Vec::new();
                for key in keys.iter().take(held).rev() {
                    if *key == VK_LWIN {
                        expected.extend([(0xff, false), (0xff, true)]);
                    }
                    expected.push((*key, true));
                }
                if expected.is_empty() {
                    assert_eq!(batches.len(), 1);
                } else {
                    assert_eq!(batches.len(), 2);
                    assert_eq!(batches[1], expected, "{gesture:?}, inserted={partial}");
                }
            }
        }
    }

    #[test]
    fn refused_win_cleanup_is_attempted_only_once() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let inputs = [keyboard(VK_LWIN, false), keyboard(VK_TAB, false)];
        let mut calls = 0;
        assert!(inject(&inputs, |batch| {
            calls += 1;
            if calls == 1 {
                1
            } else {
                assert_eq!(batch.len(), 3);
                0
            }
        })
        .is_err());
        assert_eq!(calls, 2);
    }
    #[test]
    fn all_chords_are_balanced_and_win_never_alone() {
        for g in [
            Gesture::TaskView,
            Gesture::ShowDesktop,
            Gesture::PreviousApp,
            Gesture::NextApp,
        ] {
            let keys = chord(g);
            let half = keys.len() / 2;
            assert!(half >= 2);
            assert_eq!(
                keys[..half].iter().map(|x| x.0).collect::<Vec<_>>(),
                keys[half..].iter().rev().map(|x| x.0).collect::<Vec<_>>()
            );
            assert!(keys[..half].iter().all(|x| !x.1));
            assert!(keys[half..].iter().all(|x| x.1));
        }
    }
    #[test]
    fn fractions_and_hostile_deltas_are_bounded() {
        let mut n = Native::default();
        assert_eq!(n.scaled(0, 50, 100, 4096), 0);
        assert_eq!(n.scaled(0, 50, 100, 4096), 1);
        assert_eq!(n.scaled(0, i64::MAX, 100, 4096), 4096);
        assert_eq!(n.scaled(0, 0, 100, 4096), 0);
    }
}
