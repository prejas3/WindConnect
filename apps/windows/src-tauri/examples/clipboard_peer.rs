//! CCS/1 PURPOSE:finite real QR/TLS + native clipboard peer, 100 OS-to-OS applied-ack samples, and WN-36 screenshot-sized images
//! (copy on the PC -> pasteable on the phone, -> complete there; phone -> PC image). Synthetic data only.
#[path = "../src/clipboard_image.rs"]
#[allow(dead_code)]
mod clipboard_image;
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
    clipboard::{Clipboard, Connection, IncomingImage, Sink, Status},
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
    /// WN-36: images from the phone (the last complete one's size: IMAGE_SIZE).
    images: AtomicUsize,
}
impl Sink for NativeSink {
    fn apply_image(&self, image: Arc<IncomingImage>) -> bool {
        if !self.native.write_received_image(image.clone()) {
            return false;
        }
        self.images.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            if let Ok(png) = image.wait_complete(Duration::from_secs(10)) {
                if let Ok(rgba) = clipboard_image::decode_png(&png) {
                    *IMAGE_SIZE.lock().unwrap() = Some((rgba.width, rgba.height));
                }
            }
        });
        true
    }
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
        if text.starts_with("WN35-CONTROL-") || text.starts_with("WN36-CONTROL-") {
            let _ = self.controls.try_send(text.into());
        }
        true
    }
}
/// The size of the last complete image from the phone.
static IMAGE_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);
/// A synthetic screenshot: a desktop gradient, windows with title bars and
/// rows of "text" strokes, so it compresses like a real one (not noise).
fn screenshot(width: u32, height: u32) -> clipboard_image::Rgba {
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut put = |x: u32, y: u32, c: [u8; 3]| {
        let i = ((y * width + x) * 4) as usize;
        pixels[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    };
    for y in 0..height {
        for x in 0..width {
            put(
                x,
                y,
                [
                    (20 + x * 60 / width) as u8,
                    (60 + y * 80 / height) as u8,
                    110,
                ],
            );
        }
    }
    let mut seed = 0x2545_f491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for window in 0..6u32 {
        let (wx, wy) = (60 + window * width / 8, 40 + window * height / 10);
        let (ww, wh) = (width / 2, height / 2);
        for y in wy..(wy + wh).min(height) {
            for x in wx..(wx + ww).min(width) {
                let title = y < wy + 32;
                put(x, y, if title { [40, 44, 52] } else { [248, 248, 246] });
            }
        }
        // Text rows: short dark strokes of random length.
        let mut y = wy + 48;
        while y + 12 < (wy + wh).min(height) {
            let mut x = wx + 16;
            while x + 8 < (wx + ww).min(width) - 16 {
                let word = 8 + next() % 60;
                for dy in 0..9 {
                    for dx in 0..word.min((wx + ww).min(width) - 16 - x) {
                        if (dx + dy + next() % 3) % 4 != 0 {
                            put(x + dx, y + dy, [30, 30, 36]);
                        }
                    }
                }
                x += word + 7;
            }
            y += 20;
        }
    }
    clipboard_image::Rgba {
        width,
        height,
        pixels,
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
        images: AtomicUsize::new(0),
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
    let image_starts: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let image_samples = Arc::new(Mutex::new(
        Vec::<(Option<u64>, Option<u64>, usize, bool)>::new(),
    ));
    let (image_done, mut images_done) = watch::channel(0usize);
    let (bench_starts, bench_samples) = (image_starts.clone(), image_samples.clone());
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
            if let Some(payload) = value.image.clone() {
                // WN-36: what the app does: encode off-thread, send, time "pasteable" and "complete".
                let copied = image_starts.lock().unwrap().take();
                let image = tokio::task::spawn_blocking(move || match payload {
                    clipboard_native::ImagePayload::Png(png) => {
                        clipboard_image::outgoing_from_png(png.as_ref().clone())
                    }
                    clipboard_native::ImagePayload::Dib(dib) => {
                        clipboard_image::outgoing_from_dib(&dib)
                    }
                })
                .await
                .unwrap()
                .expect("a readable screenshot");
                let size = image.png.len();
                let pasteable = Arc::new(Mutex::new(None));
                let at = pasteable.clone();
                let status = connection
                    .send_image(image, false, false, move || {
                        *at.lock().unwrap() = copied.map(|t: Instant| t.elapsed())
                    })
                    .await;
                let complete = copied.map(|t| t.elapsed());
                let pasteable = *pasteable.lock().unwrap();
                image_samples.lock().unwrap().push((
                    pasteable.map(|d| d.as_micros() as u64),
                    complete.map(|d| d.as_micros() as u64),
                    size,
                    status == Status::Applied,
                ));
                image_done.send_replace(image_samples.lock().unwrap().len());
                continue;
            }
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
        }else if command=="WN36-CONTROL-PC-IMAGE"{
            // WN-36 done-when: a PC screenshot pasteable on the phone in < 500 ms. CF_DIB only, as a
            // screenshot tool leaves it (the slower path: the PC encodes the PNG).
            tokio::time::sleep(Duration::from_millis(500)).await;
            let shots = [(1920u32, 1080u32, 10usize), (3840, 2160, 3)];
            for (width, height, count) in shots {
                let base = screenshot(width, height);
                for sample in 0..count {
                    // A different screenshot each time (a moved cursor-sized block): a repeat would be "unchanged".
                    let mut shot = clipboard_image::Rgba { width, height, pixels: base.pixels.clone() };
                    for y in 0..16u32 { for x in 0..16u32 {
                        let i = (((100 + y) * width + 100 + x + sample as u32 * 20) * 4) as usize;
                        shot.pixels[i..i + 3].copy_from_slice(&[255, sample as u8 * 20, 0]);
                    } }
                    let dib = clipboard_image::encode_dib(&shot);
                    let before = *images_done.borrow();
                    *bench_starts.lock().unwrap() = Some(Instant::now());
                    writer.put_image_test(&[], &dib).expect("synthetic screenshot copy");
                    tokio::time::timeout(Duration::from_secs(20), async { while *images_done.borrow() == before { images_done.changed().await.expect("image sender"); } })
                        .await.expect("image delivered");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
            let all = bench_samples.lock().unwrap().clone();
            let report = |from: usize, to: usize| {
                let part = &all[from..to];
                let mut pasteable: Vec<u64> = part.iter().filter_map(|s| s.0).collect();
                let mut complete: Vec<u64> = part.iter().filter_map(|s| s.1).collect();
                pasteable.sort_unstable(); complete.sort_unstable();
                serde_json::json!({"samples":part.len(),"allApplied":part.iter().all(|s| s.3),"pngBytes":part.first().map(|s| s.2),
                    "pasteableMicros":pasteable,"completeMicros":complete,
                    "pasteableMaxMicros":pasteable.last(),"completeMaxMicros":complete.last()})
            };
            std::fs::write(root.join("pc-image-benchmark.json"),serde_json::to_vec_pretty(&serde_json::json!({
                "startsAt":"native Windows clipboard copy (CF_DIB)","pasteableAt":"Applied after Android setPrimaryClip (content URI)",
                "completeAt":"ImageDone: the whole PNG on the phone",
                "fullHd":report(0,10),"uhd4k":report(10,13)})).unwrap()).unwrap();
        }else if command=="WN35-CONTROL-PC-PRIVATE"{
            tokio::time::sleep(Duration::from_millis(100)).await;writer.write_test("WN35-PRIVATE-PC",true).unwrap();
            // WN-37 done-when: a password as each kind of manager marks it (one marker alone) never leaves the PC.
            for marker in ["exclude","ignore","history","cloud"] { tokio::time::sleep(Duration::from_millis(100)).await; writer.write_marked_test(&format!("WN37-PASSWORD-{marker}"),marker).unwrap(); }
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
                std::fs::write(directory.join("clipboard-state.json"),serde_json::to_vec(&serde_json::json!({"received":sink.received.load(Ordering::SeqCst),"privacyLeak":sink.leak.load(Ordering::SeqCst),"notification":sink.notification.load(Ordering::SeqCst),"imagesFromPhone":sink.images.load(Ordering::SeqCst),"phoneImageSize":IMAGE_SIZE.lock().unwrap().map(|(w,h)|vec![w,h])}))?)?;
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
