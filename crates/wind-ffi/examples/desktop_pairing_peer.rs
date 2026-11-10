//! CCS/1 PURPOSE:finite local test client for the real Windows pairing panel; ephemeral identity, TEMP output only.
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use wind_core::{
    identity::DeviceIdentity,
    pairing::{pair_with_offer, PairedDevicesFile, PairingConfig, PairingOffer},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 {
        return Err("usage: desktop_pairing_peer <offer.txt> <main-address> <TEMP-output>".into());
    }
    let link = Zeroizing::new(std::fs::read_to_string(&args[1])?);
    let offer = PairingOffer::parse(&link)?;
    let address = args[2].to_str().ok_or("invalid address")?.parse()?;
    let directory = PathBuf::from(&args[3]);
    std::fs::create_dir_all(&directory)?;
    let identity = DeviceIdentity::generate()?;
    let pins = Arc::new(PinnedKeys::new());
    let start = Instant::now();
    pair_with_offer(
        &identity,
        &offer,
        Some("Testowy telefon"),
        &PairedDevicesFile::in_dir(&directory),
        &pins,
        &PairingConfig {
            step_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .await?;
    let endpoint = Endpoint::bind("127.0.0.1:0".parse()?, &identity, pins)?;
    let (session, _events) = tokio::time::timeout(
        Duration::from_secs(5),
        Session::dial(
            &endpoint,
            address,
            offer.key,
            LocalInfo {
                name: "Testowy telefon".into(),
                kind: DeviceKind::Phone,
                app: Some("desktop-panel-test".into()),
            },
            Router::new(),
        ),
    )
    .await??;
    session.ping().await?;
    std::fs::write(
        directory.join("connected-ms.txt"),
        start.elapsed().as_millis().to_string(),
    )?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !directory.join("done").exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    endpoint.close();
    endpoint.wait_idle().await;
    println!("Real QR pairing and authenticated PC session complete");
    Ok(())
}
