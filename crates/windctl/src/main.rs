use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{error::Error, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinSet};
use wind_core::{
    actions::{self, Action, ActionHost, Builtin, Executor, Policy},
    identity::{DeviceIdentity, PublicKey},
    network::{DiscoveryOptions, NetworkChanges, NetworkDiscovery},
    protocol::{
        cbor::{self, Value},
        Channel, DeviceKind, ErrorCode, RawMessage, MAX_STREAM_FRAME,
    },
    relay::RelayLink,
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
        /// Disable mDNS to test UDP broadcast discovery by itself.
        #[arg(long)]
        broadcast_only: bool,
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
        #[arg(long, requires = "advertise")]
        broadcast_only: bool,
        /// Echo raw clipboard/notify/media messages back to the sender (test peers only).
        #[arg(long)]
        echo: bool,
        /// Also wait on this relay (PROTOCOL.md §13) for each --trust device, e.g.
        /// wss://windconnect-relay.<subdomain>.workers.dev. Off unless given (D3).
        #[arg(long)]
        relay: Option<String>,
        /// Accept remote actions (PROTOCOL.md §9.2) from --trust devices, built-ins
        /// allowed, and print each instead of running it (ACTION_REQUEST lines).
        #[arg(long)]
        dry_run_actions: bool,
    },
    /// Ask the peer to run a remote action by id (PROTOCOL.md §9.2), or list
    /// what it would run. The peer decides by its own allowlist.
    Action {
        #[command(flatten)]
        peer: PeerArgs,
        /// Action id: lock, sleep, open-url or one of the peer's own x-... ids.
        #[arg(required_unless_present = "list", conflicts_with = "list")]
        id: Option<String>,
        /// The address for open-url (http or https only).
        #[arg(long, conflicts_with = "list")]
        url: Option<String>,
        /// List the peer's actions instead.
        #[arg(long)]
        list: bool,
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
    /// Relay to fall back to when the address does not answer (PROTOCOL.md §13). Off unless given.
    #[arg(long)]
    relay: Option<String>,
    /// auto: the address first, then the relay; lan: only the address; relay: only the relay.
    #[arg(long, value_enum, default_value_t = Route::Auto)]
    route: Route,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Route {
    Auto,
    Lan,
    Relay,
}

/// With a relay to fall back to, the address gets this long (a LAN answers in milliseconds).
const LAN_FIRST: Duration = Duration::from_millis(2000);

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
    let identity = Arc::new(load_identity(
        cli.identity
            .as_deref()
            .ok_or("--identity PATH is required")?,
    )?);
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
        Command::Connect(peer)
        | Command::Ping { peer, .. }
        | Command::Send { peer, .. }
        | Command::Action { peer, .. } => {
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
    let (tx, rx) = mpsc::channel(64);
    (routes_to(&tx), rx)
}

fn routes_to(tx: &mpsc::Sender<ChannelMessage>) -> Router {
    // CCS/1 DEC:one bounded queue per diagnostic channel; no production feature handlers.
    let mut router = Router::new();
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
    router
}

/// A session and, when it runs through the relay, the link it needs.
type Dialed = (Session, Option<RelayLink>);

/// CCS/1 FLOW:--route auto: the address (LAN_FIRST when a relay is given) -> the relay (§13)
async fn dial(
    endpoint: &Endpoint,
    identity: &DeviceIdentity,
    peer: &PeerArgs,
    local: LocalInfo,
    mut router: impl FnMut() -> Router,
) -> Result<Dialed> {
    let timeout = Duration::from_millis(peer.timeout_ms);
    let relay = match (&peer.relay, peer.route) {
        (None, Route::Relay) => return Err("--route relay needs --relay URL".into()),
        (relay, Route::Lan) | (relay @ None, _) => {
            let _ = relay;
            None
        }
        (Some(relay), _) => Some(relay),
    };
    let mut direct_error = None;
    if peer.route != Route::Relay {
        let wait = if relay.is_some() {
            timeout.min(LAN_FIRST)
        } else {
            timeout
        };
        match tokio::time::timeout(
            wait,
            Session::dial(
                endpoint,
                peer.address,
                peer.peer_key,
                local.clone(),
                router(),
            ),
        )
        .await
        {
            Ok(Ok((session, _))) => return Ok(connected(session, None, "lan")),
            Ok(Err(e)) if e.is_untrusted_peer() || relay.is_none() => return Err(e.into()),
            Ok(Err(e)) => direct_error = Some(e.to_string()),
            Err(_) if relay.is_none() => return Err("connection timed out".into()),
            Err(_) => direct_error = Some("no answer".into()),
        }
    }
    let relay = relay.expect("a relay route has a relay");
    if let Some(error) = direct_error {
        eprintln!("ROUTE lan failed ({error}); trying the relay");
    }
    let link = RelayLink::connect(relay, identity, peer.peer_key, endpoint.clone()).await?;
    let (session, _) = tokio::time::timeout(
        timeout,
        Session::dial(endpoint, link.addr(), peer.peer_key, local, router()),
    )
    .await
    .map_err(|_| "the peer did not answer through the relay (is it waiting there?)")??;
    Ok(connected(session, Some(link), "relay"))
}

fn connected(session: Session, link: Option<RelayLink>, route: &str) -> Dialed {
    println!(
        "CONNECTED name={:?} public_key={} route={route}",
        session.peer().hello.name,
        encode_hex(session.peer().key.as_bytes())
    );
    (session, link)
}

/// Keeps the PC reachable through the relay by `peer`: reconnects with
/// backoff whenever the link ends (§13; the relay closes idle sockets only
/// on redeploys or errors).
async fn wait_on_relay(
    url: String,
    identity: Arc<DeviceIdentity>,
    peer: PublicKey,
    endpoint: Endpoint,
) {
    let mut backoff =
        wind_core::transport::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    loop {
        match RelayLink::connect(&url, &identity, peer, endpoint.clone()).await {
            Ok(link) => {
                backoff.reset();
                println!("RELAY waiting peer={}", encode_hex(peer.as_bytes()));
                let reason = link.closed().await;
                eprintln!("relay link ended: {reason}");
            }
            Err(error) => eprintln!("relay: {error}"),
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

async fn execute(
    command: Command,
    identity: &Arc<DeviceIdentity>,
    endpoint: &Endpoint,
    local: LocalInfo,
) -> Result<()> {
    match command {
        Command::Identity => unreachable!(),
        Command::Discover {
            duration,
            broadcast_only,
        } => {
            let mut discovery = NetworkDiscovery::start_with_options(
                identity.public_key(),
                endpoint.clone(),
                Arc::new(NetworkChanges::default()),
                DiscoveryOptions {
                    mdns: !broadcast_only,
                    ..Default::default()
                },
            )?;
            let mut state = discovery.subscribe();
            let mut previous = std::collections::BTreeSet::new();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(duration);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    changed = state.changed() => {
                        changed?;
                        let snapshot = state.borrow_and_update().clone();
                        if let Some(error) = snapshot.error { eprintln!("mDNS: {error}"); }
                        let mut current = std::collections::BTreeSet::new();
                        for peer in snapshot.peers {
                            current.insert(peer.service_name.clone());
                            println!("DISCOVERED service={:?} fingerprint={} addresses={:?}", peer.service_name, encode_hex(&peer.fingerprint), peer.addresses);
                        }
                        for name in previous.difference(&current) { println!("REMOVED service={name:?}"); }
                        previous = current;
                    },
                }
            }
            discovery.shutdown().await;
            Ok(())
        }
        Command::Serve {
            duration,
            advertise,
            broadcast_only,
            echo,
            relay,
            trust,
            dry_run_actions,
            ..
        } => {
            let action_trust = dry_run_actions.then(|| {
                let pins = Arc::new(PinnedKeys::new());
                for peer in &trust {
                    pins.pin(*peer);
                }
                pins
            });
            let mut keepers = JoinSet::new();
            if let Some(url) = relay {
                wind_core::relay::check_url(&url)?;
                for peer in trust {
                    keepers.spawn(wait_on_relay(
                        url.clone(),
                        identity.clone(),
                        peer,
                        endpoint.clone(),
                    ));
                }
            }
            let mut discovery = if advertise {
                Some(NetworkDiscovery::start_with_options(
                    identity.public_key(),
                    endpoint.clone(),
                    Arc::new(NetworkChanges::default()),
                    DiscoveryOptions {
                        mdns: !broadcast_only,
                        ..Default::default()
                    },
                )?)
            } else {
                None
            };
            let mut discovery_state = discovery.as_ref().map(NetworkDiscovery::subscribe);
            println!("LISTENING address={}", endpoint.local_addr()?);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(duration);
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    incoming = endpoint.accept(), if peers.len() < 16 => match incoming {
                        Some(Ok(connection)) => {
                            let local = local.clone();
                            let action_trust = action_trust.clone();
                            peers.spawn(async move {
                                if let Err(error) = serve_peer(connection, local, echo, action_trust).await { eprintln!("peer: {error}"); }
                            });
                        },
                        Some(Err(error)) => eprintln!("handshake refused: {error}"),
                        None => break,
                    },
                    result = peers.join_next(), if !peers.is_empty() => {
                        if let Some(Err(error)) = result { eprintln!("peer task: {error}"); }
                    },
                    event = async {
                        match &mut discovery_state {
                            Some(state) => state.changed().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        event?;
                        if let Some(state) = &mut discovery_state {
                            if let Some(error) = &state.borrow_and_update().error { eprintln!("mDNS: {error}"); }
                        }
                    },
                }
            }
            endpoint.close();
            peers.abort_all();
            while peers.join_next().await.is_some() {}
            if let Some(discovery) = &mut discovery {
                discovery.shutdown().await;
            }
            Ok(())
        }
        Command::Action {
            peer,
            id,
            url,
            list,
        } => {
            let id = id.unwrap_or_default();
            if !list && !actions::is_action_id(&id) {
                return Err("action id: 1-64 of a-z, 0-9 and -".into());
            }
            let (session, _link) = dial(endpoint, identity, &peer, local, Router::new).await?;
            let result = if list {
                actions::list(&session).await.map(|list| {
                    for action in list {
                        println!("ACTIONS id={} label={:?}", action.id, action.label);
                    }
                })
            } else {
                actions::run(&session, &id, url.as_deref())
                    .await
                    .map(|status| println!("ACTION id={id} status={status:?}"))
            };
            session.goodbye(ErrorCode::GoingAway, None).await;
            result.map_err(Into::into)
        }
        Command::Connect(peer) => {
            let (session, _link) = dial(endpoint, identity, &peer, local, Router::new).await?;
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
            let (session, _link) = dial(endpoint, identity, &peer, local, Router::new).await?;
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
            let (tx, mut incoming) = mpsc::channel(64);
            let (session, _link) =
                dial(endpoint, identity, &peer, local, || routes_to(&tx)).await?;
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

/// Prints each allowed action instead of running it (`serve --dry-run-actions`).
struct PrintActions;

impl Executor for PrintActions {
    fn supports(&self, _: Builtin) -> bool {
        true
    }
    fn run(&self, action: &Action) -> std::result::Result<(), String> {
        match action {
            Action::Lock => println!("ACTION_REQUEST id=lock"),
            Action::Sleep => println!("ACTION_REQUEST id=sleep"),
            Action::OpenUrl(url) => println!("ACTION_REQUEST id=open-url url={url}"),
            Action::Custom(c) => println!("ACTION_REQUEST id={}", c.id),
        }
        Ok(())
    }
}

async fn serve_peer(
    connection: wind_core::transport::Connection,
    local: LocalInfo,
    echo: bool,
    action_trust: Option<Arc<PinnedKeys>>,
) -> Result<()> {
    let (mut router, mut incoming) = routes();
    let action_streams = action_trust.is_some().then(|| router.route_actions(4));
    let (session, mut events) = Session::accept(connection, local, router).await?;
    if let (Some(trust), Some(streams)) = (action_trust, action_streams) {
        let host = ActionHost::new(Arc::new(Policy::default()), Arc::new(PrintActions), trust);
        tokio::spawn(host.serve(session.clone(), streams));
    }
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
