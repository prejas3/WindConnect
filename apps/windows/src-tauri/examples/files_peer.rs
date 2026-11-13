//! CCS/1 PURPOSE:finite real QR/TLS files receiver, rooted only in the supplied TEMP fixture.
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use wind_core::{
    clipboard::{Clipboard, Sink},
    files::{FileEngine, FilesConfig, TransferState},
    identity::DeviceIdentity,
    pairing::{PairedDevicesFile, PairingConfig, PairingListener},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};

struct FixtureClipboard(PathBuf);
impl Sink for FixtureClipboard {
    fn apply(&self, text: &str) -> bool {
        std::fs::write(self.0.join("clipboard.txt"), text).is_ok()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("TEMP fixture directory required")?,
    );
    let identity = DeviceIdentity::generate()?;
    let send_directory = std::env::args().nth(2).map(PathBuf::from);
    let mut queued = false;
    let pins = Arc::new(PinnedKeys::new());
    let endpoint = Endpoint::bind("0.0.0.0:0".parse()?, &identity, pins.clone())?;
    let mut pairing = PairingListener::bind(
        "0.0.0.0:0".parse()?,
        &identity,
        Some("Test PC"),
        Arc::new(PairedDevicesFile::in_dir(&directory)),
        pins,
        PairingConfig::default(),
    )?;
    let offer = pairing.open_qr(vec![
        format!("10.0.2.2:{}", pairing.local_addr()?.port()).parse()?
    ]);
    std::fs::write(directory.join("offer.txt"), offer.to_link())?;
    let code = qrcode::QrCode::with_error_correction_level(
        offer.to_link().as_bytes(),
        qrcode::EcLevel::M,
    )?;
    let width = code.width();
    let bits: String = code
        .into_colors()
        .into_iter()
        .map(|c| if c == qrcode::Color::Dark { '1' } else { '0' })
        .collect();
    std::fs::write(directory.join("matrix.txt"), format!("{width}\n{bits}"))?;
    std::fs::write(
        directory.join("port.txt"),
        endpoint.local_addr()?.port().to_string(),
    )?;
    let mut config = FilesConfig::new(
        directory.join("Downloads/WindConnect"),
        directory.join("partials"),
    );
    config.type_folders = true;
    let engine = FileEngine::new(config)?;
    let mut updates = engine.subscribe();
    let accepting = endpoint.clone();
    let receiver = engine.clone();
    let clipboard = Arc::new(Clipboard::default());
    clipboard.set_sink(Arc::new(FixtureClipboard(directory.clone())));
    let active = Arc::new(std::sync::Mutex::new(Vec::<Session>::new()));
    let connected = active.clone();
    let acceptor = tokio::spawn(async move {
        let mut clipboard_connections = Vec::new();
        while let Some(incoming) = accepting.accept().await {
            if let Ok(connection) = incoming {
                let mut router = Router::new();
                let input = router.route_files(1, 32);
                let clipboard_input = Clipboard::route(&mut router);
                if let Ok(Ok((session, _))) = tokio::time::timeout(
                    Duration::from_secs(6),
                    Session::accept(
                        connection,
                        LocalInfo {
                            name: "Test PC".into(),
                            kind: DeviceKind::Computer,
                            app: None,
                        },
                        router,
                    ),
                )
                .await
                {
                    connected.lock().unwrap().push(session.clone());
                    clipboard_connections.push(clipboard.attach(session.clone(), clipboard_input));
                    receiver.attach(session, Some(input));
                }
            }
        }
    });
    let deadline = tokio::time::sleep(Duration::from_secs(1200));
    tokio::pin!(deadline);
    let mut stop = tokio::time::interval(Duration::from_millis(250));
    let mut dropped = false;
    let mut files_disabled = false;
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = stop.tick() => {
                if directory.join("done").exists() { break; }
                if !queued && directory.join("receive-ready").exists() {
                    if let Some(source) = &send_directory {
                        let peer = active.lock().unwrap().last().map(|session| *session.connection().peer_key().fingerprint().as_bytes());
                        if let Some(peer) = peer { engine.send(peer, source)?; queued = true; }
                    }
                }
                let disable = directory.join("disable-files").exists();
                if disable != files_disabled {
                    let sessions = active.lock().unwrap().clone();
                    for session in sessions {
                        session.set_accepted(wind_core::protocol::Channel::Files, !disable).await?;
                    }
                    files_disabled = disable;
                }
                if !dropped && directory.join("drop-once").exists() {
                    for session in active.lock().unwrap().drain(..) {
                        session.connection().close(wind_core::protocol::ErrorCode::GoingAway, b"fixture reconnect");
                    }
                    dropped = true; std::fs::write(directory.join("drop-confirmed"), "true")?;
                }
            },
            _ = pairing.next_event() => {},
            changed = updates.changed() => {
                if changed.is_err() { break; }
                let rows = updates.borrow_and_update().clone();
                let mut received = Vec::new();
                for row in rows.iter().filter(|r| r.state == TransferState::Done) {
                    let file = row.saved_as.as_ref().or(row.source.as_ref()).ok_or("missing file path")?;
                    let mut digest = Sha256::new();
                    std::io::copy(&mut std::fs::File::open(file)?, &mut digest)?;
                    let hash = digest.finalize();
                    received.push(serde_json::json!({"name":row.path,"path":file,"sha256":format!("{hash:x}"),"size":row.size}));
                }
                let temporary = directory.join("received.tmp");
                std::fs::write(&temporary, serde_json::to_vec_pretty(&received)?)?;
                let destination = directory.join("received.json");
                // Fixture readers retry during the replacement; no production data touched.
                if destination.exists() { std::fs::remove_file(&destination)?; }
                std::fs::rename(temporary, destination)?;
            },
        }
    }
    acceptor.abort();
    pairing.close_window();
    endpoint.close();
    endpoint.wait_idle().await;
    Ok(())
}
