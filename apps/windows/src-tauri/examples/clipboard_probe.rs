//! CCS/1 PURPOSE:finite native OS/privacy proof, isolated from the user's clipboard and installed app.
#[path = "../src/clipboard_native.rs"]
mod clipboard_native;
#[path = "support/private_clipboard.rs"]
mod private_clipboard;
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}
