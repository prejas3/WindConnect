# Android

## Skróty launchera (WN-72)

CCS/1
DEC:wariant A zatwierdzony przez użytkownika. „Wyślij schowek” działa; pliki i touchpad są jawnie niedostępne do WN-40/WN-51. Podłączenie istniejących skrótów: WN-116.
FLOW:przytrzymanie ikony -> statyczny send_clipboard (dostępny od instalacji) i dynamiczne send_file/touchpad (publikowane przy starcie procesu). Ustawienia -> „Przypnij: …” -> requestPinShortcut; można też przeciągnąć pozycję z menu launchera na pulpit. Dialog przypięcia renderuje launcher; treść aplikacji używa własnych komponentów motywu.
RULE:stabilne ID; schowek immutable, przypinany przez samo ID. Pliki/touchpad mają „niedostępne” w etykiecie i osobny ekran wyjaśnienia zamiast pozorowania działania. Prośba do launchera nie jest potwierdzeniem przypięcia: UI mówi tylko o przekazaniu prośby. Limit dynamicznych pozycji uwzględnia statyczny schowek.
SEC:eksportowany LauncherShortcutActivity akceptuje wyłącznie trzy stałe action; ignoruje URI/extras. Schowek -> istniejący SendClipboardActivity, bez Home, odczyt dopiero przy fokusie/odblokowaniu; offline/sensitive/limity jak WN-35. Informacja o niegotowych funkcjach nie otwiera nowego klienta i nie uruchamia połączenia.
TEST:LauncherActionTest, LauncherShortcutsTest (rejestracja, brak duplikatów, bezpośrednie cele/offline, odrzucenie obcej akcji). Fizyczny S25 Ultra/One UI: przytrzymanie ikony, trzy pozycje, przeciąganie i przypięcie z Ustawień; działający schowek do PC i powrót do poprzedniej aplikacji, offline, blokada/anulowanie odblokowania; dwa niegotowe skróty z uczciwym komunikatem, bez Home. Jasny/ciemny,150% tekstu i TalkBack. Emulator nie zastępuje launchera Samsunga.

## Kafelek szybkich ustawień (WN-32)

CCS/1
FLOW: Android13+: Ustawienia -> „Dodaj do szybkich ustawień” -> systemowy dialog requestAddTileService. Dodano/już dodany -> potwierdzenie; odmowa -> brak ponawiania, akcja pozostaje w Ustawieniach. Starszy Android: instrukcja ręcznej edycji panelu zamiast przycisku. Stuknięcie kafelka -> odblokowanie -> istniejący ekran wysyłania schowka -> powrót po dostarczeniu. Przytrzymanie -> aplikacja.
STATE: aktywny = połączony komputer; nieaktywny = łączenie lub brak połączenia. Nazwa/stany są opisane dla TalkBack. SystemUI renderuje kafelek i kolory motywu systemowego; aplikacja dostarcza własną ikonę i tekst.
RULE: panel obserwuje stan istniejącego ConnectionOwner, bez nowego klienta, połączenia, timera ani odczytu schowka. Obserwacja trwa tylko podczas nasłuchu TileService. Ostatnie zwolnienie klienta resetuje stan; ponowne otwarcie panelu od razu odczytuje aktualny stan. Usługa połączenia aktualizuje ten sam stan bez otwierania Activity.
SEC: TileService jest eksportowany wyłącznie z systemowym BIND_QUICK_SETTINGS_TILE. Odczyt schowka odbywa się po odblokowaniu i uzyskaniu fokusu SendClipboardActivity; obowiązują istniejące limity/sensitive. Brak payloadu i surowych błędów w kafelku.

Testy: `:app:testDebugUnitTest` (stany, utrata połączenia, prywatność),
`ClipboardTileRegistrationTest` w `:app:connectedDebugAndroidTest` (deklaracja
usługi, uprawnienie, odkrywanie, cel przytrzymania).
Na S25 Ultra / One UI: dodaj kafelek, zamknij aplikację przy działającej usłudze,
rozłącz/przywróć PC i sprawdź zmianę stanu w otwartym panelu oraz po jego
ponownym otwarciu. Skopiuj tekst, stuknij kafelek, sprawdź schowek PC i powrót;
powtórz z blokadą ekranu i anulowanym odblokowaniem (bez wysyłki), schowkiem
wrażliwym oraz bez połączenia. Przytrzymanie otwiera WindConnect. Sprawdź
jasny/ciemny motyw systemu, tekst 150% i odczyt TalkBack. Emulator nie
potwierdza zachowania panelu Samsunga.

Ikona WN-93 „Orbita” ma warstwy adaptive i monochrome, własny biały wektor
powiadomienia oraz mipmapy mdpi–xxxhdpi. Źródła i odtwarzanie:
`assets/brand/README.md`. Na S25 Ultra sprawdź launcher One UI przy włączonych
i wyłączonych ikonach tematycznych oraz stałe powiadomienie; emulator potwierdza
zasoby/API Androida, ale nie zachowanie launchera Samsunga.

Aplikacja Kotlin / Jetpack Compose ze skanerem QR i potwierdzeniem parowania z PC.
Moduł `:wind-core` zawiera wiązania UniFFI 0.32.2, wrapper korutynowy ze
`StateFlow` i biblioteki Rust dla arm64-v8a oraz x86_64. Wymagany Android 10+
(minSdk 29). Parowanie QR korzysta z protokołu WN-19 przez UniFFI;
debug zachowuje dodatkowy ekran ręcznego testu połączenia z PC.

## Budowanie

Wymagania: JDK 17, Android SDK Platform 35, Build Tools 35.0.0,
NDK 27.0.12077973 oraz stabilny Rust/Cargo w PATH.
Zainstaluj cele Rust: `rustup target add aarch64-linux-android x86_64-linux-android`.
Ustaw `JAVA_HOME` na katalog JDK, a `ANDROID_HOME` na katalog SDK
(albo wpisz ścieżkę `sdk.dir` w lokalnym, ignorowanym `local.properties`).
Gradle Wrapper pobiera przypięty Gradle 8.11.1 i sprawdza SHA-256 dystrybucji.

Z katalogu `apps/android` na Windows:

```powershell
.\gradlew.bat --no-daemon :app:lintDebug
.\gradlew.bat --no-daemon :app:assembleDebug
```

Na Linuxie/macOS użyj `./gradlew` zamiast `.\gradlew.bat`.
Wynik: `app/build/outputs/apk/debug/app-debug.apk`.

ABI (WN-95): wywołanie z samymi zadaniami debug (`assembleDebug`,
`connectedDebugAndroidTest`, `testDebugUnitTest`, `lintDebug`…) poza CI
kompiluje i pakuje jedno ABI, domyślnie `x86_64` (emulator). Na telefon
(S25 Ultra) dodaj `-PwindAbi=arm64-v8a`; oba naraz: `-PwindAbi=all`.
Release, `build` i wszystko w CI (zmienna `CI`) budują oba ABI. Biblioteki
Rust trafiają do wspólnego `target/` repozytorium (bez drugiej kopii w
`wind-core/build`); sprzątanie: `scripts/clean-build.ps1` (MSI usuwa tylko w
wersjach starszych niż `[workspace.package]` z `Cargo.toml`, a bez odczytanej
wersji żadnego; test: `pwsh scripts/test-clean-build.ps1`).
Klucz debug jest generowany lokalnie przez narzędzia Androida; APK służy do testów.

## CI i instalacja

Workflow **Android debug APK** uruchamia lint i buduje APK dla PR,
push do `master` oraz ręcznego uruchomienia. W Actions pobierz artefakt
`WindConnect-android-debug-apk` i rozpakuj ZIP.

Na telefonie z Androidem 10+ włącz debugowanie USB, zatwierdź komputer i wykonaj:

```sh
adb install -r app-debug.apk
adb shell am start -W -n io.github.prejas3.windconnect/.MainActivity
```

Powinien pojawić się ekran „Połącz z komputerem”. Debug: przycisk „Test połączenia z PC”
otwiera diagnostykę z odciskiem i kluczem publicznym Androida.
Ikona robocza jest własnym zasobem projektu na licencji MIT. Aplikacja
uprawnienia CAMERA żąda dopiero po wybraniu skanera i naciśnięciu „Zezwól na aparat”;
odbiór linku z aparatu systemowego nie wymaga tego uprawnienia. Używa też INTERNET. Instalację na telefonie oraz pobranie APK z CI trzeba
potwierdzić po opublikowaniu repozytorium.

Gradle Wrapper jest narzędziem budowania na licencji Apache-2.0;
tekst licencji znajduje się w `gradle/wrapper/LICENSE`.


## Parowanie QR (WN-26)

CCS/1
FLOW: „Zeskanuj kod QR” -> zgoda CAMERA -> Camera2/YUV -> ZXing -> ścisły parser Rust §12.4 -> ekran nazwy i odcisku -> jawne „Sparuj urządzenia” -> uwierzytelniony protokół WN-19 -> zapis przypiętego klucza.
IO: Link `windconnect:pair?v=1&k=…&s=…&a=…[&n=…]` obsługiwany przez ACTION_VIEW/DEFAULT/BROWSABLE (schemat z PROTOCOL.md). Zewnętrzny aparat otwiera ekran potwierdzenia; ciepły start obsługuje onNewIntent. Link nigdy nie uruchamia parowania sam.
SEC: Skan/intent używają tego samego parsera Rust: limit 1024 bajty, wersja, klucz Ed25519, jednorazowy sekret, adresy i powtórzone pola. Błędny link nie nadaje zaufania. Nazwa z linku jest niezaufana; UI podaje też odcisk. Nie wyświetlamy ani nie logujemy sekretu; intent jest czyszczony, oferta nie trafia do saved state/preferences. Po odtworzeniu procesu trzeba zeskanować kod ponownie.
STATE: Oferta żyje w obiekcie Rust, wymazującym sekret przy zwolnieniu. Anulowanie zwalnia ofertę; równoległe intenty nie zastępują trwającego parowania. Próba trwa maksymalnie 30 s; po błędzie wymagany nowy kod. Sekret jest konsumowany przez FFI jednokrotnie. Potwierdzone klucze: noBackupFilesDir/paired.txt, ładowane przy otwieraniu WindCore; klucz prywatny pozostaje w istniejącym KeystoreIdentityStore.
RULE: Uszkodzone linie paired.txt są pomijane zgodnie z WN-19. load_paired zwraca poprawne devices i osobne skipped_records; poprawne klucze pozostają przypięte. Liczba pominiętych wpisów przechodzi przez WindCore do nieblokującego komunikatu na ekranie głównym. Błąd odczytu pliku nadal jest błędem magazynu; zła linia nie blokuje startu ani nie powoduje wyczyszczenia pliku.
RULE: Aparat tylko na ekranie skanera i w stanie resumed; sesja/obrazy zamykane po wyjściu lub onPause. Brak zapisu zdjęć, wysyłania klatek i usługi aparatu w tle. Odmowa zgody ma własny komunikat i przejście do ustawień; link systemowy działa bez zgody CAMERA. Brak aparatu nie blokuje instalacji.
LIMIT: Ekran urządzeń i usługa należą do kolejnych kart lane. WN-25 dodaje QR na PC i automatyczne połączenie po parowaniu: adres z odkrywania, klucz wyłącznie z zapisanego zaufania, limit 7 s. Brak połączenia nie cofa parowania; status UI śledzi sesję. Fizyczne zachowanie Samsung Camera na S25 Ultra/One UI wymaga sprawdzenia: aparat musi zaoferować otwarcie custom URI `windconnect:pair`. Obsługa VIEW potwierdzona testem Androida nie zastępuje tego testu aparatu.

Testy:

```powershell
cargo test --locked -p wind-ffi --test pairing
.\gradlew.bat --no-daemon :app:connectedDebugAndroidTest
```

CCS/1
STATE: PairingTest: VIEW intent, jawne potwierdzenie, anulowanie bez zapisu zaufania, odtworzenie Activity i kolejny intent, błędne/obce/za długie linki, QR w czterech obrotach, puste klatki i wejście/wyjście ze skanera. Bridge Rust: rzeczywisty QUIC, brak połączenia przed potwierdzeniem, trwałość klucza po ponownym otwarciu, odmowa replay i brak sekretu w błędach.
IO: Opcjonalny lokalny peer Androida: `cargo run --locked -p wind-ffi --example qr_pairing_peer -- <katalog-TEMP>`. Zapisuje testowy offer.txt (jednorazowy sekret, 120 s), nasłuchuje na loopback i podaje adres 10.0.2.2 emulatora. Test `PairingTest#explicitConfirmationPairsWithRealPc` przyjmuje argument instrumentacji pairingLink z tego pliku. Nie używać jako serwera produkcyjnego ani dla prawdziwych tożsamości; plik pozostaje wyłącznie w TEMP, poza logiem testów.

Nowa zależność dostarczana w APK:

| Komponent | Wersja | Licencja i pliki źródłowe | Gdzie trafia | Wynik audytu |
| --- | --- | --- | --- | --- |
| ZXing Core | 3.5.4 | Apache-2.0, upstream LICENSE i NOTICE; nagłówki klas core i POM | DEX; assets/ZXing-LICENSE.txt i ZXing-NOTICE.txt | Permisywna; niezmieniona biblioteka, brak runtime zależności przechodnich. Pełne teksty dołączone i zweryfikowane w APK. |

CCS/1
EXT: https://github.com/zxing/zxing/tree/zxing-3.5.4/core ; źródła licencji https://github.com/zxing/zxing/blob/zxing-3.5.4/LICENSE i NOTICE. Audyt WN-26 dotyczy przyrostu; istniejące DM Sans/AndroidX/JNA/Rust pozostają bez nowych zasobów/licencji. Camera2, własny podgląd i przyciski nie dodają biblioteki UI.

## Klucz tożsamości (Android Keystore)

`identity/KeystoreIdentityStore.kt` przechowuje klucz tożsamości urządzenia
(PKCS#8 PEM z `wind-core`) w `noBackupFilesDir/identity.key`, zaszyfrowany
AES-256-GCM kluczem z Android Keystore (StrongBox, gdy telefon go ma).
Klucza AES nie da się wyeksportować. Sam klucz Ed25519 nie może żyć w
Keystore (StrongBox nie ma Ed25519, Keystore ma go dopiero od API 33,
a TLS w `wind-core` podpisuje w procesie), więc na dysku chroni go sprzęt,
a w czasie użycia jest w pamięci aplikacji. Gdzie faktycznie jest klucz AES,
mówi `securityLevel()` (StrongBox / TEE / software), odczytane z `KeyInfo`.
Adapter UniFFI korzysta z tego magazynu w `WindCore.open`; błąd odczytu lub
odszyfrowania przerywa inicjalizację, zamiast generować nową tożsamość.

Testy instrumentacyjne (emulator albo telefon z debugowaniem USB), z
katalogu `apps/android`:

```powershell
.\gradlew.bat --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest
adb install -r -t app/build/outputs/apk/debug/app-debug.apk
adb install -r -t app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
$runner = "io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner"
$test = "io.github.prejas3.windconnect.identity.KeystoreIdentityStoreTest"
adb shell am instrument -w -r $runner
```

Na prawdziwym telefonie dodaj `-e requireHardware true`: test wymaga wtedy,
by klucz AES był w TEE albo StrongBox (emulator ma Keystore programowy, więc
tam ten wariant nie przejdzie). Status `wrappingKeySecurityLevel` w wyniku
mówi, gdzie jest klucz.

Restart urządzenia (klucz przetrwa ponowne uruchomienie):

```powershell
adb shell am instrument -w -r -e phase write -e class "$test#restartPhaseWrite" $runner
adb reboot; adb wait-for-device
# poczekaj, aż `adb shell getprop sys.boot_completed` zwróci 1
adb shell am instrument -w -r -e phase verify -e class "$test#restartPhaseVerify" $runner
```

`bootId` w obu fazach musi się różnić, a `restartVerified` równać `restartWritten`.

## Moduł Gradle i interfejs korutynowy (WN-17)

`crates/wind-ffi` jest osobnym adapterem UniFFI nad niezmienionym API `wind-core`.
Gradle buduje bibliotekę hosta do generowania Kotlin, a następnie dwie biblioteki
Androida NDK, ze stronami ELF wyrównanymi do 16 KB. Wyniki generowania trafiają
wyłącznie do `wind-core/build/`; do repozytorium trafiają źródła i przypięty
generator, a nie ręcznie edytowane wiązania lub binaria.

```powershell
.\gradlew.bat --no-daemon :wind-core:assembleRelease
.\gradlew.bat --no-daemon :wind-core:publishReleasePublicationToBuildRepository
```

AAR: `wind-core/build/outputs/aar/wind-core-release.aar`.
Lokalne repozytorium Maven z AAR, źródłami, POM i metadanymi Gradle:
`wind-core/build/repo/`. Współrzędne: `io.github.prejas3.windconnect:wind-core:0.41.0`.
Publikacja zapisuje pliki lokalnie; nie wysyła ich do zewnętrznego repozytorium.
Konsument musi korzystać z POM/metadanych (JNA i korutyny są zależnościami
przechodnimi); sama kopia pliku AAR nie dostarczy JNA.

W tym repozytorium aplikacja używa `implementation(project(":wind-core"))`.
Konsument spoza repozytorium dodaje lokalne repozytorium do
`dependencyResolutionManagement.repositories` w swoim `settings.gradle.kts`:

```kotlin
maven { url = uri("/sciezka/do/wind-core/build/repo") }
```

oraz `implementation("io.github.prejas3.windconnect:wind-core:0.41.0")`.
W aplikacji konsumenta ogranicz ABI do `arm64-v8a` i `x86_64`.

```kotlin
val core = WindCore.open(context, "Mój telefon")
// Zbieraj core.state: StateFlow<ConnectionState> w cyklu życia właściciela.
// peerKey to 32 bajty klucza Ed25519, zweryfikowane innym kanałem.
try {
    core.connect("192.168.1.10:47820", peerKey)
} finally {
    core.shutdown()
}
```

Wywołania sieciowe są zawieszalne, a inicjalizacja Keystore działa na
`Dispatchers.IO`. `Connected` oznacza zakończone mTLS, wymianę Hello/HelloAck
i udany Ping/Pong. Zamykanie właściciela zatrzymuje obserwację połączenia,
zamyka endpoint i zwalnia uchwyty UniFFI; `shutdown` jest odporne na anulowanie.
Stan sesji nie jest trwałym zapisem parowań. Moduł udostępnia w tej karcie
łączenie wychodzące i diagnostykę sesji; moduły funkcji nie są jeszcze spięte
z aplikacją. Zmiany sieci budzą odkrywanie i próbę ostatnio wskazanego peera
zgodnie z opisem WN-16 poniżej.

## Rzeczywisty test Android → PC

1. Uruchom APK. Skopiuj wyświetlony klucz publiczny Androida (64 hex).
2. W katalogu głównym repozytorium uruchom:
   `cargo run --locked -p wind-ffi --example android_peer -- <klucz_Androida> 0.0.0.0:47820`.
   Program ufa tylko temu kluczowi, wypisuje `PC_PUBLIC_KEY` i kończy po
   rozłączeniu albo po 90 s. Tożsamość PC w tym narzędziu jest tymczasowa.
3. Na ekranie telefonu wpisz IP PC i port 47820 oraz sprawdzony
   `PC_PUBLIC_KEY`. Naciśnij „Połącz z podanym kluczem”. Ekran powinien
   pokazać „Połączono: WindConnect PC test; ping … ms”, a PC `CONNECTED`.
   Zapora PC musi dopuszczać testowy port UDP.
4. Naciśnij „Rozłącz”. Program PC zakończy działanie.

Na emulatorze Android adres hosta to `10.0.2.2:47820`; proces PC może wtedy
wiązać wyłącznie `127.0.0.1:47820`. Test instrumentacyjny ekranu wykonuje te
same działania:

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreConnectionTest#identityLoadsThroughUniFFIAndSurvivesActivityRecreation $runner
# Odczytaj androidPublicKey z wyniku i uruchom android_peer z tym kluczem.
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreConnectionTest#screenConnectsToPc -e pcAddress 10.0.2.2:47820 -e pcKey <PC_PUBLIC_KEY> $runner
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreApiTest $runner
```

Pierwszy test sprawdza bibliotekę natywną, Keystore i odtworzenie aktywności;
drugi prawdziwą sesję między Androidem a PC; trzeci błędy callbacku oraz
anulowanie korutyny połączenia. Bez parametrów PC test sieciowy jest pomijany.
Sprawdzenie S25 Ultra / StrongBox i sieci fizycznej wymaga telefonu; emulator
potwierdza x86_64 i Keystore programowy. arm64-v8a jest kompilowane do AAR/APK.

## Zmiany sieci (WN-16)

Właściciel `WindCore` rejestruje `ConnectivityManager.NetworkCallback` dla
sieci niewymagających ograniczonych uprawnień, również Wi-Fi bez Internetu.
`onLost` i zmiany `LinkProperties` trafiają do kanału
`CONFLATED`; callback nie wykonuje operacji sieciowych. Obsługa na IO wywołuje
zawieszalne `CoreClient.networkChanged()` w kontekście Tokio: zmienia gniazdo
QUIC, sygnalizuje nową generację i budzi bieżącą próbę połączenia.

`core.network` jest `StateFlow<NetworkSnapshot>` z generacją, aktualnymi
odciskami/adresami i ewentualnym błędem. Każda generacja usuwa stary widok
adresów i tworzy nowy daemon mDNS z aktualnymi interfejsami i portem endpointu,
bez czekania na okresowy timer. Brak sieci lub błąd mDNS pozostaje widoczny
w stanie; kolejna zmiana sieci ponawia odkrywanie.

Zmiana to różnica interfejsu albo adresów sieci względem tego, czego używa
odkrywanie (WN-103, `NetworkChangeFilter`). Android zaraz po rejestracji
callbacku powtarza sieci, które już ma (`onAvailable`, `onLinkPropertiesChanged`);
wcześniej restartowało to mDNS ok. 1 s po starcie, w trakcie sondowania, i
przeglądarka uruchomiona od razu nie widziała telefonu w 5 s w 8 z 10
przebiegów. Obserwator czyta stan sieci przed startem odkrywania, więc zmiana
między odczytem a powtórką nadal się liczy. `onAvailable` nie decyduje (od API
26 zawsze idzie po nim `onLinkPropertiesChanged`). Sprawdzenie:
`NetworkChangeFilterTest` (`./gradlew :wind-core:testDebugUnitTest`) i na
emulatorze lub telefonie z połączonym Wi-Fi (NsdManager nie robi mDNS na
komórkowej):

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.MdnsStartupTest $runner
```

`a_browserSeesPhoneRightAfterStart` 10 razy otwiera `WindCore` i od razu
przegląda `_windconnect._udp` systemowym NsdManager (`-e runs N`); wymaga
znalezienia w 5 s i generacji 0 po 3 s. `b_networkChangeStillRestartsDiscovery`
wyłącza i włącza Wi-Fi, wymaga nowej generacji po obu zmianach i czeka na
powrót Wi-Fi. LIMIT: po włączeniu Wi-Fi sieć dochodzi do siebie krokami
(restarty co ok. 0,6 s), więc telefon wraca w przeglądarce różnie (5,5 s – ponad
20 s), tak samo przed WN-103; test to raportuje (`seenAfterWifiOnMs`). Emulator
po kilku przełączeniach Wi-Fi bywa bez połączenia; test zgłasza to jako
warunek wstępny, nie regresję.

Jeśli poprzednia próba wskazanego ręcznie peera się nie powiodła albo sesja
już się zamknęła, zdarzenie sieci uruchamia nową próbę. Trwająca próba jest
budzona bez oczekiwania na jej limit 10 s. Aktywna sesja korzysta z migracji
QUIC. Ręczne rozłączenie, anulowanie próby i odmowa zaufania usuwają zamiar
automatycznego łączenia. Zaufanie nie jest nigdy tworzone z mDNS.
Adres ostatniego peera pozostaje ręcznie wybranym adresem; automatyczny wybór
nowego IP z listy urządzeń należy do integracji zarządzania urządzeniami.

Moduł deklaruje normalne uprawnienia `ACCESS_NETWORK_STATE` i
`CHANGE_WIFI_MULTICAST_STATE`. `MulticastLock` jest brany tylko wtedy, gdy
odkrywanie jest potrzebne (WN-83, `MulticastPolicy`):

- bez aktywnej sesji przez 30 s od startu, od zmiany sieci, od utraty sesji
  albo od `WindCore.refreshDiscovery()` (np. odświeżenie listy urządzeń);
- przez cały czas, gdy otwarty jest ekran wymagający odkrywania (okno
  parowania SAS, lista urządzeń): `WindCore.discoveryScreen(true)` …
  `discoveryScreen(false)`, wywołania liczone parami.

Aktywna sesja nie trzyma locka, także po zmianie sieci: wraca przez migrację
QUIC (WN-16, WN-84). `shutdown()` zwalnia lock razem z callbackiem, zadaniami
i daemonem. Reguły sprawdza `MulticastPolicyTest`
(`./gradlew :wind-core:testDebugUnitTest`), a na urządzeniu
`MulticastLockTest`, który czyta listę „Multicast Locks held” z
`dumpsys wifi` (wymaga `android_peer` jak w teście Android → PC; `-e
toggleWifi true` dodaje prawdziwą zmianę sieci):

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.MulticastLockTest -e pcAddress 10.0.2.2:47820 -e pcKey <PC_PUBLIC_KEY> -e toggleWifi true $runner
```

Inne komputery w czasie sesji (wiele PC, WN-90) telefon znajduje zapasowym
odkrywaniem broadcast (WN-15), bez locka. Podstawą jest pytanie telefonu
wysyłane co 2 s (wysyłanie nie wymaga locka) i odpowiedź unicast każdego PC.
Zapytania broadcast innych PC docierające do telefonu są dodatkiem, nie
podstawą: bez locka i przy wyłączonym ekranie układ Wi-Fi może je filtrować.
Po zmianie sieci drugi PC wraca na listę `WindCore.network` w ciągu 5 s.
W czasie sesji nie ma okna mDNS: przy wyłączonym drugim PC trzymałoby lock
30 s po każdej zmianie sieci. Na hoście sprawdza to
`query_is_answered_by_unicast_so_the_asker_needs_no_broadcast`
(`cargo test --locked -p wind-core --test broadcast`), na emulatorze
`MulticastLockTest#otherPcFoundDuringSessionWithoutLock` przez runner
(debug i androidTest APK zbudowane,
`cargo build --locked -p wind-ffi --example android_peer`):

```powershell
node apps/android/scripts/check-multi-pc.cjs
```

Runner przekierowuje UDP emulatora (`emu redir`, domyślnie 47823 → 47821;
`REDIR_PORT`, `PC_PORT` zmieniają porty hosta), uruchamia `android_peer` jako
pierwszy PC i co 2 s wysyła pakiet odkrywania drugiego PC. LIMIT: przez NAT
emulatora pakiet przychodzi bez pytania, więc test sprawdza sesję, lock i
pamięć odkrywania po zmianie sieci. Pytanie i odpowiedź unicast sprawdza test
hosta, a filtrowanie broadcast przez prawdziwy układ Wi-Fi bez locka wymaga
telefonu.

Wersja aplikacji to 0.41.0, a Android `versionCode` to 50. Numery wydań rosną
niezależnie od kolejności kart, aby aktualizacja APK i MSI nie cofała wersji.

## Pierwsze uruchomienie (WC-34)

CCS/1
FLOW:brak sparowanego komputera -> karta „Witaj w WindConnect” na górze ekranu głównego (co jest potrzebne, przycisk „Zeskanuj kod z komputera” jak w kroku 2 na PC, zdanie o aparacie przed pytaniem systemu) -> po stuknięciu od razu pytanie o aparat -> skaner -> „Gotowe”.
RULE:„Gotowe” zadaje jedno pytanie: „Utrzymuj połączenie w tle” (ConnectionService + POST_NOTIFICATIONS na API 33+) albo „Nie teraz”. Parowanie samo już nie włącza usługi. Odmowa powiadomień: usługa działa, ale bez widocznego powiadomienia nie ma stanu komputera ani akcji „Wyślij schowek”; ekran mówi to jednym zdaniem i prowadzi do ustawień powiadomień. Ustawienia -> „Połączenie w tle” zmienia decyzję później.
A11Y:nagłówki kart „Witaj” i „Gotowe” są oznaczone dla TalkBack.

Test: `OnboardingTest` (karta „Witaj”, decyzja „Gotowe”, zdania skutków);
`PairingTest` z `-e background true` wybiera „Utrzymuj połączenie w tle”.

## Sparowane komputery (WC-28)

CCS/1
FLOW:Ekran główny / Ustawienia -> „Sparowane komputery”: nazwa lokalna, „Połącz”, „Usuń” (pytanie w karcie, bez okna dialogowego).
RULE:usunięcie z połączonym komputerem wysyła Goodbye UNPAIRED od razu; bez połączenia klucz jest odwołany natychmiast, a powiadomienie idzie, gdy odkrywanie pokaże ten komputer (raz na generację sieci, pełne uzgadnianie bez 0-RTT). Komputer, który usunął telefon, zamyka sesję kodem UNPAIRED: telefon usuwa klucz rozmówcy z TLS i nie wraca.
RULE:BUSY (komputer ma inny telefon) i odmowa TLS („Komputer odrzuca połączenie — sparuj ponownie”) wstrzymują automatyczne ponawiania usługi tła do zmiany sieci albo „Połącz”.
LIMIT:po BUSY telefon nie wraca sam, gdy komputer się zwolni. Powiadomienie o usunięciu należy się 30 dni.

Testy: `DevicesScreenTest` (lista, zmiana nazwy, pytanie o usunięcie,
komunikaty końca sesji), `cargo test --locked -p wind-ffi --test bridge`
(`ffi_paired_list_rename_unpair_both_ways_and_busy`).

## Ekran główny (WN-31)

CCS/1
FLOW: wspólny WindCore -> PairingViewModel -> rzeczywista nazwa sparowanego PC, stan sesji, ostatni RTT i trasa LAN/relay. Rozłączenie usuwa RTT; nazwa pozostaje. Bez dodatkowego pollingu i blokad CPU.
DEC: cztery kafelki z makiety są nieaktywne do podłączenia ich kanałów; historia transferów pokazuje jawny stan pusty, bez demonstracyjnych plików. Ustawienia pod kołem zębatym zachowują parowanie i opcje usługi w tle.
PERF: release używa R8 i shrinkera zasobów. Reguły konsumenckie zachowują refleksję UniFFI/JNA; wyciszone są wyłącznie cztery klasy AWT, których Android nie posiada. `ReportDrawnWhen` czeka na lokalną tożsamość i magazyn, nie na dostępność zdalnego PC.

Pomiar: zainstaluj podpisany APK **release**, ustaw `ANDROID_HOME`, `ANDROID_SERIAL`,
uruchom `cw-progress --title "Android: pomiar startu release" -- node apps/windows/scripts/check-startup-android.cjs` z katalogu głównego.
Skrypt zachowuje dane aplikacji, kończy jej proces po pomiarze i zapisuje 10 zimnych
TTID/TTFD oraz `WaitTime` 10 powrotów z Home do `%TEMP%/windconnect-startup-*`.
`WaitTime` powrotu jest czasem komendy systemowej, nie pomiarem TTID.
Domyślne `STARTUP_COMPILATION=verify` wyłącza AOT; osobne `speed` wymusza AOT
i musi być raportowane oddzielnie. Próg <400 ms należy sprawdzić na docelowym
telefonie; wynik emulatora nie jest dowodem wydajności S25 Ultra.

Test rzeczywistej sesji i zrzuty obu motywów: po zbudowaniu debug/androidTest oraz
`cargo build --locked -p windconnect-windows --example background_peer`, ustaw
`HOME_SCREEN_PROOF=1` i uruchom `apps/windows/scripts/check-background-android.cjs`.
Ten test wymaga jednorazowego emulatora i czyści wyłącznie jego dane WindConnect;
sprawdza prawdziwy QR/TLS, odtworzenie Activity, ustawienia i stan kafelków.

## Połączenie w tle (WN-29)

CCS/1
FLOW: potwierdzone parowanie -> włączenie usługi `connectedDevice` -> stałe powiadomienie z nazwą PC, rzeczywistym stanem i akcjami „Otwórz urządzenie”, „Połącz ponownie” (offline), „Zatrzymaj”. Schowek/pliki są osobnymi modułami; powiadomienie nie udaje ich wysyłania przed ich wdrożeniem.
RULE: Activity i usługa dzierżawią jeden WindCore w Application. Zamknięcie/obrót ekranu zwalnia tylko dzierżawę ekranu; ostatnia dzierżawa zamyka natywne uchwyty, discovery i sesję. Wyłączenie tła rozłącza sesję i zapisuje wyłączenie przed zatrzymaniem usługi.
FLOW: START_STICKY odtwarza usługę po odzyskaniu procesu przez system. BOOT_COMPLETED po odblokowaniu i MY_PACKAGE_REPLACED wznawiają tylko wcześniej włączoną usługę; bez uruchamiania przed odblokowaniem tożsamości. Force-stop systemowy wymaga ponownego otwarcia aplikacji. Brak uprawnień do startu w tle daje komunikat przy kolejnym otwarciu.
SEC: reconnect ładuje tylko trwałe sparowane klucze; discovery dostarcza adres, nie zaufanie. Ponawianie z backoff 3–60 s, bez domyślnego relay, alarmów lub nowych kanałów sieciowych. Obecnie wybierany ostatni zapisany komputer; wybór wielu urządzeń rozwija WN-31.
PERF: brak WAKE_LOCK i stałego podtrzymywania CPU. Keepalive QUIC co15 s po stronie PC i Androida już istnieje w transporcie. FGS nie znosi restrykcji sieci Doze; wyjątek baterii jest decyzją użytkownika. Offline bez blokady CPU. Dobowy pobór energii mierzy osobna karta WN-30; emulator nie dowodzi budżetu <1%/dobę.

### Telefon i nocny test

1. Zainstaluj `app/build/outputs/apk/debug/app-debug.apk` i sparuj PC. Zezwól na powiadomienia, aby widzieć urządzenie i akcje; po odmowie sekcja „Połączenie w tle” prowadzi do ustawień powiadomień.
2. W sekcji „Połączenie w tle” otwórz optymalizację baterii i ustaw WindConnect jako nieoptymalizowany. Sam FGS nie znosi ograniczeń sieci Doze. Aplikacja nie zmienia tej zgody samoczynnie.
3. Samsung: Ustawienia → Bateria → Limity użycia w tle → aplikacje nigdy nieusypiane; dodaj WindConnect i usuń z listy głęboko usypianych. Nazwy mogą się różnić w wydaniach One UI. Po aktualizacji sprawdź ustawienie ponownie.
4. Zanotuj model, Android/One UI, wersję APK, stan baterii i ustawienia oszczędzania. PC ma pozostać uruchomiony w tej samej sieci. Zamknij ekran aplikacji, wyłącz ekran telefonu i odłącz ładowarkę na8 h. Rano sprawdź „Połączono”, działającą sesję i zmianę baterii. Powtórz restart telefonu: po odblokowaniu usługa ma wrócić bez otwierania aplikacji. „Zatrzymaj” ma wyłączyć ją także po kolejnym restarcie.

CCS/1 LIMIT: emulator API35 z wymuszonym Doze i jawnym wyjątkiem baterii sprawdza cykl życia, nie zastępuje nocy na S25 Ultra/najnowszym One UI. Nie deklarować spełnienia nocnego kryterium bez tego pomiaru.

Źródła reguł platformy: [typ connectedDevice](https://developer.android.com/develop/background-work/services/fgs/service-types), [Doze i wyjątki baterii](https://developer.android.com/training/monitoring-device-state/doze-standby), [start FGS w tle](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start), [zarządzanie aplikacjami Samsunga](https://developer.samsung.com/mobile/app-management.html).

### Powtarzalne sprawdzenia

```powershell
cargo build --locked -p windconnect-windows --example background_peer
# ANDROID_HOME i JAVA_HOME jak w testach QR WN-25; macierz QR koduje ten sam Rust qrcode/ECC M co aplikacja PC
node apps/windows/scripts/check-background-android.cjs
```

CCS/1 STATE: fixture wymaga zweryfikowanego emulatora i czyści wyłącznie jego testową aplikację. PC peer ma efemeryczną tożsamość i limit2100 s; zainstalowana aplikacja PC nie jest zatrzymywana. Obraz QR dekodowany na Androidzie, pełne parowanie i TLS bez obejścia trust. Publiczny pakiet discovery przekazywany przez emulator NAT. Test zamyka Activity, wymusza60 s Doze (SOAK_SECONDS=1800:30 min; BATTERY_EXEMPT=0: bez wyjątku baterii), sprawdza brak blokad CPU, próbuje akcję Zatrzymaj, uruchamia ponownie usługę i restartuje emulator; wymaga nowej prawdziwej sesji po BOOT_COMPLETED. Przywraca ustawienia zasilania i usuwa surowy jednorazowy link. TEMP/windconnect-wn29-*/ zawiera wynik i logi. ConnectionOwnerTest osobno sprawdza współdzielenie, ostatnie zwolnienie i trwałą tożsamość.

Test na emulatorze/telefonie: `CoreApiTest#platformNetworkCallbackWakesDiscoveryWithoutPolling`
potwierdza rzeczywisty callback systemu i generację z natywnego odkrywania.
Opt-in `CoreApiTest#wifiReturnWakesNativeDiscovery` z argumentem instrumentacji
`-e toggleWifi true` wyłącza i przywraca Wi-Fi na wskazanym urządzeniu testowym,
a następnie potwierdza nową generację odkrywania; domyślnie ten test jest pomijany.
Test ręczny: wprowadź zweryfikowany klucz i adres PC, spróbuj połączenia bez
zasięgu PC, a potem wejdź do Wi-Fi tego PC. Próba powinna ruszyć po zdarzeniu
sieci. Sprawdzenie przejścia między fizycznymi sieciami na S25 Ultra pozostaje
testem sprzętowym.

## Przekaźnik poza domem (WN-60)

`WindCore.setRelay(url)` włącza przekaźnik E2E (PROTOCOL.md §13) z adresem
`wss://…`, a `setRelay(null)` go wyłącza. `relayUrl: StateFlow<String?>`
pokazuje stan. Ustawienie jest zapisane w SharedPreferences
(`windconnect.core`) i wraca po restarcie. Domyślnie przekaźnik jest
wyłączony (D3); przełącznik w ustawieniach to karta WN-92. Z włączonym
przekaźnikiem `connect` próbuje adresu PC przez 2 s, potem łączy przez
przekaźnik. `ConnectionState.Connected.route` to `"lan"` albo `"relay"`. Po
zmianie sieci w czasie sesji przez przekaźnik rdzeń próbuje adresu w LAN, a
jeśli odpowiada, przechodzi na LAN.

Test na emulatorze (Wi-Fi wyłączone przez test, tylko sieć komórkowa): przy
`npm run dev` w `relay/` i `windctl serve --relay ws://127.0.0.1:8787 --trust
<klucz Androida> --bind 127.0.0.1:47821`:

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.RelayRouteTest -e pcKey <PC_PUBLIC_KEY> -e relayUrl ws://10.0.2.2:8787 -e awayAddress 10.0.2.2:47999 $runner
```

`awayAddress` to adres, który nie odpowiada (dom poza zasięgiem). Wynik z
2026-11-10, emulator API 35: `network=cellular only`, `route=relay`,
połączenie po 2057 ms (w tym 2 s próby LAN), ping 6 ms. Prawdziwa sieć
komórkowa wymaga wdrożonego przekaźnika (relay/README.md) i telefonu.

## Bateria (WC-30)

Budżet planu: < 1% baterii na dobę. Pomiary składowe:

| Data | Urządzenie | Pomiar | Wynik |
| --- | --- | --- | --- |
| 2026-11-10 | emulator API 35 (x86_64), `MulticastLockTest` z `android_peer` | `MulticastLock` (WN-83) | brany od startu; zwolniony 243 ms po zestawieniu sesji; przy aktywnej sesji zdarzenie sieci i odświeżenie go nie biorą; po utracie sesji brany po 237 ms i zwolniony sam po 30 253 ms; po wyłączeniu Wi-Fi bez sesji brany po 958 ms; po `shutdown()` zwolniony |
| 2026-11-11 | emulator API 35 (x86_64), `check-multi-pc.cjs` (WN-90), 3 przebiegi | `MulticastLock` przy sesji z PC1 i drugim PC w sieci | lock nie brany przez całą sesję (sprawdzane co 200 ms); drugi PC na liście po 912–920 ms; po wyłączeniu Wi-Fi po 1 818–1 870 ms, po włączeniu po 1 850–1 872 ms (pakiet co 2 s); sesja z PC1 utrzymana |

Przed WN-83 lock był trzymany przez całe życie `WindCore`. Teraz przy stałym
połączeniu z PC jest trzymany tylko przez czas zestawiania sesji, a bez
połączenia przez okna po 30 s. Szukanie innych PC w czasie sesji (WN-90) nie
dokłada czasu z lockiem: idzie przez broadcast. Zużycie w mAh wymaga pomiaru na telefonie
(S25 Ultra, Battery Historian) i należy do WC-30.

## Motyw i komponenty (WN-22)

`ui/Theme.kt` definiuje DM Sans i paletę z zaakceptowanego WN-20. Domyślnie
Android 12+ pobiera Material You z tapety (`dynamicLightColorScheme` /
`dynamicDarkColorScheme`); Android 10/11 zachowuje paletę morską/grafitową.
Motyw System reaguje na konfigurację OS, a ręczny wybór jasny/ciemny i opcja
koloru dynamicznego są zapamiętywane w prywatnych preferencjach aplikacji.
Zmiana ustawień nie wymaga restartu. Treść respektuje safe drawing/IME insets,
skalowanie czcionek i semantykę dostępności; ikony pasków systemu dopasowują
się do motywu. Ikony i geometria kart/akcji pochodzą z makiety WN-20.

`ui/Components.kt`: własne przyciski, karty, kafelki akcji, pola oparte na
BasicTextField, przełącznik Canvas i wektorowe ikony. Brak domyślnych Button,
Switch i OutlinedTextField. Role, opis pola, błąd i postęp są w semantyce Compose;
elementy interaktywne mają co najmniej 48 dp. Ekran ręcznego połączenia nadal
korzysta z rzeczywistego rdzenia i przypiętego klucza; jego tagi testowe zachowano.

W debug naciśnij **Podgląd** w nagłówku. Ekran demonstruje oba motywy, paletę
systemową/marki, kartę SZYMON-PC, cztery akcje, transfer, switch i walidację pola.
To dane przykładowe: kliknięcie nie łączy, nie blokuje komputera i nie wysyła
plików. **Połączenie** przywraca rzeczywisty ekran. Release nie pokazuje podglądu
(`BuildConfig.DEBUG`); testy i stan przykładowego urządzenia nie trafiają do UX
wydania. Nie dodano powiadomień/QS/widgetu — to odrębne funkcje z makiet.

DM Sans jest tym samym niezmienionym fontem co w WN-21; kopia OFL i NOTICE
są w `app/src/main/assets`, font w `res/font`. Nie dodano zależności ani fontów
z sieci. Ikony i kod własny: MIT.

Testy: `./gradlew :app:connectedDebugAndroidTest` na podłączonym urządzeniu.
`ComponentsTest` sprawdza motywy, dynamiczny kolor, akcję, switch, walidację,
odtworzenie Activity i powrót do prawdziwego ekranu; zapisuje zrzuty obu palet
w cache debug aplikacji (`wn22-Light.png`, `wn22-Dark.png`). CI kompiluje APK
instrumentacji; urządzeniowe uruchomienie jest lokalne. Docelowo sprawdź na S25
Ultra / najnowszym One UI, także duży font i nawigację gestami. Emulator API 35
nie zastępuje kontroli układu i koloru tapety na Samsungu.

## Parowanie bez aparatu (WN-27)

Ekran parowania ma „Paruj bez aparatu”: lista komputerów z otwartym oknem
kodu (mDNS TXT `pair`, PROTOCOL.md §12.4; na czas ekranu włączony multicast,
`discoveryScreen`). Wybór łączy się z portem parowania (`CoreClient.startSas`),
oba ekrany pokazują ten sam kod (`SasPairing.code`), a para powstaje tylko
po „Kody są takie same” na telefonie i na komputerze (`SasPairing.decide`).
Lista niczego nie potwierdza; adres to wskazówka, uwierzytelnia porównanie
kodów (§12.6). Test FFI: `cargo test -p wind-ffi --test pairing`
(`sas_pairs_only_when_both_accept_the_same_code`).

## Wspólny schowek (WN-35)

CCS/1
FLOW:systemowy ClipboardManager -> UniFFI -> clipboard/1 na sparowanej sesji -> schowek PC; Applied wraca dopiero po udanym zapisie OS. Odbiór PC -> telefon działa także przy zamkniętej Activity, jeśli działa usługa połączenia.
RULE:automat włączony domyślnie. Przełącznik „Synchronizacja automatyczna” na Home/Ustawieniach zapamiętuje decyzję; wyłącza automatyczny odczyt oraz odbiór, ale nie zatrzymuje połączenia. Ręczne „Wyślij schowek” pozostaje dodatkiem. Samo ponowne otwarcie/fokus nie wysyła już wysłanej/odebranej treści.
LIMIT:Android 10+ czyta schowek tylko przy fokusie aplikacji. Wysyłanie telefonu działa automatycznie przy fokusie WindConnect i po powrocie do aplikacji, jeśli treść się zmieniła. W innych aplikacjach użyj „Wyślij schowek” w stałym powiadomieniu: otworzy krótko osobny ekran, poczeka na fokus/odblokowanie i po dostarczeniu wróci do poprzedniej aplikacji. Bez READ_LOGS, Accessibility ani zmiany klawiatury. Czas otwarcia ekranu i działania człowieka nie jest budżetem transportu <150 ms.
SEC:tylko plain text/linki do256 KiB bez NUL; bez otwierania URL lub odczytu URI providerów. EXTRA_IS_SENSITIVE blokuje także wysłanie ręczne. Odebrany tekst zawsze ma sensitive=true (ukryty podgląd Androida13+ i brak echa). Status nie zawiera treści; brak historii/logowania/utrwalania payloadu.
DEC:chronologii zegarów urządzeń nie porównujemy. Licznik Lamporta+id16B rozstrzyga równoczesne zmiany, hash w pamięci zapobiega echo, reconnect nie odtwarza starej zawartości. Wire i limity w PROTOCOL.md §9.3.
RULE:automat wysyła tylko schowek skopiowany po przyjęciu bieżącej sesji (ClipDescription.timestamp względem lokalnego System.currentTimeMillis). Stary tekst po restarcie procesu lub reconnect nie zastąpi schowka PC przy fokusie; ręczne wysłanie może go wysłać. Timestamp0/równy granicy jest pomijany. Po cofnięciu zegara użyj ręcznej akcji. Bez utrwalania payloadu.

Test telefonu: sparuj Windows i S25 Ultra, włącz usługę/powiadomienia. Skopiuj
zwykły tekst Unicode lub link na PC i wklej na telefonie w innej aplikacji,
gdy WindConnect jest w tle. Skopiuj nowy tekst na telefonie, stuknij akcję
powiadomienia i wklej na PC. Przy otwartym WindConnect sprawdź automat.
Tekst oznaczony przez menedżer haseł jako wrażliwy ma zostać pominięty.
Sprawdź wyłączenie/przywrócenie synchronizacji, utratę sieci i brak starej
kolejki po reconnect. Potwierdź na aktualnym One UI; emulator nie jest S25.

Automatyczny test OS: z root `cargo build --locked -p windconnect-windows
--example clipboard_peer`; zbuduj Android debug/androidTest, ustaw
`JAVA_HOME`, `ANDROID_HOME`, `CLIPBOARD_PROOF=1` i uruchom przez `cw-progress`
`node apps/windows/scripts/check-background-android.cjs`.
Test wymaga jednorazowego emulatora (czyści tylko jego dane WindConnect).
Przed uruchomieniem emulatora wyłącz jego „Enable clipboard sharing”
(Extended Controls → Settings). Współdzielenie hosta jest drugim kanałem
schowka, może nadpisywać próbki i gubić sensitive. Po zakończeniu przywróć
swoje ustawienie. Test sprawdza odczytem kontrolnym każdą kopię przy fokusie;
nie ponawia ani nie ukrywa błędnego zapisu próbki.
Windows używa osobnego pulpitu w nieinteraktywnej stacji i odmawia testu,
jeśli jej schowek nie jest pusty. Nie używa schowka interaktywnego użytkownika.
100 próbek/kierunek mierzy od kopiowania w OS do Applied po zapisie drugiego
OS (konserwatywna górna granica czasu kopiuj→wklej), osobno sprawdza odbiór
Androida bez Activity, sensitive, akcję powiadomienia i brak echa po fokusie.
Surowe próbki i zrzuty Light/Dark: `%TEMP%/windconnect-wn35-*`.

## Hasła w schowku (WN-37)

CCS/1
SEC:klip z `ClipDescription.EXTRA_IS_SENSITIVE` (`android.content.extra.IS_SENSITIVE`) nie jest wysyłany na PC, także ręcznie (test: `ClipboardProof`, „WN35-PRIVATE-ANDROID”). Telefon czyta schowek tylko z fokusem albo ręcznie, nigdy w tle. W drugą stronę hasła z menedżerów na PC nie dochodzą (test OS z 4 znacznikami Windows, `apps/windows/README.md`, WN-37).
LIMIT:menedżery ustawiają IS_SENSITIVE na Androidzie 13+; starsze lub źle napisane go nie ustawiają, a hasło skopiowane z nich wygląda jak zwykły tekst. Nie rozpoznajemy haseł po treści. Historia schowka jest tylko na Windows (Android: osobna karta).
## Wspólny schowek: obrazy (WN-36)

CCS/1
FLOW:PC -> telefon: ImageSet (miniatura, rozmiar) -> setPrimaryClip z `content://<pakiet>.clipimage/<id>` (image/png) -> Applied; pełny PNG dochodzi w tle. `ClipImageProvider` (moduł wind-core, nieeksportowany, grantUriPermissions) daje podglądom (EXTRA_SIZE, `loadThumbnail`) miniaturę od razu, a wklejeniu strumień pełnego PNG w miarę napływu.
FLOW:telefon -> PC: przy fokusie klip z URI image/* innej aplikacji (uprawnienie klipu) -> PNG bez zmian albo ponownie zakodowany -> miniatura -> clipboard/2. Własny URI dostawcy nigdy nie wraca do PC.
RULE:limit „Maks. rozmiar obrazu” 5 / 16 / 50 MB w Ustawieniach (domyślnie 16 MB), dla wysyłania i odbioru.
SEC:odebrany obraz nie ma EXTRA_IS_SENSITIVE (inaczej niż tekst), żeby podgląd z miniatury działał; pochodzi z własnego PC, który treści prywatnych nie wysyła. Obraz z IS_SENSITIVE nie jest wysyłany. Tylko pamięć, tylko ostatni obraz; po restarcie procesu stary URI daje FileNotFoundException (wklejenie się nie udaje, bez awarii).
LIMIT:wklejenie przed końcem przesyłu czeka do 10 s bez nowych bajtów; nowszy schowek albo przerwanie kończą je błędem, nigdy częściowym obrazem.

Testy: `ClipImageProviderTest` (miniatura dla podglądu, pełny PNG w miarę
napływu, błąd przy nowszym obrazie, nieznany obraz = FileNotFoundException).
Pomiar „zrzut z PC wklejalny na telefonie < 500 ms” jest częścią testu OS
WN-35 wyżej: PC kopiuje syntetyczny zrzut jako sam CF_DIB (kodowanie PNG po
stronie PC) 10× 1080p i 3× 4K; raport podaje czas do Applied (wklejalny) i do
ImageDone (cały PNG na telefonie), a telefon czyta pełny PNG przez URI i
podgląd. Czas zależy od profilu: uruchom `clipboard_peer` zbudowany
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build --release
--example clipboard_peer` i `PEER_PROFILE=release` (debugowy enkoder PNG jest
~10× wolniejszy niż wysyłana aplikacja).

## Bateria telefonu na PC (WN-49)

CCS/1
FLOW:sesja przyjęta przez `WindCore` -> `BatteryReporter` rejestruje odbiornik `ACTION_BATTERY_CHANGED` (lepki intent daje stan od razu) -> `CoreConnection.sendBattery` -> PC (`battery/1`, PROTOCOL.md §9.4). Koniec sesji wyrejestrowuje odbiornik.
RULE:jedna normalizacja `BatteryReading.from(intent)`: procent jak w pasku stanu (`level * 100 / scale`, w dół), ładowanie = podłączony, CHARGING albo FULL; nieznany poziom = brak odczytu (nic nie idzie). Z niej skorzysta też widżet (WN-33).
RULE:bez pollingu i budzenia: odbiornik tylko w czasie sesji; równe odczyty (intent przychodzi też przy zmianie napięcia i temperatury) są pomijane, a rdzeń wysyła tylko zmiany pary (poziom, ładowanie).
LIMIT:PC widzi zmiany tylko przy żywej sesji (połączenie w tle włączone). W Doze albo bez połączenia w tle stan na PC nie jest aktualizowany, więc alert 15% może przyjść późno albo wcale. Nie dotyka ConnectionService ani ekranu głównego.

## Znajdź telefon (WN-50)

CCS/1
FLOW:komputer prosi (`find/1`, PROTOCOL.md §9.5) -> `PhoneRinger`: głośność alarmu na max (poprzednia zapisana), wygenerowany ton (880/1320 Hz) przez AudioTrack z `USAGE_ALARM`, wibracje, powiadomienie -> stan 0 do PC dopiero, gdy głowica odtwarzania ruszyła.
RULE:dźwięk omija tryb cichy i wibracje (strumień alarmu); ton jest generowany (bez pliku, bez opóźnienia przygotowania, niezależny od dzwonka użytkownika). Bez WAKE_LOCK: przy wygaszonym ekranie odtwarzanie podtrzymuje audioserver.
RULE:zatrzymanie: powiadomienie na własnym kanale „Znajdź telefon” (IMPORTANCE_HIGH, bez własnego dźwięku, trwałe w czasie dzwonienia, znika po stanie końcowym, akcja „Zatrzymaj” i stuknięcie), pasek „Zatrzymaj dzwonienie” na każdym ekranie aplikacji (działa bez zgody na powiadomienia) i przycisk głośności przy otwartej aplikacji. Limit 60 s pilnuje rdzeń; koniec sesji nie zatrzymuje dzwonienia.
RULE:głośność alarmu wraca po każdym dzwonieniu; gdy proces zginie w trakcie, następny start ją przywraca (chyba że użytkownik ją zmienił). Ustawienie „Pozwól komputerowi dzwonić” (domyślnie włączone) wycofuje `find/1` także w trwającej sesji.
LIMIT:DND blokujące alarmy wycisza dźwięk (ominięcie wymaga dostępu do zasad DND, o który aplikacja nie prosi). Bez połączenia w tle albo w Doze prośba dochodzi późno albo wcale. Przycisk głośności przy wygaszonym ekranie lub poza aplikacją nie zatrzymuje dzwonienia (to wymagałoby sesji multimediów). Nie dotyka ConnectionService ani ekranu głównego; pasek jest nakładką w MainActivity.

## Odtwarzacz telefonu na PC (WN-46)

CCS/1
FLOW:sesja przyjęta + dostęp + ustawienie -> `PhoneMedia`: `getActiveSessions` i `addOnActiveSessionsChangedListener` przez `MediaListenerService`, `MediaController.Callback` na wybranej sesji -> `CoreConnection.sendMedia` (tylko zmiany, okładka raz) -> PC. Polecenie z PC (`media-control/1`, sprawdzone w rdzeniu względem ostatnich `actions`) -> `transportControls`.
RULE:wybór sesji: pierwsza w stanie PLAYING, inaczej pierwsza w kolejności systemu; zmiana przy `onActiveSessionsChanged` i zmianie stanu. Okładka ≤ 256 px JPEG ≤ 96 KiB, liczona raz na bitmapę, na wątku multimediów. Teksty przycinane do 256 B (aplikacja do 64 B) na granicy znaku.
RULE:zgoda: Android nazywa to „dostępem do powiadomień”. `MediaListenerService` nie czyta żadnych powiadomień; karta w Ustawieniach mówi to wprost. Funkcja czytania powiadomień (WN-42) NIE MOŻE po cichu korzystać z tej zgody: dostaje własny przełącznik (domyślnie wyłączony) i własne wyjaśnienie (PROTOCOL.md §9.6, zasada 6).
RULE:`media-control/1` jest ogłaszane tylko przy przyznanym dostępie i włączonym „Pokaż multimedia na komputerze”, także w trakcie sesji (`Router::start_disabled` / `set_accepted`). Wyłączone: telefon wysyła `Gone` i przestaje śledzić odtwarzacze, a usługa nasłuchu sama się odłącza (`requestUnbind`; `requestRebind` po włączeniu).
PERF:bez sesji nic nie działa w `PhoneMedia` (żadnych callbacków ani skalowania okładek). Związana usługa nasłuchu utrzymuje jednak proces przy życiu także bez sesji: nowy czynnik rezydencji dla pomiaru baterii (WN-30).
LIMIT:Spotify nie jest zainstalowany na emulatorze; test używa zwykłej `MediaSession` (Spotify wystawia taką samą). Dostęp do powiadomień przyznaje się tylko w ustawieniach systemu.
