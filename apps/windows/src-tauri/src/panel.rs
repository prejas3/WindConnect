use std::{sync::Mutex, time::Instant};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct Runtime(pub Mutex<State>);
#[derive(Default)]
pub struct State {
    pub shortcut_error: Option<String>,
    ready: bool,
    pending: bool,
    started: Option<Instant>,
    sequence: u64,
    painted_us: Option<u64>,
}
#[derive(serde::Serialize)]
pub struct Snapshot {
    shortcut_error: Option<String>,
    ready: bool,
    visible: bool,
    sequence: u64,
    painted_us: Option<u64>,
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
}

// CCS/1 IO:physical monitor/work rectangles + anchor -> panel origin; negative desktops and DPI preserved.
pub fn place(
    monitor: [i32; 4],
    work: [i32; 4],
    anchor: [i32; 2],
    size: [i32; 2],
    gap: i32,
) -> [i32; 2] {
    let [mx, my, mr, mb] = monitor;
    let [wx, wy, wr, wb] = work;
    let [ax, ay] = anchor;
    let [w, h] = size;
    let (x, y) = if wy > my {
        (ax - w / 2, wy + gap)
    } else if wx > mx {
        (wx + gap, ay - h / 2)
    } else if wr < mr {
        (wr - w - gap, ay - h / 2)
    } else {
        // CCS/1 RULE:auto-hidden taskbar can leave work==monitor; anchor chooses nearest edge.
        if wb == mb && ay - my < (mb - ay).min(ax - mx).min(mr - ax) {
            (ax - w / 2, wy + gap)
        } else if wb == mb && ax - mx < (mr - ax).min(mb - ay) {
            (wx + gap, ay - h / 2)
        } else if wb == mb && mr - ax < mb - ay {
            (wr - w - gap, ay - h / 2)
        } else {
            (ax - w / 2, wb - h - gap)
        }
    };
    [x.clamp(wx, (wr - w).max(wx)), y.clamp(wy, (wb - h).max(wy))]
}

pub fn show(app: &tauri::AppHandle) -> tauri::Result<()> {
    let runtime = app.state::<Runtime>();
    {
        let mut state = runtime.0.lock().unwrap();
        if !state.pending {
            state.started = Some(Instant::now());
        }
        state.painted_us = None;
        if !state.ready {
            state.pending = true;
            return Ok(());
        }
        state.pending = false;
        state.sequence += 1;
    }
    let window = app.get_webview_window("panel").expect("configured panel");
    if let Some(menu) = app.get_webview_window("tray-menu") {
        menu.hide()?;
    }
    let anchor = app
        .tray_by_id("connection")
        .and_then(|tray| tray.rect().ok().flatten());
    let cursor = app.cursor_position()?;
    let point = anchor
        .map(|rect| {
            let pos = rect.position.to_physical::<i32>(1.0);
            let size = rect.size.to_physical::<u32>(1.0);
            tauri::PhysicalPosition::new(
                pos.x as f64 + size.width as f64 / 2.0,
                pos.y as f64 + size.height as f64 / 2.0,
            )
        })
        .unwrap_or(cursor);
    if let Some(monitor) = app
        .monitor_from_point(point.x, point.y)?
        .or(app.primary_monitor()?)
    {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let mp = monitor.position();
        let ms = monitor.size();
        let width = (380.0 * scale).round() as u32;
        let height = (540.0 * scale).round() as u32;
        let size =
            tauri::PhysicalSize::new(width.min(area.size.width), height.min(area.size.height));
        window.set_size(size)?;
        let pos = place(
            [mp.x, mp.y, mp.x + ms.width as i32, mp.y + ms.height as i32],
            [
                area.position.x,
                area.position.y,
                area.position.x + area.size.width as i32,
                area.position.y + area.size.height as i32,
            ],
            [point.x as i32, point.y as i32],
            [size.width as i32, size.height as i32],
            (8.0 * scale).round() as i32,
        );
        window.set_position(tauri::PhysicalPosition::new(pos[0], pos[1]))?;
    }
    window.show()?;
    window.set_focus()?;
    let sequence = runtime.0.lock().unwrap().sequence;
    app.emit_to("panel", "panel-opened", sequence)?;
    Ok(())
}

#[tauri::command]
pub fn panel_ready(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "panel" {
        return Err("panel only".into());
    }
    let pending = {
        let runtime = app.state::<Runtime>();
        let mut state = runtime.0.lock().unwrap();
        state.ready = true;
        state.pending
    };
    if pending {
        show(&app).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub fn panel_painted(app: tauri::AppHandle, window: tauri::WebviewWindow, sequence: u64) {
    if window.label() != "panel" {
        return;
    }
    let runtime = app.state::<Runtime>();
    let mut state = runtime.0.lock().unwrap();
    if state.sequence == sequence {
        state.painted_us = state.started.map(|t| t.elapsed().as_micros() as u64);
    }
}
#[tauri::command]
pub fn panel_action(app: tauri::AppHandle, action: &str) -> Result<(), String> {
    match action {
        "show" => show(&app),
        "hide" => {
            app.state::<Runtime>().0.lock().unwrap().pending = false;
            app.get_webview_window("panel")
                .expect("configured panel")
                .hide()
        }
        "settings" => {
            app.get_webview_window("panel")
                .unwrap()
                .hide()
                .map_err(|e| e.to_string())?;
            crate::tray::show_main(&app)
        }
        _ => return Err("unsupported panel action".into()),
    }
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn panel_snapshot(app: tauri::AppHandle) -> Result<Snapshot, String> {
    let window = app.get_webview_window("panel").ok_or("panel unavailable")?;
    let runtime = app.state::<Runtime>();
    let state = runtime.0.lock().unwrap();
    Ok(Snapshot {
        shortcut_error: state.shortcut_error.clone(),
        ready: state.ready,
        visible: window.is_visible().map_err(|e| e.to_string())?,
        sequence: state.sequence,
        painted_us: state.painted_us,
        position: window.outer_position().map_err(|e| e.to_string())?,
        size: window.outer_size().map_err(|e| e.to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::place;
    #[test]
    fn all_taskbar_edges_and_negative_monitor() {
        let monitor = [-1920, -200, 0, 880];
        for (work, anchor, expected) in [
            ([-1920, -200, 0, 840], [-20, 860], [-380, 292]),
            ([-1920, -160, 0, 880], [-20, -180], [-380, -152]),
            ([-1880, -200, 0, 880], [-1900, 800], [-1872, 340]),
            ([-1920, -200, -40, 880], [-20, 800], [-428, 340]),
        ] {
            assert_eq!(place(monitor, work, anchor, [380, 540], 8), expected);
        }
    }
    #[test]
    fn high_dpi_small_workarea_and_auto_hide() {
        assert_eq!(
            place(
                [0, 0, 3840, 2160],
                [0, 0, 3840, 2080],
                [3800, 2120],
                [760, 1080],
                16
            ),
            [3080, 984]
        );
        assert_eq!(
            place(
                [0, 0, 320, 240],
                [0, 0, 320, 200],
                [300, 220],
                [320, 200],
                8
            ),
            [0, 0]
        );
        assert_eq!(
            place(
                [0, 0, 1920, 1080],
                [0, 0, 1920, 1080],
                [1900, 1079],
                [380, 540],
                8
            ),
            [1540, 532]
        );
    }
}
