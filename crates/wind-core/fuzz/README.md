# Fuzzing the frame codec and pairing

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for
`wind_core::protocol`. The folder is a separate workspace: it needs nightly
and libFuzzer, so it isn't part of `cargo build` or CI.

| target | input | checks |
|---|---|---|
| `frame_reader` | a stream of any channel in arbitrary pieces | no panic; every message read encodes and reads back the same |
| `datagram` | an `input` datagram | no panic; an accepted datagram round-trips |
| `cbor` | arbitrary bytes as CBOR | no panic; accepted input is already canonical (`encode(decode(x)) == x`) |
| `pair_message` | pairing frames (§12.3) from an unauthenticated side | no panic; 1 KiB limit; round-trip; names already cleaned |
| `pair_uri` | a pairing link (§12.4) | no panic; an accepted link writes and reads back the same; the name cleaned |
| `paired_file` | the paired devices file (§12.8), e.g. damaged | no panic; every line is a device or a reported skip; names cleaned; each key once |
| `actions` | the `actions` channel payload (§9.2) and the `open-url` address | no panic; an accepted request/response round-trips; the action identifier is valid; an accepted address is only http(s) in printable ASCII |

Running (Linux or WSL; from `crates/wind-core/fuzz`):

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run frame_reader -- -max_total_time=600
```

On Windows MSVC, libFuzzer needs the Visual Studio component
"C++ AddressSanitizer".
