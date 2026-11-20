# WindConnect Protocol v1

Status: version 1.0 (WN-5, v0.5.0). This document is normative for `wind-core`.
The words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described in
RFC 2119.

Scope of v1.0: transport, framing, versioning, capability negotiation,
error codes, the `control` channel and the `input` channel. Feature messages
(clipboard, notifications, media, files) are added by those features' cards as
changes to this document under the rules of section 9; until then their channels
have reserved numbers and stream rules, but no message types.

## 1. Overview

```text
QUIC v1 (RFC 9000) + TLS 1.3, ALPN "windconnect/1"
├── bidi stream  control       (one, opened by the client)   CBOR frames
├── uni stream   clipboard     (1 active per direction, §3.1) CBOR frames
├── uni stream   notify        (as above)                    CBOR frames
├── uni stream   media         (as above)                    CBOR frames
├── uni stream   notify-action (as above, §9.8)              CBOR frames
├── uni stream   volume        (as above, §9.9)              CBOR frames
├── uni stream   call          (as above, §9.10)             CBOR frames
├── uni stream   input         (as above, buttons)           CBOR frames
├── bidi stream  files × N     (1–8 per transfer, §9.1)      CBOR frames
├── bidi stream  actions × N   (1 per request, §9.2)         CBOR frames
└── QUIC datagrams input       (pointer motion, scrolling)   CBOR without length
```

- The **client** is the side that sets up the QUIC connection; the **server** is
  the side that accepts it. Every device has both roles; after connecting, the
  protocol is symmetric except for opening the `control` channel.
- Bytes on the wire are in network order (big-endian).
- Numbers in binary examples are hexadecimal.

## 2. Transport and security

- Transport: QUIC v1 (RFC 9000) with TLS 1.3 (RFC 9001). The server's default UDP
  port: **47820**; discovery (mDNS, broadcast) announces the actual port.
- ALPN: `windconnect/1`, where `1` is the **major** protocol version (section 5).
  Setting up a connection without a matching ALPN ends with the TLS alert
  `no_application_protocol`.
- The ALPN `windconnect-pair/1` belongs to the pairing protocol (section 12), on
  a separate endpoint; the main endpoint doesn't accept it.
- Authentication is mutual (mTLS): each side presents a self-signed certificate
  with the device's Ed25519 key (WC-07). A side MUST accept the other side's
  certificate only when its public key is the pinned key of a paired device
  (WC-10). The certificate's chain, name and dates are not a basis for trust.
- Refusal happens in the TLS handshake, before any application data flows: the
  refusing side ends it with the TLS alert `access_denied` (QUIC code `0x131`).
  Handshake signatures are Ed25519 only, so a certificate with another key
  already fails for lack of a common signature scheme. A TLS 1.3 client sends
  its certificate in the last flight, so a refused client may consider the
  handshake finished and only then receive the close with this alert; the
  server doesn't accept such a connection. The code `UNTRUSTED_PEER`
  (section 10) closes an already established connection when the key is no
  longer pinned (e.g. after unpairing).
- A device's identity is its TLS key alone. Message fields (e.g. the device name
  in `Hello`) are data for display, not proof of identity.
- QUIC datagrams (RFC 9221): a side that announces `input/1` SHOULD announce the
  parameter `max_datagram_frame_size` ≥ 1200. When datagrams aren't available on
  a connection, continuous events go over the `input` stream (section 8.7).
- 0-RTT data (resumption, WC-12) MAY contain only the `control` stream header,
  the `Hello` frame and `Ping` frames. All other data MUST wait for the 1-RTT
  handshake to complete. Both messages are safe to replay.
- Keep-alive and network changes (WC-12): a side sends a QUIC keep-alive (a
  `PING` frame) after every 15 s of inactivity and announces a `max_idle_timeout`
  of 45 s. Path migration (RFC 9000 §9) is allowed: a side MUST NOT announce
  `disable_active_migration`, and after a network change the client moves the
  existing connection to a new socket instead of setting up a new one.
- TLS resumption: the client keeps session tickets separately for each device
  and offers a ticket only to the device that issued it; the server name (SNI)
  is the same for all devices (`windconnect`), so it doesn't reveal whom the
  client connects to. A resumed session skips certificate verification, so after
  the handshake completes a side MUST check again that the other side's key is
  still pinned, and if it isn't, close the connection with the code
  `UNTRUSTED_PEER`.
- Reconnecting after losing the connection: exponentially growing intervals
  (from 100 ms to 30 s, with a random jitter of 50–100 %), with an immediate
  attempt after a network change is detected (WC-16). A refusal because of
  pinning (`access_denied`, `UNTRUSTED_PEER`) stops the retries.

### 2.0 mDNS discovery (WN-14)

- DNS-SD service: `_windconnect._udp.local.`. The SRV record announces the actual,
  non-zero QUIC listening port, and A/AAAA the device's addresses.
- The TXT `fingerprint` contains the full SHA-256 of the SPKI from §2.1: 64 hex
  characters. The sender uses lower case; the receiver accepts both cases.
  A missing or invalid fingerprint means the announcement is skipped.
- Instance and host names have the form `wc-<first 40 characters of the fingerprint>`
  (the host with `.local.`); mDNS may rename on a conflict.
- An announcement is an untrusted address hint. The TXT fingerprint MUST NOT
  pin a key or bypass the TLS authentication from §2.
- TXT `pair` (WN-82, §12.4): the pairing endpoint's port, only while a SAS window
  is open; it disappears when the window closes (paired, expired, closed, locked
  after attempts, replaced by a QR window). Value: 1–5 decimal digits, 1–65535;
  any other value (0, not a number, too large) means no port, and the rest of
  the announcement stays. A network change doesn't remove the field.
- An update of a resolved announcement replaces its previous addresses;
  a goodbye or the records expiring removes the service. The own fingerprint is skipped.
- The sender sends a new announcement of the same name (e.g. after a network
  change) no earlier than 1 s after its goodbye, that is after the time for which
  the receiver keeps the goodbye (RFC 6762 §10.1). The receiver then doesn't have
  to tell "rescuing" a record from a new device (WN-104).

### 2.0.1 IPv4 broadcast fallback discovery (WN-15)

- Discovery port: UDP **47821**, independent of the QUIC port. A packet has
  exactly 39 bytes: ASCII `WCD1` (4 B), kind (1 B: `1` query,
  `2` response), the actual QUIC port (2 B big-endian, non-zero),
  the full SHA-256 SPKI fingerprint (32 B, as in §2.1).
- A query announces the sender and asks for a response. It is sent to the
  broadcast addresses of the IPv4 interfaces every 2 s and right after a network
  change; when there are no such addresses, `255.255.255.255:47821` is used.
- A response is unicast to the query's source. It has the same size and doesn't
  trigger another response. Limit: one response per source IP per second,
  at most 128 tracked sources.
- The peer's address comes from the datagram's source IP, and the port from its
  content. The source port must be the discovery port. Foreign, invalid and
  excess packets and the own fingerprint are skipped. An entry that isn't
  refreshed expires after 8 s; the broadcast buffer holds at most 128 fingerprints.
- A full buffer makes room instead of rejecting new entries (WN-87), so that
  one sender of made-up fingerprints can't switch discovery off:
  - one source IP has at most one unpaired fingerprint; a newer fingerprint
    from that IP replaces the previous one (one device has one fingerprint);
  - the fingerprint of a paired device (a pinned key, §2.1) is always accepted
    and doesn't count towards the limit of one fingerprint per IP;
  - when the buffer is full, a new entry displaces the least recently refreshed
    unpaired entry; an unpaired fingerprint never displaces a paired one. When
    the buffer is full of paired ones only, an unpaired one is skipped, and a
    paired one displaces the least recently refreshed paired one.
  The priority of paired devices is only an order in the hint buffer; it
  grants no trust and pins nothing.
- Broadcast works in parallel with mDNS, also when multicast is unavailable.
  Results are merged by fingerprint, with repeated addresses removed.
  These are untrusted hints; they MUST NOT pin keys or bypass TLS.

### 2.1 Key, certificate and fingerprint

- The device key is an Ed25519 pair (RFC 8032), created on first start and kept
  across starts. An unreadable stored key is an error; an implementation MUST NOT
  then silently create a new one, because a new key means losing all pairings.
- The TLS certificate is a self-signed X.509 v3 signed with this key, with
  the `subjectAltName` DNS `windconnect`. Certificate fields other than the public
  key carry no meaning (section 2).
- The key's **fingerprint** is the SHA-256 of the DER `SubjectPublicKeyInfo` of the
  key (RFC 8410), that is `302a300506032b6570032100 ‖ key (32 B)`. It can be
  computed with standard tools, e.g.
  `openssl pkey -pubout -outform DER | sha256sum`.
- Display form: the first 160 bits of the fingerprint in Crockford's base32
  (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, without `I L O U`), 32 characters
  in eight groups of four, separated by spaces. Example for the public key from
  RFC 8032 §7.1, test 1
  (`d75a9801…f707511a`): fingerprint `06e3fd8f…1b792fa9`, displayed as
  `0VHZ V3YT 56XP 1ATS ANYY C7PV 1BPD P8RH`.
- The fingerprint lets people compare and is used for discovery; trust rests
  on the full public key (section 2).

## 3. Streams and the stream header

The side that **opens** a stream starts it with a two-byte header:

| byte | field | value |
|---|---|---|
| 0 | `version` | major protocol version, `01` in v1 |
| 1 | `channel` | channel number from the table below |

The header is followed by a sequence of frames (section 4) up to the end of the
stream (FIN). **Only the opener** writes the header. On a bidi stream the return
direction (from the side that accepted the stream) has no header: it starts right
away with the first frame. This applies to `control` (`HelloAck` is the first bytes
from the server, example in section 7.1), `files` (the file receiver's responses) and
`actions` (the response to a request).

| channel | name | stream kind | opened by | send priority |
|---|---|---|---|---|
| `00` | `control` | bidi | the client; exactly 1 per connection | 0 (highest) |
| `01` | `clipboard` | uni | the message sender | 2 |
| `02` | `notify` | uni | the message sender | 3 |
| `03` | `media` | uni | the message sender | 4 |
| `04` | `input` | uni + datagrams | the event sender | 1 |
| `05` | `files` | bidi | the file sender; 1–8 per transfer (§9.1) | 7 (lowest) |
| `06` | `actions` | bidi | the side asking for an action; 1 per request (§9.2) | 5 |
| `07` | `battery` | uni | the battery state sender (§9.4) | 6 |
| `08` | `find` | bidi | the side asking for ringing; 1 per ring (§9.5) | 5 |
| `09` | `media-control` | uni | the sender of commands for the other side's player (§9.6) | 4 |
| `0a` | `notify-action` | uni | the side showing the other side's notifications: dismiss, reply (§9.8) | 3 |
| `0b` | `volume` | uni | the side whose volume changes (`Level`), or the controlling side (`Set`) (§9.9) | 4 |
| `0c` | `call` | uni | the phone: call state (§9.10) | 1 |
| `0d` | `sms` | bidi | the computer; 1 per request for threads, messages, a picture or a watch (§9.11) | 5 |
| `0e` | `call-action` | uni | the computer: reject or silence the phone's ringing call (§9.12) | 1 |
| `0f` | `gallery` | bidi | the computer; 1 per request for the list, a thumbnail, an original or a watch (§9.13) | 5 |
| `10`–`3f` | reserved for future channels | | | |
| `40`–`ff` | not allowed in v1 | | | |

Rules:

1. A side MUST reject a stream with an unknown `version`: `STOP_SENDING`
   with the code `UNSUPPORTED_VERSION`. A `control` stream with an unknown `version`
   closes the whole connection with this code.
2. A stream with an unknown or disallowed `channel` is rejected with
   `STOP_SENDING` with the code `UNKNOWN_CHANNEL`; the connection goes on.
3. The `control` channel needs no capability. A stream of any other channel is
   accepted only when **the receiving side itself** announced (in the last
   `Hello` / `HelloAck` / `Capabilities` it sent) the capability for that channel,
   because a capability means "I accept" (section 6). Otherwise it
   rejects the stream with `STOP_SENDING` with the code `CAPABILITY_MISSING`;
   the connection goes on.
4. A second `control` stream, a `control` stream opened by the server, or a
   stream of the wrong kind (uni instead of bidi or the other way round) is a
   `PROTOCOL_VIOLATION` for the whole connection.
5. The header arrives in full before the first frame; a stream ended
   (FIN) before the full header is `MALFORMED_FRAME` for that stream.
6. Send priority: a sender SHOULD serve streams with a lower number before a
   higher one, so that `files` traffic doesn't delay `control` or
   `input`. `files` streams of equal priority share the link equally.

Example: the header of the `control` stream is `01 00`, of the `input` stream `01 04`.

### 3.1 Life cycle of uni streams

The channels `clipboard`, `notify`, `notify-action`, `media`, `media-control`, `volume`, `call`, `call-action`, `input` and `battery` have in each direction a
**message queue**: a sequence of uni streams of one sender for that channel.

1. A sender has at most **one stream of the channel open for writing** at a time.
   It opens a new stream of that channel only after ending the previous one
   on its side: FIN (a normal close) or `RESET_STREAM`
   (abandoning it). The limit applies to active streams, not to all
   streams in the connection's history.
2. A sender MUST write the header right when it opens the stream,
   at the latest together with the first frame; it doesn't open a stream "in reserve".
3. **Order within a channel.** The receiver passes the messages of uni stream `S`
   of channel `C` from sender `N` to the module only when:
   1. every uni stream from `N` with a lower QUIC number is
      **recognized** (its header has arrived) or **ended** (FIN or
      `RESET_STREAM`), and
   2. every one of them recognized as channel `C` is ended and its
      messages have already been passed on.

   Streams of other channels block only until their header arrives
   (rule 2), not until their end. A stream abandoned with `RESET_STREAM` before
   its header arrived counts as ended. QUIC stream numbers grow in the order
   streams are opened (RFC 9000 §2.1), and rule 1 guarantees that
   the channel's previous stream has a lower number, so the order of messages
   of one channel is kept also when the stream is replaced.
4. A uni stream whose header hasn't arrived within 2 s of the arrival of a stream
   with a higher number from the same sender is rejected by the receiver
   (`STOP_SENDING`, `PROTOCOL_VIOLATION`) and treated as ended.
5. **Stream limit.** Against a sender that opens streams against
   rule 1, the receiver is protected by the QUIC stream limit and flow control,
   not by rejecting:
   1. A side announces `initial_max_streams_uni` = **100** (recommended; no
      less than 16). The limit is shared by all uni channels.
   2. A side grants new streams (the `MAX_STREAMS` frame) only when
      the other side's uni stream is **served** on its side: read
      to FIN, and its messages passed to the module or accepted into the
      read-ahead buffer (point 6), or abandoned (`RESET_STREAM`),
      or rejected (`STOP_SENDING`). A stream whose messages didn't
      fit in either the module's queue or the buffer still takes
      the limit. A side MAY grant streams in batches (e.g. when more than
      1/8 of the limit has been freed).
   3. A sender that has used up the limit waits for `MAX_STREAMS`; no
      message is lost. The amount of data buffered by the receiver is
      limited by QUIC's flow-control windows (of streams and of the connection).
   4. A receiver MUST NOT reject a stream (e.g. with the code `RATE_LIMITED`)
      only because of the number of open or unserved streams of a channel:
      it can't tell a sender breaking rule 1 from a fast, correct
      sender whose ended streams are waiting for a free module.
   5. Because the limit is shared, a sender SHOULD send further
      messages of a channel on the same stream and end it only during a
      pause or when a capability is withdrawn, rather than open a stream
      for every message.
   6. **Read-ahead (WN-81).** The receiver reads a channel's streams
      further than its module takes them: the messages go into the channel's buffer,
      in order, and a stream read to FIN is served at once
      (point 2), even though the module hasn't got them yet. The buffer MUST be
      bounded; recommended **1 MiB per channel**, counting the payload of each
      message plus a fixed overhead (64 B in `wind-core`), so as to also bound
      many small messages. A full buffer stops reading the
      current stream (flow control holds the sender back); this and
      further streams of the channel then take the limit as in point 2.
      A message larger than the whole buffer waits until the buffer is empty and
      goes in alone. Withdrawing a capability (section 6, rule 5) removes from the buffer
      the messages from before the withdrawal.

   Result: a slow module of one channel **doesn't hold up** other channels
   as long as its backlog fits in the buffer. Other channels may wait only
   when (a) the channel's backlog exceeds the buffer and the sender keeps opening
   new streams of that channel until it uses up the limit, or (b) the sender breaks
   rule 1 and keeps streams without FIN. Both cases concern a
   paired device and are the sender's error, not data loss: nothing
   is lost, and the sender waits for `MAX_STREAMS`.
6. Messages from an abandoned stream (`RESET_STREAM`) may be lost;
   channels where this does harm describe retrying in their specification.

Example: a client (uni streams 2, 6, 10, …) sends clipboard A with FIN on stream 2,
then clipboard B with FIN on stream 6. All of stream 6 reaches the receiver
first:

```text
stream 6: 01 01 + B + FIN  → channel clipboard; stream 2 not yet
                             recognized → B waits (rule 3, point 1)
stream 2: 01 01 + A + FIN  → clipboard, nothing lower is missing →
                             pass on A; stream 2 ended → pass on B
```

If stream 2 were the `notify` channel, B would be passed on right after
the header `01 02` of stream 2 arrived. If stream 2 were abandoned
(`RESET_STREAM`) before its header arrived, B would be passed on right after
the `RESET_STREAM` frame arrived.

Withdrawing and re-enabling a capability (section 6, rule 5) uses
the same rules: withdrawing ends the stream, re-enabling opens
a new one, with a higher number.

## 4. Frame

```text
frame   = length payload
length  = QUIC variable-length integer (RFC 9000 §16), 1–8 bytes
payload = exactly `length` bytes: one CBOR item
```

The payload is a CBOR array of two items `[type, body]`:

- `type`: a natural number, the message type. Numbering is **separate for
  each channel** (the channel follows from the stream), so the router picks the module
  by the pair (`channel`, `type`).
- `body`: a CBOR map whose keys are natural numbers. A message
  without fields has the empty map `a0`.

Length encoding: `06` (6), `41 2c` (300), `80 01 11 70` (70 000).
A sender MUST use the shortest length encoding; a receiver MUST reject
an encoding longer than needed (`MALFORMED_FRAME`).

### 4.1 CBOR subset

A sender MUST encode deterministically (RFC 8949 §4.2.1): the shortest
encoding of numbers and lengths, definite lengths only, map keys
sorted bytewise by their encoded form. A receiver MUST accept
only:

| major type | allowed |
|---|---|
| 0, 1 | integers in the i64 / u64 range |
| 2 | byte strings of definite length |
| 3 | UTF-8 text of definite length, valid UTF-8 |
| 4 | arrays of definite length |
| 5 | maps of definite length, without repeated keys |
| 7 | only `false` (`f4`), `true` (`f5`), `null` (`f6`) |

Everything else (tags, floating-point numbers, indefinite lengths,
`undefined`, non-canonical encoding) is a `MALFORMED_FRAME` error.
Nesting (arrays and maps) doesn't exceed 16 levels. A payload must
contain exactly one CBOR item, with no extra bytes.

### 4.2 Size limits

| place | largest `length` / datagram |
|---|---|
| `Hello` and `HelloAck` frames | 4 096 B |
| other frames on `control` | 16 384 B |
| frames on `actions` (§9.2) | 16 384 B |
| frames on the other streams | 1 048 576 B (1 MiB) |
| datagram | the smaller of: the other side's `max_datagram_frame_size`, 1 200 B |

A frame of length 0 or over the limit is an error: for `control` the connection
is closed with `FRAME_TOO_LARGE` (or `MALFORMED_FRAME` for 0), for other
streams `STOP_SENDING` with that code. A receiver MUST check the length
**before** buffering the payload.

### 4.3 Fields and extensibility

- The fields in `body` are described with each message as
  `key name: type`, marked *required* or *optional*.
- A missing required field or a field of the wrong type is `MALFORMED_FRAME`.
- A receiver MUST skip unknown keys in `body` (forward compatibility).
- Texts have length limits given in UTF-8 bytes; exceeding them is
  `MALFORMED_FRAME`.

### 4.4 Unknown message types

An unknown `type` on a known channel doesn't break the connection. The receiver skips
the frame and MAY send back an `Error` with the code `UNKNOWN_MESSAGE` (section 7.6), no
more often than once per second per channel.

## 5. Versioning

The protocol version is a pair `major.minor`.

- **major** changes with a backward-incompatible change (framing, the
  meaning of existing fields). It is in the ALPN (`windconnect/1`) and in the
  `version` byte of the stream header and of datagrams. Sides without a common major
  version won't set up a connection (ALPN alert).
- **minor** changes with a compatible change: new message types, new optional
  fields, new channels from the reserved pool. A side announces its version
  in `Hello` / `HelloAck`; both sides use the **smaller** of the two minor
  versions and don't send message types added later.
- A `version` field in `Hello` with a different major version than in the ALPN is a
  `PROTOCOL_VIOLATION`.
- A client MAY offer several major versions in the ALPN (newest first); the server
  picks the newest common one.

This document describes version **1.0**.

## 6. Capability negotiation

A capability is a text `name/N`: `name` made of lower-case ASCII letters, digits and `-`
(1–24 bytes), `N` a positive decimal number, the capability version. The list
has at most 64 entries, without repetitions.

Capabilities in v1.0:

| capability | meaning |
|---|---|
| `clipboard/1` | the side handles the `clipboard` channel (text) |
| `clipboard/2` | as `clipboard/1`, plus PNG images (§9.3.1, WN-36) |
| `notify/1` | the side shows the other side's notifications: accepts `Posted`, `Icon`, `Removed`, `Replied` on the `notify` channel (§9.7, WN-42) |
| `notify-action/1` | the side's notifications can be dismissed and replied to: accepts `Dismiss`, `Reply` on the `notify-action` channel (§9.8, WN-44) |
| `volume/1` | the side's volume can be changed, and the side shows the other's volume: accepts `Set` and `Level` on the `volume` channel (§9.9, WN-48) |
| `call/1` | the side pauses its players during the other side's call or shows its calls: accepts `State` and `Missed` on the `call` channel (§9.10, WN-48, WN-57) |
| `call-action/1` | the side's ringing call can be rejected or silenced: accepts `Reject` and `Silence` on the `call-action` channel (§9.12, WN-57) |
| `media/1` | the side shows the other side's player: accepts `Now`, `Art`, `Gone` on the `media` channel (§9.6, WN-46) |
| `media-control/1` | the side's player can be controlled: accepts `Command` on the `media-control` channel (§9.6, WN-46) |
| `input/1` | the side accepts input events (the `input` channel, section 8) |
| `input/2` | as input/1, plus reliable Windows gesture barriers (section 8.9); the receiver also announces input/1 |
| `input/3` | as input/2, plus Unicode text and complete keyboard shortcuts (section 8.10); the receiver announces input/1 and input/2 |
| `files/1` | the side handles the `files` channel |
| `actions/1` | the side accepts action requests (the `actions` channel, §9.2) |
| `battery/1` | the side accepts the other side's battery state (the `battery` channel, §9.4, WN-49) |
| `find/1` | the side can ring at the other's request (the `find` channel, §9.5, WN-50) |
| `gallery/1` | the side shares its newest photos and screenshots with the other: accepts requests on the `gallery` channel (§9.13, WN-58) |
| `sms/1` | the side shares its SMS/MMS threads with the other for reading (WN-55) and, with a separate consent, sending through it (WN-56): accepts requests on the `sms` channel (§9.11) |

Rules:

1. A capability means **receiving**: a side announces in `Hello` / `HelloAck`
   the channels whose messages it **accepts**. Sending needs nothing
   announced: a side MAY send a channel's messages when the other side announced
   its capability and it can send them itself. Each direction is
   independent. The `control` channel always works, without a capability.
2. Version choice, separately for each direction: the sender uses the highest
   version `N` for which the receiver announced `name/N` and the sender can send
   messages in that version. When there is no such version, the sender doesn't send
   that channel's messages. The list may contain several versions of the same name
   (`clipboard/1`, `clipboard/2`).
3. A receiver skips unknown capabilities.
4. A feature in which the receiver answers the sender (e.g. a confirmation)
   puts the answer on the same bidi stream, or requires in its
   specification that both sides announce the capability.
5. A change during the connection (e.g. the user turned a feature off): the message
   `Capabilities` with the full new list of what the side accepts.
   - **Withdrawal:** a side that withdraws a capability, from the moment it sends
     `Capabilities`, skips incoming messages of that channel and sends
     `STOP_SENDING` (`CANCELLED`) on its streams. The other side, after
     receiving `Capabilities`, stops sending that channel's messages
     and ends its open stream with `RESET_STREAM` (`CANCELLED`).
     Messages in flight at the moment of withdrawal are lost. A stream that
     the other side opens before it receives `Capabilities` is rejected by the
     withdrawing side with the code `CAPABILITY_MISSING` (section 3, rule 3). A stream
     accepted before the withdrawal is never delivered, even when the
     capability is re-enabled before its turn came
     (section 3.1, rule 3).
   - **Re-enabling:** the other side, after receiving `Capabilities` with this
     capability, opens a **new** stream of the channel (section 3.1); a stream ended
     earlier is not resumed.
   - On withdrawal and re-enabling, the `input` channel starts a new session
     with reset state, and the receiver releases pressed buttons
     (sections 8.2 and 8.4); events from the time it was off don't reach
     the system.
   - The `files` channel (WN-86): withdrawal stops (`STOP_SENDING`,
     `CANCELLED`) the streams of all ongoing transfers to this
     side; the receiver ends them and deletes the partial files, and the sender sees
     the transfer as cancelled by the other side. After re-enabling, the
     sender starts the transfer over (section 9.1).
   - The `actions` channel (WN-54): withdrawal works as for `files`; a request
     accepted before the withdrawal whose reading is still going on is
     stopped (`CANCELLED`), and new streams get
     `CAPABILITY_MISSING`.

An asymmetric example: the computer announces `["clipboard/1", "input/1"]`,
the phone `["notify/1"]`.

| direction | allowed channels |
|---|---|
| phone → computer | `clipboard`, `input` (the computer accepts them) |
| computer → phone | `notify` (the phone accepts it) |

So the phone sends `input` events although it doesn't announce `input/1` itself.
If the computer opened an `input` stream to the phone, the phone would reject it
with the code `CAPABILITY_MISSING`, because it didn't announce `input/1` itself.

## 7. The `control` channel (`00`)

### 7.1 Connection flow

```text
client                                          server
  ── QUIC + TLS 1.3 (mTLS, ALPN windconnect/1) ──
  open bidi: 01 00  Hello  ─────────────────────▶
                         ◀─────────────────────  HelloAck
  (from now on both sides open channels according to the capabilities)
  Ping ──────────────────▶  ◀────────────────── Pong
  ...
  Goodbye ───────────────▶  CONNECTION_CLOSE(GOING_AWAY)
```

1. The client opens the `control` stream as the first stream and sends `Hello`
   as the first frame.
2. The server answers with `HelloAck` as the first frame on this stream.
3. Before sending `HelloAck` the server doesn't open streams or send
   datagrams. The client doesn't open other streams or send datagrams
   before receiving `HelloAck`. Streams and datagrams that arrive before
   this exchange completes MAY be buffered by a side up to 64 KiB in total, or
   rejected (`STOP_SENDING`, `PROTOCOL_VIOLATION`).
4. No `Hello` within 5 s of the connection being established (or `HelloAck` within 5 s of
   sending `Hello`): close with the code `HELLO_TIMEOUT`.
5. A message other than `Hello` / `HelloAck` as the first frame, a second `Hello`
   or `HelloAck` is a `PROTOCOL_VIOLATION`.
6. Either side ending the `control` stream ends the
   connection (`PROTOCOL_VIOLATION`, unless a `Goodbye` came before it).

The exact bytes at the start of the `control` stream (stream ID 0) for the examples
in section 7.3:

```text
client → server:  01 00                      header: version 1, channel control
                  31 82 00 a5 …              Hello frame (50 B)
server → client:  2c 82 01 a5 …              HelloAck frame (45 B), no header
```

### 7.2 `control` message types

| type | name | direction |
|---|---|---|
| `0` | `Hello` | client → server |
| `1` | `HelloAck` | server → client |
| `2` | `Ping` | both sides |
| `3` | `Pong` | both sides |
| `4` | `Capabilities` | both sides |
| `5` | `Error` | both sides |
| `6` | `Goodbye` | both sides |

### 7.3 `Hello` (0) and `HelloAck` (1)

Both have the same set of fields:

| key | name | type | |
|---|---|---|---|
| 0 | `version` | array `[major, minor]` of natural numbers | required |
| 1 | `name` | text, 1–64 B | required; a name for display |
| 2 | `kind` | number: 0 computer, 1 phone, 2 tablet | required; an unknown value = 0 |
| 3 | `caps` | array of texts (section 6) | required; may be empty |
| 4 | `app` | text, 1–32 B | optional; the app version |

Example `Hello` from the computer `SZYMON-PC` (version 1.0, capabilities
`clipboard/1` and `input/1`, app 0.5.0), 50 B:

```text
31 82 00 a5 00 82 01 00 01 69 53 5a 59 4d 4f 4e
2d 50 43 02 00 03 82 6b 63 6c 69 70 62 6f 61 72
64 2f 31 67 69 6e 70 75 74 2f 31 04 65 30 2e 35
2e 30
```

Breakdown:

```text
31                         length 49
82                         array [type, body]
   00                      type 0 = Hello
   a5                      map, 5 pairs
      00 82 01 00          0 version: [1, 0]
      01 69 53 5a…43       1 name: "SZYMON-PC"
      02 00                2 kind: 0 (computer)
      03 82                3 caps: array of 2
         6b 63 6c…2f 31       "clipboard/1"
         67 69 6e…2f 31       "input/1"
      04 65 30 2e 35 2e 30 4 app: "0.5.0"
```

Example `HelloAck` from the phone `Pixel 9` (capabilities `input/1`,
`notify/1`), 45 B:

```text
2c 82 01 a5 00 82 01 00 01 67 50 69 78 65 6c 20
39 02 01 03 82 67 69 6e 70 75 74 2f 31 68 6e 6f
74 69 66 79 2f 31 04 65 30 2e 35 2e 30
```

### 7.4 `Ping` (2) and `Pong` (3)

| key | name | type | |
|---|---|---|---|
| 0 | `nonce` | natural number u64 | required |

To a `Ping` the receiver answers promptly with a `Pong` with the same `nonce`.
`Ping` measures latency at the application level (e.g. `windctl ping`)
and doesn't replace the QUIC keep-alive (WC-12). A side MAY ignore `Ping`s
beyond 10 per second.

`Ping` with `nonce` 42, 7 B: `06 82 02 a1 00 18 2a`

`Pong` with `nonce` 42, 7 B: `06 82 03 a1 00 18 2a`

### 7.5 `Capabilities` (4)

| key | name | type | |
|---|---|---|---|
| 0 | `caps` | array of texts (section 6) | required; the full new list |

Example (only `input/1` remains), 14 B:

```text
0d 82 04 a1 00 81 67 69 6e 70 75 74 2f 31
```

### 7.6 `Error` (5)

An error that does **not** close the connection (e.g. an unknown message type).
Errors that end the connection or a stream go as a QUIC code (section 10).

| key | name | type | |
|---|---|---|---|
| 0 | `code` | number, a code from section 10 | required |
| 1 | `message` | text, ≤ 256 B | optional; for logs, not for the user |
| 2 | `channel` | number, the channel of the message it concerns | optional |
| 3 | `type` | number, the type of the message it concerns | optional |

Example: `UNKNOWN_MESSAGE` for type 9 on the `control` channel, 24 B:

```text
17 82 05 a4 00 06 01 6c 75 6e 6b 6e 6f 77 6e 20
74 79 70 65 02 00 03 09
```

Receiving an `Error` doesn't trigger an `Error` in response (no loops).

### 7.7 `Goodbye` (6)

An announcement that the connection is closing (leaving the app, sleep,
unpairing). After sending `Goodbye` a side closes the connection with a
`CONNECTION_CLOSE` frame with the same code; the other side then doesn't try
to reconnect immediately.

| key | name | type | |
|---|---|---|---|
| 0 | `code` | number, a code from section 10 | required |
| 1 | `message` | text, ≤ 256 B | optional |

Example: `GOING_AWAY`, 6 B: `05 82 06 a1 00 0d`

## 8. The `input` channel (`04`)

Requires the capability `input/1` on the side **receiving** the events. Continuous
events (pointer motion, scrolling) go as datagrams, because retransmission would only
delay the picture. Button transitions go over the `input` stream, because losing
them would leave a button pressed. Sections 8.2–8.6 describe how the
receiver joins both paths into one consistent sequence of events; section 8.6 collects
all the rules in a state table.

### 8.1 Datagram format

```text
datagram = version(1 B, 01) channel(1 B, 04) payload
```

`payload` is one CBOR item `[type, body]` as in a frame, but **without** the length
field (the datagram sets the boundary). A datagram with another `version`,
with a channel other than `04` or with an invalid payload is silently dropped
by the receiver (it doesn't close the connection).

### 8.2 Input session

A **session** is the lifetime of one `input` stream of the sender. All
counters from sections 8.3 and 8.4 belong to the session and start from zero together
with it.

The sender:

- The first frame of every `input` stream MUST be `InputSession`
  with the session number `s`. The first session in a connection has `s` = 0, each
  next one 1 more.
- It starts a new session (a new stream, section 3.1) only when the previous
  `input` stream has ended, e.g. after the receiver withdrew and re-enabled
  `input/1`. Path migration does **not** change the session:
  the stream and the counters go on.
- Every datagram carries the session number `s`.
- `InputSession` also carries `g`: the number of the receiver's capability
  announcement on the basis of which the sender opened the session. The receiver's
  announcements are numbered in order on the `control` stream: its `Hello` / `HelloAck`
  has number 0, each of its `Capabilities` messages 1 more. The sender
  counts them as it receives them (the `control` stream is ordered) and gives the
  number of the last announcement received that contained `input/1`.

The receiver keeps for the sender the number of the current session `cur` (at the start
of the connection: none) and its state:

| state | meaning |
|---|---|
| **none** | no session has been accepted yet |
| **live** | `InputSession` with `cur` accepted, the stream goes on, the receiver announces `input/1` |
| **closed** | session `cur` has ended: the end of its stream (FIN, `RESET_STREAM`), the receiver's `STOP_SENDING` on it, the receiver withdrawing `input/1` |

Only events of the **live** session `cur` reach the system. Closing
a session is final: no event with `s` ≤ `cur` will be applied any more,
also in the time before the next session is accepted. Datagrams
with `s` > `cur` (or any `s` when the state is "none") are a future session:
the receiver sets them aside (section 8.5) if it announces `input/1`; otherwise it skips them.

The receiver remembers `g_min`: the number of its own announcement that last
**enabled** `input/1` (0 when `input/1` was already in `Hello` / `HelloAck`).

An `InputSession` frame (`s`, `g`) must have `s` > `cur` (when `cur` exists);
otherwise it is a `PROTOCOL_VIOLATION` for the stream (`STOP_SENDING`). Apart from that
the receiver sets `cur` = `s` and resets the state from sections 8.3 and 8.4
(`pointer` = `scroll` = (0, 0), no `pointer_seq` and `scroll_seq`,
`epoch` = 0), and then:

- when it announces `input/1` and `g` ≥ `g_min`: the session is live, the set-aside
  datagrams are reviewed (section 8.5);
- otherwise the session is **closed at once**, and the receiver sends
  `STOP_SENDING` (`CANCELLED`) on the stream. This is how a session ends that
  the sender opened before `input/1` was withdrawn, but whose frames arrived
  only after it was re-enabled: its `g` is smaller than `g_min`.

The receiver skips the frames of a closed session's stream.

### 8.3 Cumulative state instead of deltas

Motion and scrolling are not sent as deltas (losing a `dx`
would mean losing motion for good), but as **cumulative state**: the sum
of all deltas since the start of the session.

- The sender keeps two i64 pair counters in the session: `pointer` = (`x`, `y`)
  and `scroll` = (`x`, `y`). Every motion or scroll event carries
  the current value of its counter; `PointerButton` carries both.
- `seq` is the sender's u64 counter, shared by all `input` events of the
  session (datagrams and stream frames other than `InputSession`): the first
  event of the session has `seq` 1, each new one 1 more.
- The receiver remembers, separately for motion and for scrolling, the last applied
  state and its `seq`: (`pointer_seq`, `pointer`) and (`scroll_seq`, `scroll`).
- **Applying** a motion state (`seq`, `x`, `y`): if `seq` ≤
  `pointer_seq`, the state is late or repeated and the receiver skips it.
  Otherwise it moves the pointer by (`x` − `pointer.x`,
  `y` − `pointer.y`) and stores the new state. Scrolling likewise,
  with (`scroll_seq`, `scroll`).
- A lost datagram doesn't lose motion: the next one carries the sum, so
  the receiver catches up the whole difference at once.
- After the last change of a counter (the end of a gesture) the sender SHOULD repeat
  the last motion or scroll datagram **unchanged** (the same `seq`)
  twice: after about 20 ms and about 100 ms. If the original arrived, the repeat
  is skipped; if it was lost, the repeat delivers the final state.
- Unit: 1/100 dp of the finger on the sender's screen; acceleration
  and conversion to pixels are done by the receiver. Scrolling: `y` positive downwards,
  `x` positive to the right.
- `t` is the sender's timestamp in microseconds from a monotonic clock,
  from an arbitrary point; it is used only for interpolating motion. The sides' clocks are
  not synchronized: the receiver compares only differences of `t`.

Example of loss and reordering (no buttons; motion along `x`, the sender
moves by +100, then +1, then scrolls):

```text
sender sends:      seq 10 motion x=100   seq 11 motion x=101   seq 12 scroll y=240
receiver gets:     seq 12 scroll y=240   → scrolls by 240   (scroll_seq=12)
                   seq 11 motion x=101   → pointer +101     (pointer_seq=11)
                   seq 10 motion x=100   → skipped, 10 ≤ 11
result: pointer +101, scrolled 240; if the datagram seq 10 had been lost,
the result would be the same.
```

### 8.4 Buttons and the epoch

Button transitions go over the `input` stream (reliably and in order),
and datagrams may overtake them or arrive late after them. So every
event carries an **epoch** `b`: the number of button transitions sent in this
session up to the moment the event was sent, and every transition carries the **full**
continuous state of the sender (`pointer` and `scroll`) at the moment of the transition.

1. `PointerButton` carries `b` after its transition: the first transition in a session
   has `b` = 1, each next one 1 more. The receiver applies them in stream
   order and checks that `b` = its `epoch` + 1; otherwise it is a
   `PROTOCOL_VIOLATION` for the stream (`STOP_SENDING`).
2. The receiver applies `PointerButton` like this: first the `pointer` state (`x`, `y`)
   and the `scroll` state (`sx`, `sy`) with the button's `seq` (section 8.3), then
   presses or releases the button, then sets `epoch` = `b`, finally
   reviews the set-aside datagrams (section 8.5).
3. A motion or scroll datagram of the live session with `b`:
   - `b` = `epoch`: applied at once (section 8.3),
   - `b` < `epoch`: skipped. This is safe for both classes: the transition
     with an epoch > `b` was sent later and brought the full state of
     `pointer` and `scroll`, which includes this datagram,
   - `b` > `epoch`: set aside (section 8.5), because the stream hasn't delivered
     the transitions that precede it yet.
4. When a session closes (section 8.2) the receiver MUST release all
   buttons pressed in that session.

Example: dragging, where a datagram overtakes the buttons (left
button, `x` axis, no scrolling):

```text
sender sends:     seq 10 down   x=100 b=1   (stream)
                  seq 11 motion x=200 b=1   (datagram)
                  seq 12 up     x=200 b=2   (stream)
                  seq 13 motion x=300 b=2   (datagram)
receiver gets:    seq 11 motion b=1 → epoch 0 < 1: set aside
                  seq 13 motion b=2 → replaces the set-aside seq 11 (newer)
                  seq 10 down x=100 → pointer to x=100, press, epoch=1;
                                      set-aside b=2 > 1: keeps waiting
                  seq 12 up x=200   → pointer to x=200, release, epoch=2;
                                      set-aside seq 13 b=2: pointer to x=300
result: press at x=100, drag to x=200, release at x=200,
then motion to x=300 without a button.
```

Example: scrolling overtaken by a button:

```text
sender sends:     seq 1 scroll y=240 b=0                (datagram)
                  seq 2 down x=0 y=0 sx=0 sy=240 b=1    (stream)
receiver gets:    seq 2 down → scroll to sy=240 (scrolls by 240), press,
                               epoch=1
                  seq 1 scroll b=0 → skipped, 0 < 1; the button already
                               brought its state
result: scrolled by 240 before the press, as on the sender.
```

### 8.5 Setting datagrams aside

The receiver keeps per class (motion, scrolling) at most one set-aside
datagram of the live session `cur` (with `b` > `epoch`) and at most one for each
future session, for at most 4 future sessions (on the fifth it skips
the set-aside ones of the oldest of them). In each place the datagram
with the larger `seq` stays. Together that is at most 10 datagrams. One per place
is enough, because the state is cumulative, and the state at the moment of each button
transition is carried by `PointerButton` itself.

**Review** of set-aside datagrams (after accepting `InputSession` and after each
applied `PointerButton`), for each set-aside datagram:

- `s` < `cur`: skip,
- `s` = `cur` and `b` < `epoch`: skip,
- `s` = `cur` and `b` = `epoch`: apply (section 8.3),
- `s` = `cur` and `b` > `epoch`: leave it in the live session's place,
- `s` > `cur`: leave it in the future session's place.

When a session closes the receiver skips the set-aside datagrams with `s` ≤ `cur`;
the set-aside ones of a future session stay (as long as the receiver still announces
`input/1`). When the receiver withdraws `input/1`, it skips all set-aside
datagrams. The `input` stream is reliable, so a set-aside datagram waits
at most as long as it takes the missing frames to arrive.

Example: a datagram of a new session before its `InputSession` (session 0 closed,
`cur` = 0):

```text
sender:   new stream 01 04 + InputSession s=1 g=2, then a motion datagram
          s=1 seq 1 x=1 b=0 and its two repeats
receiver: datagram s=1 (and the repeats) → 1 > cur: set aside (future session)
          InputSession s=1 g=2 → g ≥ g_min=2: cur=1, live, zero state; review:
                             s=1, b=0 = epoch → pointer +1
result: the motion +1 arrives, even though all copies of the datagram overtook
the InputSession frame.
```

Example: a datagram of a closed session after re-enabling, before the new
`InputSession`:

```text
session 0 live (g=0): the receiver applied pointer x=100 (seq 7)
the receiver withdraws input/1 (Capabilities no. 1) → session 0 closed;
  the sender had already sent x=150 (s=0, seq 8, b=0)
the receiver enables input/1 (Capabilities no. 2) → g_min=2;
  InputSession s=1 hasn't arrived yet
  datagram x=150 (s=0) arrives → s=0 ≤ cur=0, session closed: skipped
  (the same if it arrived while input/1 was off or after InputSession s=1)
sender: InputSession s=1 g=2, the user moves by +1 → datagram x=1 (s=1)
  receiver: zero state, x=1 → pointer +1
result: only +1 reaches the system; the +50 from the time it was off is lost.
```

Example: a session opened before the withdrawal whose frames arrived only after
re-enabling (`cur` none):

```text
the sender opens session 0 (g=0) and sends motion; frames and datagrams are delayed
the receiver withdraws input/1 (no. 1), then enables it (no. 2) → g_min=2
  datagrams s=0 arrive → state "none": set aside as a future session
  InputSession s=0 g=0 arrives → g=0 < 2: cur=0, closed, STOP_SENDING;
                                   set-aside s=0 skipped
the sender, after receiving announcement no. 2, opens session 1 (g=2): live
result: nothing from session 0 reaches the system after re-enabling.
```

### 8.6 Receiver state table

The receiver's state for one sender: `cur` and the session state (section 8.2),
`epoch`, (`pointer_seq`, `pointer`), (`scroll_seq`, `scroll`), set-aside
datagrams (section 8.5), pressed buttons. "Announces" means: the receiver
announces `input/1`.

| event | condition | action |
|---|---|---|
| datagram (`s`, `b`) | doesn't announce | skip |
| | state "none" or `s` > `cur` | set aside as a future session |
| | `s` < `cur`, or `s` = `cur` and the session closed | skip |
| | `s` = `cur`, live, `b` < `epoch` | skip |
| | `s` = `cur`, live, `b` = `epoch` | apply (8.3) |
| | `s` = `cur`, live, `b` > `epoch` | set aside in the live session's place |
| `InputSession` frame (`s`, `g`) | `cur` exists and `s` ≤ `cur` | `STOP_SENDING` `PROTOCOL_VIOLATION` |
| | doesn't announce or `g` < `g_min` | `cur` = `s`, zero state, session closed, `STOP_SENDING` `CANCELLED` |
| | otherwise | `cur` = `s`, live, zero state, review (8.5) |
| a frame of a closed session's stream | | skip |
| `PointerButton` frame `b` | `b` ≠ `epoch` + 1 | `STOP_SENDING` `PROTOCOL_VIOLATION` |
| | otherwise | apply `pointer` and `scroll`, the transition, `epoch` = `b`, review |
| `PointerMotion` / `Scroll` frame (stream, 8.7) | | apply (8.3) |
| end of the stream, the receiver's `STOP_SENDING` | live session | close: release buttons, skip set-aside ones with `s` ≤ `cur` |
| the receiver withdraws `input/1` | | close as above and skip all set-aside ones |
| the receiver enables `input/1` | | `g_min` = the number of this announcement |
| end of the connection | | release buttons, clear everything |

Frames from the `input` stream always belong to its session, which is live
as long as the stream goes on (section 3.1 guarantees that the frames of a new stream
arrive after the end of the previous one).

### 8.7 Datagrams or the stream

The path of continuous events is fixed **once per connection**:

- When datagrams are available on the connection (both sides announced
  `max_datagram_frame_size`), `PointerMotion` and `Scroll` go **only**
  as datagrams.
- When they aren't available, they go **only** over the `input` stream; then
  all events are in stream order, the epoch always matches
  and nothing is set aside.

### 8.8 `input` message types

| type | name | path |
|---|---|---|
| `0` | `PointerMotion` | datagram or stream (section 8.7) |
| `1` | `Scroll` | datagram or stream (section 8.7) |
| `2` | `PointerButton` | stream |
| `3` | `InputSession` | stream, first frame |
| `4` | `Gesture` | stream; only when the receiver announces input/2 |

#### `PointerMotion` (0)

| key | name | type | |
|---|---|---|---|
| 0 | `seq` | u64 | required |
| 1 | `x` | i64, cumulative | required |
| 2 | `y` | i64, cumulative | required |
| 3 | `t` | u64, µs | optional |
| 4 | `s` | u32, session | required |
| 5 | `b` | u64, epoch | required |

Example datagram: `seq` 1, `x` −350, `y` 120, `t` 5 000 123, session 0,
epoch 0, 24 B:

```text
01 04 82 00 a6 00 01 01 39 01 5d 02 18 78 03 1a
00 4c 4b bb 04 00 05 00
```

Motion while dragging (after the `PointerButton` from the example below): `seq` 3,
`x` −310, `y` 95, session 0, epoch 1, without `t`, 18 B:

```text
01 04 82 00 a5 00 03 01 39 01 35 02 18 5f 04 00
05 01
```

#### `Scroll` (1)

| key | name | type | |
|---|---|---|---|
| 0 | `seq` | u64 | required |
| 1 | `x` | i64, cumulative | required; horizontal |
| 2 | `y` | i64, cumulative | required; vertical, positive downwards |
| 3 | `t` | u64, µs | optional |
| 4 | `s` | u32, session | required |
| 5 | `b` | u64, epoch | required |

Example datagram: `seq` 4, `x` 0, `y` 2400, session 0, epoch 1, 17 B:

```text
01 04 82 01 a5 00 04 01 00 02 19 09 60 04 00 05
01
```

#### `PointerButton` (2)

| key | name | type | |
|---|---|---|---|
| 0 | `seq` | u64 | required |
| 1 | `button` | number: 0 left, 1 right, 2 middle | required; unknown: state and epoch only |
| 2 | `down` | bool | required; `true` pressed |
| 3 | `x` | i64, the sender's `pointer` state | required |
| 4 | `y` | i64, the sender's `pointer` state | required |
| 5 | `b` | u64, the epoch after this transition | required |
| 6 | `sx` | i64, the sender's `scroll` state | required |
| 7 | `sy` | i64, the sender's `scroll` state | required |

Example frame: `seq` 2, left pressed at the point `x` −350, `y` 120,
epoch 1, scrolling (0, 0), 23 B:

```text
16 82 02 a8 00 02 01 00 02 f5 03 39 01 5d 04 18
78 05 01 06 00 07 00
```

#### `InputSession` (3)

| key | name | type | |
|---|---|---|---|
| 0 | `s` | u32, session number | required |
| 1 | `g` | u64, the receiver's announcement number (section 8.2) | required |

Example: the start of the first `input` stream in a connection: the header
and `InputSession` with `s` 0, `g` 0 (`input/1` from `HelloAck`), 10 B together:

```text
01 04 07 82 03 a2 00 00 01 00
```

### 8.9 Windows gestures (`input/2`, WN-51)

CCS/1 COMPAT:input/2 extends input/1 on the same channel 04; the datagram format stays identical. input/1 alone allows motion, scrolling and buttons. A sender MUST check the current input/2 before sending a Gesture; withdrawing input withdraws both versions and ends the session, releasing buttons.

`Gesture` (type 4) has these required CBOR fields:

| key | name | type |
|---|---|---|
| 0 | seq | u64 |
| 1 | gesture | u64: 0 TaskView (Win+Tab), 1 ShowDesktop (Win+D), 2 PreviousApp (Alt+Shift+Tab), 3 NextApp (Alt+Tab) |
| 2 | x | i64, cumulative pointer |
| 3 | y | i64, cumulative pointer |
| 4 | b | u64, the epoch after the gesture |
| 5 | sx | i64, cumulative scroll |
| 6 | sy | i64, cumulative scroll |

CCS/1 RULE:a Gesture is a barrier like PointerButton (§8.4): the next epoch is exactly b+1, first the full pointer/scroll state, then the single gesture, then the set-aside datagrams of the new epoch. An unknown gesture code updates the state and the epoch without a system action. A known type 4 without input/2 support, or a wrong epoch, causes a PROTOCOL_VIOLATION. An older program that doesn't know type 4 skips the unknown frame under §4.4; it can't update the epoch, though, so a sender MUST NOT send a gesture to input/1. Gestures are not allowed in datagrams or in actions/1.

CCS/1 EXT:WN-52 adds keys/text through input/3 on channel 04 (§8.10), with the same seq/epoch and reliable-barrier rules. Text/keys are not part of input/2.

CCS/1 IO:the PC receiver coalesces motion over at most one frame (8.333 ms), without motion prediction; a barrier flushes earlier motion. The end of a session removes pending motion and releases buttons. SendInput limits motion to ±4096 px and the wheel to ±1200 units per frame; a gesture is one array of all key-down/key-up. After a partial send, one recovery attempt releases, in reverse order, only the keys still pressed by the accepted prefix; before Win-up it adds a neutral VK 0xFF down/up so as not to open Start. Then it withdraws input and requires it to be enabled locally again.

CCS/1 SEC:the desktop check rejects a desktop other than Default (e.g. lock/UAC); it doesn't check the integrity level of the window on the Default desktop. UIPI is not bypassed. LIMIT:SendInput reports the number of inserted events, but its result and GetLastError don't identify UIPI as the cause of a refusal; the absence of an error is not a confirmation that the target app performed the action. The program doesn't guarantee detecting a UIPI block, nor a message/withdrawal for every such situation. DEP:[Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput).

### 8.10 Keyboard (`input/3`, WN-52)

CCS/1 COMPAT:input/3 extends input/1 and input/2 on channel 04; the receiver announces all three versions. The sender checks the current input/3 before and after opening the stream. Text (type 5) and KeyStroke (type 6) are only reliable barriers on the current input stream, never datagrams. They must not be sent to input/1 or input/2. A receiver of an older version rejects a recognized keyboard frame as a PROTOCOL_VIOLATION.

| key | Text (5) | KeyStroke (6) |
|---|---|---|
| 0 | seq, u64 | seq, u64 |
| 1 | UTF-8 text, 1–4096 bytes, no NUL | key code, u64 |
| 2, 3 | cumulative x, y, i64 | cumulative x, y, i64 |
| 4 | epoch after the barrier, u64 | epoch after the barrier, u64 |
| 5, 6 | cumulative sx, sy, i64 | cumulative sx, sy, i64 |
| 7 | none | modifiers, u64: Ctrl=1, Alt=2, Shift=4 |

CCS/1 RULE:the epoch is exactly the previous one+1; first the full pointer/scroll state, then the text or a single complete shortcut, then the set-aside datagrams. An unknown key code passes the barrier without an action. Other modifier bits are an error; any combination of bits 0–2 is allowed. There is no remote holding of keys and no Win/system key. The codes have Windows VK values: Backspace=8, Tab=9, Enter=13, Escape=27, Space=32, PageUp=33, PageDown=34, End=35, Home=36, arrows Left/Up/Right/Down=37/38/39/40, Delete=46, digits 48–57, letters A–Z=65–90, F1–F12=112–123. The set is closed; an unknown VK must not be passed to the system.

CCS/1 LIMIT:the payload of a Text/KeyStroke frame, also with extension fields, is at most 8192 bytes (the general stream buffer stays bounded by §4.2). A typical Text with the maximum text has less than 4224 bytes. Android queues at most 8 batches; the keyboard sends one command per batch, at most 4096 bytes of text. The FFI accepts at most 64 commands and 4096 bytes of text in total per batch; it checks the whole before sending the first command. An overflow stops the session instead of putting input off until later.

CCS/1 IO:the Android field is a local draft, with no saving/restoring and no logging. Send works only after IME composition has finished; text that is too long stays local, without truncation. One click uses up the draft once. Pause, losing focus, withdrawing input and a connection change delete the draft and the queue. Re-enabling doesn't restore old text. An older PC keeps the touchpad, with a note that the keyboard needs an update.

CCS/1 SEC:the PC uses KEYEVENTF_UNICODE (UTF-16, wVk=0, down/up pairs for each unit, surrogate pairs too) for text; a shortcut has the modifiers' down, the key's down/up, the modifiers' up in reverse order. A guard refuses if any modifier, or any VK touched by the shortcut, is physically pressed. A partial SendInput releases only the still-pressed events of the accepted prefix, keeping wVk+wScan+the Unicode/extended flags; it doesn't create a VK0 release in place of a Unicode unit. Then it withdraws input without retrying. The Default desktop/UIPI limits and the absence of a guarantee that the app receives it from §8.9 remain; not every app accepts Unicode VK_PACKET. Source: [Microsoft KEYBDINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput).

CCS/1 IO:CBOR payload vectors, seq=1, epoch=1, pointer/scroll=0:
- Text `ą`: `82 05 a7 00 01 01 62 c4 85 02 00 03 00 04 01 05 00 06 00`.
- Ctrl+C: `82 06 a8 00 01 01 18 43 02 00 03 00 04 01 05 00 06 00 07 01`.

### 8.11 Presentation mode (`input/4`, WN-53)

CCS/1 COMPAT:input/4 extends input/1–3 on channel 04; the receiver announces all four versions. Switching slides doesn't need input/4: it is plain KeyStroke (§8.10) Right=39 (next) and Left=37 (previous), which PowerPoint and slideshows in the browser (Google Slides, reveal.js, PDF) handle without configuration. input/4 adds only the laser pointer. The sender checks the current input/4 before and after opening the stream; type 7 must not be sent to an older receiver, and a receiver without input/4 rejects a recognized frame as a PROTOCOL_VIOLATION.

`Laser` (type 7), only a reliable barrier on the input stream, never a datagram:

| key | name | type |
|---|---|---|
| 0 | seq | u64 |
| 1 | on | bool: `true` show, `false` hide |
| 2, 3 | x, y | i64, cumulative pointer |
| 4 | b | u64, the epoch after the barrier |
| 5, 6 | sx, sy | i64, cumulative scroll |

CCS/1 RULE:the epoch is exactly the previous one+1; first the full pointer/scroll state, then the visibility change, then the set-aside datagrams. Pointer motion is plain PointerMotion (§8.3), so the dot follows the system cursor. The end of a session, withdrawing input and a refusal to insert hide the pointer like releasing buttons (§8.4 point 4).

CCS/1 IO:the phone sends `on=true` when the button is held, motion from the gyroscope (X axis: tilt → y, Z axis: rotation → x, about 110 000 units per radian, dead zone 0.02 rad/s), `on=false` on release. The PC draws a red dot in a layered topmost window, transparent to clicks and without activation (WS_EX_LAYERED | TRANSPARENT | NOACTIVATE | TOOLWINDOW), so the slideshow keeps the focus; the window follows the cursor on an 8 ms timer only while the pointer is on. On a desktop other than Default the pointer doesn't show. The phone's volume buttons switch slides only in presentation mode with an active session; the screen doesn't turn off, and the talk timer runs locally on the phone (it starts at the first slide).

CCS/1 IO:CBOR payload vector of `Laser` on, seq=1, epoch=1, pointer/scroll=0: `82 07 a7 00 01 01 f5 02 00 03 00 04 01 05 00 06 00`.

## 9. Feature channels and specification changes

The channels `clipboard`, `notify`, `media` and `files` have in v1.0 only a number,
a capability and the stream rules (section 3). Their message types are added by
the features' cards (WC-35, WC-38, WC-42 (§9.7), WC-46 and later) as changes to this
document:

1. A new type or optional field: the next minor version (1.1, 1.2, …),
   a new entry in the channel's type table, a binary example as in sections 7–8.
2. A change to the meaning of an existing type or field: a new capability version
   (`clipboard/2`), and for framing a new major version.
3. Type and key numbers are never reused, even after withdrawal.
4. Common rules: messages of a uni channel are one-way, and answers
   are subject to section 6, rule 4. The `files` stream is bidirectional:
   the file sender opens it (header `01 05`, then the messages of section
   9.1), the receiver writes its answers in the return direction without a header, and
   the end of the transfer is FIN from both sides. A `files` stream is accepted only by
   a side that announced `files/1`. The same goes for `actions` (§9.2, header
   `01 06`), accepted only by a side with `actions/1`, for `sms` (§9.11,
   header `01 0d`), accepted only by a side with `sms/1`, and for `gallery`
   (§9.13, header `01 0f`), accepted only by a side with `gallery/1`.

### 9.1 The `files` channel (`05`, WC-38)

The messages of `files/1`. The channel had no message types before, so
they are versioned by the capability `files/1` itself (section 9, rule 2), without changing
the protocol's minor version.

A **transfer** is one file. The sender picks a random `id` (16 B) and splits
the file into **chunks** of a fixed size `chunk` (the last one may be shorter).
Chunk `i` starts at byte `i × chunk`. A transfer uses from 1 to
`streams` `files` streams at once; each chunk may go over any of them.
A folder is a separate transfer for each file, with a relative path.

From the file sender (the channel's types in this direction):

| type | name | `body` fields |
|---|---|---|
| 0 | `Offer` | `0 id`: bytes, 16 B; `1 path`: text, 1–1024 B; `2 size`: uint; `3 chunk`: uint; `4 streams`: uint 1–8; `5 modified`: uint, optional (ms since the Unix epoch) |
| 1 | `Join` | `0 id`: bytes, 16 B; `1 stream`: uint 1–7 |
| 2 | `Chunk` | `0 index`: uint; `1 data`: bytes; `2 sha256`: bytes, 32 B |

From the file receiver (the return direction of the same streams):

| type | name | `body` fields |
|---|---|---|
| 0 | `Accept` | `0 have`: bytes, a bitmap of the chunks the receiver already has |
| 1 | `Ack` | `0 index`: uint; the chunk is checked and written |
| 2 | `Bad` | `0 index`: uint; the checksum doesn't match |
| 3 | `Complete` | empty map; the file is under its target name |

Flow:

1. The transfer's first stream starts with `Offer`. `chunk` is a
   multiple of 65 536 in the range 65 536–524 288 B, and the number of chunks
   `⌈size / chunk⌉` doesn't exceed 2²² (the `have` map has at most 512 KiB).
   Values out of range are `MALFORMED_FRAME`.
2. The receiver answers `Accept`. In `have`, chunk `i` is byte `i / 8`,
   bit `0x80 >> (i mod 8)`; an empty map means "I have nothing". A refusal is
   `STOP_SENDING` and `RESET_STREAM` of the first stream: a bad path
   `MALFORMED_FRAME`, too little space `NO_SPACE`, too many transfers
   `RATE_LIMITED`, the user's refusal `CANCELLED`.
3. After `Accept` the sender may open up to `streams − 1` further streams;
   each starts with `Join` with the `id` and the stream number (each number once).
   A `Join` whose `Offer` hasn't arrived within 5 s is stopped by the receiver
   with the code `CANCELLED`. Waiting streams count towards the receiver's limits
   (recommended: 32 at once, `RATE_LIMITED` at once beyond the limit).
4. The sender sends a `Chunk` for every chunk that isn't in `have`.
   `data` has exactly the chunk's length, and `index` is less than
   the number of chunks; otherwise `MALFORMED_FRAME`. The receiver computes the SHA-256
   of `data`: a matching chunk is written and answered with `Ack` on the same
   stream, a mismatching one is rejected and answered with `Bad`. The sender then sends
   the chunk again, at most 3 times, then ends the transfer.
5. When the receiver has all chunks, it writes the file durably, gives it
   its target name and only then sends `Complete` on the first stream
   and FIN on all of them. The sender considers the transfer finished only after
   `Complete` (not after the last `Ack`) and answers with FIN.
6. Cancellation by either side: `RESET_STREAM` and `STOP_SENDING`
   with the code `CANCELLED` on all the transfer's streams. The receiver then deletes
   the partial file.
7. Withdrawing `files/1` (section 6, rule 5) works like the receiver cancelling
   each of its ongoing transfers; new `files` streams
   while it is withdrawn get `CAPABILITY_MISSING`. Such a transfer is not
   "cancelled by the user", so one retried (the same `id`) after re-enabling
   is accepted.

**Resumption.** A broken connection doesn't end a transfer. After
reconnecting, the sender sends the same `Offer` (the same `id`), and the receiver
gives in `have` the chunks it has durably written; only the remaining ones
are sent. The receiver keeps the partial file separately for each
device, under the pair (the sender's fingerprint, `id`), because the `id` is chosen by the other side,
and **outside** the target folder (e.g. in the app's state folder), so that
no `path` from the other side can reach the partial files or their
state.
A chunk's bit goes into the stored state only after its data has been written
durably (first syncing the file, then atomically replacing the state), so
after a power failure resumption won't skip an unwritten chunk. The state
contains `size`, `chunk` and `modified` from the `Offer`; resumption requires all
three to match, otherwise the receiver starts over with an empty `have`.
The receiver deletes abandoned partial files (unchanged for 7 days, recommended).

**No whole-file hash.** Every byte of the file arrives in a chunk
whose SHA-256 the receiver checks before writing and marking it, and the durability of
the state follows the durability of the data. A whole-file hash on the sender's side
would need a second read and wouldn't detect a change of the source between the reads.
Instead, before sending the last chunk (and every 64
chunks), the sender checks whether the source's size and modification time are as in
the `Offer`; a changed source ends the transfer with the code `CANCELLED`.

**Paths.** `path` comes from the other side, so the receiver checks it
itself, regardless of the sender's system, and fixes nothing; a path
breaking any of the rules is `MALFORMED_FRAME`, without creating a file:

- the only separator is `/`; `\` in a path is not allowed;
- empty parts, `.` and `..` are not allowed (nor a leading `/`, that is an
  absolute path); at most 32 parts, each up to 255 B of UTF-8;
- disallowed characters: `:` (drive letters, NTFS streams `file:stream`),
  `<>"|?*`, control characters (Cc) and text direction characters (U+061C, U+200E,
  U+200F, U+202A–U+202E, U+2066–U+2069);
- disallowed Windows device names: `CON`, `PRN`, `AUX`, `NUL`,
  `COM1`–`COM9`, `LPT1`–`LPT9` (also with the digits ¹²³), regardless of
  case and extension (`nul.txt`), and a part ending with a
  dot or a space (Windows trims them).

The receiver creates subfolders in its target folder one at a time and rejects
any that is a link or a junction point; the target path
must lie in the target folder. It never overwrites an existing file:
on a name collision it adds " (1)", " (2)"… before the extension,
reserving the name atomically (creating a new file only when it doesn't
exist), so two transfers finishing at the same time get different names.

**Mark of origin (Windows, WN-96).** A file from the other device
may be a program or a document with macros, so the receiver on Windows
adds the NTFS stream `<file>:Zone.Identifier` (Mark-of-the-Web) to it, as
a browser does. It does so before giving the target name (renaming in
NTFS keeps the streams), so the file never lies in the target folder
without the mark; after copying from another volume it adds it once more:

```
[ZoneTransfer]
ZoneId=3
HostUrl=windconnect:<the sender's fingerprint, 64 hex characters>
```

with CRLF line endings. `ZoneId=3` (the Internet zone) makes SmartScreen
and Office's Protected View warn before opening. Writing it is
best effort: a volume without streams (FAT32, exFAT, some network
shares) or another write error changes nothing, the transfer ends with
`Complete` as usual. The mark is not part of the protocol (it doesn't go over the
network); Android doesn't have it.

**Space.** Before `Accept` the receiver checks the free space for the missing
part of the file with a margin (64 MiB recommended); too little is `NO_SPACE`. The sender doesn't
retry such a transfer by itself.

Examples (frames with length):

| message | bytes |
|---|---|
| `Offer` id `11`×16, `a/b.txt`, 300 000 B, chunk 262 144, 4 streams | `2c 82 00 a5 00 50 11…11 01 67 61 2f 62 2e 74 78 74 02 1a 00 04 93 e0 03 1a 00 04 00 00 04 04` |
| `Join` id `11`×16, stream 1 | `17 82 01 a2 00 50 11…11 01 01` |
| `Chunk` 1, `hi`, sha256 of zeros (an encoding example) | `2c 82 02 a3 00 01 01 42 68 69 02 58 20 00…00` |
| `Accept` have `80` | `06 82 00 a1 00 41 80` |
| `Ack` 1 | `05 82 01 a1 00 01` |
| `Bad` 1 | `05 82 02 a1 00 01` |
| `Complete` | `03 82 03 a0` |

### 9.2 The `actions` channel (`06`, WN-54)

Remote actions: a device asks the other to lock the screen, sleep,
open a link or run an action that the user of the **other**
device added to their allowed list themselves. The messages of `actions/1`;
like `files/1`, they are versioned by the capability itself, without changing the minor version.

**The overriding rule.** A request names an action only by identifier
(and an address for `open-url`). The protocol has no field for a command, a program,
a path or arguments: what a custom action does is decided only by the user of
the device that runs it, on that device. A side MUST NOT run
anything that isn't on its own allowed list at the moment of
running.

**Stream.** One request is one bidi stream opened by the
requester: header `01 06`, one message, FIN. The executor answers with
one message in the return direction (without a header) and FIN. Frames on this
channel have a limit of 16 384 B (§4.2).

From the requester:

| type | name | `body` fields |
|---|---|---|
| 0 | `Run` | `0 action`: text, the action identifier; `1 url`: text 1–2048 B, optional, only for `open-url` |
| 1 | `List` | empty map; which actions the side would run now |

From the executor:

| type | name | `body` fields |
|---|---|---|
| 0 | `Result` | `0 status`: uint, the result of `Run` (table below) |
| 1 | `Actions` | `0 actions`: an array of up to 64 pairs `[id, label]`; `label` text 1–64 B, for display |

**Identifiers.** 1–64 bytes of `a-z`, `0-9` and `-`; any other character is
`MALFORMED_FRAME`. Built-in: `lock` (lock the screen), `sleep`
(sleep), `open-url` (open an address). Custom actions start with
`x-`, so they never shadow a built-in one. A side that can't do something
(a phone doesn't put a computer to sleep) doesn't offer it in `Actions` and
answers `Run` with status `1`.

**Labels (WN-165).** The receiver shows a built-in action by its identifier,
in its own user's language; the `label` is only a fallback, for an identifier
the receiver doesn't know. The executor sends built-ins with English labels
(`Lock`, `Sleep`, `Open link`); versions up to 0.81.0 sent Polish ones, which
a receiver that knows the identifier never shows. A custom action's `label`
is the name its owner gave it and is shown as it is.

| `status` | meaning |
|---|---|
| 0 | done (for `sleep`: sleep scheduled, the answer goes out before it) |
| 1 | refused: the action isn't on the allowed list, is turned off or unavailable |
| 2 | bad argument: a missing or disallowed `url`, or a `url` with an action other than `open-url` |
| 3 | allowed, but running it failed |
| 4 | busy: too many requests in a short time |

The requester treats an unknown `status` like `3`.

**Order of checks at the executor:**

1. The stream is accepted only by a side that announced `actions/1` (§3,
   rule 3); a connection exists only with a pinned key (§2).
2. The request must arrive within 5 s of the stream opening (otherwise
   `CANCELLED`). An invalid message: `MALFORMED_FRAME`, an unknown type:
   `UNKNOWN_MESSAGE`; in both cases `STOP_SENDING` and `RESET_STREAM`,
   no answer and nothing run.
3. **Before every answer** the side checks again that the other side's key
   is still pinned. If not (unpaired during the
   connection), it closes the connection with the code `UNTRUSTED_PEER` and runs
   nothing.
4. The allowed list is read at the moment of running, not at
   connection time: turning an action off takes effect at once. A list that can't be
   read means that nothing is allowed.
5. `open-url` accepts only an absolute `http://` or `https://` address
   (any case of the scheme): printable ASCII without spaces and without the
   characters `"` `<` `>` `\` `^` `` ` `` `{` `|` `}`, a non-empty host, without
   a `user@` part, a port (if any) of 1–65535. Any other address
   (`file:`, `javascript:`, app links, relative addresses, characters outside
   ASCII; international names only in the `xn--` form) is status `2`;
   the side doesn't fix it.
6. At most one action runs at a time per connection, at most 10 within
   10 s (status `4` beyond that); at most 4 requests wait, further
   streams get `RATE_LIMITED`.
7. A custom action starts the program chosen by the user of this
   device with its fixed arguments, without a system shell (on
   Windows only an `.exe` file, not a `.bat`/`.cmd` script).
8. **The phone as the executor (WN-100).** The phone announces `actions/1` and
   offers only `open-url` (it doesn't lock, doesn't sleep, has no custom
   actions) as long as its user has "Open links from the computer"
   turned on; turned off gives status `1`. After the check from rule 5 the address
   opens at once when the app is on screen; in the background Android doesn't
   allow starting an activity, so the phone shows a notification to
   tap and answers `0` (the link was handed to the person).

Examples (frames with length):

| message | bytes |
|---|---|
| `Run` `lock` | `09 82 00 a1 00 64 6c 6f 63 6b` |
| `Run` `open-url`, `https://a.pl/` | `1c 82 00 a2 00 68 6f 70 65 6e 2d 75 72 6c 01 6d 68 74 74 70 73 3a 2f 2f 61 2e 70 6c 2f` |
| `List` | `03 82 01 a0` |
| `Result` 0 (done) | `05 82 00 a1 00 00` |
| `Result` 1 (refused) | `05 82 00 a1 00 01` |
| `Actions` `[["lock", "Zablokuj"]]` (a Polish label) | `14 82 01 a1 00 81 82 64 6c 6f 63 6b 68 5a 61 62 6c 6f 6b 75 6a` |

### 9.3 The `clipboard` channel (`01`, WN-35)

CCS/1
PURPOSE:text and links as plain UTF-8, both ways over the existing mTLS/E2E session. A link is not a command to open a URL. The first messages of the `clipboard/1` capability, like the initial `files/1`, without changing the ALPN/framing or the minor version.
RULE:both sides announce `clipboard/1`; the answer goes over a separate uni stream in the opposite direction, as in §6. A missing capability blocks sending.

| type | name | body map fields |
| --- | --- | --- |
| 0 | Set | `0`: a random 16 B identifier (bytes); `1`: a positive Lamport counter uint64; `2`: UTF-8 text (text), 0–262144 B |
| 1 | Applied | `0`: the 16 B Set identifier; `1`: result uint: 0 applied, 1 refused, 2 failed |

Validation: reject an invalid ID type/size, a zero/negative counter,
NUL in the text, an excess size, invalid values and repeated keys.
Unknown optional fields are skipped; an unknown type/invalid message
doesn't touch the clipboard and gets no false confirmation.
The answer **applied** is made only after a successful write through the system API,
**refused** when receiving is off or for an older/duplicate change,
**failed** when the OS write failed. The sender has up to 4 pending answers,
each with a 3 s deadline and cancellation when the session closes. Unknown/old
answer IDs don't change the state. No answer means no confirmation,
never success. The router's queue holds 4 messages; there is no history or offline queue.

Order and echo: the active session keeps the counter and the last key
`(counter, id)` in memory, and the process the SHA-256 of the last sent/received content. A local change
increments the counter; receiving raises it to the maximum of its own and the received one.
Keys are compared lexicographically (id as 16 unsigned bytes).
A Set with a key less than or equal to the last one is rejected. For
simultaneous changes with the same counter the larger random id wins;
both sides converge to that value without comparing wall clocks.
A failed write also moves the key, but not the hash of the applied content.
The automatic mode doesn't send the same hash again on the callback of its own
write or when focus returns. A manual attempt may resend the same content.
A reconnect doesn't restore old content. An adopted new session starts the counter
from 0 and removes the old key, so that a restart of the other side doesn't block its changes.
The hash stays. A change of generation ends the old receiving and the pending answers.
Android additionally requires a clipboard timestamp later than the session's acceptance
(ClipDescription.timestamp, the local System.currentTimeMillis); content
from before the session/process restart doesn't go out when focus is regained. An unknown
timestamp, or one equal to the boundary, is skipped; after the clock is set back
a manual send may be needed. The boundary needs no stored payload
and no comparing of the devices' clocks. A manual action bypasses only this boundary,
not the check for sensitive content.
Late Sets from an old session can't change the OS. A backup connection
trying to return from the relay to the LAN is not adopted for this feature before
the connection owner decides. There is no persistent clipboard log.

The sender's privacy (also manual): Windows doesn't read/send the payload
when the format `ExcludeClipboardContentFromMonitorProcessing` is present, or
`CanIncludeInClipboardHistory`/`CanUploadToCloudClipboard` contains DWORD 0
(damaged opt-out data is treated conservatively as private). Android
doesn't send content with `ClipDescription.extras[android.content.extra.IS_SENSITIVE]`.
Windows removes the CF_UNICODETEXT terminator before validation; HTML, images,
files, intents and provider URIs are not coerced to text.

The receiver's privacy: Windows writes the formats
`CanIncludeInClipboardHistory` = 0 and `CanUploadToCloudClipboard` = 0 before the text.
Android always writes received text with `EXTRA_IS_SENSITIVE` = true,
which also masks the preview in the Android 13+ overlay. This way the receiver
doesn't pass the content on through its own automatic mode/history/cloud. No content
goes into logs, notifications, frontend IPC or app files.

App policy: the automatic mode is the default. The switch turns off automatic
reading and receiving/overwriting; a manual send doesn't bypass the sensitivity flags.
Windows reacts to WM_CLIPBOARDUPDATE. Android reads only with focus:
a listener and a comparison of the current content when returning to the app. In the background
the phone sends after the tap of a notification action that opens a visible,
non-exported Activity from a direct immutable PendingIntent;
reading waits for focus/unlocking, without READ_LOGS, Accessibility or an IME.
Receiving PC→phone works in the connection service, also without an Activity.

Example Set, id=16 zeros, counter=1, text `hi` (without the header/frame):
`82 00 a3 00 50 00000000000000000000000000000000 01 01 02 62 6869`.
Applied for this ID: `82 01 a2 00 50 00000000000000000000000000000000 01 00`.

#### 9.3.1 Images (`clipboard/2`, WN-36)

CCS/1
PURPOSE:a clipboard image (a screenshot) that can be pasted on the other side at once: the thumbnail and the size first, the full PNG in the background.
RULE:a side that accepts images announces `clipboard/1` and `clipboard/2` (§6, rule 2: version N covers 1..N). An image goes only to a side with `clipboard/2`; for an older one the side says "doesn't support images", text keeps working.

| type | name | body map fields |
| --- | --- | --- |
| 2 | ImageSet | `0`: id 16 B; `1`: Lamport counter (the same as for text); `2`: `"image/png"` (the only one in v1); `3`: PNG size uint, 1 B–64 MiB; `4`, `5`: width and height uint, 1–16384; `6`: thumbnail PNG (bytes), 1 B–128 KiB, longer side ≤ 512 px |
| 3 | ImageChunk | `0`: the image id; `1`: offset uint; `2`: bytes 1 B–256 KiB |
| 4 | ImageDone | `0`: the image id; `1`: 0 complete, 2 failed |
| 1 | Applied | as above; for `refused` of an image over the receiver's limit, the optional field `2`: that limit in bytes |

Flow: the sender sends `ImageSet`, and after `Applied` the `ImageChunk`s
in order (offset = the sum of the previous ones) on the same stream of the channel.
The receiver gives `Applied` when the image is already in the OS clipboard (pasteable), and
not after the last byte: Android writes a `content://` URI of its
provider, Windows formats with delayed rendering. `ImageDone` goes
when the whole PNG has arrived (0) or the bytes were wrong (2: a gap, an excess).
The sender shows "sent" only after `ImageDone` 0; after `Applied` only
"sending". Waiting for `ImageDone` ends 10 s after the last
chunk.

Order and echo: an image and text share one "last clipboard" and the key
`(counter, id)` from §9.3. A newer change on either side ends
an image in flight: the receiver drops it (a reader gets an error, never
a mix of two images), skips chunks of the old id, and the sender
stops sending (`replaced`). The hash the sender remembers is the SHA-256 of the
PNG. A side doesn't send back its own write of a received image: Windows skips
a clipboard owned by its window, Android skips a clip with a URI of
its own provider (re-encoding would change the bytes, so not the hash).

Limit: the user's setting 5 / 16 / 50 MB of PNG (16 MB by default).
The sender doesn't send an image over its own limit (`too-large`), also
manually. The receiver rejects an `ImageSet` over its limit (`refused` with the field
`2`) before anything reaches the clipboard; the key doesn't move, so
a later, smaller change still wins.

Pasting before the transfer ends waits for the bytes; no new bytes for
10 s, a broken session or a newer clipboard end it with an error (the paste
fails, without a partial image). The thumbnail is only for previews
(Android: `ContentResolver.EXTRA_SIZE`, e.g. the clipboard overlay, the keyboard);
it is never pasted instead of the image. The image lives only in memory, one
at a time; after a process restart the old URI gives `FileNotFoundException`.

Privacy: the sender applies the same flags as for text (Windows: the privacy
formats, Android: `IS_SENSITIVE`) and then doesn't copy the image from the OS.
Windows writes a received image with `CanIncludeInClipboardHistory` = 0 and
`CanUploadToCloudClipboard` = 0, like text. Android does **not** mark
a received image as sensitive (unlike text): the image comes from
the user's own computer, which doesn't send private content, and
marking it would hide the thumbnail preview in the Android 13+ overlay. The phone
reads another app's image only with focus (the clip's URI permission), without
other permissions. Nothing from the content goes into logs, IPC or files.

Examples (with the frame's length header):

```text
ImageSet id 0×16, counter 1, 2×1 px, size 3, thumbnail [01 02]:
2c 82 02 a7 00 50 00…00 01 01 02 69 "image/png" 03 03 04 02 05 01 06 42 01 02
ImageChunk offset 0, bytes [07 08 09]:
1c 82 03 a3 00 50 00…00 01 00 02 43 07 08 09
ImageDone complete:         17 82 04 a2 00 50 00…00 01 00
Applied refused, limit 16 000 000:
1d 82 01 a3 00 50 00…00 01 01 02 1a 00 f4 24 00
```

### 9.4 The `battery` channel (`07`, WN-49)

The phone's battery level and charging, shown on the computer. The messages of
`battery/1`; like `actions/1`, they are versioned by the capability itself. The computer announces
`battery/1`, the phone doesn't (one direction). A uni channel, a message queue
as in §3.1; frames up to 16 384 B.

| type | name | `body` fields |
|---|---|---|
| 0 | `Battery` | `0 level`: number 0–100, percent; `1 charging`: bool, plugged in for charging (also full on the charger) |

Rules:

1. The sender sends one `Battery` right after the session starts, if the other
   side announced `battery/1`, and after that **only when** the pair
   (`level`, `charging`) **changes**. No timer and no repeats: no message means
   "no change".
2. Further messages go over one stream (§3.1, rule 5.5); the sender
   ends it when the capability is withdrawn or the session ends.
3. A missing field, a repeated key, `level` > 100 or a field of the wrong
   type: the receiver skips that message, the stream goes on. It skips unknown keys
   (§4.3).
4. The receiver keeps the last state for the session and forgets it when it ends;
   it stores nothing. Alerts (low level, fully charged) are a matter for
   the receiving app, not the protocol; the session's first message doesn't
   trigger them.

Example (with the frame's length header):

```text
stream header:                  01 07
Battery 82 %, charging:         08 82 00 a2 00 18 52 01 f5
```

### 9.5 The `find` channel (`08`, WN-50)

Find my phone: the computer asks the phone to ring loudly, also in silent
mode, and both sides can stop it. The messages of `find/1`; they are versioned by
the capability itself. The phone announces `find/1` (it can ring and allows it),
the computer doesn't. The phone's user can turn this off: then the phone doesn't
announce `find/1` (§6, rule 5), instead of refusing every request.

**Stream.** One ring is one bidi stream opened by the
requester: header `01 08`, `Ring`, then possibly `Stop`. The ringing side
answers in the return direction (without a header) with state `0` once the sound is already
playing, and then with exactly one final state and FIN; the requester ends its
direction with FIN after the final state. Frames up to 16 384 B.

From the requester:

| type | name | `body` fields |
|---|---|---|
| 0 | `Ring` | empty map |
| 1 | `Stop` | empty map |

From the ringing side:

| type | name | `body` fields |
|---|---|---|
| 0 | `State` | `0 state`: uint (table below) |

| `state` | meaning |
|---|---|
| 0 | ringing: the sound is already playing (sent only after playback starts) |
| 1 | stopped on this device by its user |
| 2 | stopped on request (`Stop`) or taken over by a newer `Ring` |
| 3 | the 60 s limit has passed |
| 4 | unavailable: the sound can't be played |

The requester treats an unknown `state` like `4` (final).

Rules:

1. `Ring` must arrive within 5 s of the stream opening (otherwise `CANCELLED`).
   An unknown type is `UNKNOWN_MESSAGE`, `Stop` before `Ring` is a
   `PROTOCOL_VIOLATION`, an invalid frame `MALFORMED_FRAME`; in every
   case `STOP_SENDING` and `RESET_STREAM`, without ringing.
2. One ring at a time. A `Ring` during ringing doesn't start it
   over: the new stream takes over the ringing (state `0` at once), and the
   previous one gets `2` and FIN. A `Stop` from the taken-over stream no longer
   stops anything.
3. Ringing lasts at most 60 s. The end of the requester's stream or session does
   **not** stop it (the user is still looking for the phone): it keeps ringing until
   the limit or until it is stopped on the phone.
4. The sound bypasses silent mode and vibration (on Android the alarm stream with
   a generated tone). Nothing is stored in the protocol; the app
   restores the volume it changed.

Examples (with the frame's length header):

```text
stream header:          01 08
Ring:                   03 82 00 a0
Stop:                   03 82 01 a0
State 0 (ringing):      05 82 00 a1 00 00
State 1 (on the phone): 05 82 00 a1 00 01
```

### 9.6 The `media` (`03`) and `media-control` (`09`) channels (WN-46)

One side's player shown and controlled on the other. Two capabilities,
because the directions do different things: `media/1` (I show the other side's player:
I accept `Now`, `Art`, `Gone` on `media`) and `media-control/1` (my
player can be controlled: I accept `Command` on `media-control`). In
WN-46 the computer announces `media/1` and the phone `media-control/1`; in the opposite
direction (WN-47: the computer's player on the phone) the computer announces
`media-control/1` and the phone `media/1`, with the same messages. Both
channels are uni with a queue as in §3.1.

The `media` channel (from the side with the player):

| type | name | `body` fields |
|---|---|---|
| 0 | `Now` | `0 title` text ≤ 256 B; `1 artist`? ≤ 256 B; `2 album`? ≤ 256 B; `3 app` text ≤ 64 B (the player's name); `4 duration_ms`? uint; `5 position_ms` uint (at the moment of sending); `6 playing` bool; `7 speed_permille` uint (1000 = normal; CBOR without floating-point numbers); `8 actions` uint, bits: 1 play, 2 pause, 4 next, 8 previous, 16 seek; `9 art`? 8 B, the `Art` key |
| 1 | `Art` | `0 key` 8 B (the first 8 B of the image's SHA-256); `1 image` JPEG or PNG ≤ 96 KiB (sender: ≤ 256 px per side) |
| 2 | `Gone` | empty map: no active player |

The `media-control` channel (from the controlling side):

| type | name | `body` fields |
|---|---|---|
| 0 | `Command` | `0 action` uint: 0 play, 1 pause, 2 toggle, 3 next, 4 previous, 5 seek; `1 position_ms` uint, only and mandatory with seek |

Rules:

1. `Now` at the start of the session (when the other side announced `media/1`) and on
   every change of the metadata, playback, speed, `actions` or a position
   other than the one following from elapsed time (a deviation > 2 s from extrapolation).
   Never periodically: the receiver computes progress itself,
   `position_ms + elapsed × speed_permille / 1000`, up to `duration_ms`.
2. `Art` before the first `Now` that names a new key; the receiver keeps
   only the last image. The receiver determines the image type from the first bytes
   (JPEG `FF D8 FF`, PNG `89 50 4E 47`), never from the sender's word; another or
   a larger image is an invalid message.
3. Texts and images come from the sender's other apps: the receiver shows
   them only as text (without interpreting markup), without control
   and bidirectional characters. Nothing is stored or logged.
4. An invalid message (text too long, a field of the wrong type, a missing field) is
   skipped; the stream goes on. A `Command` with an action that the last `Now` sent
   doesn't allow in `actions` is skipped by the side without a reaction.
5. A side announces `media-control/1` only when it can really
   control the player (on Android: access to media sessions and the setting
   turned on; on Windows: the setting "Share playback with the
   phone"), and `media/1` only when it wants to show the other side's
   player; also during the session (§6, rule 5). A side that accepts
   `media/1` again gets the current `Now` (and `Art`) from the sender anew.
6. **Consent.** On Android, reading other apps' media sessions requires
   notification access. WindConnect uses it only for the player and
   doesn't read the content of notifications. A feature that reads notifications (WN-42)
   MUST NOT silently extend this consent: it has its own switch (off by
   default) and its own explanation.
7. A side doesn't send the other side's own player back to it: a media
   session that it created itself to show the other side's player
   is never sent as `Now` (otherwise there would be an echo).

Examples (with the frame's length header):

```text
stream headers:              01 03 (media), 01 09 (media-control)
Gone:                        03 82 02 a0
Command next:                05 82 00 a1 00 03
Command seek 61 000 ms:      09 82 00 a2 00 05 01 19 ee 48
```

### 9.7 The `notify` channel (`02`, WN-42)

One side's notifications shown on the other. In WN-42 the phone sends;
the computer announces `notify/1` when it can show them (toasts, WN-43). A
uni channel with a queue as in §3.1. The channel had no message types before, so
they are versioned by the capability `notify/1` itself (section 9, rule 2).

From the side with the notifications:

| type | name | `body` fields |
|---|---|---|
| 0 | `Posted` | `0 id` 16 B; `1 app` text ≤ 64 B (the app's name); `2 package` text ≤ 128 B (the app's identifier, for the icon cache and the WN-45 filters); `3 title`? text ≤ 256 B; `4 text`? text ≤ 1024 B; `5 time` uint (ms since the Unix epoch, the moment of posting); `6 icon`? 8 B, the `Icon` key; `7 reply`? text ≤ 64 B: the label of the reply action, present only when the notification can be replied to (§9.8, WN-44); `8 quiet`? bool, only `true`: on the sender it doesn't interrupt ("Do not disturb" mode), the receiver shows it without a pop-up and sound (WN-45) |
| 1 | `Icon` | `0 key` 8 B (the first 8 B of the image's SHA-256); `1 png` PNG ≤ 16 KiB (sender: 64 px per side) |
| 2 | `Removed` | `0 id` 16 B: the notification is gone on the sender (dismissed or cancelled) |
| 3 | `Replied` | `0 id` 16 B; `1 ok` bool: whether the reply from `Reply` (§9.8) went to the app (WN-44) |

Rules:

1. `id` is constant for one notification on the sender and means nothing
   outside it: on Android it is the first 16 B of the SHA-256 of the installation's random salt
   and the notification key, so it can't be guessed from a known app.
   `Posted` with a known `id` replaces that notification.
2. Only notifications posted during the session are sent: at the
   start of a session the sender does NOT send those that already exist (no flood of
   notifications after every reconnect). A list of the current ones, should it be
   needed, is a separate message in a future version.
3. The sender skips persistent notifications (ongoing, foreground services,
   non-dismissible), group summaries, players (§9.6), secret ones
   (`VISIBILITY_SECRET`), its own and empty ones (without a title and content).
4. **Pace.** For one `id` at most one `Posted` per second: a change within
   a second of the previous one waits until the second ends and only the
   newest goes out. A notification with a progress bar goes out only at the start and
   at the end (when the bar disappears). A `Posted` with the same content as the last one sent
   (apart from `time`) doesn't go out.
5. `Icon` before the first `Posted` that names a new key in this session;
   the receiver keeps icons at least until the end of the session. When the receiver withdraws
   `notify/1` (§6, rule 5), the sender sends the icons again after it is re-enabled.
   The receiver determines the image type from the first bytes (PNG
   `89 50 4E 47`); another or a larger image is an invalid message.
6. `Removed` only for an `id` whose `Posted` went out in this session (not for
   skipped ones or ones from before the session).
7. Texts and icons come from the sender's other apps: the sender removes
   formatting, control and bidirectional characters and runs of whitespace;
   the receiver shows them only as text. Nothing is stored or
   logged. Content that the sender's system has hidden (Android 15+: one-time
   codes and other sensitive content) goes out in the form the
   system gave it; the sender doesn't get around that.
8. An invalid message (text too long, a field of the wrong type, a missing field) is
   skipped; the stream goes on.
9. **Consent.** On Android reading requires a separate consent (§9.6, rule 6):
   its own entry in the system's notification access and the setting
   "Notifications on the computer", off by default.
10. `7 reply` and `Replied` (WN-44) are compatible additions (section 9, rule 1):
    a receiver without them skips the unknown field (§4.3) and the unknown type (§4.4).
    The sender sets `reply` only for an action that accepts any text
    (on Android: `RemoteInput` with `getAllowFreeFormInput()`, not
    contextual; first `SEMANTIC_ACTION_REPLY`, then the first such one).
11. **App filter (WN-45).** The sender may skip chosen apps;
    a skipped app sends nothing (neither `Posted` nor `Icon`). When
    an app becomes skipped during a session, the sender sends `Removed`
    for its notifications sent in this session. `8 quiet` is a compatible addition
    (rule 10): the sender sets it when the notification doesn't get through
    its "Do not disturb" mode (Android: `Ranking.matchesInterruptionFilter()`
    = false). The receiver may also silence on its own (quiet hours, focus mode),
    but doesn't hide notifications and doesn't dismiss them on the sender.

Example (with the frame's length header): stream header `01 02`;
`Removed` with `id` = 16 × `07`, 22 B:

```text
15 82 02 a1 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07
```

`Replied` with the same `id`, `ok` = true, 24 B:

```text
17 82 03 a2 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 01 f5
```

### 9.8 The `notify-action` channel (`0a`, WN-44)

Dismissing a notification and replying to it from the side that shows it
(§9.7). A uni channel with a queue as in §3.1; it is opened by the side showing
the notifications and accepted by the side that sent them and announced
`notify-action/1`.

| type | name | `body` fields |
|---|---|---|
| 0 | `Dismiss` | `0 id` 16 B: dismiss this notification |
| 1 | `Reply` | `0 id` 16 B; `1 text` text 1–1024 B: send this reply through the notification's reply action |

Rules:

1. A side announces `notify-action/1` only when it sends notifications
   (§9.7, rule 9: consent and access), also during the session (§6, rule 5).
2. A side runs an action **only** for an `id` it sent in this
   session and that is still present; it skips any other `id` (to a `Reply` it answers
   only with `Replied` with `ok` = false).
3. `Dismiss` dismisses the notification on the sender just as
   the user would; the sender then sends the usual `Removed` (§9.7, rule 6).
   The showing side removes it on its side at once and tolerates a `Removed` for
   an `id` it no longer knows.
4. `Reply` goes to the app as if the text had been typed on the sender's
   device (Android: `RemoteInput.addResultsToIntent`, source
   `SOURCE_FREE_FORM_INPUT`). After the attempt the sender sends `Replied` (§9.7);
   the app usually also posts a changed notification (`Posted`).
5. The reply text is not stored or logged on either side.
   An empty text or one longer than 1024 B is an invalid message: the receiver doesn't
   trim it, it skips it; the showing side MUST NOT send such a
   text (the interface blocks sending with a message).
6. The showing side sends `Dismiss` only when the user explicitly closes
   it (not after it expires or is removed by the side itself).

Examples (with the frame's length header): stream header `01 0a`;
`id` = 16 × `07`. `Dismiss`, 22 B, and `Reply` with the text `ok`, 26 B:

```text
15 82 00 a1 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07
19 82 01 a2 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 01 62 6f 6b
```

### 9.9 The `volume` channel (`0b`, WN-48)

The output volume of each side, shown and changed by the other (on
Windows: the default playback device, Core Audio; on Android:
`STREAM_MUSIC`). A uni channel with a queue as in §3.1, in both directions.

| type | name | `body` fields |
|---|---|---|
| 0 | `Level` | `0 level` uint 0–100; `1 muted` bool: the sender's volume |
| 1 | `Set` | `0 level`? uint 0–100; `1 muted`? bool, at least one: change the receiver's volume |

Rules:

1. `volume/1` means: "my volume can be changed and I show yours".
   A side sends `Level` only when the other announced `volume/1` (and it has
   the setting turned on itself), at the start, on every change (also from keys
   or the system mixer) and again when the other side starts accepting
   `volume/1`; at most every 100 ms, the newest goes out.
2. `Set` changes only what it gives: `level` alone never turns off
   muting. The receiver applies at most 10 `Set`s per second (the newest
   wins). `level` over 100, a wrong type or an empty `Set` is an invalid
   message: the receiver doesn't clamp it, it skips it.
3. A device with a fixed volume (e.g. some HDMI/S/PDIF/USB outputs, Android
   `isVolumeFixed`) skips `level`; `Level` reports what really is.

Example (with the frame's length header): stream header `01 0b`;
`Level` 40 %, not muted:

```text
08 82 00 a2 00 18 28 01 f4
```

### 9.10 The `call` channel (`0c`, WN-48)

The phone's call state, so that the computer pauses its players. A
uni channel with a queue as in §3.1; the phone sends, the computer with `call/1` accepts.

| type | name | `body` fields |
|---|---|---|
| 0 | `State` | `0 state` uint: 0 idle, 1 ringing, 2 in a call; WN-57, optional: `1 call` uint, the call's number on the phone; `2 number` text 1–64 B; `3 name` text 1–128 B; `4 actions` uint, bits: 1 can be rejected, 2 can be silenced |
| 1 | `Missed` | `0 items`: an array of up to 20 `[number, name, date]` (text 1–64 B, text 1–128 B or `null`, uint ms), newest first (WN-57) |

Rules:

1. Without the phone's setting "Calls on the computer" (WN-57), only the state:
   without the number, the name or the app. A call also covers
   calls in messaging apps (Android: the `MODE_IN_COMMUNICATION` mode).
2. The phone sends the state at the start of every session (idle too), on
   every change and again when the computer starts accepting `call/1`.
   Ringing and a call go out at once; idle after 1.5 s of steady
   calm.
3. On "ringing" or "in a call" the computer pauses every playing
   player and remembers which; on "idle" it resumes only those it
   paused, if they are still paused, and no later than 30 min afterwards.
   The end of the session doesn't clear this list: the next `State` decides it.
4. On Android without new permissions: the audio mode (API 31+), the telephony
   state without the number (API 29–30), and in vibrate mode, when the mode doesn't
   change, a notification of the call category seen by the notification
   listener (only with its consent, §9.7 rule 9).
5. **Who is calling (WN-57).** With the setting "Calls on the computer" and
   the permissions (Android: `READ_PHONE_STATE`, `READ_CALL_LOG`; the name from contacts
   with `READ_CONTACTS`), the `State` of ringing and of an incoming call carries
   `1 call` and, when present, `2 number`, `3 name`, `4 actions`. `call` is new
   for every call; the same `call` with new fields (e.g. the name
   arrived after the number) replaces the previous one without a new notification.
   Idle ends the call. A side without WN-57 skips the new keys
   (§4.3), and a computer without them shows only the state.
6. `Missed`: the whole list of missed calls from the last 7 days, at the start of the session,
   when the computer starts accepting `call/1`, and after every missed call; with
   the setting off, empty. The computer treats the first list in a session
   as the starting state and notifies only about newer entries.
7. The computer accepts `call/1` when it has pausing playback
   or call notifications turned on; the state pauses players only with
   the first setting.

Example (with the frame's length header): stream header `01 0c`;
`State` ringing:

```text
05 82 00 a1 00 01
```

### 9.11 The `sms` channel (`0d`, WN-55)

Reading the phone's SMS/MMS threads on the computer. The messages of `sms/1`;
they are versioned by the capability itself. The phone announces `sms/1` only when
the user turned on "SMS on the computer" and the system granted SMS access
(Android: `READ_SMS`); without either of them the phone withdraws the capability (§6,
rule 5) instead of refusing every request. The computer doesn't announce `sms/1`.
WN-56 added types 4–6 (SIM cards, sending, MMS parts) and the message fields 8–9;
under §9 rule 1 this is an extension without a new capability
version: a phone with an earlier version rejects the new types with the code
`UNKNOWN_MESSAGE`, and the computer then shows that sending is unavailable.
Sending has a separate consent on the phone (the setting "Sending from the computer"
and `SEND_SMS`); without it `Sims` and `Send` get `End` 1.

**Stream.** One request is one bidi stream opened by the computer:
header `01 0d`, one message, FIN. The phone answers in the return
direction (without a header) with zero or more batches, then exactly one
`End` and FIN. Exception: `Watch` lasts until either side ends
the stream. Frames up to 1 MiB (§4.2); the phone splits the answer into batches
of about 256 KiB.

From the computer:

| type | name | `body` fields |
|---|---|---|
| 0 | `Threads` | `0 limit`: uint 1–500; the newest threads, newest first |
| 1 | `Messages` | `0 thread`: uint; `1 before`: uint, optional; `2 since`: uint, optional; `3 limit`: uint 1–2000 |
| 2 | `Photo` | `0 address`: text 1–64 B, the address of a person from the thread |
| 3 | `Watch` | empty map |
| 4 | `Sims` | empty map; the SIM cards for sending (WN-56) |
| 5 | `Send` | `0 to`: an array of 1–20 addresses (text 1–64 B); `1 body`: text up to 16 KiB; `2 sim`: uint, optional; `3 image`: bytes 1–786 432, optional; `4 mime`: text `image/jpeg` or `image/png`, together with `3` (WN-56) |
| 6 | `Part` | `0 message`: uint, the MMS `id`; `1 part`: uint, the part's `id` (WN-56) |

From the phone:

| type | name | `body` fields |
|---|---|---|
| 0 | `Threads` | `0 items`: an array of up to 500 threads |
| 1 | `Messages` | `0 items`: an array of up to 2000 messages |
| 2 | `Photo` | `0 png`: bytes 1–65 536, PNG |
| 3 | `Changed` | empty map |
| 4 | `End` | `0 status`: uint (table below) |
| 5 | `Sims` | `0 items`: an array of up to 4 cards `[id, label, slot, default]` (`id` uint, `label` text up to 64 B, `slot` uint from 0, `default` bool) |
| 6 | `Sending` | `0 state`: uint: 0 accepted by the phone, 1 sent, 2 delivered, 3 not sent; an unknown value is 3 |
| 7 | `Part` | `0 mime`: text 1–64 B; `1 data`: bytes 1–921 600 |

A thread (map): `0 id` uint, `1 date` uint (the newest message, ms since
the epoch), `2 snippet` text up to 512 B, `3 people` an array of up to 20 people
`[address, name, photo]` (`address` text 1–64 B, `name` text up to 128 B
or `null`, `photo` bool: `Photo` will find something for this address),
`4 unread` uint, `5 count` uint.

A message (map): `0 id` uint (unique within its kind), `1 mms`
bool, `2 date` uint (ms since the epoch), `3 box` uint (0 received, 1 sent,
2 being sent, 3 not sent; an unknown value is 0), `4 body` text
up to 16 KiB (may be empty), `5 address` text 1–64 B, optional (the sender
of a received message), `6 read` bool, `7 attachments` uint, optional
(MMS parts other than text), `8 parts` an array of up to 16 pairs `[id, mime]`,
optional (MMS parts to fetch with `Part`), `9 delivery` uint,
optional (the delivery report of a sent SMS: 0 waiting, 1 delivered,
2 not delivered; an unknown value is 0).

| `status` | meaning |
|---|---|
| 0 | done |
| 1 | refused: no SMS access or the feature is off |
| 2 | not found: the thread, or a picture for the address |
| 3 | busy: too many watches |
| 4 | read error |

The computer treats an unknown `status` like `4`.

Rules:

1. The request must arrive within 5 s of the stream opening (otherwise `CANCELLED`).
   An invalid message or a field out of range (e.g. `limit` 0 or over the
   maximum): `MALFORMED_FRAME`; an unknown type: `UNKNOWN_MESSAGE`; in both
   cases `STOP_SENDING` and `RESET_STREAM`, without an answer. The receiver rejects
   out-of-range values, it doesn't clamp them.
2. `Messages` returns the thread's messages newest first, at most `limit`, with
   a date no later than `before` and no earlier than `since`
   (both bounds inclusive). Drafts don't count. **Incremental
   sync:** the computer asks with `since` = the date of the newest known
   message, and for older pages with `before` = the date of the oldest; inclusive bounds
   repeat messages from the same millisecond, so the computer
   merges them by the pair (`mms`, `id`). A page without new messages ends
   scrolling back.
3. An answer with more items than the request's `limit`, or a batch
   of another type than the request, is an error: the computer stops the stream
   (`CANCELLED`) and rejects the whole answer.
4. `Watch`: the phone sends `Changed` when SMS or MMS change (new,
   deleted, read), gathered over 250 ms; the computer then asks again
   for the threads and for the open thread with `since`. At most 2 watches at a time per
   session; a further one gets `End` 3 at once. Withdrawing `sms/1` ends
   the watch (§6, rule 5).
5. At most 4 requests (excluding `Watch`) are served at a time per session; further ones
   wait.
6. `Photo` is a PNG (checked by its first bytes) up to 64 KiB; a phone
   without access to contacts answers `End` 2.
7. **Sending (WN-56).** `Send` with one address and no image is an SMS
   (split into parts by the phone); with an image or several addresses it is an
   MMS. Empty content without an image, an image without `mime` (or the other way round), an image
   that doesn't match `mime` by its first bytes (JPEG `ff d8 ff`, PNG
   `89 50 4e 47`) or more than 20 addresses is `MALFORMED_FRAME`. The phone
   answers `Sending` 0 when it has handed the message to the system, then with
   further states, and ends with `End` 0: after state 2 or 3, for MMS after
   state 1 (MMS has no delivery report), 60 s after state 1 without a report,
   or after 120 s without state 1 (then `Sending` 3 first). A refusal before
   handing it over is `End` alone (1 off, 3 limit, 4 error). At most 10
   accepted sends per minute per phone (further ones `End` 3); a send doesn't
   take a place under rule 5. A sent message goes into the phone's threads
   (Android stores it itself), so the watch reports it.
8. `Part` returns an image from an MMS as JPEG or PNG (the phone may
   scale and re-encode it); the phone doesn't hand out parts other than images
   (`End` 2). The part must belong to the given message.
9. `Sims` without access to the card list (Android: `READ_PHONE_STATE`) returns
   one default card; `Send` without `sim` goes through the phone's default card.
10. Privacy: texts and numbers don't go into either side's logs.
   The computer keeps the messages it read only in memory and deletes them when
   another phone connects or on unpairing; it shows them as text,
   without control and direction characters.

Examples (with the frame's length header):

```text
stream header:                           01 0d
Threads limit 200:                       06 82 00 a1 00 18 c8
Messages thread 7, since 1000, limit 1000: 0d 82 01 a3 00 07 02 19 03 e8 03 19 03 e8
Watch:                                   03 82 03 a0
Changed:                                 03 82 03 a0
End 0 (done):                            05 82 04 a1 00 00
Sims:                                    03 82 04 a0
Send to "123", "Hi":                     0d 82 05 a2 00 81 63 31 32 33 01 62 48 69
Sending 1 (sent):                        05 82 06 a1 00 01
```

### 9.12 The `call-action` channel (`0e`, WN-57)

The computer rejects or silences the phone's ringing call. A uni channel with
a queue as in §3.1; the computer sends, the phone with `call-action/1` accepts.
The phone announces `call-action/1` only with the setting "Calls on the
computer" (§9.10 rule 5) and withdraws it together with it.

| type | name | `body` fields |
|---|---|---|
| 0 | `Reject` | `0 call` uint |
| 1 | `Silence` | `0 call` uint |

Rules:

1. The phone runs the action only when the call with this `call` is
   ringing right now; it skips others (ended, answered, the next one) without an answer, so
   a late click won't reject a new call.
2. `Reject` ends the call (Android: `TelecomManager.endCall`, the permission
   `ANSWER_PHONE_CALLS`); without the permission the phone doesn't set bit 1 in `actions` and
   skips `Reject`.
3. `Silence` silences the ringer of this call: the system's, if it allows it, otherwise
   a muted ringer stream, restored when the call ends.
4. The result shows in the `State` on the `call` channel (idle after a rejection).

Examples (with the frame's length header): stream header `01 0e`;
`Reject` of call 7: `05 82 00 a1 00 07`; `Silence` of call 7:
`05 82 01 a1 00 07`.

### 9.13 The `gallery` channel (`0f`, WN-58)

CCS/1
PURPOSE:the phone's latest photos and screenshots in the computer's panel: the list and thumbnails at once, a new photo in the panel < 1 s after it is taken, the original in full quality only when dragged or opened.
RULE:the phone announces `gallery/1` only with the setting "Photos on the computer" turned on and access to photos (Android 13+: `READ_MEDIA_IMAGES`, 10–12: `READ_EXTERNAL_STORAGE`); without either of them it withdraws the capability (§6, rule 5). The computer doesn't announce `gallery/1`.

**Stream.** As `sms` (§9.11): one request is one bidi stream
opened by the computer, header `01 0f`, one message, FIN. The phone
answers in the return direction (without a header) with the answer and exactly
one `End`, then FIN. Exception: `Watch` lasts until either side
ends the stream. Frames up to 1 MiB (§4.2).

From the computer:

| type | name | `body` fields |
|---|---|---|
| 0 | `Recent` | `0 limit`: uint 1–60; the newest photos, newest first |
| 1 | `Thumb` | `0 id`: uint; `1 size`: uint 64–512, the longer side in px |
| 2 | `Original` | `0 id`: uint |
| 3 | `Watch` | empty map |

From the phone:

| type | name | `body` fields |
|---|---|---|
| 0 | `Items` | `0 items`: an array of up to 60 photos |
| 1 | `Thumb` | `0 jpeg`: bytes 1–65 536, JPEG (`ff d8 ff`) |
| 2 | `File` | `0 name`: text 1–255 B, one path part under §9.1 (no `/`, `\`, `:`, control characters, Windows device names); `1 mime`: text 1–64 B; `2 size`: uint 1 B–256 MiB |
| 3 | `Chunk` | `0 data`: bytes 1–262 144 |
| 4 | `Changed` | empty map |
| 5 | `End` | `0 status`: uint, as in §9.11 (0 done, 1 refused, 2 not found, 3 busy, 4 error) |

A photo (map): `0 id` uint (the phone's identifier, Android: the MediaStore `_ID`,
constant as long as the photo exists), `1 date` uint (added, ms since
the epoch; `Recent` sorts by it), `2 name` text 1–255 B, `3 mime` text
1–64 B, `4 width`, `5 height` uint (0: unknown), `6 size` uint (the bytes of
the original), `7 screenshot` bool (a screenshot, Android: the folder
`Screenshots`).

Rules:

1. The request within 5 s of the stream opening (otherwise `CANCELLED`). An invalid
   message or a field out of range: `MALFORMED_FRAME`; an unknown type:
   `UNKNOWN_MESSAGE`; in both cases `STOP_SENDING` and `RESET_STREAM`,
   without an answer. The receiver rejects out-of-range values, it doesn't clamp them.
2. `Recent` returns one `Items` with at most `limit` photos (photos and
   screenshots together; without pending ones and ones deleted to the trash), newest first,
   then `End` 0. More items than `limit` is an error in the answer.
3. `Thumb` returns one `Thumb`: a JPEG with the longer side at most `size`, up to
   64 KiB. A photo that no longer exists: `End` 2.
4. `Original` returns `File`, then `Chunk`s with the original's bytes in order,
   exactly `size` in total, then `End` 0. A name that the computer couldn't
   create is replaced by the phone before sending (`IMG_<id>.<extension>`).
   When the file gives fewer bytes than it announced (it changed while being
   read), the phone breaks off the stream (`RESET_STREAM`, `CANCELLED`), without
   `End`. A computer that gets more bytes than `size`, or `End` 0
   before the last byte, rejects the whole file. At most 20 s between frames.
5. `Watch`: the phone sends `Changed` when its photos change (new,
   deleted), gathered over 250 ms (the camera writes a photo in several steps);
   the computer then asks again with `Recent` and requests thumbnails only of
   new `id`s. At most 2 watches at a time per session; a further one gets
   `End` 3 at once. Withdrawing `gallery/1` ends the watch (§6, rule 5).
6. At most 4 requests (excluding `Watch`) are served at a time per session; further ones
   wait.
7. Privacy: names don't go into either side's logs. The computer keeps
   thumbnails only in memory, and the original only as a file handed to
   Windows when dragged or opened (the app's temporary folder,
   deleted when the session ends, when another phone connects and at start).

Examples (with the frame's length header):

```text
stream header:                01 0f
Recent limit 20:              05 82 00 a1 00 14
Thumb id 7, size 256:         09 82 01 a2 00 07 01 19 01 00
Original id 7:                05 82 02 a1 00 07
Watch:                        03 82 03 a0
Chunk [01 02 03]:             08 82 03 a1 00 43 01 02 03
Changed:                      03 82 04 a0
End 0 (done):                 05 82 05 a1 00 00
```

## 10. Error codes

The same codes are used in the `code` field of the `Error` / `Goodbye` messages and as
QUIC application codes in `CONNECTION_CLOSE`, `RESET_STREAM` and `STOP_SENDING`.

| code | name | meaning | typical use |
|---|---|---|---|
| `00` | `NO_ERROR` | normal end | FIN, closing without an error |
| `01` | `PROTOCOL_VIOLATION` | a breach of the flow rules | connection |
| `02` | `UNSUPPORTED_VERSION` | an unknown `version` byte | stream, `control`: connection |
| `03` | `FRAME_TOO_LARGE` | a frame over the limit | stream, `control`: connection |
| `04` | `MALFORMED_FRAME` | invalid CBOR, field or length | stream, `control`: connection |
| `05` | `UNKNOWN_CHANNEL` | an unknown channel number | stream |
| `06` | `UNKNOWN_MESSAGE` | an unknown type on a known channel | `Error` |
| `07` | `CAPABILITY_MISSING` | a channel without an announced capability | stream, `Error` |
| `08` | `UNTRUSTED_PEER` | the key is not pinned | connection |
| `09` | `HELLO_TIMEOUT` | no `Hello` / `HelloAck` in time | connection |
| `0a` | `INTERNAL_ERROR` | an implementation error | any |
| `0b` | `RATE_LIMITED` | too many messages (not the number of uni streams, §3.1 rule 5) | stream, `Error` |
| `0c` | `CANCELLED` | cancelled by the user or a side | stream |
| `0d` | `GOING_AWAY` | leaving, sleep, unpairing | `Goodbye`, connection |
| `0e` | `NO_SPACE` | no space for the file (§9.1) | the `files` stream |
| `0f` | `UNPAIRED` | the sender removed the pairing (§12.10) | `Goodbye`, connection |
| `10` | `BUSY` | the sender has a session with another device and keeps it (§12.10) | `Goodbye`, connection |

Codes `11`–`ff` are reserved for v1; a receiver treats an unknown code like
`INTERNAL_ERROR`. An error in a frame of a stream other than `control` closes only
that stream (`STOP_SENDING` on the receiver's side), not the connection.

## 11. Summary of examples

| message | bytes |
|---|---|
| `control` header | `01 00` |
| `input` header | `01 04` |
| start of the `control` stream | section 7.1 |
| `Hello` | section 7.3 (50 B) |
| `HelloAck` | section 7.3 (45 B) |
| `Ping` nonce 42 | `06 82 02 a1 00 18 2a` |
| `Pong` nonce 42 | `06 82 03 a1 00 18 2a` |
| `Capabilities` | `0d 82 04 a1 00 81 67 69 6e 70 75 74 2f 31` |
| `Error` | section 7.6 (24 B) |
| `Goodbye` GOING_AWAY | `05 82 06 a1 00 0d` |
| `PointerMotion` (datagram) | section 8.8 (24 B) |
| `PointerMotion` 2 (datagram) | section 8.8 (18 B) |
| `Scroll` (datagram) | section 8.8 (17 B) |
| `PointerButton` | section 8.8 (23 B) |
| `InputSession` s 0, g 0 | `07 82 03 a2 00 00 01 00` |
| `files` messages | section 9.1 |
| `actions` messages | section 9.2 |
| `Battery` 82 %, charging | `08 82 00 a2 00 18 52 01 f5` (§9.4) |
| `find` messages | section 9.5 |
| `media`, `media-control` messages | section 9.6 |
| `notify` messages | section 9.7 |
| `notify-action` messages | section 9.8 |
| `Level` 40 %, not muted | `08 82 00 a2 00 18 28 01 f4` (§9.9) |
| `State` ringing | `05 82 00 a1 00 01` (§9.10) |
| `Reject` call 7 | `05 82 00 a1 00 07` (§9.12) |
| `sms` messages | section 9.11 |
| `gallery` messages | section 9.13 |

The examples are test vectors for the frame codec (WC-06): the encoder MUST
produce exactly these bytes, and the decoder must recover these messages from them.

## 12. Pairing (`windconnect-pair/1`, WC-19)

Pairing is the only moment when a side accepts a key it hasn't
pinned yet. After a successful pairing both sides store and
pin the other's key, and from then on they connect only through §2.

Two paths:

- **QR**: the device with a screen (usually the PC) shows a QR code with a link
  (§12.4) that carries its key and a one-time 128-bit secret. The other
  device proves it knows the secret (§12.5).
- **SAS**: without a camera. Both sides show the same 6-digit code, and
  people confirm that the codes match (§12.6).

The side showing the QR code or waiting for the code comparison is the **pairing
server**, and the other one is the **client**.

### 12.1 Transport

1. The pairing server listens on a **separate** QUIC endpoint (an ephemeral
   port, announced in the link or through mDNS, §12.4), with the ALPN
   `windconnect-pair/1` only. The main endpoint (§2) never accepts
   this ALPN and still rejects unpinned keys in the TLS handshake.
2. TLS 1.3 with mutual authentication, as in §2: each side
   presents a device certificate with an Ed25519 key, and the handshake
   signatures are Ed25519 only. A client without a certificate is
   rejected in the TLS handshake. The pairing server accepts any Ed25519
   key. The client on the QR path accepts only the key from the link and
   rejects any other in TLS (alert `access_denied`); on the SAS path it
   accepts any key.
3. **Keys from the handshake only.** `K_s` (the server's) and `K_c` (the client's) in
   every formula below and in the record of the paired device are the keys from
   the certificates verified in **this** connection. No pairing message
   carries a key.
4. A pairing connection is always a **full** 1-RTT handshake: the server doesn't
   issue session tickets and keeps no session cache, the client doesn't offer
   resumption, and 0-RTT is forbidden both ways. The TLS configuration of
   the pairing endpoint is not shared with the main one.
5. The TLS session is bound by an exporter value (RFC 8446 §7.5):
   `E = TLS-Exporter("EXPORTER-windconnect-pair", "", 32)`. Both sides
   compute it after the handshake; two different TLS sessions (e.g. on both sides of
   an intermediary) have different `E`.
6. The client opens exactly one bidi stream. The stream has **no**
   header from §3 (the ALPN determines the protocol); it carries §4 frames right away.
   The pairing endpoint doesn't accept uni streams or datagrams.

### 12.2 The pairing window

The server answers only when it has a **window** open: QR (with a new
secret) or SAS. Opening a new window closes the previous one.

1. A window lasts at most 5 min. When the time runs out it closes by itself, and the secret
   is erased at once, without waiting for a connection.
2. A window handles **one attempt at a time**. A successful attempt closes the window, and the secret
   is used up: the same link won't work a second time, also for the same
   client.
3. A **failed attempt** is any ending after the TLS handshake without
   success: a bad MAC proof, a commit that doesn't match the reveal, a message out of
   order or the wrong mode, a bad or too large frame, a disconnect or a reset
   before the result, a step's time limit (10 s) running out, and a rejected or
   undecided SAS code (60 s for the decision). After **3** failed
   attempts the window closes, the secret is erased, and further connections
   get `RATE_LIMITED` until a new window is opened.
4. A connection that arrives while another attempt is going on gets
   `RATE_LIMITED` and doesn't count as an attempt.
5. The server checks the window's state **before** reading any frame and
   before any computation beyond TLS: no window or an expired window is
   `GOING_AWAY`, a locked or busy window is `RATE_LIMITED`.

The TLS handshake doesn't count as an attempt (a failed handshake doesn't
reach the window). An attacker on the local network can therefore at most close
the window with three attempts; the app shows this and lets the user open a new one.

### 12.3 Messages

Frames as in §4 (length, then CBOR `[type, body]`), with a payload limit of
**1 024 B**. A longer frame is `FRAME_TOO_LARGE`, a frame of length
0 is `MALFORMED_FRAME`; a side rejects both by the length alone, before
buffering. Unknown fields are skipped (§4.3); known ones must have their type.

| type | name | direction | fields |
|---|---|---|---|
| 0 | `PairHello` | client → server | `0`: mode (`0` QR, `1` SAS), `1`: name (optional), `2`: proof (32 B) |
| 1 | `PairNonce` | server → client (SAS) | `0`: `N_s` (32 B), `1`: name (optional) |
| 2 | `PairReveal` | client → server (SAS) | `0`: `N_c` (32 B) |
| 3 | `PairResult` | server → client (QR) | `0`: `mac_s` (32 B), `1`: name (optional) |
| 4 | `PairConfirm` | both sides (SAS) | `0`: consent (bool) |

The **device name** is for display only. It has at most 64 B of UTF-8
(a longer one is `MALFORMED_FRAME`). Before showing and storing it a side removes from
it control characters (category Cc) and characters that change the text direction
(U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069), and then
trims whitespace. An empty name counts as no name. The same
rules apply to the name in the link (§12.4).

Examples:

```text
PairHello QR, name "Pixel 9", proof 11×32:
31 82 00 a3 00 00 01 67 50 69 78 65 6c 20 39 02 58 20 11…11

PairNonce N_s 22×32, name "PC":
2a 82 01 a2 00 58 20 22…22 01 62 50 43

PairReveal N_c 33×32:   26 82 02 a1 00 58 20 33…33
PairResult mac 44×32:   26 82 03 a1 00 58 20 44…44
PairConfirm true:       05 82 04 a1 00 f5
```

### 12.4 The pairing link (QR)

```text
windconnect:pair?v=1&k=<K_s>&s=<secret>&a=<host:port>[&a=…][&n=<name>]
```

- `v`: the link version, `1`.
- `k`: the server's public key (32 B), base64url without padding.
- `s`: the secret (16 B from a cryptographic generator), base64url without
  padding.
- `a`: the address of the pairing endpoint, 1–8 times, in the form `1.2.3.4:5` or
  `[fe80::1]:5`, percent-encoded.
- `n`: the server's name, percent-encoded (§12.3).

The receiver rejects a link longer than 1 024 characters, with another version, without `k`,
`s` or `a`, with a repeated parameter other than `a`, with non-canonical
base64url, or with a key that isn't an Ed25519 point. It skips unknown
parameters. The link is a secret: the app doesn't store or
log it.

The SAS path has no link. A server with an open SAS window announces the port of
the pairing endpoint in mDNS (§2.0) in the TXT field `pair=<port>` and removes it when
the window closes (WN-82; the app connects `PairingListener` with discovery,
WC-27). The announcement is an untrusted hint, as all of mDNS is: the address comes
from the A/AAAA of the same announcement, and §12.6 authenticates.

### 12.5 The QR path

```text
mac_c = HMAC-SHA256(s, "wc-pair-v1 client" ‖ K_s ‖ K_c ‖ E)
mac_s = HMAC-SHA256(s, "wc-pair-v1 server" ‖ K_s ‖ K_c ‖ E)
```

1. The client connects (TLS checks `K_s` from the link) and sends
   `PairHello {0: 0, 1: name, 2: mac_c}`.
2. The server checks `mac_c` in constant time. If the MAC is bad: it closes
   the connection with the code `UNTRUSTED_PEER` and counts a failed attempt. If it is
   good: it closes the window (the secret is used up), stores and pins `K_c`, and then
   sends `PairResult {0: mac_s, 1: name}`.
3. The client checks `mac_s` in constant time (the server knows the secret too),
   stores and pins `K_s`, ends the stream and closes the connection with the code
   `NO_ERROR`. A bad `mac_s`: closing with the code `UNTRUSTED_PEER`, nothing
   is stored.

Without the secret an attacker can't compute `mac_c`. The MAC binds `E`, so a proof from
another TLS session (a relay) doesn't fit. The MAC also binds `K_c` from
the handshake, so one client's proof can't be used for another
key. Impersonating the server already fails in TLS (§12.1, point 2).

### 12.6 The SAS path

```text
C   = SHA-256("wc-pair-v1 commit" ‖ K_c ‖ K_s ‖ N_c)
SAS = be_u32(SHA-256("wc-pair-v1 sas" ‖ E ‖ K_c ‖ K_s ‖ N_c ‖ N_s)[0..4]) mod 10^6
```

1. The client draws `N_c` (32 B) and sends `PairHello {0: 1, 1: name, 2: C}`.
2. The server draws `N_s` (32 B) and only then sends
   `PairNonce {0: N_s, 1: name}`.
3. The client sends `PairReveal {0: N_c}` only after receiving `N_s`. The server
   checks `C`; a mismatch is a `PROTOCOL_VIOLATION` and a failed attempt.
4. Both sides compute `SAS` and show it as six digits in two groups
   (`020 627`) together with the other side's name.
5. Each side sends `PairConfirm {0: consent}` when its user
   decides. A rejection sends `false` and closes the connection with the code
   `CANCELLED`, and the other side treats such a close as a rejection.
   A side stores and pins the other's key only when its user
   accepted **and** the other side sent `true`.
6. Ending (WN-27): after both consents the server stores and pins the
   client's key, and only then ends its direction of the stream (FIN) and waits
   for the client to close the connection. After both consents the client ends its direction
   (FIN) and waits for the server's FIN (the step limit, 10 s); only after it does it store
   and pin the server's key, and then it closes the connection with the code `NO_ERROR`.
   The connection closing, data after `PairConfirm` or no FIN in time is
   an error: the client stores nothing. Thanks to this, the client closing
   won't drop its `PairConfirm(true)` in flight, and the only possible
   asymmetry is a server that stored the client while its FIN didn't arrive
   (as in §12.8).

The order is enforced: a message out of it (e.g. `PairReveal` before
`PairHello`, `PairConfirm` in place of `PairReveal`, a second `PairNonce`) is a
`PROTOCOL_VIOLATION`. An attacker in the middle must fix `N_c` (with the commit)
or `N_s` (by sending it) before learning the other side's value, so they can't
match the codes on both sides other than with a chance of 1 in 10^6 per attempt.
The attempt limit (§12.2) bounds the number of tries. Without the commit an attacker could
change their share of the TLS key offline on their side until the codes matched.

### 12.7 Vectors

The keys are the Ed25519 public keys of the private keys `01×32`
(`K_s`) and `02×32` (`K_c`); `s = 11×16`, `E = 33×32`, `N_c = 44×32`,
`N_s = 55×32`.

| value | result |
|---|---|
| `mac_c` | `34061136437d946a87b158b60bfb316f361cd3ad9ea721ef346fa3cf4dc67f6f` |
| `mac_s` | `b434849080e57e61e837b556759352bf6d34d0e7d67f157a930f654fff8a37b3` |
| `C` | `649b9a0ba332e1b048f68641303bd9b699ada816d3079976c542d7caf4edac10` |
| `SAS` | `020 627` |

### 12.8 Storing paired devices

Each side stores a paired device in a file in the identity key's folder
(Windows: `%LOCALAPPDATA%\WindConnect\paired.txt`; on Unix permissions
`0600`). The file is only appended to, with one write and a sync per
record `\n<key hex> <Unix seconds>[ <percent-encoded name>]`.
Every record **starts** with a newline character, so a record cut off by
a crash stays alone on its line, and the next one starts cleanly. Two
processes appending at the same time don't lose their records. A later record
of the same key changes the name and the time. Reading skips lines that
can't be read, reports them and never rewrites the file.

Removal is appended too (WC-28): a record `\n-<key hex> <seconds>`
removes the key and marks that its device is owed a notice
(§12.10); `\n=<key hex> <seconds>` closes the matter (the notice
was delivered, or that device removed the pairing). A later ordinary
record of this key pairs it again. A rename is an ordinary record with the
same pairing time and the new name; the name is local and never
sent. An older version of the app treats `-`/`=` as damaged lines and
keeps the key: versions don't go backwards.

The server stores the client in step 2 (QR) or after both consents (SAS), before
it sends the result. If the connection breaks later, the server has the key stored,
and the client doesn't. Such a key opens nothing (the client didn't pin it), and
pairing again or unpairing (§12.10) puts the state in order.

### 12.9 Error codes

Closing a pairing connection carries only a code from §10, with an empty reason.
No message says which step failed.

| situation | code |
|---|---|
| no window, window expired | `GOING_AWAY` |
| window locked after 3 attempts, another attempt is going on | `RATE_LIMITED` |
| bad `mac_c` or `mac_s` | `UNTRUSTED_PEER` |
| wrong mode, order, commit | `PROTOCOL_VIOLATION` |
| bad or too large frame | `MALFORMED_FRAME`, `FRAME_TOO_LARGE` |
| no step within 10 s | `HELLO_TIMEOUT` |
| code rejected, no decision, disconnect | `CANCELLED` |
| a write error on the server's side | `INTERNAL_ERROR` (doesn't count as an attempt) |
| success (the client closes) | `NO_ERROR` |

### 12.10 Unpairing and a single session (WC-28)

Unpairing on one end removes the key on the other, also when the other end
happens to be off the network. One path for both cases:

1. Side A writes `-<key B>` (§12.8), unpins key B and marks it as
   **revoked**, and the endpoint forgets B's TLS tickets (no resumption and no
   0-RTT from a ticket from before the unpairing).
2. A sends `Goodbye` with the code `UNPAIRED`: at once in an ongoing session with B
   or at B's next connection (point 4). Then it closes the connection
   with the same code.
3. After `Goodbye UNPAIRED` (or a close with this code) B removes **the
   peer's key from the TLS handshake** of this connection (an `=` record), unpins it and
   doesn't reconnect. The message carries no key, and its
   `message` is ignored.
4. **Notice mode.** A connection with a revoked key completes
   the handshake (proof of key possession), but it isn't a session: the app
   sees it only after the full handshake and the key check, so it doesn't
   read 0-RTT data before this decision. The revoking side doesn't install
   a router, doesn't accept streams other than `control`, answers `Hello`
   with its name and an empty capability list (nothing new), sends
   `Goodbye UNPAIRED` and closes. If it is the one calling (a phone that removed
   the computer while the computer was off the network), it does a full handshake without 0-RTT
   after seeing the computer in discovery (the LAN or the relay §13).
5. A writes `=<key B>` only when B has acknowledged receiving the whole
   `control` stream after its end (a QUIC acknowledgement, not local sending).
   A break before that leaves `-` and the notice goes again.
   "Delivered" therefore means: B's transport accepted these bytes, not that B's app
   processed them. That is why the receiver MUST handle `UNPAIRED` in every
   accepted session, also a rejected one (`BUSY`) and one that ended
   before it was used: the computer, before answering `BUSY`, waits up to 500 ms for
   that session's `Goodbye UNPAIRED`, and the phone checks the close code of
   a connection that didn't become a session. A late notice (a slow
   phone, a large RTT, the relay §13) may cross with `BUSY`: the computer,
   after sending `Goodbye BUSY`, checks once more whether `UNPAIRED`
   arrived, and then removes the key and closes with `NO_ERROR` instead of `BUSY`.
   And after the acknowledgement A waits up to 1 s for B to close: a close with
   `BUSY` means that B's app didn't read it, so `-` stays.
6. The notice is owed for **30 days**. After that, or when B removes
   A's key in another way, B's attempts end with a TLS refusal (§2,
   `UNTRUSTED_PEER`); B shows "The computer refuses the connection — pair
   again" and stops retrying by itself.

Several phones paired with one computer: the computer has one session
at a time. The same side (the same key) replaces its old connection
(e.g. after a network change). Another phone gets `Goodbye BUSY` and doesn't retry
by itself until its network changes or the user chooses "Connect", so two
phones don't throw each other out. The exceptions are a person's choice at the
computer: a phone paired on it a moment ago (2 min) takes over the session,
and "Disconnect" in the panel closes the session with `GOING_AWAY` and for 10 min (or until
another phone's session) answers that phone `BUSY`, so that another one can
connect. LIMIT: after `BUSY` the phone doesn't come back by itself even when the computer
becomes free; it waits for a network change or "Connect".

## 13. The relay outside the home (`/v1/relay`, WN-59)

The relay lets two paired devices on different networks talk
with the same QUIC as on the LAN (§2). It carries the session's UDP packets in
WebSocket frames and knows no session key and no device key, so it
can't read or change the content: the encryption and the authentication of the ends
stay in TLS 1.3 with pinned keys. The relay is an option off
by default (decision D3); the client turns it on in the settings (WC-60). The server is
a Cloudflare Worker with a Durable Object in the `relay/` folder.

### 13.1 Connecting

- A device opens `wss://<host>/v1/relay?me=<key>&peer=<key>`, where
  both keys are 64 lower-case hex characters of Ed25519 keys (§2.1): its own and
  the paired device's. Other shapes end with HTTP 400; a missing
  `Upgrade: websocket` header gives 426; an exceeded limit of new
  connections from one IP (30 per minute) gives 429.
- The pair's meeting place is
  `SHA-256("wc-relay-v1 pair" ‖ lower key ‖ higher key)`, where
  "lower" means smaller in byte order. Both devices must name
  each other; a device that hasn't named anyone is unreachable
  for others.
- A pair holds at most 4 sockets before authentication. A new socket with
  a full pool closes the oldest unauthenticated one with code 4004, instead of being
  rejected, so someone who knows both public keys can't block the pair
  with unsigned sockets. Authenticated sockets don't count: each
  device has at most one (§13.2).

### 13.2 Authentication

Text frames are JSON objects with a field `t`. The relay starts with
`{"t":"challenge","nonce":"<64 hex>"}` with 32 random bytes. The device
answers `{"t":"auth","sig":"<128 hex>"}`, an Ed25519 signature with its device
key over

```text
"wc-relay-v1 auth" ‖ nonce ‖ own key ‖ peer key
```

The context separates this signature from TLS signatures with the same key. A signature with
another nonce, for another peer, with a wrong key or more than 10 s late
closes the socket with code 4001. After success the relay sends
`{"t":"ready","peer":<bool>}` (whether the peer is already waiting), and later
`{"t":"peer","present":<bool>}` when the peer joins or leaves. The signature
proves only which device is connecting; trust in the peer comes only from the
pinned key in TLS.

One device has at most one authenticated socket in a pair: a new one
closes the previous one with code 4003, and the peer doesn't get `present:false` then.

### 13.3 Data

- After authentication, binary frames carry one QUIC UDP packet each and
  reach the other device unchanged, if it is present. Without a peer
  a frame is dropped like a lost UDP packet.
- A binary frame over 2048 B closes the socket with code 1009. A binary frame
  before authentication and a text frame after it (other than ping) close it
  with code 4002.
- `{"t":"ping"}` gets `{"t":"pong"}` (keep-alive without waking the object).
- Each device has a token bucket: 1 MiB/s, bursts up to 4 MiB
  (the variables `RATE_BYTES_PER_SEC`, `BURST_BYTES`). A frame over the limit is dropped
  without closing the socket; QUIC adjusts its pace.
- The relay doesn't store or log the content of frames.

| close code | meaning |
|---|---|
| 4001 | authentication failed or late |
| 4002 | a frame not allowed in this state |
| 4003 | replaced by a newer connection of this device |
| 4004 | an unauthenticated socket displaced by a newer one (full pool) |
| 1009 | a data frame over 2048 B |

### 13.4 What the relay sees

The relay's operator (Cloudflare and the account owner) sees the IP addresses of both
devices, both public keys and the pair identifier at connection time, the times of
connections, and the number and size of frames. It doesn't see the content, the device names or
the message types: all of that is inside QUIC. The Worker doesn't store this
metadata: Workers Logs are off (`observability.enabled: false`), and the code
logs nothing. Turning the relay on (D3) means agreeing that the
operator can see this metadata.

### 13.5 The client (WN-60)

- The relay is off until the user gives its address
  (`wss://…`; `ws://` only to a loopback or private address, for a
  local `wrangler dev`). A relay that is off is never used.
- The client first connects to the device's address on the LAN. With the
  relay on, this attempt has 2 s; then it connects through the relay. A trust
  refusal (`UNTRUSTED_PEER`) doesn't fall over to the relay.
- The same QUIC session (§2) works through the relay: the device keeps
  a local UDP address that stands in for the peer. Packets of its own endpoint
  sent to this address go to the relay, and frames from the relay reach
  the endpoint as if from the peer. Only packets from the current address of
  its own endpoint go to the relay.
- A PC with the relay on waits on it for every paired device
  and reconnects with a growing delay (1–60 s) when the connection
  ends. An idle socket sends `{"t":"ping"}` every 25 s.
- After a network change during a session through the relay, the client tries the
  LAN address (2 s). If it answers, a new session on the LAN replaces the session through
  the relay.
- Turning the relay off takes effect at once (WN-92): the phone ends the session
  through the relay (reason `relay-off`) and tries the LAN address; the PC breaks off
  all connections to the relay. The switch "Relay outside the home"
  is in the settings of both apps, off by default, with the description from §13.4.
