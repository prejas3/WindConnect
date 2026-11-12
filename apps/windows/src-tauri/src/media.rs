//! CCS/1 M:WN-46 the phone's player in the panel (PROTOCOL.md §9.6): media/1 in, media-control/1 out.
//! SEC:titles, names and art come from other apps on the phone: texts go through the clipboard preview's sanitiser (one line,
//! no control/bidi/zero-width characters) and the panel sets them as text only; art becomes a data: URL only for real
//! JPEG/PNG bytes, typed by their magic bytes. Nothing is stored or logged.
use wind_core::media::{image_mime, Message, Now};

/// The phone's player as the panel gets it in `pairing-state`.
#[derive(Clone, PartialEq, Debug, serde::Serialize)]
pub struct MediaView {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub app: String,
    pub duration_ms: Option<u64>,
    pub position_ms: u64,
    /// Wall-clock ms when this state arrived: the panel extrapolates progress from it (no polling).
    pub received_ms: u64,
    pub playing: bool,
    pub speed_permille: u64,
    pub actions: u64,
    /// `data:image/jpeg|png;base64,…`, or none.
    pub art: Option<String>,
}

/// What the PC keeps of the phone's player during a session.
#[derive(Default)]
pub struct Player {
    art: Option<([u8; 8], String)>,
    pub view: Option<MediaView>,
}

fn clean(text: &str) -> String {
    crate::clipboard_history::preview(text)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Player {
    /// Folds one message in; true when the view changed.
    pub fn apply(&mut self, message: Message) -> bool {
        let before = self.view.clone();
        match message {
            Message::Art { key, image } => {
                // Typed by its own bytes; the core already refused anything else.
                if let Some(mime) = image_mime(&image) {
                    let url = format!(
                        "data:{mime};base64,{}",
                        crate::clipboard_history::base64(&image)
                    );
                    self.art = Some((key, url));
                }
                // An Art alone shows nothing new until a Now points at it.
                return false;
            }
            Message::Gone => self.view = None,
            Message::Now(now) => self.view = Some(self.view_of(now)),
        }
        self.view != before
    }

    fn view_of(&self, now: Now) -> MediaView {
        let art = now.art.and_then(|key| {
            self.art
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, url)| url.clone())
        });
        MediaView {
            title: clean(&now.title),
            artist: now.artist.as_deref().map(clean).filter(|s| !s.is_empty()),
            album: now.album.as_deref().map(clean).filter(|s| !s.is_empty()),
            app: clean(&now.app),
            duration_ms: now.duration_ms,
            position_ms: now.position_ms,
            received_ms: now_ms(),
            playing: now.playing,
            speed_permille: now.speed_permille,
            actions: now.actions,
            art,
        }
    }
}

/// The panel's button names to commands.
pub fn action(name: &str, position_ms: Option<u64>) -> Option<wind_core::media::Action> {
    use wind_core::media::Action;
    Some(match (name, position_ms) {
        ("play", _) => Action::Play,
        ("pause", _) => Action::Pause,
        ("toggle", _) => Action::Toggle,
        ("next", _) => Action::Next,
        ("previous", _) => Action::Previous,
        ("seek", Some(at)) => Action::Seek(at),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_core::media::{allow, art_key};

    fn song(art: Option<[u8; 8]>) -> Now {
        Now {
            title: "Ka\u{202E}onashi\nx".into(),
            artist: Some("\u{200B}".into()),
            album: None,
            app: "Spotify".into(),
            duration_ms: Some(200_000),
            position_ms: 1_000,
            playing: true,
            speed_permille: 1000,
            actions: allow::PLAY | allow::PAUSE,
            art,
        }
    }

    #[test]
    fn texts_are_sanitised_and_art_follows_its_key() {
        let jpeg = vec![0xff, 0xd8, 0xff, 1, 2];
        let key = art_key(&jpeg);
        let mut p = Player::default();
        assert!(!p.apply(Message::Art { key, image: jpeg }));
        assert!(p.apply(Message::Now(song(Some(key)))));
        let v = p.view.clone().unwrap();
        assert_eq!(v.title, "Ka onashi x", "bidi override and newline gone");
        assert_eq!(v.artist, None, "only a zero-width character: nothing");
        assert!(v.art.unwrap().starts_with("data:image/jpeg;base64,/9j/"));
        // A Now pointing at art we don't have shows none.
        p.apply(Message::Now(song(Some([7; 8]))));
        assert_eq!(p.view.as_ref().unwrap().art, None);
        assert!(p.apply(Message::Gone));
        assert!(p.view.is_none());
    }

    #[test]
    fn panel_actions() {
        use wind_core::media::Action;
        assert_eq!(action("toggle", None), Some(Action::Toggle));
        assert_eq!(action("seek", Some(5)), Some(Action::Seek(5)));
        assert_eq!(action("seek", None), None);
        assert_eq!(action("eject", None), None);
    }
}
