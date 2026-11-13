//! The file transfer engine over real sessions (WC-38, PROTOCOL.md §9.1).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::watch;
use wind_core::files::{
    wire::{Offer, ToReceiver},
    Direction, FailReason, FileEngine, FilesConfig, TransferInfo, TransferState,
};
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Channel, DeviceKind, ErrorCode};
use wind_core::session::{write_raw, LocalInfo, RecvError, Router, Session};
use wind_core::transport::{Connection, Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(60);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: Some("test".into()),
    }
}

/// Two paired endpoints that can connect again and again.
struct Net {
    a: Endpoint,
    b: Endpoint,
    b_key: wind_core::identity::PublicKey,
    a_fp: [u8; 32],
    b_fp: [u8; 32],
}

impl Net {
    fn new() -> Self {
        let (ia, ib) = (
            DeviceIdentity::generate().unwrap(),
            DeviceIdentity::generate().unwrap(),
        );
        let (pa, pb) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
        pa.pin(ib.public_key());
        pb.pin(ia.public_key());
        Net {
            a: Endpoint::bind(LOCALHOST, &ia, pa).unwrap(),
            b: Endpoint::bind(LOCALHOST, &ib, pb).unwrap(),
            b_key: ib.public_key(),
            a_fp: *ia.fingerprint().as_bytes(),
            b_fp: *ib.fingerprint().as_bytes(),
        }
    }

    async fn connections(&self) -> (Connection, Connection) {
        let addr = self.b.local_addr().unwrap();
        let b = self.b.clone();
        let accept = tokio::spawn(async move { b.accept().await.unwrap().unwrap() });
        let client = self.a.connect(addr, self.b_key).await.unwrap();
        (client, accept.await.unwrap())
    }

    /// Sessions a → b, each engine attached (b receives).
    async fn attach(&self, ea: &FileEngine, eb: &FileEngine) -> (Session, Session) {
        let (c, s) = self.connections().await;
        let mut rb = Router::new();
        let incoming_b = rb.route_files(1, 16);
        let mut ra = Router::new();
        let incoming_a = ra.route_files(1, 16);
        let server = tokio::spawn(Session::accept(s, info("b"), rb));
        let (sa, _ea) = Session::connect(c, info("a"), ra).await.unwrap();
        let (sb, _eb) = server.await.unwrap().unwrap();
        ea.attach(sa.clone(), Some(incoming_a));
        eb.attach(sb.clone(), Some(incoming_b));
        (sa, sb)
    }
}

/// Partial files of the engine whose inbox is `inbox`: a sibling folder.
fn partials_of(inbox: &Path) -> PathBuf {
    PathBuf::from(format!("{}.parts", inbox.display()))
}

fn engine(inbox: &Path) -> FileEngine {
    FileEngine::new(FilesConfig::new(inbox, partials_of(inbox))).unwrap()
}

fn random_file(dir: &Path, name: &str, len: usize, seed: u8) -> PathBuf {
    let path = dir.join(name);
    let mut data = vec![0u8; len];
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ u64::from(seed);
    for chunk in data.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let b = x.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    std::fs::write(&path, &data).unwrap();
    path
}

fn sha(path: &Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(path).unwrap()).into()
}

/// Waits until `f` holds for the transfer list.
async fn until(
    rx: &mut watch::Receiver<Vec<TransferInfo>>,
    what: &str,
    f: impl Fn(&[TransferInfo]) -> bool,
) -> Vec<TransferInfo> {
    tokio::time::timeout(T, async {
        loop {
            {
                let now = rx.borrow_and_update();
                if f(&now) {
                    return now.clone();
                }
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}: {:#?}", rx.borrow()))
}

fn all_done(list: &[TransferInfo], n: usize) -> bool {
    list.len() == n && list.iter().all(|t| t.state == TransferState::Done)
}

fn no_partials(inbox: &Path) -> bool {
    walkdir(&partials_of(inbox))
        .iter()
        .all(|p| !p.extension().is_some_and(|e| e == "part"))
}

#[tokio::test(flavor = "multi_thread")]
async fn received_batch_uses_type_folders_without_overwriting_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let sources = temp.path().join("sources");
    std::fs::create_dir_all(&sources).unwrap();
    let inbox = temp.path().join("Downloads/WindConnect");
    let sender = engine(&temp.path().join("sender"));
    let mut config = FilesConfig::new(&inbox, partials_of(&inbox));
    config.type_folders = true;
    let receiver = FileEngine::new(config).unwrap();
    std::fs::create_dir_all(inbox.join("Obrazy")).unwrap();
    let existing = inbox.join("Obrazy/photo0.JPG");
    std::fs::write(&existing, b"keep me").unwrap();
    let net = Net::new();
    let (_a, _b) = net.attach(&sender, &receiver).await;
    let mut expected = std::collections::HashMap::new();
    for i in 0..20 {
        let name = format!("photo{i}.JPG");
        let file = random_file(&sources, &name, 80_000 + i, i as u8);
        expected.insert(name, ("Obrazy", sha(&file)));
        sender.send(net.b_fp, &file).unwrap();
    }
    for (i, name, folder) in [
        (21, "video.mp4", "Wideo"),
        (22, "paper.pdf", "Dokumenty"),
        (23, "photo.jpg.exe", "Inne"),
        (24, "noextension", "Inne"),
    ] {
        let file = random_file(&sources, name, 20_000, i);
        expected.insert(name.into(), (folder, sha(&file)));
        sender.send(net.b_fp, &file).unwrap();
    }
    let mut updates = receiver.subscribe();
    let received = until(&mut updates, "all categorized files", |rows| {
        all_done(rows, 24)
    })
    .await;
    for row in received {
        let (folder, hash) = expected.get(&row.path).unwrap();
        let saved = row.saved_as.unwrap();
        assert_eq!(
            saved.parent().unwrap().canonicalize().unwrap(),
            inbox.join(folder).canonicalize().unwrap()
        );
        assert_eq!(&sha(&saved), hash);
        #[cfg(windows)]
        assert!(
            std::fs::read_to_string(format!("{}:Zone.Identifier", saved.display()))
                .unwrap()
                .contains("ZoneId=3")
        );
    }
    assert_eq!(std::fs::read(existing).unwrap(), b"keep me");
    assert!(inbox.join("Obrazy/photo0 (1).JPG").is_file());
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                out.extend(walkdir(&e.path()));
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_arrives_whole_over_parallel_streams() {
    let dir = tempfile::tempdir().unwrap();
    let (src, inbox_a, inbox_b) = (
        dir.path().join("src"),
        dir.path().join("a"),
        dir.path().join("b"),
    );
    std::fs::create_dir_all(&src).unwrap();
    // Not a multiple of the chunk size.
    let file = random_file(&src, "big.bin", 24 * 1024 * 1024 + 12_345, 1);
    let (ea, eb) = (engine(&inbox_a), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let mut rb = eb.subscribe();
    let mut ra = ea.subscribe();
    ea.send(net.b_fp, &file).unwrap();
    let got = until(&mut rb, "incoming done", |l| all_done(l, 1)).await;
    until(&mut ra, "outgoing done", |l| all_done(l, 1)).await;
    let t = &got[0];
    assert_eq!(t.direction, Direction::Incoming);
    assert_eq!(t.peer, net.a_fp);
    assert_eq!(t.done, t.size);
    let saved = t.saved_as.clone().unwrap();
    assert_eq!(saved, inbox_b.canonicalize().unwrap().join("big.bin"));
    assert_eq!(sha(&saved), sha(&file));
    assert!(no_partials(&inbox_b));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queue_of_files_and_a_folder_all_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("Album/2026")).unwrap();
    let files = [
        random_file(&src, "one.txt", 1, 1),
        random_file(&src, "empty.bin", 0, 2),
        random_file(&src.join("Album"), "a.jpg", 700_000, 3),
        random_file(&src.join("Album/2026"), "b.jpg", 300_000, 4),
    ];
    let (inbox_a, inbox_b) = (dir.path().join("a"), dir.path().join("b"));
    let (ea, eb) = (engine(&inbox_a), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let mut rb = eb.subscribe();
    ea.send(net.b_fp, &files[0]).unwrap();
    ea.send(net.b_fp, &files[1]).unwrap();
    let ids = ea.send(net.b_fp, &src.join("Album")).unwrap();
    assert_eq!(ids.len(), 2);
    let got = until(&mut rb, "four files", |l| all_done(l, 4)).await;
    let root = inbox_b.canonicalize().unwrap();
    for (source, rel) in [
        (&files[0], "one.txt"),
        (&files[1], "empty.bin"),
        (&files[2], "Album/a.jpg"),
        (&files[3], "Album/2026/b.jpg"),
    ] {
        let t = got.iter().find(|t| t.path == rel).expect(rel);
        let saved = t.saved_as.clone().unwrap();
        assert_eq!(saved, root.join(rel));
        assert_eq!(sha(&saved), sha(source));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_files_with_one_name_never_replace_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let (s1, s2) = (dir.path().join("s1"), dir.path().join("s2"));
    std::fs::create_dir_all(&s1).unwrap();
    std::fs::create_dir_all(&s2).unwrap();
    let f1 = random_file(&s1, "same.bin", 2_000_000, 1);
    let f2 = random_file(&s2, "same.bin", 2_000_000, 2);
    let inbox_b = dir.path().join("b");
    std::fs::create_dir_all(&inbox_b).unwrap();
    // Already there: must survive.
    std::fs::write(inbox_b.join("same.bin"), b"keep me").unwrap();
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let mut rb = eb.subscribe();
    ea.send(net.b_fp, &f1).unwrap();
    ea.send(net.b_fp, &f2).unwrap();
    let got = until(&mut rb, "both", |l| all_done(l, 2)).await;
    assert_eq!(std::fs::read(inbox_b.join("same.bin")).unwrap(), b"keep me");
    let mut saved: Vec<_> = got.iter().map(|t| t.saved_as.clone().unwrap()).collect();
    saved.sort();
    saved.dedup();
    assert_eq!(saved.len(), 2);
    let mut hashes: Vec<_> = saved.iter().map(|p| sha(p)).collect();
    hashes.sort();
    let mut want = vec![sha(&f1), sha(&f2)];
    want.sort();
    assert_eq!(hashes, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_connection_resumes_by_itself_without_starting_over() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let size = 32 * 1024 * 1024;
    let file = random_file(&src, "resume.bin", size, 7);
    let (inbox_a, inbox_b) = (dir.path().join("a"), dir.path().join("b"));
    let (ea, eb) = (engine(&inbox_a), engine(&inbox_b));
    let net = Net::new();
    let (sa, _sb) = net.attach(&ea, &eb).await;
    let (mut ra, mut rb) = (ea.subscribe(), eb.subscribe());
    ea.send(net.b_fp, &file).unwrap();
    let half = size as u64 / 2;
    until(&mut rb, "half", |l| {
        l.first().is_some_and(|t| t.done >= half)
    })
    .await;
    // The connection breaks in the middle.
    sa.connection().close(ErrorCode::InternalError, b"cut");
    until(&mut ra, "sender waits", |l| {
        l[0].state == TransferState::Waiting
    })
    .await;
    let waiting = until(&mut rb, "receiver waits", |l| {
        l[0].state == TransferState::Waiting
    })
    .await;
    assert!(waiting[0].done >= half && waiting[0].done < size as u64);
    // Reconnect: nothing else to do; the transfer goes on by itself.
    let _again = net.attach(&ea, &eb).await;
    let resumed = until(&mut rb, "receiver resumes", |l| {
        l[0].state == TransferState::Running
    })
    .await;
    // The resumed transfer starts from what was on disk (all of it: the
    // receiver flushed when the connection went), not from zero, so less
    // than 60% of the file travels again.
    assert_eq!(resumed[0].done, waiting[0].done);
    assert!(resumed[0].done * 10 >= size as u64 * 4);
    let got = until(&mut rb, "done", |l| all_done(l, 1)).await;
    until(&mut ra, "sender done", |l| all_done(l, 1)).await;
    assert_eq!(sha(got[0].saved_as.as_ref().unwrap()), sha(&file));
    assert!(no_partials(&inbox_b));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sender_cancels_and_the_receiver_deletes_the_part() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = random_file(&src, "c.bin", 48 * 1024 * 1024, 9);
    let inbox_b = dir.path().join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let (mut ra, mut rb) = (ea.subscribe(), eb.subscribe());
    let id = ea.send(net.b_fp, &file).unwrap()[0];
    until(&mut rb, "started", |l| {
        l.first().is_some_and(|t| t.done > 0)
    })
    .await;
    ea.cancel(&id).unwrap();
    until(&mut ra, "cancelled here", |l| {
        l[0].state == TransferState::Cancelled { by_peer: false }
    })
    .await;
    until(&mut rb, "cancelled there", |l| {
        l[0].state == TransferState::Cancelled { by_peer: true }
    })
    .await;
    assert!(no_partials(&inbox_b));
    assert!(!inbox_b.join("c.bin").exists());
    // Queued again, it goes through.
    ea.retry(&id).unwrap();
    until(&mut ra, "retried", |l| all_done(l, 1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_changed_while_sending_ends_the_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = random_file(&src, "moving.bin", 48 * 1024 * 1024, 11);
    let inbox_b = dir.path().join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let (mut ra, mut rb) = (ea.subscribe(), eb.subscribe());
    ea.send(net.b_fp, &file).unwrap();
    until(&mut rb, "started", |l| {
        l.first().is_some_and(|t| t.done > 0)
    })
    .await;
    // Same size, another modification time.
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    until(&mut ra, "source changed", |l| {
        l[0].state == TransferState::Failed(FailReason::SourceChanged)
    })
    .await;
    until(&mut rb, "receiver drops it", |l| {
        l[0].state == TransferState::Cancelled { by_peer: true }
    })
    .await;
    assert!(no_partials(&inbox_b));
    assert!(!inbox_b.join("moving.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_cancels_and_the_sender_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = random_file(&src, "r.bin", 48 * 1024 * 1024, 10);
    let inbox_b = dir.path().join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let (mut ra, mut rb) = (ea.subscribe(), eb.subscribe());
    ea.send(net.b_fp, &file).unwrap();
    let started = until(&mut rb, "started", |l| {
        l.first().is_some_and(|t| t.done > 0)
    })
    .await;
    eb.cancel(&started[0].id).unwrap();
    until(&mut ra, "sender sees it", |l| {
        l[0].state == TransferState::Cancelled { by_peer: true }
    })
    .await;
    until(&mut rb, "cancelled here", |l| {
        l[0].state == TransferState::Cancelled { by_peer: false }
    })
    .await;
    assert!(no_partials(&inbox_b));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_room_is_refused_with_no_space() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = random_file(&src, "n.bin", 1000, 1);
    let inbox_b = dir.path().join("b");
    let mut cfg = FilesConfig::new(&inbox_b, partials_of(&inbox_b));
    cfg.space_margin = u64::MAX / 2;
    let (ea, eb) = (engine(&dir.path().join("a")), FileEngine::new(cfg).unwrap());
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let mut ra = ea.subscribe();
    ea.send(net.b_fp, &file).unwrap();
    until(&mut ra, "no space", |l| {
        l[0].state == TransferState::Failed(FailReason::NoSpace)
    })
    .await;
    assert!(no_partials(&dir.path().join("b")));
}

/// Opens a raw `files` stream from `sa` and sends `first`; returns how the
/// receiver ended it.
async fn raw_offer(sa: &Session, first: ToReceiver) -> u64 {
    let mut s = sa.open_files().await.unwrap();
    write_raw(&mut s.send, Channel::Files, &first.encode())
        .await
        .unwrap();
    let r = tokio::time::timeout(T, s.recv.next_raw()).await.unwrap();
    match r {
        Err(RecvError::Read(quinn::ReadError::Reset(code))) => code.into_inner(),
        other => panic!("expected a reset, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_paths_are_refused_and_nothing_lands_outside_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_b = dir.path().join("in").join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let (sa, _sb) = net.attach(&ea, &eb).await;
    for (i, path) in [
        "../x",
        "a/../../x",
        "C:/x",
        "/x",
        "a\\..\\x",
        "x:ads",
        "nul",
        "con.txt",
        "a.",
        "a ",
        "evil\u{202E}gpj.exe",
    ]
    .into_iter()
    .enumerate()
    {
        let offer = ToReceiver::Offer(Offer {
            id: [i as u8 + 1; 16],
            path: path.into(),
            size: 10,
            chunk: 256 * 1024,
            streams: 1,
            modified: None,
        });
        assert_eq!(
            raw_offer(&sa, offer).await,
            ErrorCode::MalformedFrame.code(),
            "{path:?}"
        );
    }
    let everything = walkdir(dir.path());
    assert!(
        everything.iter().all(|p| [
            inbox_b.clone(),
            partials_of(&inbox_b),
            dir.path().join("a"),
            partials_of(&dir.path().join("a")),
        ]
        .iter()
        .any(|ok| p.starts_with(ok))),
        "{everything:?}"
    );
    assert!(eb.transfers().is_empty());
}

#[tokio::test]
async fn partials_inside_the_inbox_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("in");
    for partials in [
        inbox.join(".wcpart"),
        inbox.clone(),
        dir.path().to_path_buf(),
    ] {
        let err = FileEngine::new(FilesConfig::new(&inbox, &partials))
            .err()
            .unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{partials:?}");
    }
}

/// A peer's path can name anything inside the inbox, so partial files are
/// kept outside it: paths shaped like the partials store land as ordinary
/// files and leave another device's partials alone.
#[tokio::test(flavor = "multi_thread")]
async fn peer_paths_never_reach_the_partials_store() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let inbox_b = dir.path().join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    // Another device's partial file and state, already on B.
    let other = "ab".repeat(32);
    let stem = "00112233445566778899aabbccddeeff";
    let store = partials_of(&inbox_b).join(&other);
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join(format!("{stem}.part")), b"theirs").unwrap();
    std::fs::write(store.join(format!("{stem}.state")), b"theirs").unwrap();
    let before = walkdir(&partials_of(&inbox_b));
    let folder = src.join(".wcpart").join(&other);
    std::fs::create_dir_all(&folder).unwrap();
    random_file(&folder, &format!("{stem}.state"), 100, 1);
    random_file(&folder, &format!("{stem}.part"), 100, 2);
    let net = Net::new();
    let _s = net.attach(&ea, &eb).await;
    let mut rb = eb.subscribe();
    ea.send(net.b_fp, &src.join(".wcpart")).unwrap();
    let got = until(&mut rb, "both", |l| all_done(l, 2)).await;
    for t in &got {
        assert!(t
            .saved_as
            .as_ref()
            .unwrap()
            .starts_with(inbox_b.canonicalize().unwrap()));
    }
    assert_eq!(walkdir(&partials_of(&inbox_b)), before);
    assert_eq!(
        std::fs::read(store.join(format!("{stem}.state"))).unwrap(),
        b"theirs"
    );
    assert_eq!(
        std::fs::read(store.join(format!("{stem}.part"))).unwrap(),
        b"theirs"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_join_without_its_offer_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&dir.path().join("b")));
    let net = Net::new();
    let (sa, _sb) = net.attach(&ea, &eb).await;
    let started = Instant::now();
    let code = raw_offer(
        &sa,
        ToReceiver::Join {
            id: [5; 16],
            stream: 1,
        },
    )
    .await;
    assert_eq!(code, ErrorCode::Cancelled.code());
    assert!(started.elapsed() >= wind_core::files::JOIN_TIMEOUT - Duration::from_millis(100));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_without_files_refuses_the_send() {
    let dir = tempfile::tempdir().unwrap();
    let file = random_file(dir.path(), "x.bin", 10, 1);
    let ea = engine(&dir.path().join("a"));
    let net = Net::new();
    let (c, s) = net.connections().await;
    let server = tokio::spawn(Session::accept(s, info("b"), Router::new()));
    let (sa, _e) = Session::connect(c, info("a"), Router::new()).await.unwrap();
    let _sb = server.await.unwrap().unwrap();
    ea.attach(sa, None);
    let mut ra = ea.subscribe();
    ea.send(net.b_fp, &file).unwrap();
    until(&mut ra, "refused", |l| {
        l[0].state == TransferState::Failed(FailReason::Refused(ErrorCode::CapabilityMissing))
    })
    .await;
}

/// Throughput against raw QUIC on the same loopback (the stand-in for
/// "1 GB over Wi-Fi 6 at ≥ 80% of the link", which needs two devices).
/// Run with `cargo test --release -p wind-core --test files -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "1 GiB on disk; run in release"]
async fn throughput_is_at_least_80_percent_of_raw_quic() {
    const SIZE: usize = 1 << 30;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = random_file(&src, "gig.bin", SIZE, 42);
    let net = Net::new();

    // Raw QUIC: one bidi stream, 1 MiB writes, the reader discards.
    let raw = {
        let (c, s) = net.connections().await;
        let reader = tokio::spawn(async move {
            let (_send, mut recv) = s.quinn().accept_bi().await.unwrap();
            let mut buf = vec![0u8; 1 << 20];
            let mut n = 0usize;
            while let Some(k) = recv.read(&mut buf).await.unwrap() {
                n += k;
            }
            n
        });
        let (mut send, _recv) = c.quinn().open_bi().await.unwrap();
        let block = vec![7u8; 1 << 20];
        let t = Instant::now();
        for _ in 0..(SIZE >> 20) {
            send.write_all(&block).await.unwrap();
        }
        send.finish().unwrap();
        assert_eq!(reader.await.unwrap(), SIZE);
        t.elapsed()
    };

    let (ea, eb) = (engine(&dir.path().join("a")), engine(&dir.path().join("b")));
    let _s = net.attach(&ea, &eb).await;
    let mut rb = eb.subscribe();
    let t = Instant::now();
    ea.send(net.b_fp, &file).unwrap();
    until(&mut rb, "1 GiB", |l| all_done(l, 1)).await;
    let engine_time = t.elapsed();
    let mbps = |d: Duration| SIZE as f64 / d.as_secs_f64() / 1e6;
    let ratio = raw.as_secs_f64() / engine_time.as_secs_f64();
    println!(
        "raw QUIC {:.0} MB/s, engine {:.0} MB/s, ratio {:.2}",
        mbps(raw),
        mbps(engine_time),
        ratio
    );
    assert!(ratio >= 0.8, "engine at {ratio:.2} of raw QUIC");
}

/// WN-86: withdrawing `files/1` (§6 rule 5) stops a running transfer with
/// `CANCELLED`, refuses new `files` streams with `CAPABILITY_MISSING`, and
/// after re-enabling a new transfer goes through.
#[tokio::test(flavor = "multi_thread")]
async fn withdrawing_files_cancels_transfers_and_reenabling_works() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let big = random_file(&src, "big.bin", 48 * 1024 * 1024, 21);
    let small = random_file(&src, "after.bin", 700_000, 22);
    let inbox_b = dir.path().join("b");
    let (ea, eb) = (engine(&dir.path().join("a")), engine(&inbox_b));
    let net = Net::new();
    let (sa, sb) = net.attach(&ea, &eb).await;
    let (mut ra, mut rb) = (ea.subscribe(), eb.subscribe());
    let big_id = ea.send(net.b_fp, &big).unwrap()[0];
    until(&mut rb, "running", |l| {
        l.first().is_some_and(|t| t.done > 0)
    })
    .await;

    // B stops accepting files mid-transfer.
    assert!(sb.set_accepted(Channel::Files, false).await.unwrap());
    // The sender sees its streams stopped with CANCELLED.
    until(&mut ra, "sender stopped", |l| {
        l[0].state == TransferState::Cancelled { by_peer: true }
    })
    .await;
    until(&mut rb, "receiver withdrawn", |l| {
        l[0].state == TransferState::Failed(FailReason::Withdrawn)
    })
    .await;
    assert!(no_partials(&inbox_b));
    assert!(!inbox_b.join("big.bin").exists());

    // A stream opened while withdrawn (as by a sender that hasn't seen the
    // Capabilities yet) is refused with CAPABILITY_MISSING.
    let (mut send, mut recv) = sa.connection().quinn().open_bi().await.unwrap();
    send.write_all(&[0x01, 0x05]).await.unwrap();
    let mut byte = [0u8; 1];
    let refused = tokio::time::timeout(T, recv.read(&mut byte)).await.unwrap();
    match refused {
        Err(quinn::ReadError::Reset(code)) => {
            assert_eq!(code.into_inner(), ErrorCode::CapabilityMissing.code())
        }
        other => panic!("expected CAPABILITY_MISSING, got {other:?}"),
    }
    // Once A has the new list, the engine doesn't even try.
    tokio::time::timeout(T, async {
        while sa.peer_capabilities().iter().any(|c| c.name() == "files") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let refused_id = ea.send(net.b_fp, &small).unwrap()[0];
    until(&mut ra, "refused locally", |l| {
        l.iter().any(|t| {
            t.id == refused_id
                && t.state
                    == TransferState::Failed(FailReason::Refused(ErrorCode::CapabilityMissing))
        })
    })
    .await;

    // Re-enabled: a new transfer goes through.
    assert!(sb.set_accepted(Channel::Files, true).await.unwrap());
    tokio::time::timeout(T, async {
        while !sa.peer_capabilities().iter().any(|c| c.name() == "files") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let id = ea.send(net.b_fp, &small).unwrap()[0];
    let got = until(&mut rb, "after re-enable", |l| {
        l.iter()
            .any(|t| t.id == id && t.state == TransferState::Done)
    })
    .await;
    let t = got.iter().find(|t| t.id == id).unwrap();
    assert_eq!(sha(t.saved_as.as_ref().unwrap()), sha(&small));
    // And the earlier refused one can simply be retried.
    ea.retry(&refused_id).unwrap();
    until(&mut ra, "retried", |l| {
        l.iter()
            .any(|t| t.id == refused_id && t.state == TransferState::Done)
    })
    .await;
    // The withdrawn transfer itself, retried with its id, is taken too
    // (a withdrawal is not a user's cancel, §9.1 point 7).
    ea.retry(&big_id).unwrap();
    let got = until(&mut rb, "withdrawn one retried", |l| {
        l.iter()
            .any(|t| t.id == big_id && t.state == TransferState::Done)
    })
    .await;
    let t = got.iter().find(|t| t.id == big_id).unwrap();
    assert_eq!(sha(t.saved_as.as_ref().unwrap()), sha(&big));
}
