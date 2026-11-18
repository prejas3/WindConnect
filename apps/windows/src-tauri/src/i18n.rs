//! WN-63: the app's native texts (tray, toasts, Explorer menu, statuses and errors the UI shows) in Polish and
//! English. The language is the Windows display language, the same one WebView2 gives the frontend
//! (`navigator.language`): Polish for Polish, English for anything else. `WINDCONNECT_LANG=pl|en` overrides it.
//! Texts live in `locales/{pl,en}.json`; `{name}` placeholders are filled by [`trf`].

use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Pl,
    En,
}

const PL: &str = include_str!("../locales/pl.json");
const EN: &str = include_str!("../locales/en.json");

fn parse(source: &'static str) -> HashMap<String, &'static str> {
    let table: HashMap<String, String> = serde_json::from_str(source).expect("locales json");
    // Leaked once per language for the process lifetime: callers keep plain &'static str.
    table
        .into_iter()
        .map(|(k, v)| (k, &*Box::leak(v.into_boxed_str())))
        .collect()
}

fn table(lang: Lang) -> &'static HashMap<String, &'static str> {
    static PL_TABLE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    static EN_TABLE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    match lang {
        Lang::Pl => PL_TABLE.get_or_init(|| parse(PL)),
        Lang::En => EN_TABLE.get_or_init(|| parse(EN)),
    }
}

fn from_tag(tag: &str) -> Option<Lang> {
    match tag.trim().to_ascii_lowercase().get(..2) {
        Some("pl") => Some(Lang::Pl),
        Some("en") => Some(Lang::En),
        _ => None,
    }
}

#[cfg(all(windows, not(test)))]
fn system() -> Lang {
    // LANG_POLISH is primary language 0x15 of the user's UI LANGID.
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    if id & 0x3ff == 0x15 {
        Lang::Pl
    } else {
        Lang::En
    }
}

/// Unit tests check the Polish texts (as the frontend checks do); the English table is checked by `tests::*`.
#[cfg(any(not(windows), test))]
fn system() -> Lang {
    Lang::Pl
}

pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        std::env::var("WINDCONNECT_LANG")
            .ok()
            .as_deref()
            .and_then(from_tag)
            .unwrap_or_else(system)
    })
}

fn lookup(lang: Lang, key: &str) -> &'static str {
    table(lang)
        .get(key)
        .or_else(|| table(Lang::En).get(key))
        .copied()
        .unwrap_or_else(|| {
            debug_assert!(false, "missing text {key}");
            ""
        })
}

/// The text for `key` in the UI language.
pub fn tr(key: &str) -> &'static str {
    lookup(lang(), key)
}

/// The text for `key` with each `{name}` replaced by its value.
pub fn trf(key: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut text = tr(key).to_string();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

/// `key.one|few|many|other` for `count` (Polish: 1 plik, 2 pliki, 5 plików, English: one/other), `{count}` filled.
pub fn trp(key: &str, count: u64) -> String {
    let form = match lang() {
        Lang::Pl if count == 1 => "one",
        Lang::Pl if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) => "few",
        Lang::Pl => "many",
        Lang::En if count == 1 => "one",
        Lang::En => "other",
    };
    trf(&format!("{key}.{form}"), &[("count", &count)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = text
            .split('{')
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn both_languages_have_the_same_keys_and_placeholders() {
        let (pl, en) = (table(Lang::Pl), table(Lang::En));
        let mut pl_keys: Vec<_> = pl.keys().collect();
        let mut en_keys: Vec<_> = en.keys().collect();
        pl_keys.sort();
        en_keys.sort();
        assert_eq!(pl_keys, en_keys);
        for (key, text) in pl {
            assert!(!text.is_empty() && !en[key].is_empty(), "{key} is empty");
            assert_eq!(
                placeholders(text),
                placeholders(en[key]),
                "{key}: placeholders"
            );
        }
    }

    #[test]
    fn language_tags_and_fill() {
        assert_eq!(from_tag("pl-PL"), Some(Lang::Pl));
        assert_eq!(from_tag("en-GB"), Some(Lang::En));
        assert_eq!(from_tag("de"), None);
        assert_eq!(lang(), Lang::Pl);
        assert_eq!(lookup(Lang::En, "tray.connected"), "Connected");
        assert_eq!(
            trf("battery.toast_title", &[("level", &15)]),
            "Bateria telefonu: 15%"
        );
    }
}
