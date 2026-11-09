# Fuzzing kodera ramek

Cele [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) dla
`wind_core::protocol`. Katalog jest osobnym workspace: wymaga nightly
i libFuzzera, więc nie wchodzi do `cargo build` ani CI.

| cel | wejście | sprawdza |
|---|---|---|
| `frame_reader` | strumień dowolnego kanału w dowolnych kawałkach | brak paniki; każda odczytana wiadomość koduje się i odczytuje z powrotem tak samo |
| `datagram` | datagram `input` | brak paniki; przyjęty datagram robi round-trip |
| `cbor` | dowolne bajty jako CBOR | brak paniki; przyjęte wejście jest już kanoniczne (`encode(decode(x)) == x`) |

Uruchomienie (Linux lub WSL; z katalogu `crates/wind-core/fuzz`):

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run frame_reader -- -max_total_time=600
```

Na Windows MSVC libFuzzer wymaga składnika Visual Studio
„C++ AddressSanitizer”.
