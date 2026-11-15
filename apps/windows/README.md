# Windows

## Touchpad z telefonu (WN-51)

Touchpad jest domyślnie dozwolony dla sparowanego telefonu. Otwórz „Touchpad” na ekranie głównym Androida: jeden palec porusza kursorem, stuknięcie klika, przytrzymanie przeciąga, dwa palce przewijają (z inercją), stuknięcie dwoma palcami daje prawy klik. Przyciski pod obszarem pozwalają też przytrzymać lewy/prawy przycisk. Czułość i naturalne przewijanie są ustawiane na telefonie.

Trzy palce: w górę widok zadań, w dół pulpit, w lewo poprzednia aplikacja, w prawo następna. Wymaga PC z `input/2`; podstawowy ruch działa również z `input/1`. Klawiatura będzie osobną funkcją WN-52.

Panel PC pokazuje „Telefon steruje kursorem · nazwa”. „Wyłącz touchpad” natychmiast wycofuje przyjmowanie wejścia i zwalnia przyciski. Ustawienie pozostaje po ponownym uruchomieniu. Zamknięcie ekranu telefonu, utrata fokusu, blokada i zmiana połączenia zatrzymują wysyłkę; nic nie jest odtwarzane po ponownym połączeniu. Windows może odmówić sterowania aplikacją uruchomioną jako administrator lub bezpiecznym pulpitem: touchpad wyłącza się z komunikatem i wymaga lokalnego ponownego włączenia, bez podnoszenia uprawnień.

CCS/1 PERF:QUIC loopback + wspólny interpolator PC, 360 próbek przy 120 Hz: p95 16,285 ms idle i 11,929 ms z 268 500 992 B transferu; surowe CSV w raporcie karty. To pomiar programowy, bez SendInput i ekranu. Fizyczny dotyk → kursor oraz płynność rzeczywistego monitora 120 Hz: WN-134 (testy końcowe, powiązane z WN-77). Odtworzenie: `WIND_TOUCHPAD_SAMPLES=<folder>` oraz `cargo test -p wind-core --test input touchpad_latency_with_interpolation_idle_and_bulk -- --ignored --nocapture`.

## Postęp transferów (WN-41)

CCS/1
FLOW:panel pokazuje do50najnowszych transferów w obu kierunkach, stan, potwierdzone bajty/rozmiar, prędkość i dostępne akcje. Stabilne wiersze DOM zachowują focus podczas aktualizacji. Prędkość: monotoniczne delty, okno1s + EMA; „—” przed pierwszym oknem i podczas oczekiwania. Nieznany rozmiar ma pasek nieokreślony (obecny silnik zna rozmiar po imporcie źródła). Stany/bajty są łączone przed serializacją/IPC do najnowszego snapshotu w oknie100ms; jedno oczekujące deadline, żadnego timera idle. Systemowe powiadomienie Androida jest osobno ograniczone do1/s.
SEC:Anuluj/Ponów/Otwórz folder rozwiązywane natywnie po ID. Niejednoznaczne ID (różne urządzenia/kierunki) odrzucane. Ponów tylko nieudany/anulowany outbound i nadal sparowany oryginalny peer; brak przekierowania na aktualny telefon. Odebrane pliki wymagają ponowienia po stronie nadawcy. Otwórz folder sprawdza w chwili kliknięcia lokalny plik source lub zweryfikowany incoming saved_as, canonical/containment/reparse; SHOpenFolderAndSelectItems zaznacza plik w Eksploratorze, nie uruchamia go. Przeniesiony/usunięty plik daje czytelny komunikat.
CHECK:`check-send-ui.cjs` (Playwright): obie palety150%, keyboard/focus po100aktualizacjach, akcje, indeterminate i niezaufane nazwy. `check-transfer-performance.cjs` + release example `transfers_bench`: prawdziwy QUIC,1GiB i256x1MiB, mediana3prób off/on z produkcyjnym modelem i rendererem Chromium; cel >=95% baseline. Raw timings i event cadence w TEMP/performance.json. Początkowa nieograniczona emisja: mała partia86.2% (FAIL); po coalescing pierwszy pomiar104.2%/117.1%; powtórzenie256MiB miało91.4% (FAIL) przy dużym rozrzucie czasu. Finalny fixture używa dłuższej próby1GiB i zrównoważonej kolejności off/on: duży99.35%, mała partia101.28% (PASS); raw timings w run record. Wartości >100% oznaczają wahania hosta, nie deklarację przyspieszenia. Nie jest to pomiar realnego Wi-Fi ani wyświetlania fizycznego ekranu co100ms.

## Odbiór plików z Androida (WN-40)

CCS/1
FLOW:dopuszczona sparowana sesja -> files/1 -> istniejący FileEngine -> Pobrane/WindConnect/Obrazy, Wideo, Audio, Dokumenty lub Inne. Kategoria jest wybierana lokalnie przy zweryfikowanej finalizacji według ostatniego rozszerzenia bez rozróżniania wielkości liter; np.photo.jpg.exe i brak rozszerzenia -> Inne. Pełna tabela rozszerzeń: `wind-core::files::incoming_folder`; domyślna polityka silnika OFF, runtime Windows ON.
SEC:sygnalizowany typ nie jest oceną zawartości. Dotychczasowe sprawdzanie ścieżek, brak nadpisywania, checksum/resume i Mark-of-the-Web obowiązują także w podfolderze kategorii; częściowe pliki pozostają poza inboxem. Nigdy nie otwieramy/uruchamiamy odebranego pliku automatycznie. Router przekazuje incoming queue dopiero po dopuszczeniu sesji (BUSY/unpair nadal przed odbiorem).
IO:generowany fixture `files_peer` zapisuje tylko w przekazanym TEMP, z tą samą opt-in polityką silnika. Test20 obrazów i zestawu mieszanego jest opisany w Android README; nie dotyka Pobranych zalogowanego użytkownika ani zainstalowanej aplikacji. Wysyłanie z Windows jest WN-39; Android public Downloads receiver pozostaje WN-120, rozszerzone UI postępu WN-41.

Ikona WN-93 „Orbita”: źródła SVG i instrukcja odtwarzania w `assets/brand/README.md`.
ICO zawiera rozmiary16–256px; zasobnik ma osobne warianty16/20/24/32px
dla trzech stanów i obu palet. Nie skaluje małej bitmapy do większego DPI.

Klient Tauri 2 ze wstępnie wyrenderowanym panelem telefonu i statycznym frontendem
bez Node.js. Parowanie i uwierzytelniona sesja działają w `wind-core`.

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
Po instalacji uruchom WindConnect: panel pokazuje QR do sparowania telefonu.
Po sparowaniu i zestawieniu sesji pokazuje „Połączono” i nazwę telefonu.
Zamknij panel i sprawdź odinstalowanie w Ustawieniach.

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
bez dostępu do transportu, zaufania czy urządzeń. Wydanie udostępnia rzeczywisty
panel parowania WN-25, a galeria pozostaje tylko w debug.

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
Koordynator WN-25 publikuje te zdarzenia dla rzeczywistych sesji. Bez sesji ikona
pokazuje brak połączenia, także po samym zapisaniu parowania. Klucze z odkrywania
nie są przypinane; zaufanie pochodzi z protokołu parowania i zapisanego magazynu.

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
IO: 20 pomiarów <80 ms; preloading bez flash, Escape, utrata fokusu, otwarcie aplikacji, motywy, DM Sans, reduced-motion. Bez telefonu (debug: pusty magazyn w folderze raportu) szybkie akcje są ukryte, a wyłączone są tylko niezbudowane Dzwoń i Touchpad; Wyślij i Schowek działają po połączeniu (WN-109). Raport i PNG: %TEMP%/windconnect-wn24 (lub PANEL_ARTIFACTS). Runner zamyka własny proces i przeglądarkę.
LIMIT: Przy zajętym Win+Shift+W raport jawnie zapisuje shortcut=false i komunikat OS; pozostałe sprawdzenia działają. Pełny test fizycznego skrótu wymaga wolnego skrótu. Zmiana krawędzi paska/monitora/DPI i rzeczywisty klik ikony pozostają do ręcznej walidacji powłoki; runner nie zmienia konfiguracji pulpitu.

## QR i połączenie telefonu (WN-25)

CCS/1
FLOW: normalny start -> ukryty panel gotowy -> QR przy pierwszym uruchomieniu -> skan/potwierdzenie na Androidzie -> parowanie WN-19 -> zapis klucza -> uwierzytelniona Session -> „Połączono” + prawdziwa nazwa telefonu i ikona zasobnika.
SEC: DPAPI i paired.txt w profilu użytkownika jak w rdzeniu. Uszkodzone rekordy pomijane i liczone; błąd odczytu/tożsamości nie generuje zastępczych kluczy. Zaufane klucze ładowane przed nasłuchem głównej sesji. Samo odkrywanie lub sukces parowania nie jest stanem „Połączono”.
RULE: QR ma jednorazowy sekret i 5 min ważności. Nowy kod, anulowanie lub zmiana sieci unieważnia poprzednie okno; po wygaśnięciu/3 nieudanych próbach obraz znika. Brak adresu LAN daje opis i ponowienie. Po sparowaniu czekamy na Session; po 10 s bez niej pokazujemy zachowane sparowanie i brak połączenia. Rozłączenie usuwa stan „Połączono”, klucz pozostaje.
IO: Adresy QR: IPv4 interfejsów LAN, najpierw interfejs domyślnej trasy (maks. 8); osobny port parowania, główny endpoint dla sesji. Odkrywanie równolegle mDNS + broadcast WN-15. Wi-Fi/Ethernet w tej samej sieci; relay nie jest tu automatycznie włączany.
RULE: Kod renderowany w Rust do macierzy modułów, w canvas: czarny/biały w każdym motywie, całkowita skala, quiet zone 4 moduły. Surowy link nie trafia do DOM, localStorage ani logów. Cztery akcje telefonu pozostają nieaktywne do swoich kart; brak fikcyjnych baterii/zdjęć.
SEC (WN-94): Moduły QR kodują jednorazowy sekret, więc aplikacja czyści każdą swoją kopię. Sekret i link są `Zeroizing` w rdzeniu; bufor modułów z `qrcode` jest nadpisywany po skopiowaniu; `QrBits` zeruje się przy zwolnieniu (stan pętli, wartość `watch`, klony `publish` i `pairing_snapshot`). Zamknięcie, nowy kod, wygaśnięcie, sparowanie i koniec pętli ustawiają `qr = None`. Panel zeruje tablicę modułów po narysowaniu i czyści canvas, gdy QR znika.
LIMIT (WN-94): Poza zasięgiem aplikacji są robocze bufory kodera `qrcode` (zwalniane w nim bez zerowania), JSON serializowany przez Tauri dla IPC (zdarzenie `pairing-state` i odpowiedź `pairing_snapshot`) oraz kopie w WebView (ciąg IPC, sterta JS, bitmapa w GPU/kompozytorze), które zwalnia GC i przeglądarka. Ochrona jednorazowości nie zależy od zerowania: zamknięte okno unieważnia sekret w rdzeniu, a stary kod jest odrzucany.
FLOW: Android po parowaniu wyszukuje adres tylko dla zapisanego klucza, następnie łączy się z tym przypiętym kluczem; limit 7 s. Nieosiągalny komputer nie usuwa udanego parowania. Ekran telefonu śledzi rzeczywisty stan sesji.

Sprawdzenia natywne (zewnętrzny Playwright jak wyżej; JDK 17 w JAVA_HOME,
ZXING_CORE_JAR wskazuje na `core-3.5.4.jar` pobrany przez Gradle):

```powershell
cargo build --locked -p windconnect-windows
cargo build --locked -p wind-ffi --example desktop_pairing_peer
node apps/windows/scripts/check-pairing-native.cjs
node apps/windows/scripts/check-pairing-android.cjs
```

CCS/1
STATE: Native test dekoduje rzeczywisty canvas ZXingiem, używa prawdziwego protokołu parowania i Session, sprawdza nazwę, ikonę, zniknięcie QR, restart, anulowanie i odmowę starego kodu. Peery mają efemeryczne tożsamości; DPAPI/trust w unikalnym TEMP. Komenda dev_pairing_address i zmienne WINDCONNECT_TEST_DATA_DIR/WINDCONNECT_TEST_PAIR_IP działają wyłącznie w debug; release nie pozwala tymi zmiennymi przekierować tożsamości.
STATE: Android test wymaga emulatora-5556 (lub ANDROID_SERIAL), APK i APK testowego. Dekoduje obraz canvas w Androidzie, potwierdza w UI i wymaga „Połączono” <10 s. Emulator NAT wymaga przekierowania UDP 47823 -> 47821; test wysyła wyłącznie publiczny pakiet odkrywania z odciskiem i portem, nie przypina kluczy. Pełne parowanie/TLS/Session to kod produkcyjny. Port/timer/procesy test kończy; testowy surowy link jest usuwany. PNG i run.json: TEMP/windconnect-wn25-*.
LIMIT: Budżet jest mierzony od rozpoznania QR, z automatycznym potwierdzeniem testu, do prawdziwej sesji. Czas człowieka i fizycznego ustawiania ostrości nie jest ograniczany. Fizyczny S25 Ultra i rzeczywiste Wi-Fi/Private firewall trzeba potwierdzić na telefonie. MSI/APK do pierwszego testu są budowane lokalnie; MSI nie jest instalowany przez test.

Audyt nowej zależności dostarczanej w EXE/MSI:

| Komponent | Wersja | Licencja z pakietu | Gdzie trafia | Wynik |
| --- | --- | --- | --- | --- |
| qrcode | 0.14.1 | MIT OR Apache-2.0; wybrana MIT, LICENSE-MIT.txt | Natywny EXE; pełny tekst frontend/assets/qrcode-MIT.txt | Permisywna; bez domyślnych funkcji i zależności image/SVG/PIC. |
| if-addrs | 0.15.0 | Dotychczasowa zależność wind-core | Ten sam EXE, bez nowej biblioteki runtime | Nowe bezpośrednie wywołanie istniejącego komponentu. |

CCS/1
EXT: qrcode upstream https://github.com/kennytm/qrcode-rust (pakiet 0.14.1, niezmieniony); NOTICE w frontend/assets. Brak zewnętrznego generatora QR, zdjęć ani nowych fontów.

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

## Akcje zdalne (WN-54)

Sparowany telefon może poprosić komputer o akcję z jego listy dozwolonych
(PROTOCOL.md §9.2): zablokowanie ekranu (`LockWorkStation`), uśpienie
(`SetSuspendState`, po wysłaniu odpowiedzi), otwarcie linku `http`/`https` w
domyślnej przeglądarce (`ShellExecuteW`) albo własną akcję. Telefon wysyła
tylko identyfikator (i adres dla linku), nigdy polecenia.

Lista to `%LOCALAPPDATA%\WindConnect\actions.json`, obok tożsamości i
`paired.txt`. Okno główne pokazuje ją w sekcji „Akcje zdalne”: przełączniki
zapisują plik, a „Edytuj własne akcje” otwiera go w Notatniku. Plik jest
czytany przy każdej prośbie, więc wyłączenie działa od razu. Brak pliku:
akcje wbudowane włączone, bez własnych. Plik, którego nie da się odczytać:
nic się nie wykona, dopóki go nie poprawisz lub nie usuniesz.

```json
{
  "lock": true,
  "sleep": true,
  "openUrl": true,
  "custom": [
    { "id": "x-kopia", "label": "Kopia zapasowa", "enabled": true,
      "program": "C:\\Program Files\\Backup\\backup.exe", "args": ["--teraz"] }
  ]
}
```

Własna akcja: `id` zaczyna się od `x-` (małe litery, cyfry, `-`), `program`
to pełna ścieżka do pliku `.exe` (bez `.bat`/`.cmd` i bez powłoki), `args`
są stałe. Błędny wpis jest pominięty i opisany w oknie. Ostatnia wykonana
lub odrzucona prośba jest widoczna pod listą.

Test sekcji (Playwright jak dla `check-ui.cjs`, IPC podstawione):
`node apps/windows/scripts/check-actions-ui.cjs`.

Sprawdzenie bez telefonu: przypnij klucz `windctl` jak telefon (parowanie)
i użyj `windctl action` (crates/windctl/README.md).

## Wysyłanie plików (WN-39)

Pliki i foldery idą na telefon silnikiem WN-38 (PROTOCOL.md §9.1), do
połączonego telefonu, a gdy żaden nie jest połączony, do ostatnio
sparowanego: transfery czekają w kolejce i ruszają po połączeniu. Folder to
osobny transfer dla każdego pliku, ze ścieżką względną.

Skąd:

- **Menu Eksploratora**: „Wyślij na telefon” dla plików i folderów
  (`HKCU\Software\Classes\*\shell` i `Directory\shell`, klucz
  `WindConnect.SendToPhone`, bez uprawnień administratora). Aplikacja
  zapisuje wpis przy każdym starcie (ścieżka exe może się zmienić), a
  odinstalowanie usuwa go (`--remove-shell-menu`). W Windows 11 wpis jest w
  klasycznym menu („Pokaż więcej opcji”, Shift+F10); górne menu Windows 11
  wymaga pakietu z tożsamością (MSIX), czego MSI nie ma.
  Eksplorator uruchamia `WindConnect.exe --send "<element>"` dla każdego
  zaznaczonego elementu; taki proces przekazuje ścieżkę działającej
  aplikacji przez potok `\\.\pipe\io.github.prejas3.windconnect.send-<sesja>`
  (tylko ta sesja logowania, bez klientów zdalnych) i kończy się. Gdy
  aplikacja nie działa, pierwszy taki proces ją uruchamia.
- **Przeciągnięcie** na panel albo na okno główne. Panel nie chowa się,
  gdy przeciągany element jest nad nim; można go otworzyć w trakcie
  przeciągania skrótem Win + Shift + W.
- **Kafelek „Wyślij”** w panelu: systemowe okno wyboru wielu plików.

Ikona w zasobniku nie przyjmuje upuszczanych plików: obszar powiadomień
Windows nie przekazuje aplikacjom przeciągania (brak celu OLE dla ikon),
dlatego celem jest panel.

Panel pokazuje wysyłanie: łączny postęp, do czterech pozycji (anuluj,
ponów) i „Wyczyść” dla zakończonych. Telefon, który jeszcze nie przyjmuje
plików (`files/1`, WN-40), daje „Telefon nie przyjmuje teraz plików”.
Komputer nie ogłasza jeszcze `files/1` (odbiór to WN-40); folder
`Pobrane\WindConnect` jest przygotowany pod odbiór.

Test panelu (Playwright jak dla `check-ui.cjs`, IPC podstawione):
`node apps/windows/scripts/check-send-ui.cjs`.

## Parowanie bez aparatu (WN-27)

Panel w trybie kodu QR ma przycisk „Paruj bez aparatu”: otwiera okno SAS
(PROTOCOL.md §12.6) i ogłasza w mDNS port parowania (TXT `pair`, §12.4) na
czas okna. Telefon w „Paruj bez aparatu” pokazuje komputery z tym polem;
po wybraniu oba ekrany pokazują ten sam sześciocyfrowy kod z nazwą drugiej
strony. Klucz drugiej strony jest zapisany dopiero wtedy, gdy na **obu**
ekranach wybrano „Kody są takie same”. „Kody się różnią” albo brak decyzji
w 60 s to nieudana próba; po 3 próbach okno się zamyka (§12.2). Lista na
telefonie to niezaufane wskazówki z sieci: o zaufaniu decyduje porównanie
kodów.

Test panelu (IPC podstawione): `node apps/windows/scripts/check-sas-ui.cjs`.

## Pierwsze uruchomienie (WC-34)

CCS/1
FLOW:start bez `--autostart` i bez sparowanego telefonu -> główne okno z trzema krokami i kodem QR bez klikania (to samo okno parowania co panel) -> parowanie -> krok „Gotowe” (skrót panelu, „Uruchamiaj z Windows”) -> zwykłe okno. Ze sparowanym telefonem start otwiera panel jak dotąd.
RULE:kod QR jak w panelu (WN-94): tylko canvas z `aria-label="Kod parowania"`, moduły zerowane po narysowaniu, bez linku w DOM. Po wygaśnięciu nowy kod tylko na „Wyświetl nowy kod”. Fokus na głównym przycisku kroku. Ustawienia wymagające telefonu czekają do końca onboardingu.
RULE:„sparowany telefon” ma jedną definicję (WN-113): `pairing::has_paired_in`, telefon w `devices`; usunięty telefon, któremu wciąż należy się powiadomienie o rozparowaniu (`owed`), się nie liczy. Wybór pierwszego okna (`has_paired`) i widok onboardingu (pole `paired` w stanie parowania) czytają tę samą regułę.
LIMIT:werdykt „nowy użytkownik paruje bez instrukcji w < 60 s” pochodzi z testu z człowiekiem; skrypty mierzą tylko część, którą dokłada system.

Testy: `node apps/windows/scripts/check-onboarding-ui.cjs` (podstawione IPC:
kroki, zasady QR, SAS, wygaśnięcie, „Gotowe”, fokus, Light/Dark, tekst 150%,
start z samym telefonem `owed`)
i `node apps/windows/scripts/check-onboarding-native.cjs` (prawdziwa aplikacja
z pustym magazynem, `JAVA_HOME`, `ZXING_CORE_JAR`, zamknięty zainstalowany
WindConnect): raport `launchToQrMs`, `qrToDoneMs`, `systemAddsMs`.

## Telefony w panelu (WC-28)

CCS/1
FLOW:panel „Telefony” -> device_action(rename|unpair|disconnect) -> aktor parowania; „Dodaj telefon” otwiera kod QR także w czasie sesji.
RULE:jedna sesja naraz. Ten sam telefon zastępuje swoje stare połączenie; inny dostaje BUSY (PROTOCOL.md §12.10). Telefon sparowany w panelu przed chwilą (2 min) przejmuje sesję. „Rozłącz” odpowiada temu telefonowi BUSY przez 10 min albo do sesji innego telefonu.
SEC:rozparowanie zapisuje `-<klucz>`, odpina i odwołuje klucz od razu; telefon dostaje Goodbye UNPAIRED w sesji albo przy następnym połączeniu (tryb powiadomienia: bez routera i kanałów). Telefon, który sam usunął parowanie, usuwa tylko klucz rozmówcy z TLS.
LIMIT:nazwa jest lokalna (nie trafia do telefonu). Telefon po BUSY nie wraca sam, gdy komputer się zwolni.

Test UI (podstawione IPC): `node apps/windows/scripts/check-devices-ui.cjs`
z `PLAYWRIGHT_MODULE`. Logikę obu stron sprawdzają
`cargo test --locked -p wind-core --test unpair` (rozparowanie w sesji i
poza nią w obie strony, brak 0-RTT ze starego biletu, strumienie
odwołanego klucza bez odbiorcy, dwa telefony naraz bez przepychanki) i
`--test relay` (tryb powiadomienia przez przekaźnik, wymaga `wrangler dev`).

## Wspólny schowek (WN-35)

CCS/1
FLOW:AddClipboardFormatListener/WM_CLIPBOARDUPDATE -> odczyt CF_UNICODETEXT -> clipboard/1 -> Applied po zapisie OS telefonu. Bez odpytywania. Własne operacje są serializowane, a procedura okna odracza zdarzenie przy zajętym adapterze, zamiast blokować komunikaty właściciela schowka.
RETRY:schowek trzymany przez inny proces dłużej niż 30 ms -> jeden WM_TIMER na wątku okna, backoff 50/100/200/400/800/1600 ms (≈3,2 s), potem rezygnacja (zostaje ręczne wysłanie). Ponawia tylko sekwencję, która wywołała zdarzenie; nowa zmiana, wyłączenie monitorowania i zamknięcie okna kasują timer (WN-105).
RULE:automat domyślny; ustawienie w clipboard.json obok tożsamości zawiera tylko bool. Uszkodzony plik oznacza wyłączenie. Przełącznik wyłącza automat wysyłania i odbioru. Kafelek „Schowek” i Ctrl+Alt+Shift+C wysyłają ręcznie; konflikt skrótu jest widoczny i nie wyłącza przycisku. Win+Shift+W pozostaje skrótem panelu.
SEC:nie wysyłamy formatu ExcludeClipboardContentFromMonitorProcessing ani CanIncludeInClipboardHistory/CanUploadToCloudClipboard=0; dotyczy to także ręcznej akcji. Odbiór ustawia oba formaty opt-out=0, zanim zapisze tekst. Brak payloadu w IPC/front-endzie/logach/historii plikowej. Limit256 KiB, bez NUL (terminator Windows jest obcinany); link jest tekstem, nie poleceniem uruchomienia. Szczegóły PROTOCOL.md §9.3.
LIMIT:PC -> telefon automatycznie także bez otwartej Activity. Telefon -> PC z innej aplikacji wymaga akcji powiadomienia i krótkiego fokusu WindConnect; bez dodatkowych uprawnień systemowych. Oznaczone hasła nie są synchronizowane. Brak odtwarzania schowka po reconnect.

Test natywny bez zmieniania schowka interaktywnego:
`cargo run --locked -p windconnect-windows --example clipboard_probe`.
Tworzy osobny pulpit w nieinteraktywnej stacji, wymaga pustego schowka tej
stacji, sprawdza zdarzenia, Unicode i trzy flagi prywatności, usuwa własne
syntetyczne dane i kończy wątki. Drugi proces (ten sam przykład,
`--hold-child`) w tej samej stacji trzyma schowek swoim oknem dłużej niż 30 ms:
sprawdza dostarczenie raz, brak tekstu sensitive, limit ponowień, anulowanie
przy wyłączeniu i brak timera po zamknięciu. Stacja nieinteraktywna sesji
przeżywa uruchomienia: przerwany test może zostawić syntetyczny tekst, a
kolejny odmówi startu, dopóki ktoś go nie usunie. Test Android/PC i pomiar100 próbek w obu
kierunkach opisano w `apps/android/README.md`, sekcja WN-35.

### Obrazy (WN-36)

CCS/1
FLOW:WM_CLIPBOARDUPDATE bez tekstu -> format „PNG” (bez zmian) albo CF_DIBV5/CF_DIB (kodowany do PNG poza wątkiem okna, `png` 0.18, szybka kompresja) + miniatura ≤ 512 px -> clipboard/2 (PROTOCOL.md §9.3.1). Status: „Wysyłanie obrazu…” po Applied telefonu (obraz już wklejalny), „Obraz wysłany na telefon” po ImageDone.
RULE:odebrany obraz trafia do schowka od razu jako „PNG” i CF_DIB z opóźnionym renderowaniem; WM_RENDERFORMAT czeka na bajty (najwyżej 10 s bez postępu), potem renderuje. Nowszy schowek, przerwanie lub zastój: wklejenie się nie udaje, bez częściowego obrazu. WM_RENDERALLFORMATS przy zamknięciu renderuje tylko kompletny obraz, inaczej czyści schowek. Własnego obrazu (właściciel = nasze okno) nie odczytujemy: brak echa i czekania na samego siebie.
RULE:limit „Maks. rozmiar obrazu” 5 / 16 / 50 MB (domyślnie 16) w Ustawieniach i panelu, zapisany w clipboard-images.json obok clipboard.json; dotyczy wysyłania i odbioru.
SEC:te same flagi prywatności co dla tekstu: obrazu oznaczonego jako prywatny nie kopiujemy z OS; odebrany ma opt-out historii i chmury. Tekst ma pierwszeństwo, gdy w schowku są oba.
LIMIT:wątek okna czeka na bajty tylko przy wklejaniu przed końcem przesyłu; ścieżka odbioru fragmentów go nie potrzebuje (bez zakleszczenia). Zrzuty większe niż 16384 px nie są wysyłane. Wklejenie odebranego obrazu jako CF_DIB dekoduje PNG z limitem 64 MiB pikseli (ochrona przed „bombą dekompresyjną”): większy obraz (np. 8K) wkleja się tylko jako „PNG”.

Test OS (ta sama stacja co wyżej): `clipboard_probe` sprawdza odczyt „PNG” i
samego DIB, brak kopii obrazu prywatnego, opóźnione renderowanie (wklejenie
czeka i dostaje cały PNG, CF_DIB z PNG), błąd wklejenia przy nowszym
schowku od razu i przy zastoju po ~10 s, odpowiadające okno po zastoju i
brak odczytu własnego obrazu. Pomiar zrzutu PC -> telefon: `apps/android/README.md`, WN-36.

### Historia i hasła (WN-37)

CCS/1
FLOW:pozycja trafia do historii, gdy telefon potwierdził wysłany schowek (Applied) albo gdy schowek z telefonu został zapisany w Windows. Panel i Ustawienia pokazują „Historia schowka”: najnowsze u góry, kliknięcie (Enter) kopiuje z powrotem do schowka z flagami, jakie miała (z telefonu: poza historią Win+V i chmurą, jak przy odbiorze) i przy włączonej synchronizacji wysyła na telefon, przypinka i „Usuń z historii” obok, „Wyczyść” usuwa nieprzypięte.
RULE:wyłączona synchronizacja: nic nowego nie trafia do historii (także ręczne wysłanie), a wyłączenie czyści nieprzypięte; przypięte zostają.
RULE:20 ostatnich + do 20 przypiętych (przypięte u góry, poza dwudziestką); powtórka tej samej treści (SHA-256) przesuwa się na górę zamiast dublować. Obrazy nieprzypięte do 64 MB łącznie w pamięci (najstarszy wypada), przypięte też do 64 MB; obraz z telefonu można skopiować i przypiąć po dojściu całego PNG.
SEC:historia jest tylko w pamięci; na dysk trafiają wyłącznie przypięte, w `clipboard-pins.bin` obok clipboard.json, zaszyfrowane DPAPI bieżącego użytkownika z własnym celem (nagłówek `WCPN` 1). Plik innego użytkownika, uszkodzony albo nowszy: start bez przypiętych, plik zostaje.
SEC:do panelu (IPC) idzie tylko podgląd tekstu (jedna linia, 120 znaków) albo miniatura; pełny tekst i PNG zostają w procesie, kopiowanie idzie po id. Podgląd wstawiany jako tekst, nigdy HTML; znaki sterujące, bidi (U+202A–202E, U+2066–2069) i zerowej szerokości (U+200B–200F, U+FEFF) zamieniane na spację. Miniatury jako `data:image/png`: CSP pozwala na `data:` tylko w `img-src`.
SEC:hasła: schowek z ExcludeClipboardContentFromMonitorProcessing, „Clipboard Viewer Ignore” (KeePass, Password Safe i inne), CanIncludeInClipboardHistory = 0 albo CanUploadToCloudClipboard = 0 nie jest nawet kopiowany z OS, więc nie wychodzi z PC ani nie trafia do historii. Nie rozpoznajemy haseł po treści: liczą się tylko znaczniki, które ustawia menedżer.
LIMIT:historia tylko na Windows (Android jej nie pokazuje); po restarcie zostają tylko przypięte; bez wyszukiwania.
DATA:pliki schowka w katalogu danych aplikacji (obok tożsamości): clipboard.json (włączona synchronizacja), clipboard-images.json (limit obrazów), clipboard-pins.bin (przypięte, DPAPI). Odinstalowanie MSI nie usuwa katalogu danych (jak tożsamości); usunięcie wszystkich przypiętych usuwa clipboard-pins.bin.

Test „hasło nie opuszcza urządzenia”: `clipboard_probe` (każdy z 4 znaczników
osobno, atomowo, jak menedżer haseł: nic nie jest kopiowane; pozycja z historii
zachowuje flagi; obraz z historii wklejalny jako „PNG” i CF_DIB, bez echa) oraz test
OS z telefonem (`check-background-android.cjs`): PC kopiuje 4 hasła z
pojedynczymi znacznikami, telefon nie dostaje żadnego, a następny zwykły tekst
dochodzi. Logika historii: testy jednostkowe `clipboard_history` (kolejność,
limity, pamięć, przypinanie, plik DPAPI).
Test UI: `node apps/windows/scripts/check-clipboard-ui.cjs` z zewnętrznym
Playwright (`PLAYWRIGHT_MODULE`). Sprawdza klawiaturę, przełącznik, ręczne
wysyłanie bez połączenia, status sensitive/konfliktu skrótu, historię (podgląd jako tekst, kopiuj/przypnij/usuń/wyczyść klawiaturą, własne przewijanie w panelu) i układ Light/Dark.
Podstawione IPC sprawdza UI; dostarczenie OS potwierdza osobny test natywny.

## Bateria telefonu (WN-49)

CCS/1
FLOW:telefon (ACTION_BATTERY_CHANGED, tylko w czasie sesji) -> `Battery` na kanale `battery` (`07`, PROTOCOL.md §9.4) -> `Snapshot.battery` w `pairing-state` (wypychane, bez pollingu) -> nagłówek panelu, podpowiedź ikony w zasobniku, plakietka niskiej baterii, toast.
RULE:panel: poziom i błyskawica przy ładowaniu obok nazwy telefonu, `role="img"` z etykietą „Bateria telefonu 82%, ładuje się”; bez aria-live (zmiana procentu nie jest ogłaszana). Bez sesji albo bez odczytu element znika.
RULE:zasobnik: podpowiedź „WindConnect · Połączono · bateria 82%, ładuje się”. Przy ≤ 15% bez ładowania bursztynowa kropka w prawym dolnym rogu, nałożona na stan WN-23 (nie osobny stan); znika po podłączeniu ładowarki albo końcu sesji.
RULE:alerty (wind-core `battery::Alerts`): toast „Bateria telefonu: N%” przy spadku do ≤ 15% bez ładowania, raz na rozładowanie (ponownie po ładowaniu albo > 20%); „Telefon naładowany” przy 100% na ładowarce, raz (ponownie po odłączeniu albo < 95%). Pierwszy odczyt sesji nie daje toastu. Toast niesie tylko poziom, nazwę telefonu i stały tekst; kliknięcie otwiera panel.
DEC:toasty przez `tauri-winrt-notification` 0.7.3 (to samo, czego używa na Windowsie tauri-plugin-notification), bo daje wywołanie zwrotne kliknięcia. Wersja release rejestruje nadawcę `HKCU\Software\Classes\AppUserModelId\io.github.prejas3.windconnect` (DisplayName „WindConnect”); debug wysyła jako PowerShell, jak plugin w trybie dev. WN-43 może to przejąć.
LIMIT:zmiany dochodzą tylko przy żywej sesji (połączenie w tle włączone na telefonie). W Doze albo bez połączenia w tle komputer nie widzi zmian, więc alert 15% może przyjść późno albo wcale. Klucz AppUserModelId zostaje po odinstalowaniu (jak inne wpisy HKCU, por. WN-88).

Testy: `cargo test -p wind-core` (`battery`: koder, błędne wiadomości, alerty z histerezą; `tests/battery.rs`: prawdziwa sesja, tylko zmiany i w kolejności, PC bez `battery/1` nic nie dostaje), testy jednostkowe aplikacji (plakietka, podpowiedź, teksty), `node apps/windows/scripts/check-battery-ui.cjs` (podstawione IPC: poziom, ładowanie, niski, ukrycie bez sesji, aktualizacje bez wywołań IPC, Light/Dark, 150%) i `node apps/windows/scripts/check-battery-android.cjs` (prawdziwa aplikacja i emulator: `dumpsys battery` zmienia poziom, panel i podpowiedź nadążają, alerty 15% i pełne).

Audyt nowych zależności w EXE/MSI:

| Komponent | Wersja | Licencja z pakietu | Gdzie trafia | Wynik |
| --- | --- | --- | --- | --- |
| tauri-winrt-notification | 0.7.3 | MIT OR Apache-2.0 (LICENSE_MIT, LICENSE_APACHE-2.0, LICENSE.spdx); wybrana MIT | Natywny EXE; frontend/assets/tauri-winrt-notification-MIT.txt | Permisywna |
| windows (+ windows-core, -collections, -future, -link, -numerics, -result, -strings, -threading) | 0.61.3 (części 0.1–0.61) | MIT OR Apache-2.0 (license-mit, license-apache-2.0, identyczne we wszystkich); wybrana MIT | Natywny EXE; frontend/assets/windows-rs-MIT.txt | Permisywna; bez plików NOTICE |

## Znajdź telefon (WN-50)

CCS/1
FLOW:kafelek „Dzwoń” w panelu -> `Ring` na kanale `find` (`08`, PROTOCOL.md §9.5) -> telefon gra ton alarmu i dopiero wtedy odsyła stan 0 -> kafelek „Zatrzymaj” (aria-pressed) -> `Stop` z PC albo zatrzymanie na telefonie -> stan końcowy -> linia stanu („Zatrzymano”, „Zatrzymano na telefonie”, „Telefon dzwonił 60 s”, „Telefon nie może teraz dzwonić”).
RULE:kafelek działa tylko przy sesji i gdy telefon ogłasza `find/1`; wyłączone na telefonie („Pozwól komputerowi dzwonić”) daje kafelek nieaktywny z wyjaśnieniem. Linia stanu ma `role="status"` (odpowiedź na własną akcję).
RULE:sesja przerwana w czasie dzwonienia nie znaczy „zatrzymano”: linia mówi „Połączenie przerwane — telefon może nadal dzwonić (do 60 s)”, bo telefon dzwoni dalej do limitu albo zatrzymania na nim.
PERF:`ring_ms` w stanie parowania to czas od prośby do stanu 0, czyli górna granica startu dźwięku na telefonie.

Testy: `cargo test -p wind-core` (`find`: koder; `tests/find.rs`: dzwonienie i Stop, zatrzymanie na telefonie, limit, drugi Ring, dzwonienie po końcu sesji, brak dźwięku, wycofanie i włączenie `find/1` w trakcie sesji), `node apps/windows/scripts/check-find-ui.cjs` (podstawione IPC: stany kafelka, klawiatura, wyniki, niedostępne, Light/Dark, 150%) i `node apps/windows/scripts/check-find-android.cjs` (prawdziwa aplikacja i emulator z urządzeniem audio, `QEMU_AUDIO_DRV=none`; `FIND_NOTIFICATIONS=denied` sprawdza zatrzymanie bez powiadomień).

## Odtwarzacz telefonu w panelu (WN-46)

CCS/1
FLOW:telefon (`media/1` ogłasza PC, PROTOCOL.md §9.6) -> `Now`/`Art`/`Gone` -> `media::Player` (teksty przez sanitizer podglądu schowka, okładka tylko JPEG/PNG po magicznych bajtach) -> `Snapshot.media` w `pairing-state` -> karta „Na telefonie” (okładka, tytuł, wykonawca, aplikacja, postęp, ⏮ ⏯ ⏭, klik/strzałki na pasku = przewiń) -> `media_action` -> `Command` na `media-control` (telefon ogłasza `media-control/1`).
RULE:teksty z innych aplikacji telefonu trafiają do panelu wyłącznie przez `textContent`; okładka tylko jako `data:image/jpeg|png;base64` sprawdzony też w panelu. Przyciski zależą od `actions` odtwarzacza; telefon i tak pomija polecenie spoza `actions`.
PERF:postęp liczy panel z ostatniego stanu (`position_ms + upływ × speed`) lokalnym zegarem 1 s, tylko gdy panel widoczny i gra; nic nie jest odpytywane.

Testy: `cargo test -p wind-core` (`media`: koder, granice, ekstrapolacja, bramka akcji; `tests/media.rs`: prawdziwa sesja Now/Art/Gone tylko przy zmianie, Command z powrotem, nic bez możliwości), testy aplikacji (sanityzacja, okładka po kluczu), `node apps/windows/scripts/check-media-ui.cjs` (podstawione IPC: teksty, okładka, postęp bez IPC, polecenia, bramka akcji, wrogi tekst, ukrycie, Light/Dark, 150%) i `node apps/windows/scripts/check-media-android.cjs` (prawdziwa aplikacja + emulator z testową sesją multimediów zamiast Spotify; 10 próbek ⏯ i ⏭, przewinięcie, czasy w obie strony, polecenie spoza `actions` bez reakcji).

## Odtwarzacz komputera na telefonie (WN-47)

CCS/1
FLOW:Windows SMTC (`GlobalSystemMediaTransportControlsSessionManager`, zdarzenia sesji i menedżera, wątek MTA) -> `pc_media::PcMedia` -> `Now`/`Art`/`Gone` na `media` (telefon ogłasza `media/1`) -> telefon: karta w aplikacji i systemowe sterowanie multimediami. Polecenie z telefonu (`media-control/1`, ogłaszane przez PC) -> sprawdzenie względem `actions` -> `TryPause/Play/TogglePlayPause/SkipNext/SkipPrevious/ChangePlaybackPosition`.
RULE:wybór sesji: pierwsza odtwarzana, inaczej bieżąca sesja systemu, jeśli wolno ją śledzić, inaczej pierwsza dozwolona. Okładka: miniatura sesji dekodowana i kodowana jako JPEG ≤ 256 px (Windows.Graphics.Imaging), ponad 96 KiB bez okładki. Po własnym poleceniu stan czytany od razu raz jeszcze (odpowiedź na akcję, nie odpytywanie).
RULE:ustawienie w oknie głównym „Udostępnij odtwarzanie telefonowi” (domyślnie włączone, `pc-media.json`): wyłączone wycofuje `media-control/1` także w trwającej sesji, wysyła `Gone` i kończy subskrypcje SMTC. Gdy telefon znów przyjmuje `media/1`, PC wysyła stan od nowa.
SEC:w wersji debug `WINDCONNECT_TEST_MEDIA_APP=<AUMID>` ogranicza śledzenie do jednej aplikacji, by test nie wstrzymał odtwarzaczy użytkownika; release tę zmienną pomija.

Testy: testy jednostkowe (`pc_media`: wybór sesji, akcje, czas w tickach, nazwy, granice), `node apps/windows/scripts/check-pc-media-android.cjs` (prawdziwa aplikacja + `examples/smtc_player` z prawdziwą sesją SMTC `WindConnect.TestPlayer` + emulator: karta i powiadomienie na telefonie, 10× pauza/odtwarzanie ze sterowania Androida i z aplikacji, polecenie spoza `actions` bez reakcji, przełącznik i `Gone` sprzątają, brak echa).

## Powiadomienia z telefonu (WN-43)

CCS/1
FLOW:telefon `Posted`/`Icon`/`Removed` na `notify` (PC ogłasza `notify/1`, PROTOCOL.md §9.7) -> pętla sesji -> `notify::Notifications` (czy wyskakuje) -> jeden wątek roboczy (kolejność zachowana) -> `ToastNotifier.Show` / `ToastNotificationHistory`.
RULE:toast `ToastGeneric`: tytuł (bez tytułu nazwa aplikacji), treść, „aplikacja · telefon”, ikona aplikacji (`appLogoOverride`), nagłówek na aplikację telefonu (grupowanie w centrum powiadomień). Tag = id powiadomienia (hex), grupa `phone`: zmiana zastępuje toast bez ponownego wyskakiwania, `Removed` go zdejmuje. Najwyżej 5 wyskakujących w 2 s; reszta trafia cicho do centrum powiadomień. Kliknięcie otwiera panel.
SEC:teksty pochodzą z innych aplikacji telefonu: druga sanityzacja na PC (jedna linia, bez znaków sterujących, dwukierunkowych i zerowej szerokości, przycięte) i escapowanie XML. Ikona (PNG ≤ 16 KiB) jest dekodowana i kodowana od nowa jako PNG 64 px (Windows.Graphics.Imaging), więc Windows nie czyta bajtów nadawcy; plik `notify-icons\<klucz hex>.png` w katalogu danych aplikacji, najwyżej 256 plików (najstarsze usuwane). Nic nie jest logowane.
RULE:Windows przechowuje treść toastów w swojej bazie powiadomień (centrum powiadomień): każdy wygasa po 24 h, a wyłączenie ustawienia „Powiadomienia z telefonu” (okno główne, domyślnie włączone, `notify.json`; wycofuje `notify/1` także w trwającej sesji) albo usunięcie parowania (z PC lub z telefonu) usuwa od razu wszystkie toasty z telefonu i pamięć ikon. Zwykły koniec sesji ich nie usuwa.
### Odpowiedź i odrzucanie (WN-44)

FLOW:pole „Odpowiedz…” w toaście (gdy `Posted` ma `reply`) i w sekcji „Powiadomienia” panelu (20 najnowszych z sesji) -> `Reply` na `notify-action` (telefon ogłasza `notify-action/1`, PROTOCOL.md §9.8) -> telefon wysyła odpowiedź akcją aplikacji (RemoteInput) i odsyła `Replied` (stan „Wysłano” / „Nie udało się”).
RULE:**Zmiana zachowania:** zamknięcie wyskakującego toastu (✕, przesunięcie) albo ✕ w panelu odrzuca powiadomienie także na telefonie (`Dismiss`); wygaśnięcie i usunięcie przez aplikację nie. Odrzucone na telefonie znika z Windows (`Removed`, jak w WN-43). Bez osobnego ustawienia.
SEC:tekst odpowiedzi nie jest logowany ani przechowywany po wysłaniu; w panelu pole czyści się dopiero po `Replied` ok, po błędzie tekst zostaje. Ponad 1024 B UTF-8: wysłanie zablokowane z komunikatem, nigdy przycinane (z toastu tekst trafia do pola w panelu). Akcje tylko dla powiadomień pokazanych w tej sesji (sprawdza PC i telefon).
LIMIT:aplikacja bez pakietu (bez aktywatora COM) dostaje zdarzenia toastu tylko, dopóki jego obiekt żyje w procesie: wątek roboczy trzyma je do `Removed`/odrzucenia/wygaśnięcia (najwyżej 256). Po ponownym uruchomieniu aplikacji stare toasty w centrum powiadomień najwyżej ją otwierają, a ich pole odpowiedzi nie działa. Wyczyszczenie w centrum powiadomień nie daje zdarzenia, więc nie odrzuca na telefonie.

Testy: testy jednostkowe (`notify`: pole odpowiedzi w XML tylko z `reply`, aktywacja toastu po argumentach i tekście (odpowiedź, pusta, za długa bez przycinania), tylko zamknięcie przez osobę odrzuca na telefonie, tag -> id, lista panelu: limit 20, stany, szkic, usuwanie), `wind-core` (kodek `Dismiss`/`Reply`/`Replied`, granice, wektory z PROTOCOL.md §9.7–9.8, sesja: `notify-action/1` tylko za zgodą telefonu), Kotlin (`NotificationRules.replyAction`: `SEMANTIC_ACTION_REPLY`, dowolny tekst, bez kontekstowych), `node apps/windows/scripts/check-notify-panel-ui.cjs` (sekcja panelu z podstawionym IPC: Enter, pole i fokus przetrwają odświeżenie, czyszczenie dopiero po `Replied` ok, tekst zostaje po błędzie, > 1024 B zablokowane, szkic z toastu, ✕, Light/Dark) i `node apps/windows/scripts/check-notify-reply-android.cjs` (prawdziwa aplikacja debug + emulator + Google Messages zasilane `adb emu sms send`: toast z polem odpowiedzi, odpowiedź z panelu i z toastu (`dev_toast_activate`, ta sama funkcja co WinRT) wysłana jako SMS, ✕ w panelu znika z telefonu, uśpione na telefonie znika z PC, zamknięcie popupu przez osobę odrzuca, wygaśnięcie nie). Prawdziwego kliknięcia i zamknięcia systemowego toastu nie da się tu zautomatyzować: sprawdza je krótki test ręczny na telefonie. Wymaga tego samego co test WN-43; asystent powiadomień jest na czas testu wyłączony (Android 15 ukrywa treść SMS jako wrażliwą).

### Filtry i tryb skupienia (WN-45)

RULE:wybór aplikacji jest na telefonie (wyłączona aplikacja nie wysyła nic). PC tylko wycisza: w „Cichych godzinach” (okno główne, domyślnie wyłączone, 22:00–07:00, krok 15 min, przez północ), gdy `SHQueryUserNotificationState` mówi o pełnym ekranie, grze Direct3D, prezentacji albo czasie ciszy po instalacji, i gdy telefon oznaczy powiadomienie `quiet` (jego „Nie przeszkadzać”), toast idzie bez wyskakiwania i z `<audio silent="true"/>` do centrum powiadomień i panelu, nie zużywając limitu wyskakujących. Tryb „Nie przeszkadzać”/skupienia Windows sam wstrzymuje toasty aplikacji spoza listy priorytetowej, więc nie czytamy go (bez nieudokumentowanego WNF). Sprawdzane przy każdym `Posted`, bez timerów. Wyciszony toast nie odrzuca niczego na telefonie (tylko UserCanceled, WN-44).
RULE:`notify.json` to `{enabled, quiet:{on,from,to}}` (minuty od lokalnej północy); stary plik `true`/`false` jest czytany, uszkodzony daje ustawienia domyślne; zapis przez plik tymczasowy i zmianę nazwy.

Testy: testy jednostkowe (`notify`: ciche godziny przez północ, `from == to`, granice; migracja i uszkodzony `notify.json`, zapis; stany `SHQueryUserNotificationState`; ciche bez wyskakiwania i dźwięku, bez zużycia limitu, zamknięcie nie odrzuca na telefonie), `node apps/windows/scripts/check-notify-quiet-ui.cjs` (ustawienie z podstawionym IPC, klawisze, północ, Light/Dark) i `node apps/windows/scripts/check-notify-filter-android.cjs` (prawdziwa aplikacja + emulator, powłoka i Google Messages jako nadawcy: zablokowana aplikacja nic nie przekazuje do rdzenia, „Tylko wybrane” bez wyboru nic, zablokowanie usuwa jej powiadomienia z PC, „Nie przeszkadzać” na telefonie i ciche godziny na PC bez wyskakiwania, a powiadomienie zostaje na telefonie).

## Połączenia i głośność (WN-48)

FLOW:telefon `State` na `call` (PC ogłasza `call/1` przy ustawieniu „Wstrzymaj odtwarzanie przy połączeniu”, okno główne, domyślnie włączone, PROTOCOL.md §9.10) -> pętla sesji -> jeden wątek `call_pause` (żyje między sesjami) -> `CallPause` decyduje -> SMTC `TryPauseAsync` na każdej grającej sesji / `TryPlayAsync` na tych, które wstrzymał, jeśli nadal są wstrzymane (≤ 30 min). W debug `WINDCONNECT_TEST_MEDIA_APP` ogranicza to do jednego odtwarzacza.
FLOW:głośność (`volume/1`, §9.9, ustawienie „Głośność z telefonu”, domyślnie włączone): Core Audio (domyślne urządzenie, `IAudioEndpointVolume` + jego wywołanie zmian, `IMMNotificationClient` przy zmianie urządzenia, bez odpytywania) -> `Level` do telefonu; `Set` z telefonu -> Core Audio. Panel: „Głośność telefonu” (suwak: strzałki 5 %, PageUp/PageDown 10 %, Home/End; wycisz), bez „odbijania”: przez 500 ms po własnej zmianie i podczas przeciągania przychodzący poziom nie przesuwa suwaka.
LIMIT:urządzenie o stałej głośności (część wyjść HDMI/S/PDIF/USB) nie zmienia poziomu; wyciszenie działa. W trybie wibracji część telefonów nie zmienia trybu dźwięku: wtedy pauzę daje powiadomienie połączenia, które wymaga dostępu „Powiadomienia na komputerze” (opis ustawienia o tym mówi). „Rozmowa” to też rozmowy w komunikatorach (Discord, Teams, notatki głosowe).

Testy: testy jednostkowe (`call_pause`: jedna pauza na połączenie, wznowienie tego, co wstrzymał, zerwana sesja rozstrzygnięta przez następny stan, limit 30 min, ustawienia; `pc_volume`: skala; ręczny `cargo test pc_volume -- --ignored` czyta domyślne urządzenie), `wind-core` (kodeki `State`/`Level`/`Set`, granice, wektory §9.9–9.10, „najnowszy wygrywa”), `node apps/windows/scripts/check-call-volume-ui.cjs` (suwak, klawisze, wyciszenie, brak odbijania, przełączniki, Light/Dark) i `node apps/windows/scripts/check-call-volume-android.cjs` (prawdziwa aplikacja + `examples/smtc_player` + emulator: 5 połączeń `adb emu gsm call`, dzwonek -> pauza p95 < 1 s (zmierzone ok. 0,2-0,3 s; drugi dzwonek ok. 6 s), rozłączenie -> wznowienie, odebrane, zatrzymane ręcznie, ustawienie wyłączone, tryb wibracji, głośność w obie strony; głośność i wyciszenie PC przywracane od razu).

RULE:nadawca toastów jak przy baterii (`toast.rs`): w debug PowerShell, w release zarejestrowany AUMID „WindConnect” (sprawdzenie z instalatora: WN-118).

Testy: testy jednostkowe (`notify`: XML, escapowanie i sanityzacja, brak tytułu, granice, limit wyskakujących, ciche aktualizacje, nazwy i limit pamięci ikon), `node apps/windows/scripts/check-notify-settings-ui.cjs` (przełącznik z podstawionym IPC, Light/Dark) i `node apps/windows/scripts/check-notify-toasts-android.cjs` (prawdziwa aplikacja debug + emulator; parowanie QR, potem `NotifyProof` dwa razy w jednej sesji: z asystentem powiadomień i bez). Sprawdza: dokładnie 21 toastów na przebieg (20 + rozmowa; odtwarzacz i niezmienione ponowienie nic), uśpione na telefonie znika z Windows, pozostałe 20 w historii, ≤ 5 wyskakujących w 2 s, jedna ikona 64 px, wyłączenie ustawienia czyści historię i ikony. Opóźnienie (zegary skorygowane o przesunięcie emulatora): wywołanie słuchacza na telefonie -> `Show` na PC p95 < 250 ms (sprawdzane; zmierzone ok. 20 ms), `postTime` -> `Show` raportowane (z asystentem ok. 230 ms przez systemowe wstrzymanie ok. 200 ms, bez niego ok. 30 ms). Wymaga `JAVA_HOME`, `ZXING_CORE_JAR`, `PLAYWRIGHT_MODULE`, `ANDROID_SERIAL`.

## Wiadomości SMS (WN-55)

Przycisk „Wiadomości” w panelu (gdy telefon jest połączony) otwiera osobne
okno: wątki po lewej, rozmowa po prawej (PROTOCOL.md §9.11, `sms/1`).
Telefon udostępnia SMS dopiero po włączeniu „SMS na komputerze” i zgodzie na
dostęp do SMS; bez tego okno mówi, co włączyć. Wątek otwiera się z pamięci
(`sms.rs`) i dociąga tylko nowsze wiadomości (`since`); pierwsze otwarcie
pobiera 1000 najnowszych, starsze strony przy przewinięciu do góry. Zmiany na
telefonie przychodzą przez obserwację (`Watch` → `sms-changed`). Treść jest
tylko w pamięci aplikacji (bez zapisu na dysku), znika przy połączeniu innego
telefonu albo rozparowaniu i trafia na stronę wyłącznie jako tekst.
Pomiar: `scripts/check-sms-android.cjs` (prawdziwa aplikacja + emulator,
1000 SMS w wątku, otwarcie od kliknięcia do narysowania).

### Wysyłanie SMS i MMS (WN-56)

W oknie „Wiadomości” pole na dole wysyła odpowiedź w otwartym wątku, a „+”
zaczyna nową wiadomość (numery oddzielone przecinkami). Gdy telefon ma kilka
kart SIM, obok jest wybór karty. Ikona obrazu dołącza zdjęcie: okno zmniejsza
je do 1280 px i JPEG do ok. 300 KB, a wiadomość idzie jako MMS (także przy
kilku odbiorcach). Stan wysyłki widać od razu (wysyłanie…, wysłano,
dostarczono, nie wysłano); potem wiadomość zastępuje zapis z telefonu.
Zdjęcia z odebranych MMS ładują się przy przewijaniu. Wysyłanie trzeba
włączyć na telefonie („Wysyłanie z komputera”); do tego czasu pole to mówi.
Pomiar: `scripts/check-sms-send-android.cjs`.

## Ostatnie zdjęcia w panelu (WN-58)

Gdy telefon udostępnia zdjęcia („Zdjęcia na komputerze”, PROTOCOL.md §9.13,
`gallery/1`), panel pokazuje 10 najnowszych zdjęć i zrzutów ekranu (dwa rzędy
po pięć). Nowe zdjęcie przychodzi przez obserwację (`Watch` → `Recent` →
miniatury tylko nowych), bez odpytywania. Naciśnięcie zdjęcia zaczyna
pobierać oryginał; przeciągnięcie oddaje go Windows jako plik (OLE
`DoDragDrop`, obiekt danych powłoki z `SHCreateItemFromParsingName`, tylko
kopiowanie), a kliknięcie lub Enter otwiera go w domyślnej przeglądarce. Gdy
przycisk puszczono, zanim oryginał doszedł, panel prosi o ponowne
przeciągnięcie (plik już czeka). Własne przeciągnięcie nie jest brane za
pliki do wysłania na telefon ani nie chowa panelu.
Miniatury są tylko w pamięci; oryginały w `%TEMP%\WindConnect\gallery`
(najwyżej 8), usuwane z końcem sesji, przy połączeniu innego telefonu i przy
starcie. CCS/1 SEC:PC dopuszcza wyłącznie końcowe rozszerzenia jpg, jpeg,
png, webp, heic, heif, gif, avif, bmp (bez rozróżniania wielkości liter).
Inną nazwę zastępuje `IMG_<id>.<rozszerzenie z MIME>`; brak dozwolonego
MIME oznacza odmowę. Przed otwarciem/przeciągnięciem sprawdza sygnaturę
obrazu i zapisuje `Zone.Identifier` (`ZoneId=3`); błąd oznacza odmowę
i usunięcie pliku częściowego. Nazwa pozostaje jedną częścią ścieżki.
Pomiar: `scripts/check-gallery-android.cjs` (prawdziwa aplikacja + emulator:
nowe zdjęcie od polecenia do narysowanej miniatury < 1 s, oryginał zgodny co
do SHA-256, ustawienie telefonu wył./wł.). Samo przeciągnięcie myszą wymaga
testu ręcznego.

## Połączenia z telefonu (WN-57)

Ustawienie „Połączenia z telefonu” (okno główne, domyślnie włączone,
`call-volume.json`) pokazuje połączenie przychodzące jako powiadomienie
Windows (`calls.rs`, scenariusz incomingCall, bez dźwięku): nazwa kontaktu lub
numer, „Wycisz” i „Odrzuć” (tylko te, które telefon umie), znika po odebraniu
lub końcu. Nieodebrane: powiadomienie o nowych i lista w panelu („Nieodebrane
połączenia”, najwyżej 5). Telefon musi to udostępnić („Połączenia na
komputerze”). Komputer przyjmuje `call/1`, gdy włączone jest to ustawienie
albo wstrzymywanie odtwarzania; pauza działa tylko przy tym drugim.
Pomiar: `scripts/check-calls-android.cjs` (połączenie -> powiadomienie na PC).

## Otwórz na telefonie (WN-100)

Pole „Otwórz na telefonie” w panelu (gdy telefon przyjmuje `actions/1`)
wysyła adres http(s) do telefonu (PROTOCOL.md §9.2 `open-url`, sprawdzony
`check_url` już na PC). Telefon otwiera go od razu albo pokazuje
powiadomienie, gdy jest w tle; wyłączone na telefonie -> „Telefon
odrzucił…”. Telefon wykonuje tylko otwarcie linku, nic innego.
Pomiar: `scripts/check-actions-android.cjs`.
