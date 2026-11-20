use crate::indicator::{Indicator, Phase};
#[cfg(debug_assertions)]
use std::time::Duration;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tokio::sync::watch;

#[derive(Clone, serde::Serialize)]
pub struct Applied {
    pub sequence: u64,
    pub phase: Phase,
    pub latency_us: u64,
    pub error: Option<String>,
}
pub struct TrayRuntime {
    pub indicator: Indicator,
    pub applied: watch::Receiver<Applied>,
    worker: tauri::async_runtime::JoinHandle<()>,
}
impl Drop for TrayRuntime {
    fn drop(&mut self) {
        self.indicator.unavailable();
        self.worker.abort();
    }
}

// CCS/1 FLOW:taskbar DPI/theme changes select a pre-rendered Orbita variant, also while disconnected.
pub fn refresh_icon(app: &tauri::AppHandle) {
    if let (Some(runtime), Some(tray)) =
        (app.try_state::<TrayRuntime>(), app.tray_by_id("connection"))
    {
        let phase = runtime.indicator.snapshot().phase;
        if let Err(error) = tray
            .set_icon(Some(crate::brand_icon::current(phase)))
            .and_then(|_| tray.set_tooltip(Some(tooltip(phase))))
        {
            eprintln!("tray icon: {error}");
        }
    }
}
/// "WindConnect · Connected", plus the phone's battery while connected (WN-49).
pub fn tooltip(phase: Phase) -> String {
    let battery = match phase {
        Phase::Connected => crate::battery::tooltip(crate::battery::current()),
        _ => String::new(),
    };
    crate::i18n::trf(
        "tray.tooltip",
        &[("state", &phase.label()), ("battery", &battery)],
    )
}
pub fn show_main(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(panel) = app.get_webview_window("panel") {
        panel.hide()?;
    }
    if let Some(menu) = app.get_webview_window("tray-menu") {
        menu.hide()?;
    }
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }
    Ok(())
}
pub fn show_menu(
    app: &tauri::AppHandle,
    position: tauri::PhysicalPosition<f64>,
) -> tauri::Result<()> {
    if let Some(panel) = app.get_webview_window("panel") {
        panel.hide()?;
    }
    let Some(window) = app.get_webview_window("tray-menu") else {
        return Ok(());
    };
    let scale = app
        .monitor_from_point(position.x, position.y)?
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let (width, height) = (280.0 * scale, 250.0 * scale);
    let (mut x, mut y) = (position.x - width, position.y - height);
    if let Some(monitor) = app.monitor_from_point(position.x, position.y)? {
        let area = monitor.work_area();
        x = x.clamp(
            area.position.x as f64,
            (area.position.x as f64 + area.size.width as f64 - width).max(area.position.x as f64),
        );
        y = y.clamp(
            area.position.y as f64,
            (area.position.y as f64 + area.size.height as f64 - height).max(area.position.y as f64),
        );
    }
    window.set_position(tauri::PhysicalPosition::new(x as i32, y as i32))?;
    window.show()?;
    window.set_focus()?;
    app.emit_to("tray-menu", "tray-menu-opened", ())?;
    Ok(())
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<TrayRuntime> {
    let indicator = Indicator::default();
    let tray = TrayIconBuilder::with_id("connection")
        .icon(crate::brand_icon::current(Phase::Unavailable))
        .tooltip(tooltip(crate::indicator::Phase::Unavailable))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let result = match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => crate::panel::show(tray.app_handle()),
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } => show_menu(tray.app_handle(), position),
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("tray window: {error}");
            }
        })
        .build(app)?;
    let mut updates = indicator.subscribe();
    let (applied_sender, applied) = watch::channel(Applied {
        sequence: 0,
        phase: Phase::Unavailable,
        latency_us: 0,
        error: None,
    });
    let app_handle = app.clone();
    let worker = tauri::async_runtime::spawn(async move {
        while updates.changed().await.is_ok() {
            let update = updates.borrow_and_update().clone();
            let tray = tray.clone();
            let sender = applied_sender.clone();
            let event_app = app_handle.clone();
            if let Err(error) = app_handle.run_on_main_thread(move || {
                let result = tray
                    .set_icon(Some(crate::brand_icon::current(update.phase)))
                    .and_then(|_| tray.set_tooltip(Some(tooltip(update.phase))));
                let applied = Applied {
                    sequence: update.sequence,
                    phase: update.phase,
                    latency_us: update.changed.elapsed().as_micros() as u64,
                    error: result.err().map(|e| e.to_string()),
                };
                let _ = event_app.emit_to("tray-menu", "connection-status", &applied);
                sender.send_replace(applied);
            }) {
                eprintln!("tray dispatch: {error}");
                break;
            }
        }
    });
    Ok(TrayRuntime {
        indicator,
        applied,
        worker,
    })
}

/// CCS/1 IO:real authenticated loopback Session -> native tray -> timing; debug only, no trust changes outside test endpoints.
#[cfg(debug_assertions)]
pub async fn probe(runtime: &TrayRuntime) -> Result<Vec<Applied>, String> {
    use std::sync::Arc;
    use wind_core::{
        identity::DeviceIdentity,
        protocol::{DeviceKind, ErrorCode},
        session::{LocalInfo, Router, Session},
        transport::{Endpoint, PinnedKeys},
    };
    let a = DeviceIdentity::generate().map_err(|e| e.to_string())?;
    let b = DeviceIdentity::generate().map_err(|e| e.to_string())?;
    let pa = Arc::new(PinnedKeys::new());
    let pb = Arc::new(PinnedKeys::new());
    pa.pin(b.public_key());
    pb.pin(a.public_key());
    let first =
        Endpoint::bind("127.0.0.1:0".parse().unwrap(), &a, pa).map_err(|e| e.to_string())?;
    let second =
        Endpoint::bind("127.0.0.1:0".parse().unwrap(), &b, pb).map_err(|e| e.to_string())?;
    let generation = runtime.indicator.searching();
    let mut applied = runtime.applied.clone();
    let mut results = vec![wait_phase(&mut applied, Phase::Searching).await?];
    let (client, server) = tokio::join!(
        first.connect(
            second.local_addr().map_err(|e| e.to_string())?,
            b.public_key()
        ),
        second.accept()
    );
    let info = |name: &str| LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: Some("tray-probe".into()),
    };
    let (left, right) = tokio::join!(
        Session::connect(
            client.map_err(|e| e.to_string())?,
            info("tray-a"),
            Router::default()
        ),
        Session::accept(
            server.ok_or("accept ended")?.map_err(|e| e.to_string())?,
            info("tray-b"),
            Router::default()
        )
    );
    let (session, _events) = left.map_err(|e| e.to_string())?;
    let (remote, _remote_events) = right.map_err(|e| e.to_string())?;
    let indicator = runtime.indicator.clone();
    let observer = tauri::async_runtime::spawn(async move {
        indicator.track_session(generation, &session).await;
    });
    results.push(wait_phase(&mut applied, Phase::Connected).await?);
    remote
        .connection()
        .close(ErrorCode::GoingAway, b"probe complete");
    results.push(wait_phase(&mut applied, Phase::Unavailable).await?);
    observer.await.map_err(|e| e.to_string())?;
    first.close();
    second.close();
    first.wait_idle().await;
    second.wait_idle().await;
    Ok(results)
}
#[cfg(debug_assertions)]
async fn wait_phase(
    receiver: &mut watch::Receiver<Applied>,
    phase: Phase,
) -> Result<Applied, String> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = receiver.borrow_and_update().clone();
            if value.phase == phase {
                if let Some(error) = &value.error {
                    return Err(error.clone());
                }
                return Ok(value);
            }
            receiver.changed().await.map_err(|e| e.to_string())?;
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
