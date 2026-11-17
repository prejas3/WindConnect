//! CCS/1 PURPOSE:finite real pairing/TLS peer for Android service tests, independent of the installed desktop app.
use std::{path::PathBuf, sync::Arc, time::Duration};
use wind_core::{
    identity::{DeviceIdentity, FileKeyStore},
    pairing::{PairedDevicesFile, PairingConfig, PairingListener},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("temporary directory required")?,
    );
    // WN-126: `--keep` is a PC that restarts (an update): the identity and the
    // paired list live in `directory` and are loaded again, as the app does.
    let keep = std::env::args().nth(2).as_deref() == Some("--keep");
    let identity = if keep {
        DeviceIdentity::load_or_create(&FileKeyStore::new(directory.join("identity.key")))?
    } else {
        DeviceIdentity::generate()?
    };
    let pins = Arc::new(PinnedKeys::new());
    if keep {
        PairedDevicesFile::in_dir(&directory).load()?.pin_all(&pins);
    }
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
    std::fs::write(directory.join("offer.txt"), offer.to_link().as_bytes())?;
    // CCS/1 RULE:use the desktop's production encoder/ECC, not a second QR encoder in the fixture.
    let code = qrcode::QrCode::with_error_correction_level(
        offer.to_link().as_bytes(),
        qrcode::EcLevel::M,
    )?;
    let width = code.width();
    let bits: String = code
        .into_colors()
        .into_iter()
        .map(|color| {
            if color == qrcode::Color::Dark {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    std::fs::write(directory.join("matrix.txt"), format!("{width}\n{bits}"))?;
    std::fs::write(
        directory.join("port.txt"),
        endpoint.local_addr()?.port().to_string(),
    )?;
    let accepting = endpoint.clone();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let acceptor = tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            let Ok(connection) = incoming.inspect_err(|e| eprintln!("incoming: {e}")) else {
                continue;
            };
            {
                let accepted = tokio::time::timeout(
                    Duration::from_secs(6),
                    Session::accept(
                        connection,
                        LocalInfo {
                            name: "Test PC".into(),
                            kind: DeviceKind::Computer,
                            app: None,
                        },
                        Router::new(),
                    ),
                )
                .await;
                eprintln!(
                    "session: {:?}",
                    accepted
                        .as_ref()
                        .map(|r| r.as_ref().map(|_| "ok").map_err(|e| e.to_string()))
                );
                if let Ok(Ok(session)) = accepted {
                    if sender.send(session).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = tokio::time::sleep(Duration::from_secs(2100));
    tokio::pin!(deadline);
    let mut active: Option<(Session, _)> = None;
    let mut sessions = 0u32;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = tick.tick() => {
                if directory.join("done").exists() { break; }
                std::fs::write(directory.join("connected.txt"), if active.is_some() { "true" } else { "false" })?;
            },
            _ = pairing.next_event() => {},
            session = receiver.recv() => {
                active = session;
                sessions += 1;
                std::fs::write(directory.join("sessions.txt"), sessions.to_string())?;
            },
            _ = async { match &active { Some((session, _)) => { session.closed().await; }, None => std::future::pending().await } } => { active = None; },
        }
    }
    acceptor.abort();
    pairing.close_window();
    endpoint.close();
    endpoint.wait_idle().await;
    Ok(())
}
