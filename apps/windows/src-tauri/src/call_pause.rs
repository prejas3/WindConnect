//! CCS/1 M:WN-48 pausing the PC's players while the phone rings or is in a call (call/1, PROTOCOL.md §9.10).
//! FLOW:phone `State` -> session loop -> [`CallPauser::state`] -> one worker thread (order kept, lives across sessions) ->
//! [`CallPause`] decides -> SMTC `TryPauseAsync` on every Playing session / `TryPlayAsync` on those it paused.
//! RULE:resume only what this paused, only if it is still Paused (the person may have stopped it), and not after
//! [`RESUME_WITHIN`]. A session's end doesn't clear what was paused: the next `State` (idle at every session start) or the
//! limit settles it. In debug, `WINDCONNECT_TEST_MEDIA_APP` limits it to one player, as in WN-47.
use std::sync::mpsc;
use std::time::{Duration, Instant};

use wind_core::call::CallState;

/// Players paused for a call are resumed only within this long.
pub const RESUME_WITHIN: Duration = Duration::from_secs(30 * 60);

/// What to do on a call state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// Pause every Playing session (and remember which).
    PauseAll,
    /// Resume these (by AUMID) if still Paused.
    Resume(Vec<String>),
}

/// The decision, pure: one pause per call, a resume of what it paused.
#[derive(Debug, Default)]
pub struct CallPause {
    in_call: bool,
    paused: Vec<(String, Instant)>,
}

impl CallPause {
    pub fn state(&mut self, state: CallState, now: Instant) -> Step {
        match state {
            CallState::Ringing | CallState::Offhook => {
                if self.in_call {
                    Step::Nothing
                } else {
                    self.in_call = true;
                    Step::PauseAll
                }
            }
            CallState::Idle => {
                self.in_call = false;
                let due: Vec<String> = self
                    .paused
                    .drain(..)
                    .filter(|(_, at)| now.duration_since(*at) <= RESUME_WITHIN)
                    .map(|(aumid, _)| aumid)
                    .collect();
                if due.is_empty() {
                    Step::Nothing
                } else {
                    Step::Resume(due)
                }
            }
        }
    }

    /// The sessions [`Step::PauseAll`] paused.
    pub fn paused(&mut self, aumids: Vec<String>, at: Instant) {
        for aumid in aumids {
            if !self.paused.iter().any(|(a, _)| *a == aumid) {
                self.paused.push((aumid, at));
            }
        }
    }
}

/// `call-volume.json`: "Pause playback during calls" and "Volume from the phone", both on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CallVolumeSettings {
    #[serde(default = "on")]
    pub pause_on_call: bool,
    #[serde(default = "on")]
    pub volume: bool,
    /// WN-57: "Calls from the phone": the incoming-call toast and missed calls (when the phone shares them).
    #[serde(default = "on")]
    pub calls: bool,
}

fn on() -> bool {
    true
}

impl Default for CallVolumeSettings {
    fn default() -> Self {
        Self {
            pause_on_call: true,
            volume: true,
            calls: true,
        }
    }
}

impl CallVolumeSettings {
    /// Missing or damaged -> the defaults.
    pub fn parse(bytes: Option<&[u8]>) -> Self {
        bytes
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default()
    }

    pub fn load(path: &std::path::Path) -> Self {
        Self::parse(std::fs::read(path).ok().as_deref())
    }

    /// Written whole: a temporary file, then renamed over it.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&temp, path)
    }
}

/// The worker: states in, SMTC calls out, in order.
pub struct CallPauser {
    jobs: mpsc::Sender<CallState>,
}

#[cfg(debug_assertions)]
static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Debug: what the worker did ("paused <aumid>", "resumed <aumid>"), for check-call-volume-android.cjs. No content.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_call_log() -> Vec<String> {
    LOG.lock().unwrap().clone()
}

impl CallPauser {
    pub fn start() -> Self {
        let (jobs, rx) = mpsc::channel::<CallState>();
        let _ = std::thread::Builder::new()
            .name("call-pause".into())
            .spawn(move || {
                let mut decide = CallPause::default();
                // Warm: the session manager is made once, before the first ring.
                let mut smtc = platform::Smtc::new();
                for state in rx {
                    match decide.state(state, Instant::now()) {
                        Step::Nothing => {}
                        Step::PauseAll => {
                            let paused = smtc.pause_playing();
                            #[cfg(debug_assertions)]
                            LOG.lock()
                                .unwrap()
                                .extend(paused.iter().map(|a| format!("paused {a}")));
                            decide.paused(paused, Instant::now());
                        }
                        Step::Resume(list) => {
                            let resumed = smtc.resume(&list);
                            #[cfg(debug_assertions)]
                            LOG.lock()
                                .unwrap()
                                .extend(resumed.iter().map(|a| format!("resumed {a}")));
                            let _ = resumed;
                        }
                    }
                }
            });
        Self { jobs }
    }

    pub fn state(&self, state: CallState) {
        let _ = self.jobs.send(state);
    }
}

#[cfg(windows)]
mod platform {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as SmtcSession,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };

    /// The session manager, made once when the worker starts (warm before the first ring), again if it failed.
    pub(super) struct Smtc(Option<Manager>);

    fn request() -> Option<Manager> {
        Manager::RequestAsync().and_then(|op| op.get()).ok()
    }

    fn status(s: &SmtcSession) -> Option<Status> {
        s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).ok()
    }

    impl Smtc {
        pub(super) fn new() -> Self {
            Self(request())
        }

        fn sessions(&mut self) -> Vec<(String, SmtcSession)> {
            if self.0.is_none() {
                self.0 = request();
            }
            let Some(list) = self.0.as_ref().and_then(|m| m.GetSessions().ok()) else {
                return Vec::new();
            };
            list.into_iter()
                .filter_map(|s| {
                    let aumid = s.SourceAppUserModelId().ok()?.to_string();
                    crate::pc_media::allowed_source(&aumid).then_some((aumid, s))
                })
                .collect()
        }

        /// Pauses every Playing session; the AUMIDs it paused.
        pub(super) fn pause_playing(&mut self) -> Vec<String> {
            self.sessions()
                .into_iter()
                .filter(|(_, s)| status(s) == Some(Status::Playing))
                .filter(|(_, s)| s.TryPauseAsync().and_then(|op| op.get()).unwrap_or(false))
                .map(|(aumid, _)| aumid)
                .collect()
        }

        /// Plays those of `aumids` that are still Paused; the AUMIDs it resumed.
        pub(super) fn resume(&mut self, aumids: &[String]) -> Vec<String> {
            self.sessions()
                .into_iter()
                .filter(|(a, s)| aumids.contains(a) && status(s) == Some(Status::Paused))
                .filter(|(_, s)| s.TryPlayAsync().and_then(|op| op.get()).unwrap_or(false))
                .map(|(aumid, _)| aumid)
                .collect()
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub(super) struct Smtc;
    impl Smtc {
        pub(super) fn new() -> Self {
            Self
        }
        pub(super) fn pause_playing(&mut self) -> Vec<String> {
            Vec::new()
        }
        pub(super) fn resume(&mut self, _: &[String]) -> Vec<String> {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pause_per_call_and_a_resume_of_what_was_paused() {
        let t = Instant::now();
        let mut c = CallPause::default();
        assert_eq!(c.state(CallState::Ringing, t), Step::PauseAll);
        c.paused(vec!["Spotify".into(), "Chrome".into()], t);
        assert_eq!(
            c.state(CallState::Ringing, t),
            Step::Nothing,
            "a second ringing doesn't pause again"
        );
        assert_eq!(
            c.state(CallState::Offhook, t),
            Step::Nothing,
            "answered: nothing more"
        );
        assert_eq!(
            c.state(CallState::Idle, t + Duration::from_secs(60)),
            Step::Resume(vec!["Spotify".into(), "Chrome".into()])
        );
        assert_eq!(
            c.state(CallState::Idle, t),
            Step::Nothing,
            "already settled"
        );
        // A call with nothing playing resumes nothing.
        assert_eq!(c.state(CallState::Offhook, t), Step::PauseAll);
        assert_eq!(c.state(CallState::Idle, t), Step::Nothing);
    }

    #[test]
    fn a_dropped_session_is_settled_by_the_next_state() {
        let t = Instant::now();
        let mut c = CallPause::default();
        assert_eq!(c.state(CallState::Ringing, t), Step::PauseAll);
        c.paused(vec!["Spotify".into()], t);
        // The session breaks here; nothing is cleared. The new session's first State is idle.
        assert_eq!(
            c.state(CallState::Idle, t + Duration::from_secs(120)),
            Step::Resume(vec!["Spotify".into()])
        );
        // Still in a call on the new session: nothing to do until idle.
        let mut d = CallPause::default();
        d.state(CallState::Offhook, t);
        d.paused(vec!["Spotify".into()], t);
        assert_eq!(d.state(CallState::Offhook, t), Step::Nothing);
        assert_eq!(
            d.state(CallState::Idle, t),
            Step::Resume(vec!["Spotify".into()])
        );
    }

    #[test]
    fn not_resumed_after_half_an_hour() {
        let t = Instant::now();
        let mut c = CallPause::default();
        c.state(CallState::Ringing, t);
        c.paused(vec!["Spotify".into()], t);
        assert_eq!(
            c.state(CallState::Idle, t + RESUME_WITHIN + Duration::from_secs(1)),
            Step::Nothing
        );
    }

    #[test]
    fn settings_default_on_and_survive_damage() {
        assert_eq!(
            CallVolumeSettings::parse(None),
            CallVolumeSettings::default()
        );
        assert_eq!(
            CallVolumeSettings::parse(Some(b"{\"pause_on_call\":false}")),
            CallVolumeSettings {
                pause_on_call: false,
                volume: true,
                calls: true
            }
        );
        assert_eq!(
            CallVolumeSettings::parse(Some(b"{broken")),
            CallVolumeSettings::default()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("call-volume.json");
        let s = CallVolumeSettings {
            pause_on_call: false,
            volume: false,
            calls: false,
        };
        s.save(&path).unwrap();
        assert_eq!(CallVolumeSettings::load(&path), s);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn idle_first_does_nothing() {
        assert_eq!(
            CallPause::default().state(CallState::Idle, Instant::now()),
            Step::Nothing
        );
    }
}
