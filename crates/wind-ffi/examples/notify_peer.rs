//! CCS/1 PURPOSE:WN-42 PC stand-in that takes notify/1 (PROTOCOL.md §9.7) until WN-43's toasts: trusts one Android key, accepts
//! one session, prints each decoded notify message as a line (NOTIFY_POSTED / NOTIFY_ICON / NOTIFY_REMOVED / NOTIFY_MALFORMED).
//! Test tool only: it prints the notifications' text, which the real PC never logs (§9.7 rule 3).
use std::{sync::Arc, time::Duration};
use wind_core::{
    identity::{DeviceIdentity, PublicKey},
    notify::{self, Message},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let key = args
        .next()
        .ok_or("usage: notify_peer <Android public key: 64 hex> [bind IP:port] [seconds]")?;
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid public key".into());
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&key[index * 2..index * 2 + 2], 16)?;
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
    let seconds: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(180);
    println!("PC_PUBLIC_KEY={}", hex(identity.public_key().as_bytes()));
    println!("PC_ADDRESS={}", endpoint.local_addr()?);
    let result = tokio::time::timeout(Duration::from_secs(seconds), async {
        let connection = endpoint.accept().await.ok_or("endpoint closed")??;
        let mut router = Router::new();
        let mut shown = notify::route(&mut router);
        let (session, _) = Session::accept(
            connection,
            LocalInfo {
                name: "WindConnect PC test".into(),
                kind: DeviceKind::Computer,
                app: Some(wind_core::VERSION.into()),
            },
            router,
        )
        .await?;
        println!("CONNECTED {}", session.peer().hello.name);
        loop {
            tokio::select! {
                _ = session.closed() => break,
                m = shown.recv() => {
                    let Some(m) = m else { break };
                    match Message::decode(&m.message) {
                        Some(Message::Posted(n)) => println!(
                            "NOTIFY_POSTED id={} app={:?} package={:?} title={:?} text={:?} time={} icon={}",
                            hex(&n.id), n.app, n.package, n.title, n.text, n.time_ms,
                            n.icon.map(|k| hex(&k)).unwrap_or_default()
                        ),
                        Some(Message::Icon { key, png }) => println!("NOTIFY_ICON key={} bytes={}", hex(&key), png.len()),
                        Some(Message::Removed { id }) => println!("NOTIFY_REMOVED id={}", hex(&id)),
                        None => println!("NOTIFY_MALFORMED type={}", m.message.ty),
                    }
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    println!("PEER_DONE");
    endpoint.close();
    endpoint.wait_idle().await;
    result??;
    Ok(())
}
