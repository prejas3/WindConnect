# Fuzzing kodera ramek i parowania

Cele [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) dla
`wind_core::protocol`. Katalog jest osobnym workspace: wymaga nightly
i libFuzzera, więc nie wchodzi do `cargo build` ani CI.

| cel | wejście | sprawdza |
|---|---|---|
| `frame_reader` | strumień dowolnego kanału w dowolnych kawałkach | brak paniki; każda odczytana wiadomość koduje się i odczytuje z powrotem tak samo |
| `datagram` | datagram `input` | brak paniki; przyjęty datagram robi round-trip |
| `cbor` | dowolne bajty jako CBOR | brak paniki; przyjęte wejście jest już kanoniczne (`encode(decode(x)) == x`) |
| `pair_message` | ramki parowania (§12.3) od nieuwierzytelnionej strony | brak paniki; limit 1 KiB; round-trip; nazwy już oczyszczone |
| `pair_uri` | link parowania (§12.4) | brak paniki; przyjęty link zapisuje się i czyta z powrotem tak samo; nazwa oczyszczona |
| `paired_file` | plik sparowanych urządzeń (§12.8), np. uszkodzony | brak paniki; każda linia to urządzenie albo zgłoszone pominięcie; nazwy oczyszczone; klucz raz |

Uruchomienie (Linux lub WSL; z katalogu `crates/wind-core/fuzz`):

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run frame_reader -- -max_total_time=600
```

Na Windows MSVC libFuzzer wymaga składnika Visual Studio
„C++ AddressSanitizer”.
