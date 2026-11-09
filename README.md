# WindConnect

WindConnect łączy Windows 11 z Androidem 10+ (API 29+).
Projekt jest na etapie fundamentów: Windows ma szkielet Tauri 2,
a Android — Kotlin / Jetpack Compose. Protokół jest opisany w
[PROTOCOL.md](PROTOCOL.md); `wind-core` ma już koder ramek
(`wind_core::protocol`) i tożsamość urządzenia (`wind_core::identity`:
klucz Ed25519, certyfikat TLS, odcisk; na Windows klucz chroni DPAPI,
na Androidzie klucz z Android Keystore),
połączenie QUIC z przypiętymi kluczami, keep-alive, migracją ścieżki
i wznawianiem 0-RTT (`wind_core::transport`)
oraz sesję protokołu ze strumieniami kanałów i routerem do modułów
(`wind_core::session`).

## Struktura

```text
Cargo.toml                 workspace Rust
Cargo.lock                 wersje zależności workspace
PROTOCOL.md                specyfikacja protokołu v1
crates/wind-core/          współdzielona biblioteka Rust
crates/wind-core/fuzz/     cele cargo-fuzz dla kodera ramek
apps/windows/              aplikacja Tauri 2 i konfiguracja MSI
apps/android/              aplikacja Kotlin / Jetpack Compose
LICENSE                    licencja MIT
CONTRIBUTING.md            zasady współpracy
```

## Budowanie

Wymagany stabilny toolchain Rust z Cargo (np. zainstalowany przez rustup).
W katalogu głównym repozytorium uruchom:

```sh
cargo build --locked
```

Polecenie buduje domyślny pakiet `wind-core`. Klient Windows również należy
do workspace; jego wymagania i budowanie MSI opisano w
[apps/windows/README.md](apps/windows/README.md).
Budowanie APK opisano w [apps/android/README.md](apps/android/README.md).

## Kierunek projektu

- Własny protokół oparty na QUIC, bez zgodności z KDE Connect.
- Planowana dystrybucja Androida: APK przez GitHub Releases oraz F-Droid.
- Opcjonalny przekaźnik Cloudflare planowany na etap M3.
- Docelowe publiczne repozytorium: `prejas3/WindConnect`.

Kod udostępniany na licencji [MIT](LICENSE). Zasady zmian: [CONTRIBUTING.md](CONTRIBUTING.md).

