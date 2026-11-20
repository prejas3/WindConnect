//! CCS/1 M:WN-100 the PC opens a link on the phone (actions/1, PROTOCOL.md §9.2). The phone hosts `actions` with the
//! core's ActionHost: only `open-url` is supported (lock, sleep and custom actions never), allowed while "Open links from
//! the computer" is on (default on; off -> status 1 "refused"); the URL has passed `check_url` before Kotlin sees it.
//! SEC:no command, program or path ever reaches the phone; Kotlin only gets an http(s) URL to show (ACTION_VIEW).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wind_core::actions::{Action, Builtin, Executor, Policy, PolicySource};

/// WN-100: opens a checked http(s) URL for the person (a browser, or a notification to tap when Android doesn't let the
/// app start one in the background). True when it was shown or offered. Called off the UI thread.
#[uniffi::export(callback_interface)]
pub trait UrlOpener: Send + Sync {
    fn open_url(&self, url: String) -> bool;
}

pub(crate) struct PhoneExecutor(pub Arc<dyn UrlOpener>);

impl Executor for PhoneExecutor {
    fn supports(&self, builtin: Builtin) -> bool {
        builtin == Builtin::OpenUrl
    }
    fn run(&self, action: &Action) -> Result<(), String> {
        match action {
            Action::OpenUrl(url) if self.0.open_url(url.clone()) => Ok(()),
            Action::OpenUrl(_) => Err("not opened".into()),
            _ => Err("not supported on the phone".into()),
        }
    }
}

/// The phone's allowlist: only links, while the setting is on; read for every request.
pub(crate) struct PhonePolicy(pub Arc<AtomicBool>);

impl PolicySource for PhonePolicy {
    fn policy(&self) -> Policy {
        Policy {
            lock: false,
            sleep: false,
            open_url: self.0.load(Ordering::SeqCst),
            custom: Vec::new(),
        }
    }
}

pub(crate) type ActionRoute = (
    Arc<dyn UrlOpener>,
    tokio::sync::mpsc::Receiver<wind_core::session::ActionStream>,
);

#[cfg(test)]
mod tests {
    use super::*;
    use wind_core::actions::{offered, Policy};

    struct Never;
    impl UrlOpener for Never {
        fn open_url(&self, _: String) -> bool {
            false
        }
    }

    #[test]
    fn the_phone_offers_only_links_and_only_when_allowed() {
        let on = Arc::new(AtomicBool::new(true));
        let executor = PhoneExecutor(Arc::new(Never));
        let listed = offered(&PhonePolicy(on.clone()).policy(), &executor);
        assert_eq!(
            listed.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            ["open-url"]
        );
        on.store(false, Ordering::SeqCst);
        assert!(offered(&PhonePolicy(on).policy(), &executor).is_empty());
        // Even a policy that allows everything can't make the phone lock or sleep.
        let all = Policy::default();
        assert_eq!(offered(&all, &executor).len(), 1);
        assert!(executor.run(&Action::Lock).is_err());
        assert!(executor
            .run(&Action::OpenUrl("https://a.pl/".into()))
            .is_err());
    }
}
