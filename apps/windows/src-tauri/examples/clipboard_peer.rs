//! CCS/1 PURPOSE:finite real QR/TLS + native clipboard peer, 100 OS-to-OS applied-ack samples. Synthetic data only.
#[path = "../src/clipboard_native.rs"]
#[allow(dead_code)]
// CCS/1 DEP:uses the subset of the production native adapter needed by this proof.
mod clipboard_native;
#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use wind_core::{
    clipboard::{Clipboard, Connection, Sink, Status},
    identity::DeviceIdentity,
    pairing::{PairedDevicesFile, PairingConfig, PairingListener},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
type Error = Box<dyn std::error::Error + Send + Sync>;
struct NativeSink {
    native: Arc<clipboard_native::Native>,
    controls: mpsc::Sender<String>,
    received: AtomicUsize,
    leak: AtomicBool,
    notification: AtomicBool,
}
impl Sink for NativeSink {
    fn apply(&self, text: &str) -> bool {
        if !self.native.write_received(text) {
            eprintln!("native test write failed");
            return false;
        }
        let read_back = clipboard_native::read_test();
        if !read_back
            .as_ref()
            .ok()
            .and_then(|v| v.as_ref())
            .is_some_and(|v| v.sensitive && v.text == text)
        {
            eprintln!(
                "native verification: present={}, sensitive={}, text_matches={}, error={:?}",
                read_back.as_ref().ok().is_some_and(|v| v.is_some()),
                read_back
                    .as_ref()
                    .ok()
                    .and_then(|v| v.as_ref())
                    .is_some_and(|v| v.sensitive),
                read_back
                    .as_ref()
                    .ok()
                    .and_then(|v| v.as_ref())
                    .is_some_and(|v| v.text == text),
                read_back.err()
            );
            return false;
        }
        self.received.fetch_add(1, Ordering::SeqCst);
        if text == "WN35-PRIVATE-ANDROID" || text == "WN35-STALE-BEFORE-CONNECT" {
            self.leak.store(true, Ordering::SeqCst);
        }
        if text == "WN35-ANDROID-NOTIFICATION" {
            self.notification.store(true, Ordering::SeqCst);
        }
        if text.starts_with("WN35-CONTROL-") {
            let _ = self.controls.try_send(text.into());
        }
        true
    }
}
fn main() -> Result<(), Error> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("temporary directory required")?,
    );
    let isolation = private_clipboard::PrivateClipboard::enter()?;
    let desktop = isolation.desktop();
    let (native, input) = clipboard_native::Native::start()?;
    let native = Arc::new(native);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_start(move || unsafe {
            assert_ne!(
                windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _),
                0
            );
        })
        .build()?;
    let result = runtime.block_on(run(directory, native.clone(), input));
    runtime.shutdown_timeout(Duration::from_secs(3));
    let cleanup = native.clear_test();
    drop(native);
    drop(isolation);
    result?;
    cleanup?;
    Ok(())
}
async fn run(
    directory: PathBuf,
    native: Arc<clipboard_native::Native>,
    mut input: watch::Receiver<Option<clipboard_native::Capture>>,
) -> Result<(), Error> {
    let identity = DeviceIdentity::generate()?;
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
        .iter()
        .map(|v| if *v == qrcode::Color::Dark { '1' } else { '0' })
        .collect();
    std::fs::write(directory.join("matrix.txt"), format!("{width}\n{bits}"))?;
    std::fs::write(
        directory.join("port.txt"),
        endpoint.local_addr()?.port().to_string(),
    )?;
    let hub = Arc::new(Clipboard::default());
    let (controls, mut commands) = mpsc::channel(8);
    let sink = Arc::new(NativeSink {
        native: native.clone(),
        controls,
        received: AtomicUsize::new(0),
        leak: AtomicBool::new(false),
        notification: AtomicBool::new(false),
    });
    hub.set_sink(sink.clone());
    let current: Arc<Mutex<Option<Arc<Connection>>>> = Arc::new(Mutex::new(None));
    let (tx, mut sessions) = mpsc::channel(1);
    let mut workers = tokio::task::JoinSet::new();
    let accepting = endpoint.clone();
    workers.spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            if let Ok(incoming) = incoming {
                let mut router = Router::new();
                let queue = Clipboard::route(&mut router);
                if let Ok(Ok(session)) = tokio::time::timeout(
                    Duration::from_secs(6),
                    Session::accept(
                        incoming,
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
                    if tx.send((session, queue)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    let starts: Arc<Mutex<std::collections::HashMap<usize, Instant>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let measurements = Arc::new(Mutex::new(Vec::<u64>::new()));
    let (complete, mut completed) = watch::channel(None);
    let (active, times, samples) = (current.clone(), starts.clone(), measurements.clone());
    workers.spawn(async move {
        while input.changed().await.is_ok() {
            let value = input.borrow_and_update().clone();
            let Some(value) = value else { continue };
            if value.sensitive {
                continue;
            }
            if value.sequence
                != unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
            {
                continue;
            }
            let connection = active.lock().unwrap().clone();
            let Some(connection) = connection else {
                continue;
            };
            let index = value
                .text
                .strip_prefix("WN35-PC-BENCH-")
                .and_then(|v| v.parse::<usize>().ok());
            let start = index.and_then(|i| times.lock().unwrap().remove(&i));
            let status = connection.send(value.text, false, false).await;
            if let Some(index) = index {
                if let Some(start) = start {
                    samples
                        .lock()
                        .unwrap()
                        .push(start.elapsed().as_micros() as u64);
                }
                complete.send_replace(Some((index, status == Status::Applied)));
            }
        }
    });
    let (writer, root) = (native.clone(), directory.clone());
    workers.spawn(async move{while let Some(command)=commands.recv().await{
        if command=="WN35-CONTROL-PC-BENCH"{
            tokio::time::sleep(Duration::from_millis(750)).await;
            for index in 0..100{
                starts.lock().unwrap().insert(index,Instant::now());
                writer.write_test(&format!("WN35-PC-BENCH-{index}"),false).expect("native synthetic copy");
                tokio::time::timeout(Duration::from_secs(5),async{loop{
                    if completed.borrow().is_some_and(|v|v.0==index&&v.1){break;}
                    completed.changed().await.expect("clipboard sender running");
                }}).await.expect("OS applied ack");
            }
            let values=measurements.lock().unwrap().clone();assert_eq!(values.len(),100);let mut sorted=values.clone();sorted.sort_unstable();
            std::fs::write(root.join("pc-benchmark.json"),serde_json::to_vec_pretty(&serde_json::json!({"samplesMicros":values,"p95Micros":sorted[94],"maxMicros":sorted[99],"startsAt":"native Windows clipboard copy","endsAt":"ack after Android ClipboardManager write","samples":100,"isolatedNoninteractiveClipboard":true})).unwrap()).unwrap();
        }else if command=="WN35-CONTROL-PC-PRIVATE"{
            tokio::time::sleep(Duration::from_millis(100)).await;writer.write_test("WN35-PRIVATE-PC",true).unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;writer.write_test("WN35-PC-PRIVATE-CHECK",false).unwrap();
        }
    }});
    let deadline = tokio::time::sleep(Duration::from_secs(180));
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut active: Option<Session> = None;
    loop {
        tokio::select! {
            _=&mut deadline=>break,
            _=tick.tick()=>{
                if directory.join("done").exists(){break;}
                if let Some(Err(error))=workers.try_join_next(){return Err(error.into());}
                std::fs::write(directory.join("connected.txt"),if active.is_some(){"true"}else{"false"})?;
                std::fs::write(directory.join("clipboard-state.json"),serde_json::to_vec(&serde_json::json!({"received":sink.received.load(Ordering::SeqCst),"privacyLeak":sink.leak.load(Ordering::SeqCst),"notification":sink.notification.load(Ordering::SeqCst)}))?)?;
            },
            _=pairing.next_event()=>{},
        incoming=sessions.recv()=>{if let Some(((session,_),queue))=incoming{*current.lock().unwrap()=Some(hub.attach(session.clone(),queue));native.set_monitoring(true);active=Some(session);}else{break;}},
        _=async{match &active{Some(session)=>{session.closed().await;},None=>std::future::pending().await}}=>{active=None;native.set_monitoring(false);current.lock().unwrap().take();},
        }
    }
    current.lock().unwrap().take();
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    pairing.close_window();
    endpoint.close();
    endpoint.wait_idle().await;
    Ok(())
}
