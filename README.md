# WindConnect

WindConnect łączy Windows 11 z Androidem 10+ (API 29+).
Projekt jest na etapie szkieletu monorepo; aplikacje i protokół nie są jeszcze zaimplementowane.

## Struktura

```text
Cargo.toml                 workspace Rust
Cargo.lock                 wersje zależności workspace
crates/wind-core/          współdzielona biblioteka Rust
apps/windows/              miejsce na aplikację Tauri 2
apps/android/              miejsce na aplikację Kotlin / Jetpack Compose
LICENSE                    licencja MIT
CONTRIBUTING.md            zasady współpracy
```

## Budowanie

Wymagany stabilny toolchain Rust z Cargo (np. zainstalowany przez rustup).
W katalogu głównym repozytorium uruchom:

```sh
cargo build --locked
```

Polecenie buduje `wind-core`. Katalogi aplikacji są obecnie placeholderami
i nie należą do workspace Cargo. Szkielet nie ma zależności zewnętrznych.

## Kierunek projektu

- Własny protokół oparty na QUIC, bez zgodności z KDE Connect.
- Planowana dystrybucja Androida: APK przez GitHub Releases oraz F-Droid.
- Opcjonalny przekaźnik Cloudflare planowany na etap M3.
- Docelowe publiczne repozytorium: `prejas3/WindConnect`.

Kod udostępniany na licencji [MIT](LICENSE). Zasady zmian: [CONTRIBUTING.md](CONTRIBUTING.md).

