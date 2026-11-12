//! CCS/1 PURPOSE:finite native OS/privacy proof, isolated from the user's clipboard and installed app.
#[path = "../src/clipboard_image.rs"]
#[allow(dead_code)]
mod clipboard_image;
#[path = "../src/clipboard_native.rs"]
#[allow(dead_code)]
mod clipboard_native;
#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{atomic::Ordering::SeqCst, Arc},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
/// CCS/1 PURPOSE:second process in the same private station; holds the clipboard on command (WN-105).
fn child(station: &str, desktop: &str) -> Result<()> {
    private_clipboard::PrivateClipboard::join(station, desktop)?;
    let (native, _) = clipboard_native::Native::start()?;
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let (command, arg) = line.split_once(' ').unwrap_or((&line, ""));
        match command {
            "public" => native.write_test(arg, false)?,
            "clear" => native.clear_test()?,
            "sensitive" => {
                native.write_test(arg, false)?;
                native.mark_test("ExcludeClipboardContentFromMonitorProcessing", 0)?;
            }
            // Open (no change) and release later: no new WM_CLIPBOARDUPDATE follows.
            "hold" => native.hold_open_test(Duration::from_millis(arg.parse()?), || {
                let _ = writeln!(out, "held").and_then(|_| out.flush());
            })?,
            _ => return Err(format!("unknown child command {command}").into()),
        }
        writeln!(out, "ok")?;
        out.flush()?;
    }
    Ok(())
}
struct Holder {
    process: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Holder {
    fn spawn(private: &private_clipboard::PrivateClipboard) -> Result<Self> {
        let (station, desktop) = private.names()?;
        let mut process = Command::new(std::env::current_exe()?)
            .args(["--hold-child", &station, &desktop])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        Ok(Self {
            input: process.stdin.take(),
            output: BufReader::new(process.stdout.take().unwrap()),
            process,
        })
    }
    fn send(&mut self, line: &str) -> Result<()> {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{line}")?;
        input.flush()?;
        Ok(())
    }
    fn expect(&mut self, want: &str) -> Result<()> {
        let mut line = String::new();
        self.output.read_line(&mut line)?;
        if line.trim() != want {
            return Err(format!("child said {line:?}, expected {want}").into());
        }
        Ok(())
    }
    /// The child changes the clipboard while this process's gate defers the listener, then holds it.
    fn change_then_hold(
        &mut self,
        native: &clipboard_native::Native,
        kind: &str,
        text: &str,
        hold_ms: u64,
    ) -> Result<()> {
        let gate = native.hold_gate_test();
        self.send(&format!("{kind} {text}"))?;
        self.expect("ok")?;
        self.send(&format!("hold {hold_ms}"))?;
        self.expect("held")?;
        drop(gate); // re-posts the deferred update while the other process holds the clipboard
        Ok(())
    }
    fn released(&mut self) -> Result<()> {
        self.expect("ok")
    }
    fn finish(mut self) -> Result<()> {
        self.input.take();
        let status = self.process.wait()?;
        if !status.success() {
            return Err(format!("child failed: {status}").into());
        }
        Ok(())
    }
}
fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    while !done() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}
fn retry_proof(
    private: &private_clipboard::PrivateClipboard,
    native: Arc<clipboard_native::Native>,
    changed: &mut tokio::sync::watch::Receiver<Option<clipboard_native::Capture>>,
) -> Result<()> {
    let stats = native.stats_test();
    let mut holder = Holder::spawn(private)?;
    native.set_monitoring(true);
    changed.borrow_and_update();
    // 1. Public text held 300 ms (> the 30 ms acquire budget), released without a change: delivered once.
    let delivered = stats.delivered.load(SeqCst);
    let (retries, busy) = (stats.retries.load(SeqCst), stats.busy.load(SeqCst));
    holder.change_then_hold(&native, "public", "WN105 retry public", 300)?;
    holder.released()?;
    assert!(
        wait_until(Duration::from_secs(3), || stats.delivered.load(SeqCst)
            > delivered),
        "held change delivered after release"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(stats.delivered.load(SeqCst), delivered + 1, "exactly once");
    assert!(
        stats.busy.load(SeqCst) > busy,
        "read hit the held clipboard"
    );
    assert!(stats.retries.load(SeqCst) > retries, "read retried");
    assert!(!stats.pending.load(SeqCst), "no timer left");
    let value = changed.borrow_and_update().clone().unwrap();
    assert_eq!(value.text, "WN105 retry public");
    assert!(!value.sensitive);
    // 2. Sensitive text, same timing: delivered once as a marker, the text never.
    let delivered = stats.delivered.load(SeqCst);
    holder.change_then_hold(&native, "sensitive", "WN105 retry secret", 300)?;
    holder.released()?;
    assert!(wait_until(Duration::from_secs(3), || stats
        .delivered
        .load(SeqCst)
        > delivered));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(stats.delivered.load(SeqCst), delivered + 1);
    let value = changed.borrow_and_update().clone().unwrap();
    assert!(
        value.sensitive && value.text.is_empty(),
        "sensitive never read"
    );
    // 3. Held past the whole retry budget (~3.15 s): bounded, gives up while held, nothing delivered.
    let (delivered, given_up) = (stats.delivered.load(SeqCst), stats.given_up.load(SeqCst));
    let started = Instant::now();
    holder.change_then_hold(&native, "public", "WN105 retry limit", 4500)?;
    assert!(
        wait_until(Duration::from_secs(5), || stats.given_up.load(SeqCst)
            > given_up),
        "retry is bounded"
    );
    assert!(
        started.elapsed() < Duration::from_millis(4500),
        "gave up while still held"
    );
    assert!(!stats.pending.load(SeqCst));
    holder.released()?;
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        stats.delivered.load(SeqCst),
        delivered,
        "no read after the limit"
    );
    // 4. Disabling monitoring cancels a pending retry.
    let delivered = stats.delivered.load(SeqCst);
    holder.change_then_hold(&native, "public", "WN105 retry disabled", 600)?;
    assert!(wait_until(Duration::from_secs(1), || stats
        .pending
        .load(SeqCst)));
    native.set_monitoring(false);
    assert!(wait_until(Duration::from_millis(200), || !stats
        .pending
        .load(SeqCst)));
    holder.released()?;
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        stats.delivered.load(SeqCst),
        delivered,
        "cancelled retry reads nothing"
    );
    // 5. Shutdown with a retry pending: the worker joins and no timer survives.
    native.set_monitoring(true);
    holder.change_then_hold(&native, "public", "WN105 retry shutdown", 600)?;
    assert!(wait_until(Duration::from_secs(1), || stats
        .pending
        .load(SeqCst)));
    let native = Arc::try_unwrap(native).map_err(|_| "native still shared")?;
    let started = Instant::now();
    drop(native);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "worker joined"
    );
    assert!(!stats.pending.load(SeqCst), "no timer after shutdown");
    holder.released()?;
    holder.send("clear")?; // the child owns the last synthetic text; the station must stay empty
    holder.expect("ok")?;
    holder.finish()?;
    println!("{{\"heldClipboardRetry\":true,\"deliveredOnce\":true,\"sensitiveNeverRead\":true,\"retryBounded\":true,\"cancelOnDisable\":true,\"cleanShutdown\":true}}");
    Ok(())
}
/// Another app pasting `format` on its own thread: what it got, and how long it took.
fn paste(desktop: usize, format: u32) -> std::thread::JoinHandle<(Option<Vec<u8>>, Duration)> {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    std::thread::spawn(move || unsafe {
        assert_ne!(
            windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _),
            0
        );
        let started = Instant::now();
        let mut open = false;
        for _ in 0..200 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                open = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(open, "paster opens the clipboard");
        let handle = GetClipboardData(format);
        let bytes = if handle.is_null() {
            None
        } else {
            let size = GlobalSize(handle);
            let p = GlobalLock(handle);
            let out = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle);
            Some(out)
        };
        CloseClipboard();
        (bytes, started.elapsed())
    })
}
fn png_format() -> u32 {
    let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
    unsafe { windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW(name.as_ptr()) }
}
/// WN-36: images in and out of the real clipboard (private station).
fn image_proof(desktop: usize, native: &clipboard_native::Native) -> Result<()> {
    use clipboard_native::ImagePayload;
    use wind_core::clipboard::IncomingImage;
    let pixels: Vec<u8> = (0..64u32 * 48)
        .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 5) as u8, 90, 255])
        .collect();
    let rgba = clipboard_image::Rgba {
        width: 64,
        height: 48,
        pixels: pixels.clone(),
    };
    let png = clipboard_image::encode_png(&rgba);
    let dib = clipboard_image::encode_dib(&rgba);
    // Read: the "PNG" format as it is; a DIB alone; an image marked private is not copied.
    native.put_image_test(&png, &dib)?;
    let captured = clipboard_native::read()?.expect("an image");
    assert!(captured.text.is_empty() && !captured.sensitive);
    assert!(matches!(&captured.image, Some(ImagePayload::Png(b)) if **b == png));
    native.put_image_test(&[], &dib)?;
    let Some(ImagePayload::Dib(read_dib)) = clipboard_native::read()?.unwrap().image else {
        panic!("a DIB");
    };
    let out = clipboard_image::outgoing_from_dib(&read_dib).unwrap();
    assert_eq!((out.width, out.height), (64, 48));
    assert!(clipboard_image::decode_png(&out.png).unwrap().pixels == pixels);
    native.put_image_test(&png, &dib)?;
    native.mark_test("ExcludeClipboardContentFromMonitorProcessing", 0)?;
    let private = clipboard_native::read()?.unwrap();
    assert!(
        private.sensitive && private.image.is_none(),
        "private image not copied"
    );

    // Delayed rendering: pasteable at once, rendered when the bytes are in.
    let image = Arc::new(IncomingImage::new(
        [1; 16],
        png.len() as u64,
        64,
        48,
        vec![1],
    ));
    let half = png.len() / 2;
    image.push(0, &png[..half]).unwrap();
    native.write_image_test(image.clone(), true)?;
    assert!(
        clipboard_native::read()?.is_none(),
        "own received image is never read back (no echo)"
    );
    let pasting = paste(desktop, png_format());
    std::thread::sleep(Duration::from_millis(150));
    image.push(half as u64, &png[half..]).unwrap();
    let (got, took) = pasting.join().unwrap();
    assert!(got.as_deref() == Some(&png[..]), "the full PNG");
    assert!(took >= Duration::from_millis(100) && took < Duration::from_secs(2));
    let (got_dib, _) = paste(desktop, 8).join().unwrap();
    let decoded = clipboard_image::decode_dib(&got_dib.expect("CF_DIB rendered")).unwrap();
    assert!(decoded.pixels == pixels, "CF_DIB from the PNG");

    // WN-37: a history image put back: pasteable at once as "PNG" and CF_DIB, not read back (the app sends it).
    assert!(native.write_history_image(&png, vec![1], 64, 48, true));
    assert!(
        clipboard_native::read()?.is_none(),
        "history image not read back"
    );
    let (got, _) = paste(desktop, png_format()).join().unwrap();
    assert!(got.as_deref() == Some(&png[..]), "history PNG");
    let (got_dib, _) = paste(desktop, 8).join().unwrap();
    assert!(
        clipboard_image::decode_dib(&got_dib.expect("history CF_DIB"))
            .unwrap()
            .pixels
            == pixels
    );

    // Replaced while a paste waits: that paste fails at once, no partial image.
    let replaced = Arc::new(IncomingImage::new(
        [2; 16],
        png.len() as u64,
        64,
        48,
        vec![1],
    ));
    replaced.push(0, &png[..half]).unwrap();
    native.write_image_test(replaced.clone(), true)?;
    let pasting = paste(desktop, png_format());
    std::thread::sleep(Duration::from_millis(150));
    replaced.fail();
    let (got, took) = pasting.join().unwrap();
    assert!(
        got.is_none() && took < Duration::from_secs(1),
        "replaced: fails at once"
    );

    // Stalled: the paste fails after the stall time; the window answers afterwards.
    let stalled = Arc::new(IncomingImage::new(
        [3; 16],
        png.len() as u64,
        64,
        48,
        vec![1],
    ));
    native.write_image_test(stalled, true)?;
    let (got, took) = paste(desktop, png_format()).join().unwrap();
    assert!(got.is_none(), "stalled: nothing pasted");
    assert!(
        took >= Duration::from_secs(9) && took < Duration::from_secs(12),
        "stalled paste ends in ~10 s: {took:?}"
    );
    native.write_test("after a stalled paste", false)?;
    assert_eq!(
        clipboard_native::read()?.unwrap().text,
        "after a stalled paste",
        "the window is responsive again"
    );
    println!(
        r#"{{"imageRead":true,"privateImageNotCopied":true,"delayedRender":true,"dibFromPng":true,"replacedPasteFails":true,"stalledPasteMs":{},"responsiveAfterStall":true,"noEcho":true,"historyImageCopy":true}}"#,
        took.as_millis()
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--hold-child") {
        return child(&args[2], &args[3]);
    }
    let _private = private_clipboard::PrivateClipboard::enter()?;
    let (native, mut changed) =
        clipboard_native::Native::start().map_err(|e| format!("Native::start: {e}"))?;
    let native = std::sync::Arc::new(native);
    native.set_monitoring(true);
    native
        .write_test("Zażółć 👩‍💻\nhttps://example.com/ż", false)
        .map_err(|e| format!("write public: {e}"))?;
    let captured = clipboard_native::read()
        .map_err(|e| format!("read public: {e}"))?
        .unwrap();
    assert_eq!(captured.text, "Zażółć 👩‍💻\nhttps://example.com/ż");
    assert!(!captured.sensitive);
    assert_ne!(captured.sequence, 0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !changed.has_changed()? && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(changed.has_changed()?, "real WM_CLIPBOARDUPDATE");
    changed.borrow_and_update();
    for format in [
        "ExcludeClipboardContentFromMonitorProcessing",
        "Clipboard Viewer Ignore",
        "CanIncludeInClipboardHistory",
        "CanUploadToCloudClipboard",
    ] {
        native.write_test("synthetic private test", false)?;
        native.mark_test(format, 0)?;
        let captured = clipboard_native::read()?.unwrap();
        assert!(captured.sensitive);
        assert!(captured.text.is_empty());
    }
    for format in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
        native.write_test("public test", false)?;
        native.mark_test(format, 1)?;
        assert!(!clipboard_native::read()?.unwrap().sensitive);
    }
    // WN-37: what a password manager leaves (one marker, atomically) is never copied out.
    for marker in ["exclude", "ignore", "history", "cloud"] {
        native.write_marked_test("WN37 synthetic password", marker)?;
        let captured = clipboard_native::read()?.unwrap();
        assert!(captured.sensitive && captured.text.is_empty(), "{marker}");
    }
    // A history entry put back keeps its flags: sent from the PC → ordinary; from the phone → protected.
    assert!(native.write_history_text("WN37 from history", false));
    let captured = clipboard_native::read()?.unwrap();
    assert!(!captured.sensitive && captured.text == "WN37 from history");
    // One that came from the phone keeps its flags (out of Win+V history and the cloud).
    assert!(native.write_history_text("WN37 from the phone", true));
    let captured = clipboard_native::read()?.unwrap();
    assert!(captured.sensitive && captured.text.is_empty());
    assert!(native.write_received("remote test"));
    let captured = clipboard_native::read()?.unwrap();
    assert!(captured.sensitive);
    assert!(captured.text.is_empty());
    println!("{{\"nativeClipboard\":true,\"isolatedNoninteractiveClipboard\":true,\"privacyFlags\":4,\"osListener\":true,\"historyCloudOptOut\":true,\"passwordManagerMarkers\":4,\"historyCopyKeepsFlags\":true}}");
    let desktop = _private.desktop();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_start(move || unsafe {
            assert_ne!(
                windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _),
                0
            );
        })
        .build()?;
    runtime.block_on(async {
        let native = native.clone();
        tokio::task::spawn_blocking(move || {
            for index in 0..100 {
                let text = format!("WN35 pool-{index} 👩‍💻");
                assert!(native.write_received(&text));
                let value = clipboard_native::read_test().unwrap().unwrap();
                assert!(value.sensitive, "pool privacy flags");
                assert_eq!(value.text, text);
            }
        })
        .await
        .unwrap();
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    native.set_monitoring(false);
    changed.borrow_and_update();
    native.write_test("WN35 disabled monitoring", false)?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!changed.has_changed()?, "disabled monitor reads nothing");
    image_proof(desktop, &native)?;
    native.clear_test()?;
    drop(native);
    // WN-105: a fresh listener for the held-clipboard retry proof (it ends by dropping it).
    let (native, mut changed) = clipboard_native::Native::start()?;
    retry_proof(&_private, Arc::new(native), &mut changed)?;
    Ok(())
}
