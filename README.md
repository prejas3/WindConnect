# WindConnect

WindConnect łączy komputer z Windows 11 z telefonem z Androidem 10+ w tej samej
sieci Wi-Fi: wspólny schowek, pliki w obie strony, powiadomienia, SMS i połączenia
na komputerze, sterowanie odtwarzaczem i głośnością, telefon jako touchpad.
Bez konta i bez chmury: telefon i komputer łączą się bezpośrednio (QUIC z
przypiętymi kluczami po parowaniu kodem QR); opcjonalny własny przekaźnik
pozwala połączyć się spoza domu.

> **Status: wersja testowa przed 1.0.** Aplikacje działają na co dzień, ale
> wydania 0.x są do testów: mogą zawierać błędy, a format danych i protokół
> mogą się jeszcze zmienić. APK jest podpisany kluczem testowym (właściwy klucz
> wydań przyjdzie przed 1.0), MSI nie jest podpisany.

## Co potrafi

- **Schowek:** automatyczna synchronizacja tekstu, linków i obrazów, historia z przypinaniem.
- **Pliki:** wysyłanie z telefonu na PC i z PC na telefon (także z menu Eksploratora), postęp i ponawianie.
- **Powiadomienia telefonu na PC:** z odpowiadaniem i odrzucaniem; wybór aplikacji.
- **SMS i połączenia na PC:** wątki SMS i wysyłanie, kto dzwoni, odrzucenie i wyciszenie (wersja GitHub).
- **Multimedia:** odtwarzacz telefonu na PC i odtwarzacz PC na telefonie, głośność w obie strony.
- **Sterowanie:** telefon jako touchpad i klawiatura, tryb prezentacji ze wskaźnikiem laserowym.
- **Akcje:** zadzwoń na telefon (Znajdź telefon), zablokuj PC, otwórz link na drugim urządzeniu,
  ostatnie zdjęcia telefonu w panelu PC, bateria telefonu w zasobniku.
- **Na Androidzie:** widżet stanu PC, kafelek szybkich ustawień, skróty na ekranie głównym.
- Interfejs po polsku i angielsku, jasny i ciemny motyw, dostępność (Narrator, TalkBack, duży tekst).

## Instalacja

Pliki są w [Releases](https://github.com/prejas3/WindConnect/releases). Sumy SHA-256 są w `SHA256SUMS.txt`.

1. **Windows 11:** pobierz `WindConnect_<wersja>_x64_en-US.msi` i uruchom (raz poprosi o UAC).
   Aktualizacja to instalacja nowszego MSI na starszy; parowanie i ustawienia zostają.
   `WindConnect-<wersja>-portable.exe` uruchamia się bez instalacji (do szybkiego podglądu;
   bez autostartu, menu Eksploratora i reguły zapory).
2. **Android 10+:** pobierz `WindConnect-<wersja>.apk` na telefon i otwórz (zezwól na instalację
   z tego źródła) albo `adb install -r WindConnect-<wersja>.apk`.
3. **Parowanie:** otwórz panel WindConnect przy zegarze w Windows i zeskanuj kod QR aplikacją
   na telefonie (albo „Paruj bez aparatu” i porównaj kod na obu ekranach). Telefon i komputer
   muszą być w tej samej sieci; sieć Windows ustaw jako prywatną.

Wersja z Google Play (bez SMS i rejestru połączeń) i F-Droid są planowane.

## Dla deweloperów

Protokół jest opisany w [PROTOCOL.md](PROTOCOL.md). Wspólny rdzeń w Rust
`wind-core` ma koder ramek (`wind_core::protocol`) i tożsamość urządzenia
(`wind_core::identity`: klucz Ed25519, certyfikat TLS, odcisk; na Windows klucz
chroni DPAPI, na Androidzie klucz z Android Keystore),
połączenie QUIC z przypiętymi kluczami, keep-alive, migracją ścieżki
i wznawianiem 0-RTT (`wind_core::transport`)
sesję protokołu ze strumieniami kanałów i routerem do modułów
(`wind_core::session`) oraz kanał wejścia z datagramami QUIC
(`wind_core::input`).

Most Androida to `crates/wind-ffi` (UniFFI) i moduł Gradle
`apps/android/wind-core` z korutynami oraz `StateFlow`. Ekran testowy klienta
łączy się z PC przez uwierzytelnioną sesję rdzenia; budowanie AAR i procedura
testu są opisane w [instrukcji Androida](apps/android/README.md).

Narzędzie [windctl](crates/windctl/README.md) uruchamia węzły testowe,
odkrywa urządzenia, wysyła surowe wiadomości i mierzy RTT Ping/Pong z p50/p95.
Budowanie: `cargo build --locked -p windctl`.

`wind_core::network::NetworkChanges` rozsyła sygnały zmiany sieci;
`NetworkDiscovery` natychmiast odtwarza odkrywanie na aktualnych interfejsach.
Na Windows właściciel automatycznie rejestruje `NotifyNetworkConnectivityHintChange`
i wyrejestrowuje callback przy zamykaniu. Rejestruje go bez początkowego
powiadomienia (WN-89): odkrywanie startuje już po rejestracji, a restart mDNS
zaraz po starcie ukrywał urządzenie przed przeglądarką uruchomioną w tej chwili.
Po prawdziwej zmianie sieci nowy demon przegląda od razu, a ogłasza się dopiero
1 s po goodbye poprzedniego (WN-104): mdns-sd 0.21.7 traktuje ogłoszenie w
sekundzie po goodbye jako odświeżenie i nie zgłasza urządzenia przeglądarce,
która go wcześniej nie znała. Seria zmian daje jedno goodbye i jedno ogłoszenie;
nowa przeglądarka widzi urządzenie po ok. 1,8 s. Zmiana TXT `pair` czeka do 2 s
od ostatniej rejestracji, bo wcześniejsza nie dochodziła na część interfejsów.
Test ręczny: `cargo test --locked -p wind-core --test discovery -- --ignored --test-threads=1`.
`windctl discover` oraz
`serve --advertise` korzystają z tego mechanizmu. Szkielet Tauri utrzymuje
rejestrację przez życie aplikacji i udostępnia diagnostyczne `network_generation`;
uruchamianie sesji desktopu należy do dalszej integracji aplikacji.

Odkrywanie ma również fallback IPv4 broadcast UDP 47821. `NetworkDiscovery`
łączy go z mDNS i wygasza nieodświeżone wpisy. `windctl discover --broadcast-only`
pozwala sprawdzić tę ścieżkę z całkowicie wyłączonym mDNS. MSI dodaje regułę
prywatnej sieci LAN dla UDP instalowanej aplikacji (opis w instrukcji Windows).

Odkrywanie LAN udostępnia `wind_core::discovery`: `Discovery::start` przyjmuje
klucz publiczny urządzenia i rzeczywisty port z `Endpoint::local_addr()`.
`next().await` zwraca rozwiązane adresy lub usunięcie usługi; `monitor()`
udostępnia błędy demona. Właściciel usuwa `Discovery` przy wyłączeniu usługi
(Drop wysyła goodbye i zatrzymuje wątek; może czekać do 2 s).

Do ręcznego pomiaru na dwóch komputerach w tej samej sieci uruchom jednocześnie
`cargo run --locked -p wind-core --example discover`. Przykład wiąże port QUIC,
ogłasza tymczasową tożsamość, wypisuje zdarzenia i kończy po bezczynności 10 s
lub po 30 s. Zapora musi przepuszczać multicast mDNS UDP 5353.
Automatyczny lokalny pomiar dwóch instancji:
`cargo test --locked -p wind-core --test discovery -- --ignored --nocapture`.
Test wymaga multicastu i sprawdza wzajemne odkrycie poniżej 1 s; nie zastępuje
pomiaru na dwóch fizycznych urządzeniach. Klienci aplikacji nie uruchamiają
jeszcze tego modułu automatycznie.

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
- Dystrybucja Androida: APK przez GitHub Releases, później F-Droid i Google Play.
- Opcjonalny przekaźnik (Cloudflare Workers) do połączeń spoza domu: [relay/](relay/).
- Repozytorium: [prejas3/WindConnect](https://github.com/prejas3/WindConnect).

Kod udostępniany na licencji [MIT](LICENSE). Zasady zmian: [CONTRIBUTING.md](CONTRIBUTING.md).

