//! SMS read on the PC over real sessions (WN-55, PROTOCOL.md §9.11): threads, a 1000-message thread in batches,
//! incremental `since`, paging with `before`, photos, statuses, watches, and a phone that doesn't offer `sms/1`.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Channel, DeviceKind};
use wind_core::session::{LocalInfo, Router, Session, SessionError};
use wind_core::sms::{self, AskError, Message, MessageBox, Person, Server, Source, Status, Thread};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);
const PNG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Phone,
        app: Some("test".into()),
    }
}

/// Thread 1: 1000 messages dated 1..=1000 ms, ids 1..=1000, every tenth sent; thread 2: one MMS.
struct Store {
    denied: AtomicBool,
}

fn msg(id: u64) -> Message {
    Message {
        id,
        mms: false,
        date_ms: id,
        folder: if id.is_multiple_of(10) {
            MessageBox::Sent
        } else {
            MessageBox::Inbox
        },
        body: format!("Wiadomość numer {id} — ąęłżź"),
        address: None,
        read: true,
        attachments: 0,
    }
}

impl Source for Store {
    fn threads(&self, limit: u32) -> Result<Vec<Thread>, Status> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(Status::Denied);
        }
        let t = vec![
            Thread {
                id: 1,
                date_ms: 1000,
                snippet: "Wiadomość numer 1000".into(),
                people: vec![Person {
                    address: "+48600100200".into(),
                    name: Some("Ola".into()),
                    photo: true,
                }],
                unread: 0,
                count: 1000,
            },
            Thread {
                id: 2,
                date_ms: 5,
                snippet: String::new(),
                people: vec![Person {
                    address: "+48600100300".into(),
                    name: None,
                    photo: false,
                }],
                unread: 1,
                count: 1,
            },
        ];
        Ok(t.into_iter().take(limit as usize).collect())
    }
    fn messages(
        &self,
        thread: u64,
        before: Option<u64>,
        since: Option<u64>,
        limit: u32,
    ) -> Result<Vec<Message>, Status> {
        match thread {
            1 => Ok((1..=1000)
                .rev()
                .filter(|d| before.is_none_or(|b| *d <= b) && since.is_none_or(|s| *d >= s))
                .take(limit as usize)
                .map(msg)
                .collect()),
            2 => Ok(vec![Message {
                mms: true,
                body: String::new(),
                attachments: 1,
                read: false,
                ..msg(5)
            }]),
            _ => Err(Status::NotFound),
        }
    }
    fn photo(&self, address: &str) -> Result<Vec<u8>, Status> {
        match address {
            "+48600100200" => Ok(PNG.to_vec()),
            "+48600100999" => Ok(b"not a png".to_vec()),
            _ => Err(Status::NotFound),
        }
    }
}

struct Pair {
    pc: Session,
    _phone: Session,
    server: Arc<Server>,
    store: Arc<Store>,
    _endpoints: (Endpoint, Endpoint),
}

async fn pair(offers: bool) -> Pair {
    let (iphone, ipc) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pphone, ppc) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pphone.pin(ipc.public_key());
    ppc.pin(iphone.public_key());
    let a = Endpoint::bind(LOCALHOST, &iphone, pphone).unwrap();
    let b = Endpoint::bind(LOCALHOST, &ipc, ppc).unwrap();
    let addr = b.local_addr().unwrap();
    let accepting = b.clone();
    let accept = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let c = a.connect(addr, ipc.public_key()).await.unwrap();
    let s = accept.await.unwrap();
    let server_task = tokio::spawn(Session::accept(s, info("PC"), Router::new()));
    let mut router = Router::new();
    let streams = router.route_sms(8);
    if !offers {
        router.start_disabled(Channel::Sms);
    }
    let (phone, _) = Session::connect(c, info("Pixel"), router).await.unwrap();
    let (pc, _) = server_task.await.unwrap().unwrap();
    let store = Arc::new(Store {
        denied: AtomicBool::new(false),
    });
    let server = Server::new(store.clone());
    tokio::spawn(server.clone().serve(streams));
    Pair {
        pc,
        _phone: phone,
        server,
        store,
        _endpoints: (a, b),
    }
}

#[tokio::test]
async fn threads_and_a_thousand_messages() {
    let p = pair(true).await;
    let threads = sms::threads(&p.pc, 200).await.unwrap();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].people[0].name.as_deref(), Some("Ola"));
    let start = Instant::now();
    let all = sms::messages(&p.pc, 1, None, None, 1000).await.unwrap();
    let took = start.elapsed();
    assert_eq!(all.len(), 1000);
    assert_eq!(all[0].id, 1000, "newest first");
    assert_eq!(all[999].body, "Wiadomość numer 1 — ąęłżź");
    assert!(all[0].folder.outgoing());
    // The wire's share of the < 300 ms budget, on loopback in a debug build.
    assert!(took < Duration::from_millis(300), "took {took:?}");
}

#[tokio::test]
async fn since_and_before_page_and_sync() {
    let p = pair(true).await;
    let newest = sms::messages(&p.pc, 1, None, None, 100).await.unwrap();
    assert_eq!(newest.last().unwrap().date_ms, 901);
    // Older page: `before` is inclusive, the PC dedups the boundary.
    let older = sms::messages(&p.pc, 1, Some(901), None, 100).await.unwrap();
    assert_eq!(older[0].date_ms, 901);
    assert_eq!(older.last().unwrap().date_ms, 802);
    // Incremental: only what is at least as new as the newest known.
    let fresh = sms::messages(&p.pc, 1, None, Some(1000), 2000)
        .await
        .unwrap();
    assert_eq!(fresh.iter().map(|m| m.id).collect::<Vec<_>>(), [1000]);
    let mms = sms::messages(&p.pc, 2, None, None, 10).await.unwrap();
    assert!(mms[0].mms && mms[0].attachments == 1 && mms[0].body.is_empty());
}

#[tokio::test]
async fn statuses_and_photos() {
    let p = pair(true).await;
    assert!(matches!(
        sms::messages(&p.pc, 99, None, None, 10).await,
        Err(AskError::Status(Status::NotFound))
    ));
    assert_eq!(
        sms::photo(&p.pc, "+48600100200").await.unwrap(),
        Some(PNG.to_vec())
    );
    assert_eq!(sms::photo(&p.pc, "+48600100300").await.unwrap(), None);
    // The phone's source gave no PNG: not passed on.
    assert_eq!(sms::photo(&p.pc, "+48600100999").await.unwrap(), None);
    p.store.denied.store(true, Ordering::SeqCst);
    assert!(matches!(
        sms::threads(&p.pc, 10).await,
        Err(AskError::Status(Status::Denied))
    ));
}

#[tokio::test]
async fn watch_reports_changes_and_ends_with_the_capability() {
    let p = pair(true).await;
    let mut changes = sms::watch(&p.pc).await.unwrap();
    // The watch is registered once its stream reaches the phone.
    tokio::time::sleep(Duration::from_millis(100)).await;
    p.server.changed();
    p.server.changed();
    tokio::time::timeout(T, changes.recv())
        .await
        .unwrap()
        .expect("a change");
    // Debounced: the two changes above are one.
    assert!(
        tokio::time::timeout(Duration::from_millis(600), changes.recv())
            .await
            .is_err()
    );
    p._phone.set_accepted(Channel::Sms, false).await.unwrap();
    assert_eq!(tokio::time::timeout(T, changes.recv()).await.unwrap(), None);
}

#[tokio::test]
async fn a_third_watch_is_busy() {
    let p = pair(true).await;
    let _a = sms::watch(&p.pc).await.unwrap();
    let _b = sms::watch(&p.pc).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut c = sms::watch(&p.pc).await.unwrap();
    assert_eq!(tokio::time::timeout(T, c.recv()).await.unwrap(), None);
}

#[tokio::test]
async fn nothing_without_the_capability() {
    let p = pair(false).await;
    assert!(matches!(
        sms::threads(&p.pc, 10).await,
        Err(AskError::Session(SessionError::CapabilityMissing(
            Channel::Sms
        )))
    ));
}
