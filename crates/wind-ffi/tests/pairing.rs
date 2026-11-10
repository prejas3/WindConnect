use std::sync::{Arc, Mutex};
use wind_core::{
    identity::DeviceIdentity,
    pairing::{PairedDevicesFile, PairingConfig, PairingListener},
    transport::PinnedKeys,
};
use wind_ffi::{inspect_qr_link, open_client, CoreError, IdentityStorage};

#[derive(Default, Clone)]
struct Memory(Arc<Mutex<Option<Vec<u8>>>>);
impl IdentityStorage for Memory {
    fn load(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn store_new(&self, pem: Vec<u8>) -> Result<bool, CoreError> {
        let mut stored = self.0.lock().unwrap();
        if stored.is_some() {
            return Ok(false);
        }
        *stored = Some(pem);
        Ok(true)
    }
}

#[tokio::test]
async fn qr_preview_does_not_pair_then_confirmation_persists_and_consumes_offer() {
    let root = std::env::temp_dir().join(format!(
        "windconnect-ffi-qr-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pc_store = Arc::new(PairedDevicesFile::in_dir(root.join("pc")));
    let phone_dir = root.join("phone").to_string_lossy().into_owned();
    let identity = DeviceIdentity::generate().unwrap();
    let mut listener = PairingListener::bind(
        "127.0.0.1:0".parse().unwrap(),
        &identity,
        Some("Test PC"),
        pc_store.clone(),
        Arc::new(PinnedKeys::new()),
        PairingConfig::default(),
    )
    .unwrap();
    let link = listener
        .open_qr(vec![listener.local_addr().unwrap()])
        .to_link()
        .to_string();
    let offer = inspect_qr_link(link.clone()).unwrap();
    assert_eq!(offer.preview().name, "Test PC");
    assert_eq!(offer.preview().public_key, identity.public_key().as_bytes());
    assert!(pc_store.load().unwrap().devices.is_empty());
    assert!(!root.join("phone").exists());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.next_event())
            .await
            .is_err()
    );
    let storage = Memory::default();
    let client = open_client(
        Box::new(storage.clone()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let peer = client
        .pair_qr(offer.clone(), phone_dir.clone())
        .await
        .unwrap();
    assert_eq!(peer.name, "Test PC");
    assert_eq!(pc_store.load().unwrap().devices.len(), 1);
    assert_eq!(
        client.load_paired(phone_dir.clone()).unwrap().devices.len(),
        1
    );
    assert!(matches!(
        client.pair_qr(offer, phone_dir.clone()).await,
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(client
        .pair_qr(inspect_qr_link(link).unwrap(), phone_dir.clone())
        .await
        .is_err());
    client.shutdown().await;
    let reopened = open_client(Box::new(storage), "Phone".into(), "127.0.0.1:0".into())
        .await
        .unwrap();
    assert_eq!(
        reopened.load_paired(phone_dir).unwrap().devices[0].public_key,
        identity.public_key().as_bytes()
    );
    reopened.shutdown().await;
    drop(listener);
    assert!(root.starts_with(std::env::temp_dir()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_links_never_echo_secrets_in_errors() {
    for link in [
        "https://example.org/",
        "windconnect:pair?v=2&s=SECRET",
        "windconnect:pair?v=1&k=broken&s=SECRET",
    ] {
        let error = match inspect_qr_link(link.into()) {
            Err(error) => error,
            Ok(_) => panic!("invalid link accepted"),
        };
        assert!(!error.to_string().contains("SECRET"));
    }
    assert!(inspect_qr_link("x".repeat(1025)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn damaged_record_is_reported_and_valid_key_still_authenticates() {
    use std::io::Write;
    use wind_core::{
        identity::PublicKey,
        pairing::{PairedDevice, PairedStore},
        protocol::DeviceKind,
        session::{LocalInfo, Router, Session},
        transport::Endpoint,
    };
    let root = std::env::temp_dir().join(format!(
        "windconnect-ffi-damaged-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let identity = DeviceIdentity::generate().unwrap();
    let store = PairedDevicesFile::in_dir(&root);
    store
        .add(&PairedDevice::now(
            identity.public_key(),
            Some("Saved PC".into()),
        ))
        .unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(store.path())
        .unwrap()
        .write_all(b"\nbroken record\n")
        .unwrap();
    let before = std::fs::read(store.path()).unwrap();
    let client = open_client(
        Box::new(Memory::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let loaded = client
        .load_paired(root.to_string_lossy().into_owned())
        .unwrap();
    assert_eq!(loaded.skipped_records, 1);
    assert_eq!(loaded.devices.len(), 1);
    assert_eq!(
        loaded.devices[0].public_key,
        identity.public_key().as_bytes()
    );
    assert_eq!(std::fs::read(store.path()).unwrap(), before);
    let pins = Arc::new(PinnedKeys::new());
    pins.pin(PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap());
    let server = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &identity, pins).unwrap();
    let address = server.local_addr().unwrap().to_string();
    let server_task = tokio::spawn(async move {
        let connection = server.accept().await.unwrap().unwrap();
        let (session, _) = Session::accept(
            connection,
            LocalInfo {
                name: "Saved PC".into(),
                kind: DeviceKind::Computer,
                app: None,
            },
            Router::new(),
        )
        .await
        .unwrap();
        session.closed().await;
        server.close();
        server.wait_idle().await;
    });
    // CCS/1 SEC:no explicit pin_peer: a successful TLS/session proves load_paired trusted the intact record.
    let connection = client
        .connect(address, identity.public_key().as_bytes().to_vec())
        .await
        .unwrap();
    assert_eq!(connection.peer().name, "Saved PC");
    connection.ping().await.unwrap();
    connection.disconnect().await;
    tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap();
    client.shutdown().await;
    assert!(root.starts_with(std::env::temp_dir()));
    std::fs::remove_dir_all(root).unwrap();
}
