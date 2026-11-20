# WindConnect relay (WN-59)

A Cloudflare Worker with a Durable Object that connects two paired devices on
different networks. It carries only QUIC packets, already encrypted end to end with
TLS 1.3 and pinned keys. The relay has no session or device key,
so it can't read the content. The protocol is described in
[PROTOCOL.md §13](../PROTOCOL.md).

The option is off by default (decision D3): the apps connect to the
relay only after it is turned on in the settings (WC-60).

## What the server does

- Rendezvous by public keys: a Durable Object for each pair,
  named `SHA-256("wc-relay-v1 pair" ‖ lower key ‖ higher key)`. Both
  devices must name each other.
- Each device signs a random challenge with its Ed25519 key, so
  nobody can take someone else's place in a pair.
- It forwards binary frames unchanged and without storing them. Without a peer
  a frame is dropped, as in UDP.
- Limits:

  | limit | value |
  |---|---|
  | frame | 2048 B |
  | throughput per device | 1 MiB/s, bursts up to 4 MiB (variables in `wrangler.jsonc`) |
  | sockets before authentication | 4 per pair; a new one displaces the oldest (4004) |
  | authentication | 10 s |
  | new connections | 30 per minute from one IP |

- WebSocket Hibernation: a waiting device uses no CPU time, and
  `{"t":"ping"}` gets an answer without waking the object.
- Workers Logs are off: the request address carries both keys, and a log would add
  the IP. The operator can still see IPs, keys, times and traffic volume, but not
  the content (PROTOCOL.md §13.4).

## Locally

```powershell
cd relay
npm ci
npm test            # tests in the Workers environment (workerd): the §13 rules and the vectors from wind-core
npm run typecheck
npm run dev         # wrangler dev on http://127.0.0.1:8787
```

Proof that the relay sees only ciphertext: with
`npm run dev` running, run from the repository root

```powershell
$env:RELAY_URL = "ws://127.0.0.1:8787"
cargo test -p wind-core --test relay -- --ignored --nocapture
```

The test sets up a real QUIC session through the relay and sends a clipboard
message. It checks every frame sent to the relay in both directions for
the message text, a fragment of it and the device names. None of them
contains them, and the other side reads the message. A measurement from 2026-11-10 (Windows,
`wrangler dev`): 27–31 frames, about 15 KB, RTT 6–9 ms.

The client (WN-60): `cargo test -p wind-core --test relay -- --ignored`
also checks `RelayLink`, and `cargo test -p wind-ffi --test bridge -- --ignored`
checks the switch from the LAN to the relay and back. With `windctl`:

```powershell
windctl --identity pc.key serve --trust <phone key> --relay ws://127.0.0.1:8787
windctl --identity phone.key send --address 192.0.2.1:47820 --peer-key <PC key> --relay ws://127.0.0.1:8787 --text "clipboard"
```

When the address doesn't answer, the second command prints `route=relay`, and the first
`RECEIVED channel=Clipboard`.

## Deployment (owner only)

Agents don't deploy the relay. The owner runs, in Claude Code, from the repository
root:

```text
! cd relay && npm ci && npx wrangler login && npx wrangler deploy
```

`wrangler login` is needed only for the first deployment on a given
computer. After deployment the address is
`wss://windconnect-relay.<subdomain>.workers.dev/v1/relay`; it goes into
the app's settings (WC-60). Removing the relay:
`! cd relay && npx wrangler delete`.
