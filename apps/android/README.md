# Android

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
`wind-core/build/repo/`. Współrzędne: `io.github.prejas3.windconnect:wind-core:0.30.0`.
Publikacja zapisuje pliki lokalnie; nie wysyła ich do zewnętrznego repozytorium.
Konsument musi korzystać z POM/metadanych (JNA i korutyny są zależnościami
przechodnimi); sama kopia pliku AAR nie dostarczy JNA.

W tym repozytorium aplikacja używa `implementation(project(":wind-core"))`.
Konsument spoza repozytorium dodaje lokalne repozytorium do
`dependencyResolutionManagement.repositories` w swoim `settings.gradle.kts`:

```kotlin
maven { url = uri("/sciezka/do/wind-core/build/repo") }
```

oraz `implementation("io.github.prejas3.windconnect:wind-core:0.30.0")`.
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
`onAvailable`, `onLost` i zmiany `LinkProperties` trafiają do kanału
`CONFLATED`; callback nie wykonuje operacji sieciowych. Obsługa na IO wywołuje
zawieszalne `CoreClient.networkChanged()` w kontekście Tokio: zmienia gniazdo
QUIC, sygnalizuje nową generację i budzi bieżącą próbę połączenia.

`core.network` jest `StateFlow<NetworkSnapshot>` z generacją, aktualnymi
odciskami/adresami i ewentualnym błędem. Każda generacja usuwa stary widok
adresów i tworzy nowy daemon mDNS z aktualnymi interfejsami i portem endpointu,
bez czekania na okresowy timer. Brak sieci lub błąd mDNS pozostaje widoczny
w stanie; kolejna zmiana sieci ponawia odkrywanie.

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
Wersja aplikacji to 0.30.0, a Android `versionCode` to 39. Numery wydań rosną
niezależnie od kolejności kart, aby aktualizacja APK i MSI nie cofała wersji.

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

Przed WN-83 lock był trzymany przez całe życie `WindCore`. Teraz przy stałym
połączeniu z PC jest trzymany tylko przez czas zestawiania sesji, a bez
połączenia przez okna po 30 s. Zużycie w mAh wymaga pomiaru na telefonie
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
