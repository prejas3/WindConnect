//! CCS/1 PURPOSE:local Android integration fixture, ephemeral identity; never a production pairing server.
use std::{path::PathBuf, sync::Arc, time::Duration};
use wind_core::{
    identity::DeviceIdentity,
    pairing::{PairedDevicesFile, PairingConfig, PairingEvent, PairingListener},
    transport::PinnedKeys,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("provide an explicit temporary output directory")?,
    );
    std::fs::create_dir_all(&directory)?;
    let identity = DeviceIdentity::generate()?;
    let mut listener = PairingListener::bind(
        "127.0.0.1:0".parse()?,
        &identity,
        Some("Test PC"),
        Arc::new(PairedDevicesFile::in_dir(&directory)),
        Arc::new(PinnedKeys::new()),
        PairingConfig {
            ttl: Duration::from_secs(120),
            ..Default::default()
        },
    )?;
    let address = format!("10.0.2.2:{}", listener.local_addr()?.port()).parse()?;
    let offer = listener.open_qr(vec![address]);
    std::fs::write(directory.join("offer.txt"), offer.to_link().as_bytes())?;
    println!("QR fixture ready; offer.txt is a one-time test secret; expires in 120s");
    tokio::time::timeout(Duration::from_secs(120), async {
        while let Some(event) = listener.next_event().await {
            if let PairingEvent::Paired(device) = event {
                std::fs::write(
                    directory.join("paired-key.txt"),
                    device.key.fingerprint().to_hex(),
                )?;
                println!("Android paired successfully");
                return Ok::<(), std::io::Error>(());
            }
        }
        Err(std::io::Error::other(
            "pairing fixture ended without a peer",
        ))
    })
    .await??;
    drop(listener);
    Ok(())
}
