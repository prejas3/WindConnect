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
`wind-core 0.25.0 · gotowy`. Zamknij okno i sprawdź odinstalowanie w Ustawieniach.

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

## Zasobnik i autostart (WN-23)

Lewy klik ikony otwiera panel WN-24, prawy otwiera własne menu (DM Sans, motyw
aplikacji, strzałki/Home/End/Tab, Enter/Spacja, Escape). Menu jest też dostępne
przez **Opcje zasobnika** w oknie. Utrata fokusu chowa menu. X/Alt+F4 chowa
okno do zasobnika; **Zakończ** kończy proces i usuwa ikonę. Kolejne uruchomienie
przywraca istniejące okno, zamiast dodawać drugą ikonę (mutex sesji logowania).

Trzy własne ikony 32 px mają odrębny kształt i kolor: znacznik/teal połączenia,
trzy kropki/amber wyszukiwania i przekreślony okrąg/gray braku połączenia.
Tooltip i nagłówek menu zawierają opis słowny. `indicator.rs` publikuje zdarzenia
przez watch, bez timerów. Natywny odbiorca wymienia ikonę i tooltip na wątku UI;
`tray_snapshot` podaje czas od publikacji do zakończenia wywołań natywnych.
To pomiar API, nie momentu odmalowania przez Explorer.

Integracja koordynatora sesji Windows (kolejne karty parowania): pobierz
`app.state::<TrayRuntime>().indicator`, wywołaj `searching()` przed próbą,
`attempt_failed(generation)` po błędzie lub uruchom `track_session(generation,
&session)` dla uwierzytelnionej sesji. Jej zamknięcie przełącza ikonę na brak
połączenia. Generacja chroni przed spóźnionym zdarzeniem poprzedniej sesji.
Obecny panel nie zawiera jeszcze parowania/koordynatora Windows: bez sesji
produkcyjna ikona uczciwie pokazuje brak połączenia. WN-23 dostarcza odbiornik
zdarzeń i adapter prawdziwej `Session`, nie przypina kluczy z odkrywania.

**Uruchamiaj po zalogowaniu** jest domyślnie wyłączone. Jawny wybór użytkownika
zapisuje REG_SZ w HKCU `Software\Microsoft\Windows\CurrentVersion\Run`, wartość
`WindConnect`, z cytowaną ścieżką bieżącego EXE i `--autostart`. Bez powłoki
i uprawnień administratora. Uruchomienie z tym argumentem tworzy tylko ikonę.
MSI nie włącza autostartu; próbuje usunąć wpis przy deinstalacji w kontekście bieżącego
użytkownika. Sprzątanie jest nieblokujące: brak lub blokada EXE albo błąd rejestru
nie zatrzymuje deinstalacji; wpis Run może wtedy pozostać. Wpisy innych profili nie są usuwane. Po zmianie lokalizacji EXE
włącz opcję ponownie. Task Manager/polityka Windows może wyłączyć start niezależnie
od obecności Run. Nie ma usługi ani zadania Harmonogramu.

Testy: `cargo test -p windconnect-windows` (stara generacja, cytowanie, odczyt/
zapis/usunięcie w prywatnym kluczu HKCU testu, nigdy w Run). `check-ui.cjs`
sprawdza też własne menu i checkbox autostartu z atrapą IPC. Test natywny:

```powershell
node apps/windows/scripts/check-tray-native.cjs target/debug/windconnect-windows.exe
node apps/windows/scripts/check-tray-native.cjs target/release/windconnect-windows.exe --release
```

Ustaw `PLAYWRIGHT_MODULE` jak dla testu UI. Runner startuje i zamyka własną
aplikację/WebView2, sprawdza cichy start, zamknięcie do zasobnika, drugą instancję
i klawiaturę menu. W debug `dev_tray_probe` tworzy dwa lokalne uwierzytelnione
węzły QUIC/Session, mierzy 15 przejść ikony i wymaga każdego <100 ms; release
odmawia tej operacji. Pozostają ręczne testy powłoki (prawy/lewy klik rzeczywistej
ikony, restart Explorera, logowanie i deinstalacja na VM, różne DPI/monitory).

## Panel przy zasobniku (WN-24)

CCS/1
M: Windows / panel 380×540 DIP
FLOW: start -> ukryty WebView2 -> font i layout gotowe -> lewy klik zasobnika lub Win+Shift+W -> pokaż istniejące okno.
RULE: Escape, X, Alt+F4 i utrata fokusu chowają panel; przycisk „Otwórz aplikację” otwiera główne okno. Prawy klik zachowuje menu WN-23.
RULE: Skrót rejestrowany przez RegisterHotKey bez powtarzania; zamknięcie procesu zwalnia rejestrację. Konflikt skrótu nie przerywa startu: panel pokazuje wyjaśnienie, zasobnik nadal działa.
IO: Fizyczny prostokąt ikony -> monitor, DPI i work area -> pozycja przy odpowiedniej krawędzi paska. Rozmiar ograniczony do work area; mniejsze ekrany mają przewijanie.
RULE: Lokalny DM Sans, tokeny WN-20, cztery własne kafle, motyw aplikacji, wejście 160 ms / 8 px; reduced-motion wyłącza animację. Bez sparowanego urządzenia akcje są nieaktywne; brak fikcyjnych zdjęć, powiadomień i baterii.
PERF: panel_snapshot.painted_us mierzy czas od rozpoczęcia natywnej obsługi otwarcia do potwierdzenia dwóch requestAnimationFrame w WebView2. To potwierdzenie zgłoszenia klatki, nie pomiar fizycznego ekranu; nie czeka na zakończenie animacji 160 ms.
STATE: Test geometrii: cztery krawędzie paska, ujemne współrzędne, 200% DPI, mały monitor i auto-hide. Test rejestracji: konflikt, dispatch i zwolnienie skrótu z Ctrl+Alt+Shift+F24 (nie przejmuje Win+Shift+W użytkownika).

```powershell
node apps/windows/scripts/check-panel-native.cjs target/debug/windconnect-windows.exe
node apps/windows/scripts/check-panel-native.cjs target/release/windconnect-windows.exe
```

CCS/1
DEP: PLAYWRIGHT_MODULE jak dla check-ui.cjs.
IO: 20 pomiarów <80 ms; preloading bez flash, Escape, utrata fokusu, otwarcie aplikacji, motywy, DM Sans, reduced-motion. Raport i PNG: %TEMP%/windconnect-wn24 (lub PANEL_ARTIFACTS). Runner zamyka własny proces i przeglądarkę.
LIMIT: Przy zajętym Win+Shift+W raport jawnie zapisuje shortcut=false i komunikat OS; pozostałe sprawdzenia działają. Pełny test fizycznego skrótu wymaga wolnego skrótu. Zmiana krawędzi paska/monitora/DPI i rzeczywisty klik ikony pozostają do ręcznej walidacji powłoki; runner nie zmienia konfiguracji pulpitu.

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

