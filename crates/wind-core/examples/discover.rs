use std::sync::Arc;
use std::time::{Duration, Instant};
use wind_core::{discovery::Discovery, identity::DeviceIdentity, transport};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = DeviceIdentity::generate()?;
    let endpoint = transport::Endpoint::bind(
        "0.0.0.0:0".parse()?,
        &identity,
        Arc::new(transport::PinnedKeys::default()),
    )?;
    let discovery = Discovery::start(&identity.public_key(), endpoint.local_addr()?.port())?;
    println!("local fingerprint: {}", identity.fingerprint().to_hex());
    let start = Instant::now();
    while let Some(remaining) = Duration::from_secs(30).checked_sub(start.elapsed()) {
        let Ok(event) =
            tokio::time::timeout(remaining.min(Duration::from_secs(10)), discovery.next()).await
        else {
            break;
        };
        println!("{:?}: {:?}", start.elapsed(), event?);
        if start.elapsed() >= Duration::from_secs(30) {
            break;
        }
    }
    drop(discovery);
    endpoint.close();
    endpoint.wait_idle().await;
    Ok(())
}
