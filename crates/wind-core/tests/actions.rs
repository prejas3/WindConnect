//! Remote actions over real sessions (WN-54, PROTOCOL.md §9.2): only what
//! the receiving device allows runs, only for a pinned key, and a request
//! never carries a command.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wind_core::actions::{
    self, Action, ActionHost, Builtin, CustomAction, Executor, Handled, Listed, Policy,
    PolicySource, Request, RequestError, Status,
};
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Channel, DeviceKind, ErrorCode};
use wind_core::session::{LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(20);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: Some("test".into()),
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Action>>);

impl Executor for Recorder {
    fn supports(&self, _: Builtin) -> bool {
        true
    }
    fn run(&self, action: &Action) -> Result<(), String> {
        self.0.lock().unwrap().push(action.clone());
        Ok(())
    }
}

struct Shared(Mutex<Policy>);

impl PolicySource for Shared {
    fn policy(&self) -> Policy {
        self.0.lock().unwrap().clone()
    }
}

struct Pair {
    /// The requester (a phone).
    phone: Session,
    /// The device that runs actions (a PC).
    pc: Session,
    ran: Arc<Recorder>,
    policy: Arc<Shared>,
    /// The PC's pins: unpinning the phone here is unpairing it.
    pins: Arc<PinnedKeys>,
    phone_key: wind_core::identity::PublicKey,
    handled: tokio::sync::mpsc::UnboundedReceiver<Handled>,
    _endpoints: (Endpoint, Endpoint),
}

fn tool() -> CustomAction {
    CustomAction {
        id: "x-backup".into(),
        label: "Kopia zapasowa".into(),
        enabled: true,
        program: "C:\\Tools\\backup.exe".into(),
        args: vec!["--now".into()],
    }
}

/// A phone and a PC, paired; the PC routes `actions` (if `host`).
async fn pair(host: bool) -> Pair {
    let (iphone, ipc) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pphone, ppc) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pphone.pin(ipc.public_key());
    ppc.pin(iphone.public_key());
    let a = Endpoint::bind(LOCALHOST, &iphone, pphone).unwrap();
    let b = Endpoint::bind(LOCALHOST, &ipc, ppc.clone()).unwrap();
    let addr = b.local_addr().unwrap();
    let accepting = b.clone();
    let accept = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let c = a.connect(addr, ipc.public_key()).await.unwrap();
    let s = accept.await.unwrap();

    let mut router = Router::new();
    let streams = host.then(|| router.route_actions(8));
    let server = tokio::spawn(Session::accept(s, info("PC"), router));
    let (phone, _) = Session::connect(c, info("Pixel"), Router::new())
        .await
        .unwrap();
    let (pc, _) = server.await.unwrap().unwrap();

    let ran = Arc::new(Recorder::default());
    let policy = Arc::new(Shared(Mutex::new(Policy {
        custom: vec![tool()],
        ..Policy::default()
    })));
    let (tx, handled) = tokio::sync::mpsc::unbounded_channel();
    if let Some(streams) = streams {
        let h = ActionHost::new(policy.clone(), ran.clone(), ppc.clone()).with_events(tx);
        tokio::spawn(h.serve(pc.clone(), streams));
    }
    Pair {
        phone,
        pc,
        ran,
        policy,
        pins: ppc,
        phone_key: iphone.public_key(),
        handled,
        _endpoints: (a, b),
    }
}

async fn run(p: &Pair, action: &str, url: Option<&str>) -> Result<Status, RequestError> {
    tokio::time::timeout(T, actions::run(&p.phone, action, url))
        .await
        .expect("reply in time")
}

fn ran(p: &Pair) -> Vec<Action> {
    p.ran.0.lock().unwrap().clone()
}

#[tokio::test]
async fn allowed_actions_run_and_are_listed() {
    let mut p = pair(true).await;
    assert_eq!(run(&p, "lock", None).await.unwrap(), Status::Done);
    assert_eq!(
        run(&p, "open-url", Some("https://example.com/a?b=1"))
            .await
            .unwrap(),
        Status::Done
    );
    assert_eq!(run(&p, "x-backup", None).await.unwrap(), Status::Done);
    assert_eq!(
        ran(&p),
        vec![
            Action::Lock,
            Action::OpenUrl("https://example.com/a?b=1".into()),
            // What runs is the PC's own entry, program and arguments.
            Action::Custom(tool()),
        ]
    );
    let h = p.handled.recv().await.unwrap();
    assert_eq!(
        (h.peer.as_str(), h.action.as_str(), h.status),
        ("Pixel", "lock", Status::Done)
    );

    let list = tokio::time::timeout(T, actions::list(&p.phone))
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<_> = list.iter().map(|l: &Listed| l.id.as_str()).collect();
    assert_eq!(ids, ["lock", "sleep", "open-url", "x-backup"]);
}

#[tokio::test]
async fn nothing_off_the_allowlist_runs() {
    let p = pair(true).await;
    {
        let mut policy = p.policy.0.lock().unwrap();
        policy.lock = false;
        policy.custom[0].enabled = false;
    }
    for (action, url, want) in [
        ("lock", None, Status::Refused),
        ("x-backup", None, Status::Refused),
        ("shutdown", None, Status::Refused),
        ("x-unknown", None, Status::Refused),
        ("sleep", Some("https://a.pl/"), Status::BadArgument),
        ("open-url", None, Status::BadArgument),
        (
            "open-url",
            Some("file:///C:/Windows/System32/calc.exe"),
            Status::BadArgument,
        ),
        ("open-url", Some("ms-settings:privacy"), Status::BadArgument),
        (
            "open-url",
            Some("https://bank.pl@evil.example/"),
            Status::BadArgument,
        ),
    ] {
        assert_eq!(
            run(&p, action, url).await.unwrap(),
            want,
            "{action} {url:?}"
        );
    }
    assert!(ran(&p).is_empty());
    let list = actions::list(&p.phone).await.unwrap();
    assert!(list.iter().all(|l| l.id != "lock" && l.id != "x-backup"));
}

#[tokio::test]
async fn a_command_in_place_of_an_id_is_malformed() {
    let p = pair(true).await;
    for action in ["cmd /c calc", "Lock", "x-a;calc", "..\\x"] {
        match run(&p, action, None).await {
            Err(RequestError::Refused(ErrorCode::MalformedFrame)) => {}
            other => panic!("{action}: {other:?}"),
        }
    }
    assert!(ran(&p).is_empty());
    // The session is fine afterwards.
    assert_eq!(run(&p, "lock", None).await.unwrap(), Status::Done);
}

#[tokio::test]
async fn an_unpaired_device_runs_nothing() {
    let p = pair(true).await;
    // Unpaired while connected: the handshake key is no longer pinned.
    p.pins.unpin(&p.phone_key);
    let r = run(&p, "lock", None).await;
    assert!(r.is_err(), "{r:?}");
    // The PC closed it; the phone sees why.
    let closed = tokio::time::timeout(T, p.phone.closed()).await.unwrap();
    assert!(
        wind_core::transport::is_untrusted_close(&closed),
        "{closed:?}"
    );
    assert!(ran(&p).is_empty());
}

#[tokio::test]
async fn a_device_without_actions_refuses_and_withdrawal_stops() {
    let p = pair(false).await;
    match run(&p, "lock", None).await {
        Err(RequestError::Session(SessionError::CapabilityMissing(Channel::Actions))) => {}
        other => panic!("{other:?}"),
    }

    let p = pair(true).await;
    assert!(p.pc.set_accepted(Channel::Actions, false).await.unwrap());
    tokio::time::timeout(T, async {
        while p
            .phone
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "actions")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    match run(&p, "lock", None).await {
        Err(RequestError::Session(SessionError::CapabilityMissing(Channel::Actions))) => {}
        other => panic!("{other:?}"),
    }
    assert!(ran(&p).is_empty());
    assert!(p.pc.set_accepted(Channel::Actions, true).await.unwrap());
    tokio::time::timeout(T, async {
        while !p
            .phone
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "actions")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run(&p, "lock", None).await.unwrap(), Status::Done);
}

#[tokio::test]
async fn too_many_runs_are_busy() {
    let p = pair(true).await;
    for _ in 0..actions::MAX_RUNS_PER_WINDOW {
        assert_eq!(run(&p, "lock", None).await.unwrap(), Status::Done);
    }
    assert_eq!(run(&p, "lock", None).await.unwrap(), Status::Busy);
    assert_eq!(ran(&p).len(), actions::MAX_RUNS_PER_WINDOW);
    // Refused requests don't use the budget up, but don't run either.
    assert_eq!(run(&p, "x-unknown", None).await.unwrap(), Status::Refused);
}

#[tokio::test]
async fn list_is_a_request_too() {
    let p = pair(true).await;
    let reply = actions::request(&p.phone, &Request::List).await.unwrap();
    assert!(matches!(reply, actions::Reply::Actions(l) if l.len() == 4));
}
