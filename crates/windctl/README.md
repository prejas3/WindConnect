# windctl

A diagnostic CLI for testing `wind-core`: identity, mDNS, the QUIC connection,
raw messages and RTT measurement between nodes. It requires an explicit identity
file and verified public keys of the peers.

From the repository root: `cargo build --locked -p windctl`.
The program: `target/debug/windctl.exe` on Windows or `target/debug/windctl`
on Linux/macOS. You can also use `cargo run --locked -p windctl -- <arguments>`.
`windctl --help`, `windctl ping --help` and `windctl --version` don't create a key.

## Two computers

On each node choose a separate file in the user's private folder.
On Windows the key is encrypted with DPAPI for the current user;
on Unix `FileKeyStore` writes PKCS#8 with permissions 0600. An existing
unreadable key is an error: it is never replaced. Don't use one file
or a copy of a key on both nodes.

1. On A: `windctl --identity <file_A> identity`.
2. On B: `windctl --identity <file_B> identity`.
3. Exchange and verify the `public_key` (64 hex characters) over another channel.
   `fingerprint` is the SHA-256 of the SPKI, not the public key; don't pass it as
   `--peer-key`. No mDNS announcement grants trust.
4. On B run:

   ```text
   windctl --identity <file_B> --name PC-B serve --bind 0.0.0.0:47820 --trust <public_key_A> --duration 300 --advertise --echo
   ```

5. On A, with B's actual LAN address:

   ```text
   windctl --identity <file_A> discover --duration 5
   windctl --identity <file_A> connect --address 192.168.1.20:47820 --peer-key <public_key_B>
   windctl --identity <file_A> ping --address 192.168.1.20:47820 --peer-key <public_key_B> --count 100 --warmup 5 --interval-ms 50
   ```

`serve` prints the actual port (`--bind ...:0` picks a free one), accepts
only the keys from the repeatable `--trust`, answers Ping/Pong and announces
diagnostic channels. It runs 60 s by default, 86400 s at most; Ctrl+C ends it
earlier. It handles up to 16 parallel sessions. On computer B allow the test UDP
port in the firewall; mDNS needs UDP 5353 and a network that lets multicast
through. The program doesn't change firewall rules.

The broadcast fallback works on UDP 47821. To check a network without multicast,
run B with `serve --advertise --broadcast-only` (other arguments as above)
and A with `discover --duration 5 --broadcast-only`. This mode doesn't start
mDNS at all; a found `broadcast-<fingerprint>` entry gives the sender's IP and its
actual QUIC port. Then check `connect` or `ping` with B's verified key.
Broadcast entries that aren't refreshed expire after 8 s. The fallback covers local IPv4;
IPv6 discovery still uses mDNS.

IPv6: `serve --bind [::]:47820`, on the client side e.g.
`--address [2001:db8::20]:47820`. The default server `0.0.0.0` is IPv4;
the listening address family is chosen explicitly.

`discover` uses the core's mDNS module, prints `DISCOVERED` and `REMOVED` changes
with the fingerprint and addresses, and ends after the given time. It also announces
its own temporary endpoint with the same key; its own results are skipped. A
connection still needs a verified public key.

## Latency measurement

Example output (values depend on the network):

```text
PING seq=1 rtt_ms=1.234
...
SUMMARY sent=100 received=100 lost=0 warmup=5 p50_ms=1.234 p95_ms=2.345 min_ms=0.987 max_ms=3.456
```

What is measured is the RTT of the control Ping/Pong after mTLS and the Hello/HelloAck
exchange, without the session setup time. The warmup isn't part of the samples. Each
core Ping has a 5 s limit. Failed attempts count as `lost`, not in the percentiles.
Nearest rank: after sorting the results, percentile P is the sample at index
`ceil(P*N/100)` (counted from 1). A single measurement gives identical p50 and p95.
The program returns an error if it gets no valid sample; partial
loss stays visible in the summary. Number of measurements: 1–10000.
`--timeout-ms` limits the session setup (10000 ms by default) and waiting
for the echo; it doesn't change the internal Ping/Pong limit.

`connect` succeeds after both keys are verified, Hello/HelloAck
and one successful Ping/Pong. Wrong arguments, a TLS refusal, a key error or
a missing required answer give a non-zero exit code.

## Diagnostic messages

To a `windctl serve --echo` node:

```text
windctl --identity <file_A> send --address 192.168.1.20:47820 --peer-key <public_key_B> --channel notify --message-type 65000 --text "test from node A" --expect-echo
windctl --identity <file_A> send --address 192.168.1.20:47820 --peer-key <public_key_B> --channel media --message-type 65001 --body-hex a100182a --expect-echo
```

`--text` encodes the body `{0: text}`. `--body-hex` takes the message body alone
as a deterministic CBOR map with natural keys (`a100182a` = `{0: 42}`).
Encoding, framing, limits and sending over a stream belong to the core.
Channels: `clipboard`, `notify`, `media`. This is a raw test tool;
types 65000/65001 are not the schema of the clipboard, notification or media features
and are not added to the normative protocol.

`--expect-echo` confirms receiving the identical message back, including
the channel and type. Without it `SENT ... echo_verified=false` confirms
local sending and a live control session, not that the receiver ran a feature.
The server prints `RECEIVED` with the CBOR body in hex; without `--echo` it doesn't send it back.

## Relay (WN-60)

The relay (PROTOCOL.md §13) is used only when `--relay URL` is given
(`wss://…`; `ws://` only to a loopback or private address, e.g. a local
`wrangler dev`):

```text
windctl --identity <file_PC> serve --trust <public_key_phone> --relay wss://windconnect-relay.<subdomain>.workers.dev
windctl --identity <file_A> send --address 192.168.1.20:47820 --peer-key <public_key_PC> --relay wss://windconnect-relay.<subdomain>.workers.dev --text "clipboard"
```

`serve --relay` waits on the relay for every device from `--trust` and
reconnects with a growing delay when the connection ends
(`RELAY waiting peer=…`). For `connect`, `ping` and `send` the `--route` option
takes `auto` (the default: the address for 2 s, then the relay), `lan` or
`relay`. The printed line `CONNECTED … route=lan|relay` says which way
the session went.

## Remote actions (WN-54)

`action` asks the other side for an action from its allowed list (PROTOCOL.md
§9.2): by identifier alone, without commands. `serve --dry-run-actions` accepts
built-in actions and prints `ACTION_REQUEST` without running anything.

```text
windctl --identity <file_B> serve --bind 0.0.0.0:47820 --trust <public_key_A> --dry-run-actions
windctl --identity <file_A> action --address 192.168.1.20:47820 --peer-key <public_key_B> --list
windctl --identity <file_A> action --address 192.168.1.20:47820 --peer-key <public_key_B> lock
windctl --identity <file_A> action --address 192.168.1.20:47820 --peer-key <public_key_B> open-url --url https://example.com/
```

The result: `ACTION id=lock status=Done` (or `Refused`, `BadArgument`,
`Failed`, `Busy`). The same `action` against the WindConnect app on a PC
really locks the screen, puts the computer to sleep or opens the link, if the PC has
the `windctl` key pinned as a paired phone and the action is enabled.

## Tests

```text
cargo test --locked -p windctl
cargo test --locked -p windctl --test cli -- --ignored --nocapture
```

The first command checks the percentiles and the parser, starts two separate processes
with separate keys, and tests the session, Ping/Pong, both message formats,
the echo and the refusal of an unknown key. The second needs working multicast and checks
discovering a separate process over mDNS. Every test process is time-limited
and cleaned up after the test. These local tests don't replace a measurement on two
physical computers on a LAN following the procedure above.
