//! Remote actions (PROTOCOL.md §9.2, WN-54): lock, sleep, open a link, and
//! the user's own actions, run on request of a paired device.
//!
//! Security model, in the order the host checks it:
//! 1. The request arrives on a session whose peer key is pinned (mTLS, §2);
//!    before running anything the host asks [`PeerTrust`] again, so a
//!    device unpaired while connected runs nothing: the connection is
//!    closed with `UNTRUSTED_PEER`.
//! 2. A request names an action by id and nothing else (a `url` for
//!    `open-url`). It never carries a command, a program or arguments:
//!    what a custom action runs is fixed on this device, by its user, in
//!    the [`Policy`].
//! 3. Only actions on this device's allowlist run: built-ins switched on,
//!    and custom actions listed and switched on. Anything else is
//!    `Refused`. The policy is read anew for every request, so switching
//!    an action off takes effect at once.
//! 4. `open-url` takes only an absolute `http`/`https` URL ([`check_url`]).
//! 5. One action runs at a time per session, at most
//!    [`MAX_RUNS_PER_WINDOW`] per [`RUN_WINDOW`] (`Busy` beyond), at most
//!    [`MAX_PENDING`] requests wait (`RATE_LIMITED` beyond).

mod wire;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Semaphore};

pub use wire::{
    is_action_id, Listed, Reply, Request, Status, MAX_ID, MAX_LABEL, MAX_LISTED, MAX_URL,
};

use crate::pairing::clean_name;
use crate::protocol::{Channel, ErrorCode};
use crate::session::{varint_code, write_raw, ActionStream, RecvError, Session, SessionError};
use crate::transport::PeerTrust;

/// Built-in action ids.
pub const LOCK: &str = "lock";
pub const SLEEP: &str = "sleep";
pub const OPEN_URL: &str = "open-url";
/// Every custom action id starts with this, so it never meets a built-in.
pub const CUSTOM_PREFIX: &str = "x-";

/// Wait for the request once a stream is in.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait for an action to run before answering `Failed`.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(10);
/// Wait for the reply, on the requesting side.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(15);
/// Requests waiting their turn on one session; more are `RATE_LIMITED`.
pub const MAX_PENDING: usize = 4;
/// Actions run in [`RUN_WINDOW`] on one session; more are `Busy`.
pub const MAX_RUNS_PER_WINDOW: usize = 10;
pub const RUN_WINDOW: Duration = Duration::from_secs(10);

/// A custom action as this device's user set it up. The peer only ever
/// sends `id`; the program and its arguments never leave this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAction {
    /// `x-` followed by `a-z`, `0-9`, `-` ([`is_custom_id`]).
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Whether `id` can name a custom action: `x-` and at least one more
/// character, an action id as a whole.
pub fn is_custom_id(id: &str) -> bool {
    id.len() > CUSTOM_PREFIX.len() && id.starts_with(CUSTOM_PREFIX) && is_action_id(id)
}

/// This device's allowlist: what a paired device may run here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub lock: bool,
    pub sleep: bool,
    pub open_url: bool,
    pub custom: Vec<CustomAction>,
}

impl Default for Policy {
    /// Built-ins on, no custom actions.
    fn default() -> Self {
        Policy {
            lock: true,
            sleep: true,
            open_url: true,
            custom: Vec::new(),
        }
    }
}

/// An action that passed every check and is about to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Lock,
    Sleep,
    /// Checked with [`check_url`].
    OpenUrl(String),
    Custom(CustomAction),
}

/// Which built-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Lock,
    Sleep,
    OpenUrl,
}

/// Runs actions on this platform. Called on a blocking thread.
pub trait Executor: Send + Sync + 'static {
    /// Whether this device can run the built-in at all (a phone can't
    /// sleep a PC's way); one it can't is never offered nor run.
    fn supports(&self, builtin: Builtin) -> bool;
    /// Runs it. For `Sleep`, returns once sleep is scheduled, so the reply
    /// still leaves.
    fn run(&self, action: &Action) -> Result<(), String>;
}

/// Where the policy comes from; read for every request.
pub trait PolicySource: Send + Sync + 'static {
    fn policy(&self) -> Policy;
}

impl PolicySource for Policy {
    fn policy(&self) -> Policy {
        self.clone()
    }
}

/// What the host did with a request, for the app to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handled {
    /// The peer's name from `Hello`, for display.
    pub peer: String,
    /// The action id, as requested.
    pub action: String,
    pub status: Status,
}

/// Whether `url` may be opened: an absolute `http` or `https` URL of
/// printable ASCII (no spaces, nor any of `"<>\^{|}` or a backtick), at most
/// [`MAX_URL`] bytes, with a host and no user info (`user@`), and a port,
/// if any, of 1–65535. Anything else (other schemes such as `file:`,
/// `javascript:` or app links, relative or odd forms) is refused, not
/// repaired.
pub fn check_url(url: &str) -> bool {
    if url.is_empty()
        || url.len() > MAX_URL
        || !url
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && !b"\"<>\\^`{|}".contains(&b))
    {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let Some((inside, after)) = v6.split_once(']') else {
            return false;
        };
        if inside.is_empty()
            || !inside
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return false;
        }
        match after {
            "" => (inside, None),
            p => match p.strip_prefix(':') {
                Some(p) => (inside, Some(p)),
                None => return false,
            },
        }
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty()
        || host.starts_with('.')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:".contains(&b))
    {
        return false;
    }
    match port {
        None => true,
        Some(p) => {
            !p.is_empty()
                && p.len() <= 5
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
        }
    }
}

/// Labels of the built-ins, as offered in `Actions`.
pub fn builtin_label(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::Lock => "Zablokuj",
        Builtin::Sleep => "Uśpij",
        Builtin::OpenUrl => "Otwórz link",
    }
}

/// What runs here now: built-ins switched on and supported, then custom
/// actions switched on (well-formed ones only), at most [`MAX_LISTED`].
pub fn offered(policy: &Policy, executor: &dyn Executor) -> Vec<Listed> {
    let builtins = [
        (Builtin::Lock, LOCK, policy.lock),
        (Builtin::Sleep, SLEEP, policy.sleep),
        (Builtin::OpenUrl, OPEN_URL, policy.open_url),
    ];
    builtins
        .into_iter()
        .filter(|(b, _, on)| *on && executor.supports(*b))
        .map(|(b, id, _)| Listed {
            id: id.into(),
            label: builtin_label(b).into(),
        })
        .chain(
            policy
                .custom
                .iter()
                .filter(|c| c.enabled && is_custom_id(&c.id))
                .map(|c| Listed {
                    id: c.id.clone(),
                    label: clean_name(&c.label).unwrap_or_else(|| c.id.clone()),
                }),
        )
        .take(MAX_LISTED)
        .collect()
}

/// Resolves a `Run` against the policy: the action to run, or why not.
pub fn resolve(
    policy: &Policy,
    executor: &dyn Executor,
    action: &str,
    url: Option<&str>,
) -> Result<Action, Status> {
    let builtin = |b: Builtin, on: bool| {
        if on && executor.supports(b) {
            Ok(())
        } else {
            Err(Status::Refused)
        }
    };
    match action {
        LOCK | SLEEP => {
            let (b, on) = if action == LOCK {
                (Builtin::Lock, policy.lock)
            } else {
                (Builtin::Sleep, policy.sleep)
            };
            builtin(b, on)?;
            if url.is_some() {
                return Err(Status::BadArgument);
            }
            Ok(if b == Builtin::Lock {
                Action::Lock
            } else {
                Action::Sleep
            })
        }
        OPEN_URL => {
            builtin(Builtin::OpenUrl, policy.open_url)?;
            match url {
                Some(u) if check_url(u) => Ok(Action::OpenUrl(u.to_string())),
                _ => Err(Status::BadArgument),
            }
        }
        id => {
            let custom = policy
                .custom
                .iter()
                .find(|c| c.enabled && is_custom_id(&c.id) && c.id == id)
                .ok_or(Status::Refused)?;
            if url.is_some() {
                return Err(Status::BadArgument);
            }
            Ok(Action::Custom(custom.clone()))
        }
    }
}

/// Runs the requests of one session's `actions` streams.
pub struct ActionHost {
    policy: Arc<dyn PolicySource>,
    executor: Arc<dyn Executor>,
    trust: Arc<dyn PeerTrust>,
    events: Option<mpsc::UnboundedSender<Handled>>,
}

impl ActionHost {
    pub fn new(
        policy: Arc<dyn PolicySource>,
        executor: Arc<dyn Executor>,
        trust: Arc<dyn PeerTrust>,
    ) -> Self {
        ActionHost {
            policy,
            executor,
            trust,
            events: None,
        }
    }

    /// Reports every `Run` handled (not `List`).
    pub fn with_events(mut self, events: mpsc::UnboundedSender<Handled>) -> Self {
        self.events = Some(events);
        self
    }

    /// Serves `streams` (from [`crate::session::Router::route_actions`])
    /// for `session` until the session ends.
    pub async fn serve(self, session: Session, mut streams: mpsc::Receiver<ActionStream>) {
        let host = Arc::new(self);
        let turn = Arc::new(Semaphore::new(1));
        let waiting = Arc::new(Semaphore::new(MAX_PENDING));
        let runs = Arc::new(Mutex::new(VecDeque::<Instant>::new()));
        loop {
            let stream = tokio::select! {
                s = streams.recv() => match s {
                    Some(s) => s,
                    None => return,
                },
                _ = session.closed() => return,
            };
            let Ok(place) = waiting.clone().try_acquire_owned() else {
                refuse(stream, ErrorCode::RateLimited);
                continue;
            };
            let (host, session, turn, runs) =
                (host.clone(), session.clone(), turn.clone(), runs.clone());
            tokio::spawn(async move {
                host.handle(&session, stream, &turn, &runs).await;
                drop(place);
            });
        }
    }

    async fn handle(
        &self,
        session: &Session,
        mut stream: ActionStream,
        turn: &Semaphore,
        runs: &Mutex<VecDeque<Instant>>,
    ) {
        let raw = match tokio::time::timeout(REQUEST_TIMEOUT, stream.recv.next_raw()).await {
            Err(_) => return refuse(stream, ErrorCode::Cancelled),
            Ok(Ok(Some(raw))) => raw,
            Ok(Ok(None)) => return refuse(stream, ErrorCode::MalformedFrame),
            Ok(Err(RecvError::Frame(e))) => return refuse(stream, e.code()),
            Ok(Err(_)) => return refuse(stream, ErrorCode::Cancelled),
        };
        let request = match Request::decode(raw) {
            Ok(Some(r)) => r,
            Ok(None) => return refuse(stream, ErrorCode::UnknownMessage),
            Err(e) => return refuse(stream, e.code()),
        };
        // Nothing more is read from the requester (one request a stream).
        stream.recv.stop(ErrorCode::NoError);
        // The key may have been unpinned since the handshake (§2).
        let peer = session.peer();
        if !self.trust.is_trusted(&peer.key) {
            session
                .connection()
                .close(ErrorCode::UntrustedPeer, b"device is no longer paired");
            return;
        }
        let reply = match request {
            Request::List => Reply::Actions(offered(&self.policy.policy(), &*self.executor)),
            Request::Run { action, url } => {
                let status = self.run(session, &action, url.as_deref(), turn, runs).await;
                if let Some(events) = &self.events {
                    let _ = events.send(Handled {
                        peer: clean_name(&peer.hello.name).unwrap_or_default(),
                        action,
                        status,
                    });
                }
                Reply::Result(status)
            }
        };
        if write_raw(&mut stream.send, Channel::Actions, &reply.encode())
            .await
            .is_ok()
        {
            let _ = stream.send.finish();
        }
    }

    async fn run(
        &self,
        session: &Session,
        action: &str,
        url: Option<&str>,
        turn: &Semaphore,
        runs: &Mutex<VecDeque<Instant>>,
    ) -> Status {
        let Ok(_turn) = turn.acquire().await else {
            return Status::Failed;
        };
        // Withdrawn while this request waited (§6, rule 5): nothing runs.
        if !session
            .accepted_capabilities()
            .iter()
            .any(|c| c.name() == "actions")
        {
            return Status::Refused;
        }
        // Read now, after waiting: what runs is what is allowed when it runs.
        let resolved = match resolve(&self.policy.policy(), &*self.executor, action, url) {
            Ok(a) => a,
            Err(status) => return status,
        };
        {
            let mut runs = runs.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            while runs
                .front()
                .is_some_and(|t| now.duration_since(*t) >= RUN_WINDOW)
            {
                runs.pop_front();
            }
            if runs.len() >= MAX_RUNS_PER_WINDOW {
                return Status::Busy;
            }
            runs.push_back(now);
        }
        let executor = self.executor.clone();
        let task = tokio::task::spawn_blocking(move || executor.run(&resolved));
        match tokio::time::timeout(RUN_TIMEOUT, task).await {
            Ok(Ok(Ok(()))) => Status::Done,
            _ => Status::Failed,
        }
    }
}

fn refuse(mut stream: ActionStream, code: ErrorCode) {
    stream.recv.stop(code);
    let _ = stream.send.reset(varint_code(code));
}

/// Why a request got no reply.
#[derive(Debug)]
pub enum RequestError {
    /// The peer doesn't accept `actions/1` now, or the session failed.
    Session(SessionError),
    /// The peer ended the stream with this code (e.g. `RATE_LIMITED`).
    Refused(ErrorCode),
    /// No reply in [`REPLY_TIMEOUT`], or not a reply to this request.
    NoReply,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session(e) => write!(f, "{e}"),
            Self::Refused(code) => write!(f, "request refused: {code:?}"),
            Self::NoReply => f.write_str("no reply to the action request"),
        }
    }
}

impl std::error::Error for RequestError {}

/// Sends one request to the peer and waits for its reply.
pub async fn request(session: &Session, request: &Request) -> Result<Reply, RequestError> {
    let mut stream = session
        .open_actions()
        .await
        .map_err(RequestError::Session)?;
    write_raw(&mut stream.send, Channel::Actions, &request.encode())
        .await
        .map_err(RequestError::Session)?;
    let _ = stream.send.finish();
    let raw = match tokio::time::timeout(REPLY_TIMEOUT, stream.recv.next_raw()).await {
        Err(_) => return Err(RequestError::NoReply),
        Ok(Ok(Some(raw))) => raw,
        Ok(Ok(None)) => return Err(RequestError::NoReply),
        Ok(Err(RecvError::Read(quinn::ReadError::Reset(code)))) => {
            return Err(RequestError::Refused(ErrorCode::from_code(
                code.into_inner(),
            )))
        }
        Ok(Err(RecvError::Frame(e))) => {
            stream.recv.stop(e.code());
            return Err(RequestError::NoReply);
        }
        Ok(Err(_)) => return Err(RequestError::NoReply),
    };
    let reply = Reply::decode(raw).map_err(|_| RequestError::NoReply)?;
    match (request, reply) {
        (Request::Run { .. }, Some(r @ Reply::Result(_)))
        | (Request::List, Some(r @ Reply::Actions(_))) => Ok(r),
        _ => Err(RequestError::NoReply),
    }
}

/// Runs `action` on the peer; `url` for `open-url` only.
pub async fn run(
    session: &Session,
    action: &str,
    url: Option<&str>,
) -> Result<Status, RequestError> {
    match request(
        session,
        &Request::Run {
            action: action.into(),
            url: url.map(Into::into),
        },
    )
    .await?
    {
        Reply::Result(status) => Ok(status),
        Reply::Actions(_) => Err(RequestError::NoReply),
    }
}

/// The actions the peer would run now.
pub async fn list(session: &Session) -> Result<Vec<Listed>, RequestError> {
    match request(session, &Request::List).await? {
        Reply::Actions(list) => Ok(list),
        Reply::Result(_) => Err(RequestError::NoReply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct All;
    impl Executor for All {
        fn supports(&self, _: Builtin) -> bool {
            true
        }
        fn run(&self, _: &Action) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn urls() {
        for ok in [
            "https://example.com",
            "http://example.com/",
            "HTTPS://Example.COM/a?b=c#d",
            "https://192.168.1.10:8080/x",
            "https://[::1]:443/",
            "https://pl.wikipedia.org/wiki/%C5%81%C3%B3d%C5%BA",
            "https://xn--d-uga.pl/",
        ] {
            assert!(check_url(ok), "{ok}");
        }
        for bad in [
            "",
            "example.com",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "search-ms:query=x",
            "\\\\server\\share",
            "https://",
            "https:///path",
            "https://user@example.com/",
            "https://example.com@evil.com/",
            "https://exa mple.com/",
            "https://example.com/\"onload",
            "https://example.com/\u{202e}",
            "https://łódź.pl/",
            "https://example.com:0/",
            "https://example.com:99999/",
            "https://example.com:/",
            "https://[zz]/",
            "https://.example.com/",
            "https://example.com/\n",
            "http:/\\example.com",
        ] {
            assert!(!check_url(bad), "{bad:?}");
        }
        assert!(!check_url(&format!("https://a.pl/{}", "a".repeat(MAX_URL))));
    }

    #[test]
    fn resolve_follows_the_allowlist() {
        let tool = CustomAction {
            id: "x-backup".into(),
            label: "Kopia".into(),
            enabled: true,
            program: "C:\\Tools\\backup.exe".into(),
            args: vec!["--now".into()],
        };
        let off = CustomAction {
            id: "x-off".into(),
            enabled: false,
            ..tool.clone()
        };
        let mut policy = Policy {
            custom: vec![tool.clone(), off],
            ..Policy::default()
        };
        assert_eq!(resolve(&policy, &All, LOCK, None), Ok(Action::Lock));
        assert_eq!(resolve(&policy, &All, SLEEP, None), Ok(Action::Sleep));
        assert_eq!(
            resolve(&policy, &All, OPEN_URL, Some("https://a.pl/")),
            Ok(Action::OpenUrl("https://a.pl/".into()))
        );
        assert_eq!(
            resolve(&policy, &All, OPEN_URL, Some("file:///c:/x")),
            Err(Status::BadArgument)
        );
        assert_eq!(
            resolve(&policy, &All, OPEN_URL, None),
            Err(Status::BadArgument)
        );
        assert_eq!(
            resolve(&policy, &All, LOCK, Some("https://a.pl/")),
            Err(Status::BadArgument)
        );
        assert_eq!(
            resolve(&policy, &All, "x-backup", None),
            Ok(Action::Custom(tool))
        );
        assert_eq!(resolve(&policy, &All, "x-off", None), Err(Status::Refused));
        assert_eq!(resolve(&policy, &All, "x-nope", None), Err(Status::Refused));
        assert_eq!(
            resolve(&policy, &All, "shutdown", None),
            Err(Status::Refused)
        );
        policy.lock = false;
        assert_eq!(resolve(&policy, &All, LOCK, None), Err(Status::Refused));
        // A custom entry can't shadow a built-in or use a bad id.
        policy.custom.push(CustomAction {
            id: "lock".into(),
            label: "x".into(),
            enabled: true,
            program: "C:\\x.exe".into(),
            args: vec![],
        });
        assert_eq!(resolve(&policy, &All, LOCK, None), Err(Status::Refused));
        assert!(!is_custom_id("x-"));
        assert!(!is_custom_id("backup"));
        assert!(is_custom_id("x-a"));
    }

    #[test]
    fn offered_lists_only_what_runs() {
        struct NoSleep;
        impl Executor for NoSleep {
            fn supports(&self, b: Builtin) -> bool {
                b != Builtin::Sleep
            }
            fn run(&self, _: &Action) -> Result<(), String> {
                Ok(())
            }
        }
        let policy = Policy {
            lock: false,
            custom: vec![CustomAction {
                id: "x-a".into(),
                label: "\u{202e}Kopia\n".into(),
                enabled: true,
                program: "C:\\a.exe".into(),
                args: vec![],
            }],
            ..Policy::default()
        };
        let ids: Vec<_> = offered(&policy, &NoSleep)
            .into_iter()
            .map(|l| (l.id, l.label))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("open-url".to_string(), "Otwórz link".to_string()),
                ("x-a".to_string(), "Kopia".to_string())
            ]
        );
    }
}
