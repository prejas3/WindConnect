//! The phone's photos in the PC's panel over real sessions (WN-58, PROTOCOL.md §9.13): recent items, thumbnails,
//! an original in several chunks, a file that shrinks while read, statuses, a watch and its latency, a phone
//! that doesn't offer `gallery/1`, and a photo outside the newest ones that is never served (WN-62).

use std::io::Cursor;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wind_core::gallery::{self, AskError, FileInfo, Item, Opened, Server, Source, Status};
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Channel, DeviceKind};
use wind_core::session::{LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);
const JPEG: [u8; 6] = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
/// Photo 3's bytes: over two chunks.
const BIG: usize = 600_000;

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Phone,
        app: Some("test".into()),
    }
}

fn item(id: u64) -> Item {
    Item {
        id,
        date_ms: 1000 + id,
        name: format!("PXL_{id}.jpg"),
        mime: "image/jpeg".into(),
        width: 4000,
        height: 3000,
        size: BIG as u64,
        screenshot: id == 2,
    }
}

fn big() -> Vec<u8> {
    (0..BIG).map(|i| (i % 251) as u8).collect()
}

/// Photos 3, 2, 1, 4, 5, 9 (newest first); 3 has an original, 4 claims more bytes than it has, 5 has a name Windows
/// refuses, 9's thumbnail isn't a JPEG. Photo 7 has a thumbnail and an original but is listed only once `seven` is set
/// (an older photo the PC must not reach by its id).
struct Photos {
    denied: AtomicBool,
    seven: AtomicBool,
}

impl Source for Photos {
    fn recent(&self, limit: u32) -> Result<Vec<Item>, Status> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(Status::Denied);
        }
        let seven = self.seven.load(Ordering::SeqCst).then_some(7);
        Ok(seven
            .into_iter()
            .chain([3, 2, 1, 4, 5, 9])
            .map(item)
            .take(limit as usize)
            .collect())
    }
    fn thumb(&self, id: u64, size: u32) -> Result<Vec<u8>, Status> {
        match id {
            1..=3 | 7 if size <= 512 => Ok(JPEG.to_vec()),
            9 => Ok(b"\x89PNG not a jpeg".to_vec()),
            _ => Err(Status::NotFound),
        }
    }
    fn original(&self, id: u64) -> Result<Opened, Status> {
        let (name, size, bytes) = match id {
            3 => ("PXL_3.jpg", BIG as u64, big()),
            7 => ("PXL_7.jpg", 3, vec![1, 2, 3]),
            4 => ("PXL_4.jpg", 10, vec![1, 2, 3]),
            5 => ("CON.jpg", 3, vec![1, 2, 3]),
            _ => return Err(Status::NotFound),
        };
        Ok(Opened {
            info: FileInfo {
                name: name.into(),
                mime: "image/jpeg".into(),
                size,
            },
            bytes: Box::new(Cursor::new(bytes)),
        })
    }
}

struct Pair {
    pc: Session,
    phone: Session,
    server: Arc<Server>,
    photos: Arc<Photos>,
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
    let streams = router.route_gallery(8);
    if !offers {
        router.start_disabled(Channel::Gallery);
    }
    let (phone, _) = Session::connect(c, info("Pixel"), router).await.unwrap();
    let (pc, _) = server_task.await.unwrap().unwrap();
    let photos = Arc::new(Photos {
        denied: AtomicBool::new(false),
        seven: AtomicBool::new(false),
    });
    let server = Server::new(photos.clone());
    tokio::spawn(server.clone().serve(streams));
    Pair {
        pc,
        phone,
        server,
        photos,
        _endpoints: (a, b),
    }
}

async fn fetch(pc: &Session, id: u64) -> Result<(FileInfo, Vec<u8>), AskError> {
    let mut d = gallery::original(pc, id).await?;
    let mut out = vec![];
    while let Some(b) = d.next().await? {
        out.extend(b);
    }
    Ok((d.info.clone(), out))
}

#[tokio::test]
async fn recent_thumbs_and_originals() {
    let p = pair(true).await;
    let items = tokio::time::timeout(T, gallery::recent(&p.pc, 20))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        items.iter().map(|i| i.id).collect::<Vec<_>>(),
        [3, 2, 1, 4, 5, 9]
    );
    assert!(items[1].screenshot);
    assert_eq!(gallery::recent(&p.pc, 2).await.unwrap().len(), 2);
    assert_eq!(gallery::thumb(&p.pc, 3, 256).await.unwrap(), JPEG);
    assert!(matches!(
        gallery::thumb(&p.pc, 8, 256).await,
        Err(AskError::Status(Status::NotFound))
    ));
    // Not a JPEG: the phone's server refuses to pass it on.
    assert!(matches!(
        gallery::thumb(&p.pc, 9, 256).await,
        Err(AskError::Status(Status::Failed))
    ));

    let (info, bytes) = tokio::time::timeout(T, fetch(&p.pc, 3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.name, "PXL_3.jpg");
    assert_eq!(info.size, BIG as u64);
    assert!(bytes == big(), "the original arrives whole and in order");
    // A name Windows can't create is replaced on the phone.
    assert_eq!(fetch(&p.pc, 5).await.unwrap().0.name, "IMG_5.jpg");
    assert!(matches!(
        fetch(&p.pc, 6).await,
        Err(AskError::Status(Status::NotFound))
    ));
    // Fewer bytes than announced: the PC gets an error, never a short file as done.
    assert!(fetch(&p.pc, 4).await.is_err());

    p.photos.denied.store(true, Ordering::SeqCst);
    assert!(matches!(
        gallery::recent(&p.pc, 20).await,
        Err(AskError::Status(Status::Denied))
    ));
    // Requests keep working after the failures.
    p.photos.denied.store(false, Ordering::SeqCst);
    assert_eq!(gallery::recent(&p.pc, 20).await.unwrap().len(), 6);
}

#[tokio::test]
async fn only_the_newest_photos_are_served() {
    let p = pair(true).await;
    // Photo 7 exists on the phone but isn't among the newest: answered as if it didn't exist.
    assert!(matches!(
        gallery::thumb(&p.pc, 7, 256).await,
        Err(AskError::Status(Status::NotFound))
    ));
    assert!(matches!(
        fetch(&p.pc, 7).await,
        Err(AskError::Status(Status::NotFound))
    ));
    // It becomes one of the newest: a change on the phone drops the remembered list at once.
    p.photos.seven.store(true, Ordering::SeqCst);
    p.server.changed();
    assert_eq!(gallery::thumb(&p.pc, 7, 256).await.unwrap(), JPEG);
    assert_eq!(fetch(&p.pc, 7).await.unwrap().1, [1, 2, 3]);
    // Without photo access, the check itself answers `Denied`.
    p.photos.denied.store(true, Ordering::SeqCst);
    p.server.changed();
    assert!(matches!(
        gallery::thumb(&p.pc, 3, 256).await,
        Err(AskError::Status(Status::Denied))
    ));
}

#[tokio::test]
async fn watch_reports_a_new_photo_well_within_a_second() {
    let p = pair(true).await;
    let mut changes = gallery::watch(&p.pc).await.unwrap();
    let mut second = gallery::watch(&p.pc).await.unwrap();
    // A third watch is refused: its stream ends at once.
    let mut third = gallery::watch(&p.pc).await.unwrap();
    assert_eq!(tokio::time::timeout(T, third.recv()).await.unwrap(), None);
    // Let the watches reach the server before the change.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let start = Instant::now();
    // Three quick changes (a camera's steps) make one `Changed`.
    p.server.changed();
    p.server.changed();
    p.server.changed();
    tokio::time::timeout(T, changes.recv())
        .await
        .unwrap()
        .unwrap();
    // From the observer to the PC's fresh list, the panel's whole path but the pixels.
    let items = gallery::recent(&p.pc, 20).await.unwrap();
    let thumb = gallery::thumb(&p.pc, items[0].id, 256).await.unwrap();
    assert_eq!(thumb, JPEG);
    let took = start.elapsed();
    assert!(took < Duration::from_millis(700), "took {took:?}");
    second.recv().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(400), changes.recv())
            .await
            .is_err(),
        "debounced into one"
    );
}

#[tokio::test]
async fn withdrawn_or_not_offered() {
    let p = pair(false).await;
    assert!(matches!(
        gallery::recent(&p.pc, 20).await,
        Err(AskError::Session(SessionError::CapabilityMissing(
            Channel::Gallery
        )))
    ));
    // Turned on, the PC can ask; turned off again, its watch ends.
    p.phone.set_accepted(Channel::Gallery, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(gallery::recent(&p.pc, 1).await.unwrap().len(), 1);
    let mut changes = gallery::watch(&p.pc).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    p.phone.set_accepted(Channel::Gallery, false).await.unwrap();
    assert_eq!(tokio::time::timeout(T, changes.recv()).await.unwrap(), None);
}
