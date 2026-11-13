//! CCS/1 CHECK:finite real QUIC transfers with the production snapshot observer, optionally rendered by Playwright.
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use wind_core::{
    files::{FileEngine, FilesConfig, TransferState},
    identity::DeviceIdentity,
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Connection, Endpoint, PinnedKeys},
};
#[path = "../src/transfer_model.rs"]
mod transfer_model;
#[path = "../src/transfer_progress.rs"]
mod transfer_progress;
const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or("TEMP directory required")?);
    std::fs::create_dir_all(&root)?;
    let source = root.join("source");
    std::fs::create_dir_all(&source)?;
    let block = vec![42u8; 1024 * 1024];
    let large = source.join("large.bin");
    let mut file = std::fs::File::create(&large)?;
    for _ in 0..1024 {
        file.write_all(&block)?;
    }
    file.sync_all()?;
    let mut small = Vec::new();
    for i in 0..256 {
        let p = source.join(format!("small-{i}.bin"));
        std::fs::write(&p, &block)?;
        small.push(p);
    }
    for kind in ["large", "small"] {
        for round in 0..3 {
            for observe in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let label = format!("{kind}-{round}-{observe}");
                let dir = root.join(&label);
                let ea = FileEngine::new(FilesConfig::new(dir.join("a"), dir.join("ap")))?;
                let eb = FileEngine::new(FilesConfig::new(dir.join("b"), dir.join("bp")))?;
                let net = Net::new();
                let sessions = net.attach(&ea, &eb).await;
                let observer = if observe {
                    let mut rx = ea.subscribe();
                    let name = label.clone();
                    Some(tokio::spawn(async move {
                        let begin = Instant::now();
                        let mut rates =
                            std::collections::HashMap::<String, transfer_progress::Rate>::new();
                        let mut last = None;
                        while let Some(rows) =
                            transfer_model::next_snapshot(&mut rx, &mut last).await
                        {
                            let mut snapshot = transfer_model::snapshot_of(None, &rows);
                            for row in &mut snapshot.rows {
                                row.rate = rates.entry(row.key.clone()).or_default().update(
                                    begin.elapsed().as_millis() as u64,
                                    row.done,
                                    row.state == "running",
                                );
                            }
                            println!(
                                "{}",
                                serde_json::json!({"event":"snapshot","run":name,"timeMs":begin.elapsed().as_millis(),"snapshot":snapshot})
                            );
                        }
                    }))
                } else {
                    None
                };
                let mut updates = eb.subscribe();
                let paths = if kind == "large" {
                    vec![large.clone()]
                } else {
                    small.clone()
                };
                let count = paths.len();
                let total = if kind == "large" {
                    1024 * 1024 * 1024u64
                } else {
                    256 * 1024 * 1024u64
                };
                println!(
                    "{}",
                    serde_json::json!({"event":"start","run":label,"observe":observe,"kind":kind,"bytes":total})
                );
                let started = Instant::now();
                for p in paths {
                    ea.send(net.b_fp, &p)?;
                }
                tokio::time::timeout(Duration::from_secs(120), async {
                    loop {
                        let rows = updates.borrow_and_update().clone();
                        if rows.len() == count
                            && rows.iter().all(|r| r.state == TransferState::Done)
                        {
                            break;
                        }
                        updates.changed().await.unwrap();
                    }
                })
                .await?;
                let elapsed = started.elapsed().as_secs_f64();
                if let Some(task) = observer {
                    task.abort();
                    let _ = task.await;
                }
                println!(
                    "{}",
                    serde_json::json!({"event":"end","run":label,"kind":kind,"observe":observe,"seconds":elapsed,"bytes":total})
                );
                drop(sessions);
                net.a.close();
                net.b.close();
                net.a.wait_idle().await;
                net.b.wait_idle().await;
            }
        }
    }
    Ok(())
}
