use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn run(identity: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_windctl"))
        .arg("--identity")
        .arg(identity)
        .args(args)
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        text(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    text(&output)
}

fn key(identity: &Path) -> String {
    let output = success(run(identity, &["identity"]));
    output
        .split("public_key=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .into()
}

struct Server {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
    address: String,
}

impl Server {
    fn start(identity: &Path, trust: &str, advertise: bool) -> Self {
        Self::start_mode(identity, trust, advertise, false)
    }

    fn start_mode(identity: &Path, trust: &str, advertise: bool, broadcast_only: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_windctl"));
        command.arg("--identity").arg(identity).args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--trust",
            trust,
            "--echo",
            "--duration",
            "4",
        ]);
        if advertise {
            command.arg("--advertise");
        }
        if broadcast_only {
            command.arg("--broadcast-only");
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if let Ok(line) = line {
                    let _ = sender.send(line);
                } else {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            lines,
            reader: Some(reader),
            address: String::new(),
        };
        loop {
            let line = server
                .lines
                .recv_timeout(Duration::from_secs(15))
                .expect("server must announce readiness");
            if let Some(address) = line.strip_prefix("LISTENING address=") {
                server.address = address.into();
                break;
            }
        }
        server
    }

    fn finish(&mut self) -> String {
        assert!(self.child.wait().unwrap().success());
        self.reader.take().unwrap().join().unwrap();
        self.lines.try_iter().collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn two_nodes_connect_exchange_messages_and_report_latency_percentiles() {
    let temporary = tempfile::tempdir().unwrap();
    let client_path = temporary.path().join("client.key");
    let server_path = temporary.path().join("server.key");
    let client_key = key(&client_path);
    let server_key = key(&server_path);
    assert_eq!(key(&client_path), client_key);
    let mut server = Server::start(&server_path, &client_key, false);
    let address = server.address.as_str();
    assert!(success(run(
        &client_path,
        &["connect", "--address", address, "--peer-key", &server_key]
    ))
    .contains("READY rtt_ms="));
    let output = success(run(
        &client_path,
        &[
            "ping",
            "--address",
            address,
            "--peer-key",
            &server_key,
            "--count",
            "5",
            "--warmup",
            "1",
            "--interval-ms",
            "0",
        ],
    ));
    assert!(output.contains("SUMMARY sent=5 received=5 lost=0 warmup=1"));
    println!(
        "{}",
        output
            .lines()
            .find(|line| line.starts_with("SUMMARY "))
            .unwrap()
    );
    let value = |name: &str| -> f64 {
        output
            .split(name)
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(value("p95_ms=") >= value("p50_ms="));
    let output = success(run(
        &client_path,
        &[
            "send",
            "--address",
            address,
            "--peer-key",
            &server_key,
            "--channel",
            "notify",
            "--text",
            "hello from another process",
            "--expect-echo",
        ],
    ));
    assert!(output.contains("SENT channel=Notify type=65000 echo_verified=true"));
    let output = success(run(
        &client_path,
        &[
            "send",
            "--address",
            address,
            "--peer-key",
            &server_key,
            "--channel",
            "media",
            "--body-hex",
            "a100182a",
            "--message-type",
            "65001",
            "--expect-echo",
        ],
    ));
    assert!(output.contains("SENT channel=Media type=65001 echo_verified=true"));
    let server_log = server.finish();
    assert!(server_log.contains("RECEIVED channel=Notify type=65000"));
    assert!(server_log.contains("RECEIVED channel=Media type=65001 body_hex=a100182a"));
}

#[test]
fn unknown_client_and_wrong_expected_server_key_are_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let client = temporary.path().join("client.key");
    let server_path = temporary.path().join("server.key");
    let stranger = temporary.path().join("stranger.key");
    let client_key = key(&client);
    let server_key = key(&server_path);
    let wrong_key = key(&stranger);
    let mut server = Server::start(&server_path, &client_key, false);
    let output = run(
        &stranger,
        &[
            "connect",
            "--address",
            &server.address,
            "--peer-key",
            &server_key,
        ],
    );
    assert!(!output.status.success());
    assert!(!text(&output).contains("READY"));
    let output = run(
        &client,
        &[
            "connect",
            "--address",
            &server.address,
            "--peer-key",
            &wrong_key,
        ],
    );
    assert!(!output.status.success());
    assert!(!text(&output).contains("READY"));
    server.finish();
}

#[test]
fn invalid_arguments_do_not_create_identity_files_and_help_works() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("identity.key");
    for args in [&["--help"][..], &["--version"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_windctl"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for args in [
        vec!["ping", "--address", "127.0.0.1:9", "--peer-key", "bad"],
        vec!["serve", "--trust", "bad"],
        vec!["discover", "--duration", "0"],
    ] {
        assert!(!run(&path, &args).status.success());
        assert!(!path.exists());
    }
}

#[test]
fn corrupt_identity_is_not_replaced_and_invalid_cbor_is_not_sent() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("identity.key");
    let public = key(&path);
    for body in ["80", "a1180000", "zz"] {
        let output = run(
            &path,
            &[
                "send",
                "--address",
                "127.0.0.1:9",
                "--peer-key",
                &public,
                "--body-hex",
                body,
            ],
        );
        assert!(!output.status.success());
        assert!(!text(&output).contains("CONNECTED"));
    }
    std::fs::write(&path, b"corrupt identity").unwrap();
    assert!(!run(&path, &["identity"]).status.success());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt identity");
}

#[test]
#[ignore = "requires multicast-capable LAN and UDP 5353 firewall permission"]
fn discovery_reports_a_separate_advertising_process() {
    let temporary = tempfile::tempdir().unwrap();
    let client = temporary.path().join("client.key");
    let server_path = temporary.path().join("server.key");
    let client_key = key(&client);
    let server_key = key(&server_path);
    let mut server = Server::start(&server_path, &client_key, true);
    let output = success(run(&client, &["discover", "--duration", "2"]));
    assert!(output.contains("DISCOVERED service="));
    assert!(output.contains(
        &windctl::parse_key(&server_key)
            .unwrap()
            .fingerprint()
            .to_hex()
    ));
    let port = server.address.split(':').next_back().unwrap();
    assert!(output.contains(&format!(":{port}")));
    server.finish();
}

#[test]
#[ignore = "requires IPv4 broadcast-capable host; no mDNS backend is started"]
fn broadcast_discovers_a_separate_process_without_mdns() {
    let temporary = tempfile::tempdir().unwrap();
    let client = temporary.path().join("client.key");
    let server_path = temporary.path().join("server.key");
    let client_key = key(&client);
    let server_key = key(&server_path);
    let mut server = Server::start_mode(&server_path, &client_key, true, true);
    let output = success(run(
        &client,
        &["discover", "--duration", "3", "--broadcast-only"],
    ));
    assert!(
        output.contains("DISCOVERED service=\"broadcast-"),
        "{output}"
    );
    assert!(output.contains(
        &windctl::parse_key(&server_key)
            .unwrap()
            .fingerprint()
            .to_hex()
    ));
    assert!(output.contains(&format!(
        ":{}",
        server.address.split(':').next_back().unwrap()
    )));
    server.finish();
}
