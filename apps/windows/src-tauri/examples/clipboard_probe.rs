//! CCS/1 PURPOSE:finite native OS/privacy proof, isolated from the user's clipboard and installed app.
#[path = "../src/clipboard_native.rs"]
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
    assert!(native.write_received("remote test"));
    let captured = clipboard_native::read()?.unwrap();
    assert!(captured.sensitive);
    assert!(captured.text.is_empty());
    println!("{{\"nativeClipboard\":true,\"isolatedNoninteractiveClipboard\":true,\"privacyFlags\":3,\"osListener\":true,\"historyCloudOptOut\":true}}");
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
    native.clear_test()?;
    drop(native);
    // WN-105: a fresh listener for the held-clipboard retry proof (it ends by dropping it).
    let (native, mut changed) = clipboard_native::Native::start()?;
    retry_proof(&_private, Arc::new(native), &mut changed)?;
    Ok(())
}
