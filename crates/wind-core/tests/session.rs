//! Sessions, streams and the router (WC-11).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::cbor::Value;
use wind_core::protocol::{
    Capability, Channel, DeviceKind, ErrorCode, RawMessage, ALPN, MAX_STREAM_FRAME,
};
use wind_core::session::{
    write_raw, ChannelMessage, LocalInfo, Router, Session, SessionError, SessionEvent,
};
use wind_core::transport::{Connection, Endpoint, PinnedKeys, MAX_UNI_STREAMS};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

/// Two connected, paired endpoints: (client side, server side). The
/// endpoints are kept for the whole test.
async fn connections() -> (Connection, Connection) {
    let (ia, ib) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pa, pb) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pa.pin(ib.public_key());
    pb.pin(ia.public_key());
    let a = Endpoint::bind(LOCALHOST, &ia, pa).unwrap();
    let b = Endpoint::bind(LOCALHOST, &ib, pb).unwrap();
    let addr = b.local_addr().unwrap();
    let accept = tokio::spawn(async move {
        let conn = b.accept().await.unwrap().unwrap();
        (conn, b)
    });
    let client = a.connect(addr, ib.public_key()).await.unwrap();
    let (server, b) = accept.await.unwrap();
    // Tests are short: keep both endpoints until the process ends.
    std::mem::forget(a);
    std::mem::forget(b);
    (client, server)
}

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: Some("test".into()),
    }
}

struct Pair {
    client: Session,
    client_events: mpsc::UnboundedReceiver<SessionEvent>,
    server: Session,
    server_events: mpsc::UnboundedReceiver<SessionEvent>,
}

async fn sessions(client_router: Router, server_router: Router) -> Pair {
    let (c, s) = connections().await;
    let server = tokio::spawn(Session::accept(s, info("server"), server_router));
    let (client, client_events) = Session::connect(c, info("client"), client_router)
        .await
        .unwrap();
    let (server, server_events) = server.await.unwrap().unwrap();
    Pair {
        client,
        client_events,
        server,
        server_events,
    }
}

fn msg(ty: u64, n: u64) -> RawMessage {
    RawMessage::new(ty, Value::Map(vec![(Value::Int(0), Value::Int(n.into()))]))
}

fn n_of(m: &ChannelMessage) -> u64 {
    match &m.message.body {
        Value::Map(p) => match p[0].1 {
            Value::Int(n) => n as u64,
            _ => panic!(),
        },
        _ => panic!(),
    }
}

async fn recv(rx: &mut mpsc::Receiver<ChannelMessage>) -> ChannelMessage {
    tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn hello_exchange_announces_routed_channels() {
    let mut client_router = Router::new();
    let _n = client_router.route(Channel::Notify, 1, 8);
    let mut server_router = Router::new();
    let _c = server_router.route(Channel::Clipboard, 1, 8);
    let _i = server_router.route_input(8);
    let _f = server_router.route_files(1, 8);
    let p = sessions(client_router, server_router).await;

    let caps = |v: &[&str]| -> Vec<Capability> {
        v.iter().map(|s| Capability::parse(s).unwrap()).collect()
    };
    assert_eq!(p.client.peer().hello.name, "server");
    assert_eq!(p.server.peer().hello.name, "client");
    assert_eq!(
        p.client.peer_capabilities(),
        caps(&["clipboard/1", "files/1", "input/1"])
    );
    assert_eq!(p.server.peer_capabilities(), caps(&["notify/1"]));
    assert_eq!(p.client.peer().key, p.client.connection().peer_key());
    assert_eq!(p.server.peer().hello.app.as_deref(), Some("test"));
}

#[tokio::test]
async fn messages_reach_their_module_in_order_across_streams() {
    // Small module queues and a sender far ahead of them: nothing may be
    // lost or reordered while finished streams wait to be read.
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 4);
    let mut notify = server_router.route(Channel::Notify, 1, 4);
    let p = sessions(Router::new(), server_router).await;

    let sender = p.client.clone();
    let send = tokio::spawn(async move {
        for n in 0..60u64 {
            sender.send(Channel::Clipboard, &msg(1, n)).await.unwrap();
            sender.send(Channel::Notify, &msg(2, n)).await.unwrap();
            // New streams now and then (§3.1): order must hold across them.
            if n % 7 == 6 {
                sender.finish(Channel::Clipboard).await.unwrap();
            }
            if n % 11 == 10 {
                sender.finish(Channel::Notify).await.unwrap();
            }
        }
    });
    for n in 0..60 {
        let c = recv(&mut clip).await;
        assert_eq!(
            (c.channel, c.message.ty, n_of(&c)),
            (Channel::Clipboard, 1, n)
        );
        let m = recv(&mut notify).await;
        assert_eq!((m.channel, m.message.ty, n_of(&m)), (Channel::Notify, 2, n));
    }
    send.await.unwrap();
}

/// A message numbered `n` with `bytes` of payload.
fn big_msg(ty: u64, n: u64, bytes: usize) -> RawMessage {
    RawMessage::new(
        ty,
        Value::Map(vec![
            (Value::Int(0), Value::Int(n.into())),
            (Value::Int(1), Value::Bytes(vec![0x5a; bytes])),
        ]),
    )
}

#[tokio::test]
async fn fast_sender_waits_for_a_slow_module_and_loses_nothing() {
    // §3.1 rule 5: one finished stream per message, far more than the
    // stream limit plus what the read-ahead budget holds (64 KiB messages:
    // about 16 fit in 1 MiB), and a module that doesn't read yet. The
    // sender must wait for MAX_STREAMS, not be refused, and every message
    // must arrive.
    let total = u64::from(MAX_UNI_STREAMS) + 50;
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 1);
    let p = sessions(Router::new(), server_router).await;

    let sent = Arc::new(AtomicU64::new(0));
    let (sender, counter) = (p.client.clone(), sent.clone());
    let send = tokio::spawn(async move {
        for n in 0..total {
            sender
                .send(Channel::Clipboard, &big_msg(1, n, 64 * 1024))
                .await
                .unwrap();
            sender.finish(Channel::Clipboard).await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    // The sender runs into the limit and waits there.
    let mut last = u64::MAX;
    loop {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = sent.load(Ordering::SeqCst);
        if now == last {
            break;
        }
        last = now;
    }
    assert!(last < total, "the sender never waited ({last} of {total})");
    assert!(!send.is_finished());

    for n in 0..total {
        assert_eq!(n_of(&recv(&mut clip).await), n);
    }
    tokio::time::timeout(T, send).await.unwrap().unwrap();
}

#[tokio::test]
async fn slow_module_of_one_channel_does_not_hold_back_another() {
    // WN-81, §3.1 rule 5.6: more finished clipboard streams than the whole
    // stream limit wait on a module that doesn't read, yet a notify message
    // still gets through, and no clipboard message is lost or reordered.
    let total = u64::from(MAX_UNI_STREAMS) + 50;
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 1);
    let mut notify = server_router.route(Channel::Notify, 1, 8);
    let p = sessions(Router::new(), server_router).await;

    let sender = p.client.clone();
    let clipboard_done = tokio::spawn(async move {
        for n in 0..total {
            sender.send(Channel::Clipboard, &msg(1, n)).await.unwrap();
            sender.finish(Channel::Clipboard).await.unwrap();
        }
    });
    // Every clipboard stream got a place: the sender never waited.
    tokio::time::timeout(T, clipboard_done)
        .await
        .expect("clipboard streams were held by the stream limit")
        .unwrap();

    p.client.send(Channel::Notify, &msg(2, 7)).await.unwrap();
    p.client.finish(Channel::Notify).await.unwrap();
    let m = recv(&mut notify).await;
    assert_eq!((m.channel, n_of(&m)), (Channel::Notify, 7));

    for n in 0..total {
        assert_eq!(n_of(&recv(&mut clip).await), n);
    }
}

#[tokio::test]
async fn streams_opened_without_fin_are_held_by_the_stream_limit() {
    // A sender breaking §3.1 rule 1 (streams left open) is held back by
    // MAX_STREAMS: none of its streams is refused, the next open waits,
    // and finishing one lets it go on.
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn().clone();

    let mut open = Vec::new();
    for n in 0..u64::from(MAX_UNI_STREAMS) {
        let mut s = tokio::time::timeout(T, q.open_uni())
            .await
            .unwrap()
            .unwrap();
        let mut data = vec![1, 1];
        data.extend(frame(&msg(1, n)));
        s.write_all(&data).await.unwrap();
        open.push(s);
    }
    // The first stream's message arrives; the rest wait for it to end.
    assert_eq!(n_of(&recv(&mut clip).await), 0);

    assert!(
        tokio::time::timeout(Duration::from_millis(500), q.open_uni())
            .await
            .is_err(),
        "a stream past the limit was granted"
    );
    let last = open.len() - 1;
    for i in [0, 1, last] {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), open[i].stopped())
                .await
                .is_err(),
            "stream {i} was refused"
        );
    }

    // Finishing streams frees their places: the next open goes on. quinn
    // grants new streams in batches, once over 1/8 of the limit is free.
    let freed = MAX_UNI_STREAMS as usize / 8 + 1;
    for (n, s) in open.iter_mut().enumerate().take(freed) {
        s.finish().unwrap();
        assert_eq!(n_of(&recv(&mut clip).await), n as u64 + 1);
    }
    tokio::time::timeout(T, q.open_uni())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn later_stream_of_a_channel_waits_for_the_earlier_one() {
    // §3.1 rule 3: stream B (complete) is not delivered before stream A of
    // the same channel ends.
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn();

    let mut a = q.open_uni().await.unwrap();
    let mut first = vec![1, 1];
    first.extend(frame(&msg(1, 100)));
    a.write_all(&first).await.unwrap();
    let mut b = q.open_uni().await.unwrap();
    let mut second = vec![1, 1];
    second.extend(frame(&msg(1, 200)));
    b.write_all(&second).await.unwrap();
    b.finish().unwrap();

    assert_eq!(n_of(&recv(&mut clip).await), 100);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), clip.recv())
            .await
            .is_err(),
        "B was delivered before A ended"
    );
    a.finish().unwrap();
    assert_eq!(n_of(&recv(&mut clip).await), 200);
}

fn frame(m: &RawMessage) -> Vec<u8> {
    wind_core::protocol::encode_raw_frame(Channel::Clipboard, m).unwrap()
}

async fn stopped_with(stream: &mut quinn::SendStream) -> Option<u64> {
    tokio::time::timeout(T, stream.stopped())
        .await
        .unwrap()
        .unwrap()
        .map(|c| c.into_inner())
}

#[tokio::test]
async fn bad_uni_streams_are_stopped_and_the_connection_goes_on() {
    let mut server_router = Router::new();
    let _clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn().clone();

    for (header, code) in [
        ([1u8, 0x3f], ErrorCode::UnknownChannel),
        ([1, 0x40], ErrorCode::UnknownChannel),
        ([2, 1], ErrorCode::UnsupportedVersion),
        ([1, 3], ErrorCode::CapabilityMissing), // media: not routed
    ] {
        let mut s = q.open_uni().await.unwrap();
        s.write_all(&header).await.unwrap();
        s.write_all(&frame(&msg(1, 1))).await.unwrap();
        assert_eq!(stopped_with(&mut s).await, Some(code.code()), "{header:?}");
    }
    // The session's own check, before anything is sent.
    assert!(matches!(
        p.client.send(Channel::Media, &msg(1, 1)).await,
        Err(SessionError::CapabilityMissing(Channel::Media))
    ));
    // Still connected.
    p.client.ping().await.unwrap();
}

#[tokio::test]
async fn stream_without_header_is_dropped_two_seconds_after_a_later_one() {
    // §3.1 rule 4.
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn().clone();

    let mut stuck = q.open_uni().await.unwrap();
    stuck.write_all(&[1]).await.unwrap(); // half a header
    let mut later = q.open_uni().await.unwrap();
    let mut bytes = vec![1, 1];
    bytes.extend(frame(&msg(1, 7)));
    later.write_all(&bytes).await.unwrap();
    later.finish().unwrap();

    let start = Instant::now();
    assert_eq!(n_of(&recv(&mut clip).await), 7);
    let waited = start.elapsed();
    assert!(waited >= Duration::from_millis(1800), "{waited:?}");
    assert_eq!(
        stopped_with(&mut stuck).await,
        Some(ErrorCode::ProtocolViolation.code())
    );
}

#[tokio::test]
async fn second_control_stream_closes_the_connection() {
    let p = sessions(Router::new(), Router::new()).await;
    let (mut s, _r) = p.client.connection().quinn().open_bi().await.unwrap();
    s.write_all(&[1, 0]).await.unwrap();
    let closed = tokio::time::timeout(T, p.client.closed()).await.unwrap();
    match closed {
        quinn::ConnectionError::ApplicationClosed(c) => {
            assert_eq!(
                c.error_code.into_inner(),
                ErrorCode::ProtocolViolation.code()
            )
        }
        other => panic!("{other}"),
    }
}

#[tokio::test]
async fn uni_channel_on_a_bidi_stream_closes_the_connection() {
    let mut server_router = Router::new();
    let _clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let (mut s, _r) = p.client.connection().quinn().open_bi().await.unwrap();
    s.write_all(&[1, 1]).await.unwrap();
    let closed = tokio::time::timeout(T, p.client.closed()).await.unwrap();
    assert!(
        matches!(&closed, quinn::ConnectionError::ApplicationClosed(c)
            if c.error_code.into_inner() == ErrorCode::ProtocolViolation.code()),
        "{closed}"
    );
}

#[tokio::test]
async fn files_stream_reaches_its_module_and_answers() {
    let mut server_router = Router::new();
    let mut files = server_router.route_files(1, 4);
    let p = sessions(Router::new(), server_router).await;

    let mut out = p.client.open_files().await.unwrap();
    write_raw(&mut out.send, Channel::Files, &msg(0, 42))
        .await
        .unwrap();
    out.send.finish().unwrap();

    let mut incoming = tokio::time::timeout(T, files.recv())
        .await
        .unwrap()
        .unwrap();
    let got = incoming.recv.next_raw().await.unwrap().unwrap();
    assert_eq!(got, msg(0, 42));
    assert!(incoming.recv.next_raw().await.unwrap().is_none());
    write_raw(&mut incoming.send, Channel::Files, &msg(1, 43))
        .await
        .unwrap();
    incoming.send.finish().unwrap();

    // The reply has no header (§3).
    assert_eq!(out.recv.next_raw().await.unwrap().unwrap(), msg(1, 43));
}

#[tokio::test]
async fn files_refused_when_not_routed() {
    let p = sessions(Router::new(), Router::new()).await;
    assert!(matches!(
        p.client.open_files().await,
        Err(SessionError::CapabilityMissing(Channel::Files))
    ));
    let (mut s, _r) = p.client.connection().quinn().open_bi().await.unwrap();
    s.write_all(&[1, 5]).await.unwrap();
    assert_eq!(
        stopped_with(&mut s).await,
        Some(ErrorCode::CapabilityMissing.code())
    );
}

#[tokio::test]
async fn ping_and_goodbye() {
    let mut p = sessions(Router::new(), Router::new()).await;
    let rtt = p.client.ping().await.unwrap();
    assert!(rtt < Duration::from_secs(1));
    p.server.ping().await.unwrap();

    p.client
        .goodbye(ErrorCode::GoingAway, Some("bye".into()))
        .await;
    let event = tokio::time::timeout(T, p.server_events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&event, SessionEvent::Goodbye(g) if g.code == ErrorCode::GoingAway),
        "{event:?}"
    );
    let closed = tokio::time::timeout(T, p.server.closed()).await.unwrap();
    assert!(
        matches!(&closed, quinn::ConnectionError::ApplicationClosed(c)
            if c.error_code.into_inner() == ErrorCode::GoingAway.code()),
        "{closed}"
    );
    let _ = &mut p.client_events;
}

#[tokio::test]
async fn no_hello_closes_with_hello_timeout() {
    let (client, server) = connections().await;
    let start = Instant::now();
    let err = Session::accept(server, info("server"), Router::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, SessionError::Protocol(ErrorCode::HelloTimeout, _)),
        "{err}"
    );
    assert!(start.elapsed() >= Duration::from_millis(4900));
    let closed = tokio::time::timeout(T, client.quinn().closed())
        .await
        .unwrap();
    assert!(
        matches!(&closed, quinn::ConnectionError::ApplicationClosed(c)
            if c.error_code.into_inner() == ErrorCode::HelloTimeout.code()),
        "{closed}"
    );
}

#[test]
fn alpn_and_priorities_follow_the_spec() {
    use wind_core::session::send_priority;
    assert_eq!(ALPN, b"windconnect/1");
    let order = [
        Channel::Control,
        Channel::Input,
        Channel::Clipboard,
        Channel::Notify,
        Channel::Media,
        Channel::Files,
    ];
    for w in order.windows(2) {
        assert!(send_priority(w[0]) > send_priority(w[1]), "{w:?}");
    }
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// Acceptance (WC-11): a large transfer doesn't delay `control` by more
/// than 10 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_transfer_does_not_delay_control_by_more_than_10_ms() {
    let mut server_router = Router::new();
    let mut files = server_router.route_files(1, 4);
    let p = sessions(Router::new(), server_router).await;

    // The receiving module drains the transfer as fast as it can.
    let received = Arc::new(AtomicU64::new(0));
    let counter = received.clone();
    let sink = tokio::spawn(async move {
        let mut s = files.recv().await.unwrap();
        while let Ok(Some(m)) = s.recv.next_raw().await {
            if let Value::Map(p) = &m.body {
                if let Value::Bytes(b) = &p[0].1 {
                    counter.fetch_add(b.len() as u64, Ordering::Relaxed);
                }
            }
        }
    });

    async fn pings(s: &Session, n: usize) -> Vec<f64> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(s.ping().await.unwrap().as_secs_f64() * 1000.0);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        v
    }

    let mut idle = pings(&p.client, 100).await;

    let mut out = p.client.open_files().await.unwrap();
    let chunk = RawMessage::new(
        0,
        Value::Map(vec![(
            Value::Int(0),
            Value::Bytes(vec![0x5a; MAX_STREAM_FRAME - 16]),
        )]),
    );
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_flag = stop.clone();
    let transfer = tokio::spawn(async move {
        while !stop_flag.load(Ordering::Relaxed) {
            write_raw(&mut out.send, Channel::Files, &chunk)
                .await
                .unwrap();
        }
        out.send.finish().unwrap();
    });
    // Let the transfer reach full speed.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = received.load(Ordering::Relaxed);
    let t0 = Instant::now();
    let mut loaded = pings(&p.client, 200).await;
    let moved = received.load(Ordering::Relaxed) - before;
    let secs = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    transfer.await.unwrap();
    sink.await.unwrap();

    let idle_p50 = percentile(&mut idle, 0.5);
    let loaded_p50 = percentile(&mut loaded, 0.5);
    let loaded_p99 = percentile(&mut loaded, 0.99);
    let loaded_max = percentile(&mut loaded, 1.0);
    let mbps = moved as f64 * 8.0 / secs / 1e6;
    println!(
        "control RTT idle p50 {idle_p50:.2} ms; under a {mbps:.0} Mbit/s transfer \
         p50 {loaded_p50:.2} ms, p99 {loaded_p99:.2} ms, max {loaded_max:.2} ms"
    );
    // The load is real: the transfer kept moving while we measured.
    assert!(moved > 50_000_000, "only {moved} B moved");
    // The delay added by the transfer stays within 10 ms (p99).
    assert!(
        loaded_p99 - idle_p50 <= 10.0,
        "control delayed by {:.2} ms",
        loaded_p99 - idle_p50
    );
}

// ---------------------------------------------------------------------------
// Changing what this side accepts mid-connection (§6, rule 5; WN-74).

fn caps(v: &[&str]) -> Vec<Capability> {
    v.iter().map(|s| Capability::parse(s).unwrap()).collect()
}

/// The next `Capabilities` the session reports.
async fn next_caps(events: &mut mpsc::UnboundedReceiver<SessionEvent>) -> Vec<Capability> {
    loop {
        match tokio::time::timeout(T, events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            SessionEvent::Capabilities(c) => return c,
            SessionEvent::Closed(why) => panic!("closed: {why}"),
            _ => {}
        }
    }
}

async fn nothing_on(rx: &mut mpsc::Receiver<ChannelMessage>) {
    if let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
        panic!("delivered while withdrawn: {m:?}");
    }
}

#[tokio::test]
async fn withdrawn_channel_stops_delivery_and_reenabling_brings_it_back_in_order() {
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 64);
    let _notify = server_router.route(Channel::Notify, 1, 8);
    let mut p = sessions(Router::new(), server_router).await;
    assert_eq!(
        p.server.accepted_capabilities(),
        caps(&["clipboard/1", "notify/1"])
    );

    for n in 0..5 {
        p.client.send(Channel::Clipboard, &msg(1, n)).await.unwrap();
    }
    for n in 0..5 {
        assert_eq!(n_of(&recv(&mut clip).await), n);
    }

    // Withdraw: the peer hears the full new list.
    assert!(p
        .server
        .set_accepted(Channel::Clipboard, false)
        .await
        .unwrap());
    assert!(!p
        .server
        .set_accepted(Channel::Clipboard, false)
        .await
        .unwrap());
    assert_eq!(p.server.accepted_capabilities(), caps(&["notify/1"]));
    assert_eq!(next_caps(&mut p.client_events).await, caps(&["notify/1"]));
    assert_eq!(p.client.peer_capabilities(), caps(&["notify/1"]));
    // The sender's stream is reset and the next send is refused.
    assert!(matches!(
        p.client.send(Channel::Clipboard, &msg(1, 50)).await,
        Err(SessionError::CapabilityMissing(Channel::Clipboard))
    ));
    nothing_on(&mut clip).await;

    // Re-enable: a new stream, later messages in order, none from before.
    assert!(p
        .server
        .set_accepted(Channel::Clipboard, true)
        .await
        .unwrap());
    assert_eq!(
        next_caps(&mut p.client_events).await,
        caps(&["clipboard/1", "notify/1"])
    );
    for n in 100..120 {
        p.client.send(Channel::Clipboard, &msg(1, n)).await.unwrap();
    }
    for n in 100..120 {
        assert_eq!(n_of(&recv(&mut clip).await), n);
    }
    nothing_on(&mut clip).await;
}

#[tokio::test]
async fn withdrawal_stops_open_streams_and_refuses_new_ones_until_reenabled() {
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn().clone();

    // A stream left open, as a sender keeps one per channel (§3.1).
    let mut open = q.open_uni().await.unwrap();
    let mut first = vec![1, 1];
    first.extend(frame(&msg(1, 1)));
    open.write_all(&first).await.unwrap();
    assert_eq!(n_of(&recv(&mut clip).await), 1);

    p.server
        .set_accepted(Channel::Clipboard, false)
        .await
        .unwrap();
    assert_eq!(
        stopped_with(&mut open).await,
        Some(ErrorCode::Cancelled.code()),
        "its open stream is stopped with CANCELLED"
    );
    // A sender that hasn't heard yet opens a new stream: refused (§3 rule 3).
    let mut late = q.open_uni().await.unwrap();
    let mut data = vec![1, 1];
    data.extend(frame(&msg(1, 2)));
    late.write_all(&data).await.unwrap();
    assert_eq!(
        stopped_with(&mut late).await,
        Some(ErrorCode::CapabilityMissing.code())
    );
    nothing_on(&mut clip).await;

    p.server
        .set_accepted(Channel::Clipboard, true)
        .await
        .unwrap();
    let mut fresh = q.open_uni().await.unwrap();
    let mut data = vec![1, 1];
    data.extend(frame(&msg(1, 3)));
    fresh.write_all(&data).await.unwrap();
    fresh.finish().unwrap();
    assert_eq!(n_of(&recv(&mut clip).await), 3);
}

#[tokio::test]
async fn stream_queued_before_a_withdrawal_never_arrives_after_reenabling() {
    // A holds the channel; B waits behind it (§3.1 rule 3). The channel is
    // withdrawn and enabled again before B's turn: B belongs to the time
    // before the withdrawal and must not be delivered.
    let mut server_router = Router::new();
    let mut clip = server_router.route(Channel::Clipboard, 1, 8);
    let p = sessions(Router::new(), server_router).await;
    let q = p.client.connection().quinn().clone();

    let mut a = q.open_uni().await.unwrap();
    let mut first = vec![1, 1];
    first.extend(frame(&msg(1, 1)));
    a.write_all(&first).await.unwrap();
    assert_eq!(n_of(&recv(&mut clip).await), 1);
    let mut b = q.open_uni().await.unwrap();
    let mut second = vec![1, 1];
    second.extend(frame(&msg(1, 2)));
    b.write_all(&second).await.unwrap();
    b.finish().unwrap();
    // B is taken (header read) and queued behind A.
    tokio::time::sleep(Duration::from_millis(200)).await;

    p.server
        .set_accepted(Channel::Clipboard, false)
        .await
        .unwrap();
    p.server
        .set_accepted(Channel::Clipboard, true)
        .await
        .unwrap();
    assert_eq!(
        stopped_with(&mut a).await,
        Some(ErrorCode::Cancelled.code())
    );

    let mut c = q.open_uni().await.unwrap();
    let mut third = vec![1, 1];
    third.extend(frame(&msg(1, 3)));
    c.write_all(&third).await.unwrap();
    c.finish().unwrap();
    assert_eq!(n_of(&recv(&mut clip).await), 3, "B (2) was not delivered");
    nothing_on(&mut clip).await;
}

async fn next_input_event(
    rx: &mut mpsc::Receiver<wind_core::input::InputEvent>,
) -> wind_core::input::InputEvent {
    tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn withdrawing_input_releases_buttons_and_reenabling_starts_a_new_session() {
    use wind_core::input::InputEvent;
    use wind_core::protocol::Button;

    let mut server_router = Router::new();
    let mut input_rx = server_router.route_input(64);
    let mut p = sessions(Router::new(), server_router).await;

    let input = p.client.input().unwrap();
    input.button(Button::Left, true).await.unwrap();
    assert_eq!(
        next_input_event(&mut input_rx).await,
        InputEvent::Button {
            button: Button::Left,
            down: true
        }
    );

    p.server.set_accepted(Channel::Input, false).await.unwrap();
    // The pressed button is let go at once, on the receiving side.
    assert_eq!(
        next_input_event(&mut input_rx).await,
        InputEvent::Button {
            button: Button::Left,
            down: false
        }
    );
    assert_eq!(next_caps(&mut p.client_events).await, caps(&[]));
    assert!(matches!(
        p.client.input(),
        Err(SessionError::CapabilityMissing(Channel::Input))
    ));
    // Events sent while withdrawn don't reach the system.
    let _ = input.button(Button::Right, true).await;
    let _ = input.pointer(5, 5).await;
    if let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(300), input_rx.recv()).await {
        panic!("input delivered while withdrawn: {e:?}");
    }

    p.server.set_accepted(Channel::Input, true).await.unwrap();
    assert_eq!(next_caps(&mut p.client_events).await, caps(&["input/1"]));
    let input = p.client.input().unwrap();
    input.button(Button::Middle, true).await.unwrap();
    assert_eq!(
        next_input_event(&mut input_rx).await,
        InputEvent::Button {
            button: Button::Middle,
            down: true
        }
    );
}

#[tokio::test]
async fn only_routed_uni_channels_can_be_switched() {
    let mut server_router = Router::new();
    let _clip = server_router.route(Channel::Clipboard, 1, 8);
    let _files = server_router.route_files(1, 8);
    let p = sessions(Router::new(), server_router).await;
    for channel in [
        Channel::Control,
        Channel::Files,
        Channel::Notify,
        Channel::Input,
    ] {
        assert!(
            matches!(
                p.server.set_accepted(channel, false).await,
                Err(SessionError::WrongChannel(c)) if c == channel
            ),
            "{channel:?}"
        );
    }
    assert_eq!(
        p.server.accepted_capabilities(),
        caps(&["clipboard/1", "files/1"])
    );
}
