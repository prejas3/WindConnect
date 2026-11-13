//! CCS/1 M:WN-49 the phone's battery on the PC: panel (pairing snapshot), tray tooltip and low badge, alert toasts.
//! RULE:state only while a session is live; pushed by the phone (§9.4), never polled. Toasts carry the level and fixed
//! text only; a click opens the panel. LIMIT:no value while the phone's background connection is off (Doze, FGS off).
use std::sync::Mutex;

use wind_core::battery::{Alert, Status, LOW};

/// The battery as the panel gets it in `pairing-state`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
}

impl From<Status> for Battery {
    fn from(s: Status) -> Self {
        Self {
            level: s.level,
            charging: s.charging,
        }
    }
}

static CURRENT: Mutex<Option<Battery>> = Mutex::new(None);

/// The phone's battery now (None: no session or nothing reported yet).
pub fn current() -> Option<Battery> {
    *CURRENT.lock().unwrap()
}

/// Low and not charging: the tray icon gets its badge.
pub fn is_low(battery: Option<Battery>) -> bool {
    battery.is_some_and(|b| !b.charging && b.level <= LOW)
}

/// The tray tooltip's battery part, empty without a value.
pub fn tooltip(battery: Option<Battery>) -> String {
    match battery {
        Some(Battery {
            level,
            charging: true,
        }) => format!(" · bateria {level}%, ładuje się"),
        Some(Battery { level, .. }) => format!(" · bateria {level}%"),
        None => String::new(),
    }
}

/// Records the battery and refreshes the tray (icon badge and tooltip) when it changed.
pub fn set(app: &tauri::AppHandle, battery: Option<Battery>) {
    {
        let mut current = CURRENT.lock().unwrap();
        if *current == battery {
            return;
        }
        *current = battery;
    }
    crate::tray::refresh_icon(app);
}

/// The toast's title and text for `alert` from the phone named `phone`.
pub fn alert_text(alert: Alert, phone: &str) -> (String, String) {
    match alert {
        Alert::Low(level) => (
            format!("Bateria telefonu: {level}%"),
            format!("{phone} · podłącz ładowarkę."),
        ),
        Alert::Full => (
            "Telefon naładowany".into(),
            format!("{phone} · 100%, możesz odłączyć ładowarkę."),
        ),
    }
}

/// Alerts shown in this run (debug builds: the native check reads them).
#[cfg(debug_assertions)]
static SHOWN: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_battery_alerts() -> Vec<String> {
    SHOWN.lock().unwrap().clone()
}

/// Shows `alert` as a Windows toast; a click opens the panel.
pub fn notify(app: &tauri::AppHandle, alert: Alert, phone: &str) {
    let (title, text) = alert_text(alert, phone);
    #[cfg(debug_assertions)]
    SHOWN.lock().unwrap().push(title.clone());
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = toast(&app, &title, &text) {
            eprintln!("battery toast: {error}");
        }
    });
}

#[cfg(windows)]
fn toast(app: &tauri::AppHandle, title: &str, text: &str) -> Result<(), String> {
    use tauri_winrt_notification::Toast;
    let id = crate::toast::app_id(app)?;
    let opener = app.clone();
    Toast::new(&id)
        .title(title)
        .text1(text)
        .on_activated(move |_| {
            let panel = opener.clone();
            let _ = opener.run_on_main_thread(move || {
                let _ = crate::panel::show(&panel);
            });
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn toast(_: &tauri::AppHandle, _: &str, _: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_and_badge() {
        assert_eq!(tooltip(None), "");
        let b = |level, charging| Some(Battery { level, charging });
        assert_eq!(tooltip(b(82, false)), " · bateria 82%");
        assert_eq!(tooltip(b(82, true)), " · bateria 82%, ładuje się");
        assert!(is_low(b(15, false)));
        assert!(!is_low(b(15, true)), "charging: no badge");
        assert!(!is_low(b(16, false)));
        assert!(!is_low(None));
    }

    #[test]
    fn alert_texts_carry_level_and_name_only() {
        assert_eq!(
            alert_text(Alert::Low(14), "Pixel 9"),
            (
                "Bateria telefonu: 14%".into(),
                "Pixel 9 · podłącz ładowarkę.".into()
            )
        );
        assert_eq!(alert_text(Alert::Full, "Pixel 9").0, "Telefon naładowany");
    }
}
