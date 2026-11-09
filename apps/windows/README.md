# Windows

Szkielet Tauri 2: jedno okno, statyczny frontend bez Node.js i komenda
`core_version`, która odczytuje wersję z `wind-core`.

## Wymagania

Windows 11 x64, stabilny Rust (MSVC), Microsoft C++ Build Tools z Windows SDK,
Microsoft Edge WebView2. Do MSI potrzebna jest włączona funkcja Windows
**VBSCRIPT**. Tauri pobiera WiX przy pierwszym pakowaniu.

```powershell
cargo install tauri-cli --version 2.11.4 --locked
cd apps/windows
cargo tauri dev
```

## MSI

```powershell
cd apps/windows
cargo tauri build --bundles msi -- --locked
```

Wynik: `target/release/bundle/msi/*.msi` w katalogu głównym repozytorium.
Instalator pobiera bootstrapper WebView2, jeżeli runtime nie jest zainstalowany;
pierwsza instalacja wymaga wtedy połączenia z Internetem. MSI nie jest podpisany.

Workflow **Windows MSI** buduje pakiet na `windows-latest` dla PR, push do
`master` lub ręcznego uruchomienia. W zakładce Actions pobierz artefakt
`WindConnect-windows-x64-msi`, rozpakuj ZIP i uruchom MSI.
Po instalacji uruchom WindConnect: powinno pojawić się jedno okno ze statusem
`wind-core 0.22.0 · gotowy`. Zamknij okno i sprawdź odinstalowanie w Ustawieniach.

W pliku `tauri.conf.json` wartości `identifier` i
`bundle.windows.wix.upgradeCode` muszą pozostać stałe między wersjami,
aby aktualizacje MSI poprawnie rozpoznawały zainstalowaną aplikację.
Frontend nie używa bibliotek JavaScript ani zasobów z sieci. Lokalny DM Sans
pochodzi z zaakceptowanych makiet WN-20 (OFL 1.1). Licencje fontu i bezpośrednio
wywoływanej biblioteki Mica są dołączone w `frontend/assets/`; inwentarz i hash
fontu: `frontend/assets/NOTICE.txt`. Ikona i SVG projektu: MIT.

Przebieg CI na GitHubie oraz ręczna instalacja i deinstalacja MSI wymagają
potwierdzenia po opublikowaniu repozytorium.

## Kontrolki Windows (WN-21)

`frontend/tokens.css` definiuje paletę jasną/ciemną, DM Sans, odstępy,
promienie i ruch z WN-20. `style.css` rysuje przyciski, kafelki, przełączniki,
checkbox, pola, listę, dialog, postęp i scrollbar. Semantyczne elementy HTML
zachowują obsługę klawiatury; ich wygląd nie korzysta z domyślnych kontrolek.
Fokus jest widoczny, dialog zamyka Escape i przywraca fokus, lista obsługuje
strzałki/Home/End. `prefers-reduced-motion` usuwa animacje; forced-colors
zachowuje obramowania i wybór. Motyw System reaguje na zmianę preferencji OS,
a wybór użytkownika jest zapamiętywany lokalnie.

Okno ma własny pasek (przeciąganie, dwuklik, minimalizacja, maksymalizacja,
zamknięcie). Mica jest nakładana natywnie przez `window-vibrancy`; dopiero
udane wywołanie DWM włącza półprzezroczysty tint. Błąd lub brak wsparcia
pozostawia nieprzezroczyste tło. Windows może sam wyłączyć efekt zgodnie
z ustawieniami dostępności/energii. Nie emulujemy Mica obrazem ani rozmyciem CSS.

Galeria: `cargo tauri dev` i przycisk **Galeria kontrolek** w stopce.
Opcjonalnie `?gallery` otwiera ją od razu. Natywny `ui_dev_mode` zwraca false
w release, niezależnie od URL. Dane i akcje galerii są demonstracyjne,
bez dostępu do transportu, zaufania czy urządzeń. Wydanie pokazuje pusty stan;
parowanie/panel urządzenia należą do dalszych kart.

Test UI: zainstaluj Playwright 1.61.1 poza repozytorium i jego Chromium,
ustaw `PLAYWRIGHT_MODULE` na katalog modułu `playwright`, następnie:

```powershell
node apps/windows/scripts/check-ui.cjs
```

Runner uruchamia i zamyka lokalny serwer oraz przeglądarkę. Sprawdza oba motywy,
400/700/1024 px, wczytanie fontu, klawiaturę, walidację, dialog, reduced-motion
i bramkę release przy wstrzykniętym IPC. PNG trafiają do `%TEMP%/windconnect-wn21-ui`
(lub `UI_ARTIFACTS`). CI Windows wykonuje ten sam test. Dodatkowo na Windows 11
sprawdź natywnie przeciąganie/resize, przyciski okna, Alt+F4, Mica na tle pulpitu
i tryb wyłączonej przezroczystości; Chromium headless nie potwierdza efektu DWM.

## Odkrywanie i reguła zapory (WN-15)

Rdzeń uruchamia mDNS i IPv4 broadcast równolegle. Broadcast korzysta z portu
UDP 47821, wysyła odcisk klucza i rzeczywisty port QUIC, a wynik pozostaje
niezaufaną wskazówką do sesji z przypiętym kluczem. Fallback można sprawdzić
przez `windctl discover --broadcast-only` oraz węzeł
`windctl serve --advertise --broadcast-only` z osobnym kluczem.

Fragment `src-tauri/wix/firewall.wxs` dodaje przy instalacji regułę
**WindConnect - LAN UDP (Private)**. Reguła dotyczy wyłącznie pełnej ścieżki
instalowanego `windconnect-windows.exe`, protokołu UDP, profilu Private i
adresów LocalSubnet. Obejmuje mDNS 5353, broadcast 47821 i port QUIC wybrany
przez aplikację, również po migracji gniazda. Nie otwiera TCP, profilu Public
ani innych aplikacji (w tym osobnego `windctl.exe`).

WiX FirewallExtension zarządza dodaniem, wycofaniem instalacji i usunięciem
reguły razem z komponentem MSI. Błąd rejestracji reguły przerywa instalację
i uruchamia rollback, zamiast po cichu pozostawić niekompletną konfigurację.
Administrator może dodatkowo ograniczyć ruch polityką organizacji.

Budowanie MSI kompiluje i łączy fragment przez standardowe narzędzia WiX;
nie instaluje go i nie zmienia zapory komputera budującego. Test instalacji
i deinstalacji wykonuj na przeznaczonym do tego Windows VM: po instalacji
sprawdź `Get-NetFirewallRule -DisplayName 'WindConnect - LAN UDP (Private)'`
oraz powiązane filtry aplikacji/adresów/protokołu, a po deinstalacji brak
tej reguły. Test fallbacku w dwóch węzłach z wyłączonym mDNS jest niezależny
od lokalnej polityki instalatora.

