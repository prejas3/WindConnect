# Android

Aplikacja Kotlin / Jetpack Compose z ekranem testowym połączenia z PC.
Moduł `:wind-core` zawiera wiązania UniFFI 0.32.2, wrapper korutynowy ze
`StateFlow` i biblioteki Rust dla arm64-v8a oraz x86_64. Wymagany Android 10+
(minSdk 29). Ekran wymaga ręcznego zweryfikowania i podania klucza PC;
pełny protokół parowania powstaje w osobnej karcie.

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

Powinien pojawić się ekran „Test połączenia z PC”, odcisk i klucz publiczny Androida.
Ikona robocza jest własnym zasobem projektu na licencji MIT. Aplikacja nie wymaga
uprawnień przyznawanych w runtime; używa uprawnienia INTERNET. Instalację na telefonie oraz pobranie APK z CI trzeba
potwierdzić po opublikowaniu repozytorium.

Gradle Wrapper jest narzędziem budowania na licencji Apache-2.0;
tekst licencji znajduje się w `gradle/wrapper/LICENSE`.


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
`wind-core/build/repo/`. Współrzędne: `io.github.prejas3.windconnect:wind-core:0.18.1`.
Publikacja zapisuje pliki lokalnie; nie wysyła ich do zewnętrznego repozytorium.
Konsument musi korzystać z POM/metadanych (JNA i korutyny są zależnościami
przechodnimi); sama kopia pliku AAR nie dostarczy JNA.

W tym repozytorium aplikacja używa `implementation(project(":wind-core"))`.
Konsument spoza repozytorium dodaje lokalne repozytorium do
`dependencyResolutionManagement.repositories` w swoim `settings.gradle.kts`:

```kotlin
maven { url = uri("/sciezka/do/wind-core/build/repo") }
```

oraz `implementation("io.github.prejas3.windconnect:wind-core:0.18.1")`.
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
łączenie wychodzące i diagnostykę sesji; moduły funkcji oraz automatyczne
odkrywanie i ponowne łączenie nie są jeszcze spięte z aplikacją.

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
