//! CCS/1 PURPOSE:finite local stand-in for the phone in WN-27's native check: the same FFI calls as Android
//! (discovery -> TXT pair -> start_sas -> code -> decide -> connect); ephemeral identity, TEMP output only.
use std::{io::Write, sync::Arc, time::Duration};
use wind_ffi::{open_client, CoreError, IdentityStorage};

#[derive(Default)]
struct Memory(std::sync::Mutex<Option<Vec<u8>>>);
impl IdentityStorage for Memory {
    fn load(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn store_new(&self, pem: Vec<u8>) -> Result<bool, CoreError> {
        *self.0.lock().unwrap() = Some(pem);
        Ok(true)
    }
}

fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, decision, directory] = args.as_slice() else {
        return Err("usage: sas_pairing_peer <accept|reject> <TEMP-output>".into());
    };
    let accept = decision == "accept";
    std::fs::create_dir_all(directory)?;
    let client = open_client(
        Box::new(Memory::default()),
        "Testowy telefon".into(),
        "0.0.0.0:0".into(),
    )
    .await?;
    client.start_discovery().await?;
    let device = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = client.next_network_snapshot().await?;
            if let Some(found) = snapshot
                .peers
                .into_iter()
                .find(|p| p.pairing_port.is_some())
            {
                return Ok::<_, CoreError>(found);
            }
        }
    })
    .await??;
    say(&format!("FOUND {}", device.fingerprint));
    let attempt = client.start_sas(device.clone(), directory.clone()).await?;
    let code = attempt.code().await?;
    say(&format!("CODE {} NAME {}", code.code, code.peer_name));
    match attempt.decide(accept).await {
        Ok(peer) => {
            say(&format!("PAIRED {}", peer.name));
            let address = device
                .addresses
                .iter()
                .find(|a| !a.starts_with('['))
                .or(device.addresses.first())
                .ok_or("no address")?
                .clone();
            let connection = Arc::clone(&client)
                .connect(address, peer.public_key.clone())
                .await?;
            connection.ping().await?;
            say("SESSION");
            let done = std::path::Path::new(directory).join("done");
            for _ in 0..400 {
                if done.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            connection.disconnect().await;
        }
        Err(error) => say(&format!("NOT PAIRED {error}")),
    }
    client.shutdown().await;
    Ok(())
}
