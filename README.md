# WindConnect

WindConnect connects a Windows 11 PC with an Android 10+ phone on the same
Wi-Fi network: a shared clipboard, files both ways, notifications, SMS and calls
on the computer, media and volume control, the phone as a touchpad.
No account and no cloud: the phone and the computer connect directly (QUIC with
keys pinned when pairing by QR code); an optional relay of your own lets you
connect from outside your home.

> **Status: a test version before 1.0.** The apps work for everyday use, but
> 0.x releases are for testing: they may have bugs, and the data format and the
> protocol may still change. The APK is signed with a test key (the real release
> key comes before 1.0); the MSI is not signed.

## What it does

- **Clipboard:** automatic sync of text, links and images; a history with pinning.
- **Files:** from the phone to the PC and from the PC to the phone (also from the Explorer menu), with progress and retry.
- **Phone notifications on the PC:** reply and dismiss; choose which apps.
- **SMS and calls on the PC:** SMS threads and sending, who is calling, reject and silence (GitHub version).
- **Media:** the phone's player on the PC and the PC's player on the phone, volume both ways.
- **Control:** the phone as a touchpad and keyboard, a presentation mode with a laser pointer.
- **Actions:** ring the phone (Find my phone), lock the PC, open a link on the other device,
  the phone's latest photos in the PC's panel, the phone's battery in the tray.
- **On Android:** a PC status widget, a Quick Settings tile, home screen shortcuts.
- Polish and English UI, light and dark theme, accessibility (Narrator, TalkBack, large text).

## Installation

The files are in [Releases](https://github.com/prejas3/WindConnect/releases). SHA-256 sums are in `SHA256SUMS.txt`.

1. **Windows 11:** download `WindConnect_<version>_x64_en-US.msi` and run it (it asks for UAC once).
   To update, install a newer MSI over the older one; pairing and settings stay.
   `WindConnect-<version>-portable.exe` runs without installing (for a quick look;
   no autostart, Explorer menu or firewall rule).
2. **Android 10+:** download `WindConnect-<version>.apk` to the phone and open it (allow installing
   from this source), or `adb install -r WindConnect-<version>.apk`.
3. **Pairing:** open the WindConnect panel next to the clock in Windows and scan the QR code with the
   app on the phone (or use "Pair without the camera" and compare the code on both screens). The phone
   and the computer must be on the same network; set the Windows network to private.

A Google Play version (without SMS and the call log) and F-Droid are planned.

## For developers

The protocol is specified in [PROTOCOL.md](PROTOCOL.md). The shared Rust core
`wind-core` has the frame codec (`wind_core::protocol`) and the device identity
(`wind_core::identity`: an Ed25519 key, a TLS certificate, a fingerprint; on Windows
the key is protected by DPAPI, on Android it is a key from the Android Keystore),
the QUIC connection with pinned keys, keep-alive, path migration
and 0-RTT resumption (`wind_core::transport`),
the protocol session with channel streams and a router to the modules
(`wind_core::session`), and the input channel over QUIC datagrams
(`wind_core::input`).

The Android bridge is `crates/wind-ffi` (UniFFI) and the Gradle module
`apps/android/wind-core` with coroutines and `StateFlow`. The client's test screen
connects to the PC through an authenticated core session; building the AAR and the
test procedure are described in the [Android guide](apps/android/README.md).

The [windctl](crates/windctl/README.md) tool runs test nodes, discovers devices,
sends raw messages and measures Ping/Pong RTT with p50/p95.
Build: `cargo build --locked -p windctl`.

`wind_core::network::NetworkChanges` broadcasts network change signals;
`NetworkDiscovery` immediately restarts discovery on the current interfaces.
On Windows the owner automatically registers `NotifyNetworkConnectivityHintChange`
and unregisters the callback when closing. It registers it without the initial
notification (WN-89): discovery already starts after registration, and restarting mDNS
right after start hid the device from a browser started at that moment.
After a real network change the new daemon browses at once, and announces itself only
1 s after the previous one's goodbye (WN-104): mdns-sd 0.21.7 treats an announcement in
the second after a goodbye as a refresh and doesn't report the device to a browser
that didn't know it before. A series of changes gives one goodbye and one announcement;
a new browser sees the device after about 1.8 s. A change of the `pair` TXT record waits up to 2 s
after the last registration, because an earlier one didn't reach some interfaces.
Manual test: `cargo test --locked -p wind-core --test discovery -- --ignored --test-threads=1`.
`windctl discover` and
`serve --advertise` use this mechanism. The Tauri app keeps the
registration for the app's lifetime and exposes the diagnostic `network_generation`.

Discovery also has an IPv4 UDP broadcast fallback on port 47821. `NetworkDiscovery`
combines it with mDNS and expires entries that aren't refreshed. `windctl discover --broadcast-only`
checks this path with mDNS completely off. The MSI adds a private-network LAN rule
for UDP for the installed app (described in the Windows guide).

LAN discovery is provided by `wind_core::discovery`: `Discovery::start` takes
the device's public key and the actual port from `Endpoint::local_addr()`.
`next().await` returns resolved addresses or a service removal; `monitor()`
exposes daemon errors. The owner drops `Discovery` when the service shuts down
(Drop sends a goodbye and stops the thread; it may wait up to 2 s).

For a manual measurement on two computers on the same network, run
`cargo run --locked -p wind-core --example discover` on both at once. The example binds a QUIC port,
announces a temporary identity, prints events and ends after 10 s of inactivity
or after 30 s. The firewall must let mDNS multicast through (UDP 5353).
An automatic local measurement of two instances:
`cargo test --locked -p wind-core --test discovery -- --ignored --nocapture`.
The test needs multicast and checks mutual discovery in under 1 s; it doesn't replace
a measurement on two physical devices.

## Layout

```text
Cargo.toml                 Rust workspace
Cargo.lock                 workspace dependency versions
rust-toolchain.toml        the pinned Rust toolchain
PROTOCOL.md                protocol specification v1
crates/wind-core/          shared Rust library
crates/wind-core/fuzz/     cargo-fuzz targets for the frame codec
crates/wind-ffi/           UniFFI bridge for Android
crates/windctl/            command-line test tool
apps/windows/              Tauri 2 app and MSI configuration
apps/android/              Kotlin / Jetpack Compose app
relay/                     optional relay (Cloudflare Workers)
LICENSE                    MIT license
CONTRIBUTING.md            contribution rules
```

## Building

The Rust toolchain is pinned in `rust-toolchain.toml`; rustup installs it on first use.
From the repository root run:

```sh
cargo build --locked
```

The command builds the default package `wind-core`. The Windows client is part of the
workspace too; its requirements and building the MSI are described in
[apps/windows/README.md](apps/windows/README.md).
Building the APK is described in [apps/android/README.md](apps/android/README.md).

## Project direction

- Its own QUIC-based protocol, not compatible with KDE Connect.
- Android distribution: APK through GitHub Releases, later F-Droid and Google Play.
- An optional relay (Cloudflare Workers) for connections from outside the home: [relay/](relay/).
- Repository: [prejas3/WindConnect](https://github.com/prejas3/WindConnect).

The code is released under the [MIT](LICENSE) license. Contribution rules: [CONTRIBUTING.md](CONTRIBUTING.md).
