# Windows

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
IO: 20 pomiarów <80 ms; preloading bez flash, Escape, utrata fokusu, otwarcie aplikacji, motywy, DM Sans, reduced-motion. Raport i PNG: %TEMP%/windconnect-wn24 (lub PANEL_ARTIFACTS). Runner zamyka własny proces i przeglądarkę.
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

## Wspólny schowek (WN-35)

CCS/1
FLOW:AddClipboardFormatListener/WM_CLIPBOARDUPDATE -> odczyt CF_UNICODETEXT -> clipboard/1 -> Applied po zapisie OS telefonu. Bez odpytywania. Własne operacje są serializowane, a procedura okna odracza zdarzenie przy zajętym adapterze, zamiast blokować komunikaty właściciela schowka.
RULE:automat domyślny; ustawienie w clipboard.json obok tożsamości zawiera tylko bool. Uszkodzony plik oznacza wyłączenie. Przełącznik wyłącza automat wysyłania i odbioru. Kafelek „Schowek” i Ctrl+Alt+Shift+C wysyłają ręcznie; konflikt skrótu jest widoczny i nie wyłącza przycisku. Win+Shift+W pozostaje skrótem panelu.
SEC:nie wysyłamy formatu ExcludeClipboardContentFromMonitorProcessing ani CanIncludeInClipboardHistory/CanUploadToCloudClipboard=0; dotyczy to także ręcznej akcji. Odbiór ustawia oba formaty opt-out=0, zanim zapisze tekst. Brak payloadu w IPC/front-endzie/logach/historii plikowej. Limit256 KiB, bez NUL (terminator Windows jest obcinany); link jest tekstem, nie poleceniem uruchomienia. Szczegóły PROTOCOL.md §9.3.
LIMIT:PC -> telefon automatycznie także bez otwartej Activity. Telefon -> PC z innej aplikacji wymaga akcji powiadomienia i krótkiego fokusu WindConnect; bez dodatkowych uprawnień systemowych. Oznaczone hasła nie są synchronizowane. Brak odtwarzania schowka po reconnect.

Test natywny bez zmieniania schowka interaktywnego:
`cargo run --locked -p windconnect-windows --example clipboard_probe`.
Tworzy osobny pulpit w nieinteraktywnej stacji, wymaga pustego schowka tej
stacji, sprawdza zdarzenia, Unicode i trzy flagi prywatności, usuwa własne
syntetyczne dane i kończy wątki. Test Android/PC i pomiar100 próbek w obu
kierunkach opisano w `apps/android/README.md`, sekcja WN-35.

Test UI: `node apps/windows/scripts/check-clipboard-ui.cjs` z zewnętrznym
Playwright (`PLAYWRIGHT_MODULE`). Sprawdza klawiaturę, przełącznik, ręczne
wysyłanie bez połączenia, status sensitive/konfliktu skrótu i układ Light/Dark.
Podstawione IPC sprawdza UI; dostarczenie OS potwierdza osobny test natywny.
