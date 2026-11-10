//! CCS/1 FLOW:Explorer --send <item> -> second process -> named pipe -> running app -> files::queue (WN-39)
//! SEC:local pipe of this logon session, remote clients refused, default DACL (owner, SYSTEM, admins write);
//!     it carries only paths to queue for the paired phone, checked again on receipt.
use std::{io, path::PathBuf, time::Duration};

const MAGIC: &[u8] = b"WCS1\n";
/// Largest message, and most paths in one.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
pub const MAX_PATHS: usize = 4096;
/// How long a second process keeps trying to reach a starting app.
pub const CONNECT_PATIENCE: Duration = Duration::from_secs(10);

/// `\\.\pipe\…` for this logon session.
pub fn name() -> String {
    #[cfg(windows)]
    let session = {
        let mut id = 0u32;
        // SAFETY: plain out parameter for this process.
        unsafe {
            windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(
                std::process::id(),
                &mut id,
            )
        };
        id
    };
    #[cfg(not(windows))]
    let session = 0u32;
    format!(r"\\.\pipe\io.github.prejas3.windconnect.send-{session}")
}

pub fn encode(paths: &[PathBuf]) -> io::Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    for (i, path) in paths.iter().enumerate() {
        let text = path
            .to_str()
            .ok_or_else(|| io::Error::other("non-Unicode path"))?;
        if text.contains('\0') {
            return Err(io::Error::other("NUL in path"));
        }
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(text.as_bytes());
    }
    if out.len() > MAX_MESSAGE || paths.len() > MAX_PATHS {
        return Err(io::Error::other("too many paths"));
    }
    Ok(out)
}

/// The absolute paths of a message; anything else (other magic, bad UTF-8,
/// relative or empty paths, too many) makes the whole message void.
pub fn decode(bytes: &[u8]) -> Option<Vec<PathBuf>> {
    let body = bytes.strip_prefix(MAGIC)?;
    if bytes.len() > MAX_MESSAGE || body.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?;
    let paths: Vec<PathBuf> = text.split('\0').map(PathBuf::from).collect();
    (paths.len() <= MAX_PATHS && paths.iter().all(|p| p.is_absolute())).then_some(paths)
}

/// Hands `paths` to the running app. `Err` when no app answered in time.
#[cfg(windows)]
pub fn forward(paths: &[PathBuf]) -> io::Result<()> {
    use std::io::Write;
    let message = encode(paths)?;
    let deadline = std::time::Instant::now() + CONNECT_PATIENCE;
    loop {
        match std::fs::OpenOptions::new().write(true).open(name()) {
            Ok(mut pipe) => return pipe.write_all(&message),
            Err(e) if std::time::Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}
#[cfg(not(windows))]
pub fn forward(_: &[PathBuf]) -> io::Result<()> {
    Err(io::Error::other("Windows only"))
}

/// Serves the pipe until the app ends, handing each message's paths to `on_paths`.
#[cfg(windows)]
pub async fn serve(on_paths: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) -> io::Result<()> {
    use tokio::io::AsyncReadExt;
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = name();
    let on_paths = std::sync::Arc::new(on_paths);
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .access_outbound(false)
        .create(&name)?;
    loop {
        server.connect().await?;
        let connected = server;
        server = ServerOptions::new()
            .reject_remote_clients(true)
            .access_outbound(false)
            .create(&name)?;
        let on_paths = on_paths.clone();
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let read = tokio::time::timeout(
                Duration::from_secs(5),
                connected
                    .take(MAX_MESSAGE as u64 + 1)
                    .read_to_end(&mut bytes),
            )
            .await;
            if let (Ok(Ok(_)), Some(paths)) = (read, decode(&bytes)) {
                on_paths(paths);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_bad_ones_are_void() {
        let paths = vec![
            PathBuf::from(r"C:\Users\a\Zdjęcia\IMG 1.jpg"),
            PathBuf::from(r"D:\Projekt"),
        ];
        assert_eq!(decode(&encode(&paths).unwrap()), Some(paths));
        assert_eq!(decode(b"WCS1\nrelative\\x"), None);
        assert_eq!(decode(b"WCS1\nC:\\a\0"), None);
        assert_eq!(decode(b"WCS1\n"), None);
        assert_eq!(decode(b"XXXX\nC:\\a"), None);
        assert_eq!(decode(b"WCS1\nC:\\\xff"), None);
        assert!(encode(&[PathBuf::from("C:\\a\0b")]).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_second_process_hands_paths_over() {
        // Only when no app of this session runs (the pipe is per session).
        if std::fs::OpenOptions::new().write(true).open(name()).is_ok() {
            return;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(serve(move |p| {
            let _ = tx.send(p);
        }));
        let paths = vec![PathBuf::from(r"C:\Windows\win.ini")];
        let sent = paths.clone();
        tokio::task::spawn_blocking(move || forward(&sent))
            .await
            .unwrap()
            .unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap();
        assert_eq!(got, Some(paths));
        server.abort();
    }
}
