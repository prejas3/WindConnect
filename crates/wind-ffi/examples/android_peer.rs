//! CCS/1 PURPOSE:bounded PC counterpart for the Android integration screen
use std::{sync::Arc, time::Duration};
use wind_core::{
    identity::{DeviceIdentity, PublicKey},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let hex = args
        .next()
        .ok_or("usage: android_peer <Android public key: 64 hex> [bind IP:port]")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid public key".into());
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
    }
    let peer = PublicKey::from_bytes(&bytes).ok_or("invalid Ed25519 key")?;
    let identity = DeviceIdentity::generate()?;
    let trust = Arc::new(PinnedKeys::new());
    trust.pin(peer);
    let endpoint = Endpoint::bind(
        args.next().unwrap_or("127.0.0.1:47820".into()).parse()?,
        &identity,
        trust,
    )?;
    let hex: String = identity
        .public_key()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!("PC_PUBLIC_KEY={hex}");
    println!("PC_ADDRESS={}", endpoint.local_addr()?);
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        let connection = endpoint.accept().await.ok_or("endpoint closed")??;
        let (session, _) = Session::accept(
            connection,
            LocalInfo {
                name: "WindConnect PC test".into(),
                kind: DeviceKind::Computer,
                app: Some(wind_core::VERSION.into()),
            },
            Router::new(),
        )
        .await?;
        println!(
            "CONNECTED {} {}",
            session.peer().hello.name,
            session.peer().key.fingerprint().to_hex()
        );
        println!("PC_PING_US={}", session.ping().await?.as_micros());
        session.closed().await;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    endpoint.close();
    endpoint.wait_idle().await;
    result??;
    Ok(())
}
