use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{error::Error, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinSet};
use wind_core::{
    discovery::{Discovery, DiscoveryEvent},
    identity::{DeviceIdentity, PublicKey},
    protocol::{
        cbor::{self, Value},
        Channel, DeviceKind, ErrorCode, RawMessage, MAX_STREAM_FRAME,
    },
    session::{ChannelMessage, LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
use windctl::{decode_hex, encode_hex, parse_key, LatencyStats};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(
    version,
    about = "WindConnect diagnostic CLI (mutual TLS, explicit peer keys)"
)]
struct Cli {
    /// Persistent identity file (DPAPI on Windows, private PKCS#8 file on Unix).
    #[arg(long, global = true)]
    identity: Option<PathBuf>,
    #[arg(long, global = true, default_value = "windctl")]
    name: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create/load an identity and print its PUBLIC key and fingerprint.
    Identity,
    /// Browse mDNS advertisements (untrusted address hints).
    Discover {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600))]
        duration: u64,
    },
    /// Accept trusted peers, answer control pings, and log raw diagnostic messages.
    Serve {
        #[arg(long, default_value = "0.0.0.0:47820")]
        bind: SocketAddr,
        #[arg(long, required = true, value_parser = parse_key)]
        trust: Vec<PublicKey>,
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=86400))]
        duration: u64,
        /// Advertise the bound QUIC port using mDNS.
        #[arg(long)]
        advertise: bool,
        /// Echo raw clipboard/notify/media messages back to the sender (test peers only).
        #[arg(long)]
        echo: bool,
    },
    /// Authenticate, exchange Hello/HelloAck, perform one Ping/Pong, then disconnect.
    Connect(PeerArgs),
    /// Measure sequential control Ping/Pong RTT, reporting successful-sample p50/p95.
    Ping {
        #[command(flatten)]
        peer: PeerArgs,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=10000))]
        count: u32,
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(0..=100))]
        warmup: u32,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(0..=60000))]
        interval_ms: u64,
    },
    /// Send a diagnostic raw message. Does not define a production feature schema.
    Send {
        #[command(flatten)]
        peer: PeerArgs,
        #[arg(long, value_enum, default_value_t = DiagnosticChannel::Clipboard)]
        channel: DiagnosticChannel,
        #[arg(long, default_value_t = 65000)]
        message_type: u64,
        /// Encode {0: text} as the message body.
        #[arg(
            long,
            conflicts_with = "body_hex",
            required_unless_present = "body_hex"
        )]
        text: Option<String>,
        /// A deterministic CBOR map body in hex (not an entire frame).
        #[arg(long, conflicts_with = "text", required_unless_present = "text")]
        body_hex: Option<String>,
        /// Require a matching echo from a windctl serve --echo peer.
        #[arg(long)]
        expect_echo: bool,
    },
}

#[derive(Args)]
struct PeerArgs {
    #[arg(long)]
    address: SocketAddr,
    /// Verified PUBLIC key, 64 hexadecimal characters; never a discovered fingerprint.
    #[arg(long, value_parser = parse_key)]
    peer_key: PublicKey,
    #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u64).range(1..=120000))]
    timeout_ms: u64,
}

#[derive(Clone, Copy, ValueEnum)]
enum DiagnosticChannel {
    Clipboard,
    Notify,
    Media,
}

impl From<DiagnosticChannel> for Channel {
    fn from(value: DiagnosticChannel) -> Self {
        match value {
            DiagnosticChannel::Clipboard => Self::Clipboard,
            DiagnosticChannel::Notify => Self::Notify,
            DiagnosticChannel::Media => Self::Media,
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("windctl: {error}");
        std::process::exit(1);
    }
}

fn load_identity(path: &std::path::Path) -> Result<DeviceIdentity> {
    #[cfg(windows)]
    let store = wind_core::identity::DpapiKeyStore::new(path);
    #[cfg(not(windows))]
    let store = wind_core::identity::FileKeyStore::new(path);
    Ok(DeviceIdentity::load_or_create(&store)?)
}

async fn run(cli: Cli) -> Result<()> {
    if cli.name.is_empty() || cli.name.len() > 64 {
        return Err("name must contain 1–64 UTF-8 bytes".into());
    }
    let identity = load_identity(
        cli.identity
            .as_deref()
            .ok_or("--identity PATH is required")?,
    )?;
    println!(
        "IDENTITY public_key={} fingerprint={}",
        encode_hex(identity.public_key().as_bytes()),
        identity.fingerprint().to_hex()
    );
    if matches!(cli.command, Command::Identity) {
        return Ok(());
    }
    let trust = Arc::new(PinnedKeys::new());
    let bind = match &cli.command {
        Command::Serve {
            bind, trust: peers, ..
        } => {
            for peer in peers {
                trust.pin(*peer);
            }
            *bind
        }
        Command::Connect(peer) | Command::Ping { peer, .. } | Command::Send { peer, .. } => {
            trust.pin(peer.peer_key);
            if peer.address.is_ipv6() {
                "[::]:0"
            } else {
                "0.0.0.0:0"
            }
            .parse()?
        }
        _ => "0.0.0.0:0".parse()?,
    };
    let endpoint = Endpoint::bind(bind, &identity, trust)?;
    let local = LocalInfo {
        name: cli.name,
        kind: DeviceKind::Computer,
        app: Some(env!("CARGO_PKG_VERSION").into()),
    };
    let result = tokio::select! {
        result = execute(cli.command, &identity, &endpoint, local) => result,
        signal = tokio::signal::ctrl_c() => signal.map_err(Into::into),
    };
    endpoint.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), endpoint.wait_idle()).await;
    result
}

fn routes() -> (Router, mpsc::Receiver<ChannelMessage>) {
    // CCS/1 DEC:one bounded queue per diagnostic channel; no production feature handlers.
    let mut router = Router::new();
    let (tx, rx) = mpsc::channel(64);
    for channel in [Channel::Clipboard, Channel::Notify, Channel::Media] {
        let mut messages = router.route(channel, 1, 16);
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                if tx.send(message).await.is_err() {
                    break;
                }
            }
        });
    }
    (router, rx)
}

async fn dial(
    endpoint: &Endpoint,
    peer: &PeerArgs,
    local: LocalInfo,
    router: Router,
) -> Result<Session> {
    let (session, _) = tokio::time::timeout(
        Duration::from_millis(peer.timeout_ms),
        Session::dial(endpoint, peer.address, peer.peer_key, local, router),
    )
    .await??;
    println!(
        "CONNECTED name={:?} public_key={}",
        session.peer().hello.name,
        encode_hex(session.peer().key.as_bytes())
    );
    Ok(session)
}

async fn execute(
    command: Command,
    identity: &DeviceIdentity,
    endpoint: &Endpoint,
    local: LocalInfo,
) -> Result<()> {
    match command {
        Command::Identity => unreachable!(),
        Command::Discover { duration } => {
            let discovery =
                Discovery::start(&identity.public_key(), endpoint.local_addr()?.port())?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(duration);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    event = discovery.next() => match event? {
                        DiscoveryEvent::Resolved(peer) => println!("DISCOVERED service={:?} fingerprint={} addresses={:?}", peer.service_name, encode_hex(&peer.fingerprint), peer.addresses),
                        DiscoveryEvent::Removed(name) => println!("REMOVED service={name:?}"),
                    },
                    event = discovery.monitor().recv_async() => { eprintln!("mDNS: {:?}", event?); },
                }
            }
            Ok(())
        }
        Command::Serve {
            duration,
            advertise,
            echo,
            ..
        } => {
            let discovery = if advertise {
                Some(Discovery::start(
                    &identity.public_key(),
                    endpoint.local_addr()?.port(),
                )?)
            } else {
                None
            };
            println!("LISTENING address={}", endpoint.local_addr()?);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(duration);
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    incoming = endpoint.accept(), if peers.len() < 16 => match incoming {
                        Some(Ok(connection)) => {
                            let local = local.clone();
                            peers.spawn(async move {
                                if let Err(error) = serve_peer(connection, local, echo).await { eprintln!("peer: {error}"); }
                            });
                        },
                        Some(Err(error)) => eprintln!("handshake refused: {error}"),
                        None => break,
                    },
                    result = peers.join_next(), if !peers.is_empty() => {
                        if let Some(Err(error)) = result { eprintln!("peer task: {error}"); }
                    },
                    event = async {
                        match &discovery {
                            Some(discovery) => discovery.monitor().recv_async().await,
                            None => std::future::pending().await,
                        }
                    } => eprintln!("mDNS: {:?}", event?),
                }
            }
            endpoint.close();
            peers.abort_all();
            while peers.join_next().await.is_some() {}
            Ok(())
        }
        Command::Connect(peer) => {
            let session = dial(endpoint, &peer, local, Router::new()).await?;
            let result = session.ping().await;
            session.goodbye(ErrorCode::GoingAway, None).await;
            println!("READY rtt_ms={:.3}", result?.as_secs_f64() * 1000.0);
            Ok(())
        }
        Command::Ping {
            peer,
            count,
            warmup,
            interval_ms,
        } => {
            let session = dial(endpoint, &peer, local, Router::new()).await?;
            let result = measure(&session, count, warmup, interval_ms).await;
            session.goodbye(ErrorCode::GoingAway, None).await;
            result
        }
        Command::Send {
            peer,
            channel,
            message_type,
            text,
            body_hex,
            expect_echo,
        } => {
            let channel = channel.into();
            let body = if let Some(text) = text {
                if text.len() > MAX_STREAM_FRAME {
                    return Err("text is larger than a stream frame".into());
                }
                Value::Map(vec![(Value::Int(0), Value::Text(text))])
            } else {
                cbor::decode(&decode_hex(
                    &body_hex.unwrap_or_default(),
                    MAX_STREAM_FRAME,
                )?)
                .map_err(io::Error::other)?
            };
            let message = RawMessage::new(message_type, body);
            wind_core::protocol::encode_raw_frame(channel, &message)?;
            let (router, mut incoming) = routes();
            let session = dial(endpoint, &peer, local, router).await?;
            let result = async {
                session.send(channel, &message).await?;
                session.finish(channel).await?;
                if expect_echo {
                    tokio::time::timeout(Duration::from_millis(peer.timeout_ms), async {
                        loop {
                            let received =
                                incoming.recv().await.ok_or("peer closed without echo")?;
                            if received.channel == channel && received.message == message {
                                break;
                            }
                        }
                        Ok::<_, Box<dyn Error + Send + Sync>>(())
                    })
                    .await??;
                } else {
                    session.ping().await?;
                }
                println!(
                    "SENT channel={channel:?} type={message_type} echo_verified={expect_echo}"
                );
                Ok(())
            }
            .await;
            session.goodbye(ErrorCode::GoingAway, None).await;
            result
        }
    }
}

async fn serve_peer(
    connection: wind_core::transport::Connection,
    local: LocalInfo,
    echo: bool,
) -> Result<()> {
    let (router, mut incoming) = routes();
    let (session, mut events) = Session::accept(connection, local, router).await?;
    println!(
        "ACCEPTED name={:?} public_key={}",
        session.peer().hello.name,
        encode_hex(session.peer().key.as_bytes())
    );
    loop {
        tokio::select! {
            _ = session.closed() => break,
            event = events.recv() => match event {
                Some(event) => eprintln!("session: {event:?}"),
                None => break,
            },
            message = incoming.recv() => {
                let Some(message) = message else { break; };
                let mut bytes = Vec::new();
                cbor::encode(&message.message.body, &mut bytes);
                println!("RECEIVED channel={:?} type={} body_hex={}", message.channel, message.message.ty, encode_hex(&bytes));
                if echo { session.send(message.channel, &message.message).await?; }
            },
        }
    }
    Ok(())
}

async fn measure(session: &Session, count: u32, warmup: u32, interval_ms: u64) -> Result<()> {
    for _ in 0..warmup {
        session.ping().await?;
    }
    let mut samples = Vec::with_capacity(count as usize);
    for index in 0..count {
        match session.ping().await {
            Ok(rtt) => {
                println!(
                    "PING seq={} rtt_ms={:.3}",
                    index + 1,
                    rtt.as_secs_f64() * 1000.0
                );
                samples.push(rtt);
            }
            Err(error) => eprintln!("PING seq={} failed={error}", index + 1),
        }
        if index + 1 < count {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }
    let Some(stats) = LatencyStats::from_samples(samples) else {
        return Err("no successful ping samples".into());
    };
    println!("SUMMARY sent={count} received={} lost={} warmup={warmup} p50_ms={:.3} p95_ms={:.3} min_ms={:.3} max_ms={:.3}",
        stats.samples, count as usize-stats.samples, stats.p50.as_secs_f64()*1000.0,
        stats.p95.as_secs_f64()*1000.0, stats.min.as_secs_f64()*1000.0, stats.max.as_secs_f64()*1000.0);
    Ok(())
}
