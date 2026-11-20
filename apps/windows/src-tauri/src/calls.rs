//! CCS/1 M:WN-57 the phone's calls on the PC (PROTOCOL.md §9.10, §9.12): an incoming call as a toast with the caller's
//! name or number and "Silence"/"Reject", a missed-call toast and the panel's list of missed calls.
//! FLOW:call State (with its caller) -> one toast per call (pops once; a later caller update replaces it quietly) -> gone at
//! offhook/idle. Missed list: the first in a session is the baseline; later new entries toast. Buttons -> call-action/1.
//! RULE:only with the phone's "Calls on the computer" (it sends the caller) and this PC's "Calls from the phone".
use std::sync::mpsc;

use serde::Serialize;
use wind_core::call::{Call, CallState, Missed, CAN_REJECT, CAN_SILENCE};

use crate::notify::{clean, escape, Job};

/// The toasts' group (apart from the phone's notifications).
pub const GROUP: &str = "call";
/// Missed calls in the panel.
pub const PANEL_MISSED: usize = 10;

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct MissedView {
    pub who: String,
    /// The number, when a name is shown.
    pub number: Option<String>,
    pub date_ms: u64,
}

/// The calls of one phone session.
pub struct Calls {
    jobs: mpsc::Sender<Job>,
    phone: String,
    /// The call whose toast is up, and whether it popped.
    ringing: Option<u64>,
    missed: Vec<Missed>,
    baseline: bool,
}

fn who(number: &Option<String>, name: &Option<String>) -> String {
    name.as_deref()
        .or(number.as_deref())
        .map(|s| clean(s, 128))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::i18n::tr("calls.unknown_number").into())
}

/// The incoming call's toast: who, the number under a name, "Call · <phone>", the buttons it can use. Silent (the
/// phone rings) and `incomingCall`, so it stays up while the phone rings.
pub fn ringing_xml(call: &Call, phone: &str) -> Option<String> {
    let c = call.caller.as_ref()?;
    let mut x = String::from(
        r#"<toast scenario="incomingCall" launch="windconnect-call"><visual><binding template="ToastGeneric">"#,
    );
    x.push_str(&format!(
        "<text>{}</text>",
        escape(&who(&c.number, &c.name))
    ));
    let line = match (&c.name, &c.number) {
        (Some(_), Some(n)) => {
            crate::i18n::trf("calls.incoming_number", &[("number", &clean(n, 64))])
        }
        _ => crate::i18n::tr("calls.incoming").into(),
    };
    x.push_str(&format!("<text>{}</text>", escape(&line)));
    x.push_str(&format!(
        r#"<text placement="attribution">{}</text>"#,
        escape(&clean(phone, 64))
    ));
    x.push_str("</binding></visual><actions>");
    if c.actions & CAN_SILENCE != 0 {
        x.push_str(&format!(
            r#"<action content="{}" arguments="call-silence:{}" activationType="background"/>"#,
            escape(crate::i18n::tr("calls.silence")),
            c.call
        ));
    }
    if c.actions & CAN_REJECT != 0 {
        x.push_str(&format!(
            r#"<action content="{}" arguments="call-reject:{}" activationType="background"/>"#,
            escape(crate::i18n::tr("calls.decline")),
            c.call
        ));
    }
    // incomingCall needs a button to stay up; "Close" only closes the toast.
    x.push_str(&format!(
        r#"<action content="{}" arguments="dismiss" activationType="system"/>"#,
        escape(crate::i18n::tr("calls.close"))
    ));
    x.push_str(r#"</actions><audio silent="true"/></toast>"#);
    Some(x)
}

pub fn missed_xml(m: &Missed, phone: &str) -> String {
    let mut x = format!(
        r#"<toast launch="windconnect-call"><visual><binding template="ToastGeneric"><text>{}</text>"#,
        escape(crate::i18n::tr("calls.missed"))
    );
    x.push_str(&format!(
        "<text>{}</text>",
        escape(&who(&Some(m.number.clone()), &m.name))
    ));
    x.push_str(&format!(
        r#"<text placement="attribution">{}</text>"#,
        escape(&clean(phone, 64))
    ));
    x.push_str("</binding></visual></toast>");
    x
}

impl Calls {
    pub fn new(jobs: mpsc::Sender<Job>, phone: &str) -> Self {
        Self {
            jobs,
            phone: phone.to_owned(),
            ringing: None,
            missed: vec![],
            baseline: false,
        }
    }

    fn tag(call: u64) -> String {
        format!("call-{call}")
    }

    /// A `State`: the ringing call's toast up (popping once per call), gone otherwise. `toasts`: this PC's setting.
    pub fn state(&mut self, call: &Call, toasts: bool) {
        let id = call.caller.as_ref().map(|c| c.call);
        if let Some(old) = self
            .ringing
            .filter(|old| Some(*old) != id || call.state != CallState::Ringing)
        {
            let _ = self.jobs.send(Job::RemoveRaw {
                tag: Self::tag(old),
                group: GROUP,
            });
            self.ringing = None;
        }
        if !toasts || call.state != CallState::Ringing {
            return;
        }
        let (Some(id), Some(xml)) = (id, ringing_xml(call, &self.phone)) else {
            return;
        };
        let popup = self.ringing != Some(id);
        self.ringing = Some(id);
        let _ = self.jobs.send(Job::Raw {
            tag: Self::tag(id),
            group: GROUP,
            xml,
            popup,
        });
    }

    /// A `Missed` list: new entries toast (not the session's first list), the panel shows the newest.
    pub fn missed(&mut self, list: Vec<Missed>, toasts: bool) {
        if self.baseline && toasts {
            let newest = self.missed.first().map(|m| m.date_ms).unwrap_or(0);
            for m in list.iter().filter(|m| m.date_ms > newest).take(3) {
                let _ = self.jobs.send(Job::Raw {
                    tag: format!("missed-{}", m.date_ms),
                    group: GROUP,
                    xml: missed_xml(m, &self.phone),
                    popup: true,
                });
            }
        }
        self.baseline = true;
        self.missed = list;
    }

    pub fn items(&self) -> Vec<MissedView> {
        self.missed
            .iter()
            .take(PANEL_MISSED)
            .map(|m| MissedView {
                who: who(&Some(m.number.clone()), &m.name),
                number: m.name.as_ref().map(|_| clean(&m.number, 64)),
                date_ms: m.date_ms,
            })
            .collect()
    }

    /// The session ended or this PC's setting went off: the ringing toast goes.
    pub fn clear(&mut self) {
        if let Some(old) = self.ringing.take() {
            let _ = self.jobs.send(Job::RemoveRaw {
                tag: Self::tag(old),
                group: GROUP,
            });
        }
    }
}

impl Drop for Calls {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_core::call::Caller;

    fn ringing(id: u64, name: Option<&str>) -> Call {
        Call {
            state: CallState::Ringing,
            caller: Some(Caller {
                call: id,
                number: Some("+48600100200".into()),
                name: name.map(Into::into),
                actions: CAN_REJECT | CAN_SILENCE,
            }),
        }
    }

    fn jobs(rx: &mpsc::Receiver<Job>) -> Vec<String> {
        rx.try_iter()
            .map(|j| match j {
                Job::Raw { tag, popup, .. } => format!("show {tag} {popup}"),
                Job::RemoveRaw { tag, .. } => format!("remove {tag}"),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn a_call_pops_once_then_goes() {
        let (tx, rx) = mpsc::channel();
        let mut calls = Calls::new(tx, "Pixel");
        calls.state(&ringing(1, None), true);
        // The name arrives after the number: replaced quietly.
        calls.state(&ringing(1, Some("Ola")), true);
        calls.state(
            &Call {
                state: CallState::Offhook,
                caller: ringing(1, Some("Ola")).caller,
            },
            true,
        );
        assert_eq!(
            jobs(&rx),
            ["show call-1 true", "show call-1 false", "remove call-1"]
        );
        // A second call, and the setting off: nothing.
        calls.state(&ringing(2, None), false);
        assert!(jobs(&rx).is_empty());
    }

    #[test]
    fn toast_text_is_escaped_and_has_the_buttons() {
        let x = ringing_xml(&ringing(7, Some("<Ola & Jan>")), "Pixel").unwrap();
        assert!(x.contains("&lt;Ola &amp; Jan&gt;"), "{x}");
        assert!(x.contains(r#"arguments="call-reject:7""#), "{x}");
        assert!(x.contains(r#"arguments="call-silence:7""#), "{x}");
        assert!(x.contains("+48600100200"));
        assert!(x.contains(r#"scenario="incomingCall""#));
        assert_eq!(
            crate::notify::call_action("call-reject:7"),
            Some(wind_core::call::Action::Reject { call: 7 })
        );
        assert_eq!(crate::notify::call_action("reply"), None);
        // No caller (the phone's setting off): no toast.
        assert!(ringing_xml(
            &Call {
                state: CallState::Ringing,
                caller: None
            },
            "Pixel"
        )
        .is_none());
    }

    #[test]
    fn missed_list_toasts_only_new_ones() {
        let (tx, rx) = mpsc::channel();
        let mut calls = Calls::new(tx, "Pixel");
        let m = |d: u64| Missed {
            number: "+48600".into(),
            name: Some("Ola".into()),
            date_ms: d,
        };
        calls.missed(vec![m(10)], true);
        assert!(
            jobs(&rx).is_empty(),
            "the session's first list is the baseline"
        );
        calls.missed(vec![m(20), m(10)], true);
        assert_eq!(jobs(&rx), ["show missed-20 true"]);
        assert_eq!(calls.items()[0].who, "Ola");
        assert_eq!(calls.items()[0].number.as_deref(), Some("+48600"));
    }
}
