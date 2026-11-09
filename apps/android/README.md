# Android

Szkielet aplikacji w Kotlinie i Jetpack Compose: jeden ekran i robocza ikona.
Wymagany Android 10 lub nowszy (minSdk 29). Integracja z `wind-core` i komunikacja
z klientem Windows powstaną w kolejnych zadaniach.

## Budowanie

Wymagania: JDK 17, Android SDK Platform 35, Build Tools 35.0.0.
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

Powinien pojawić się ekran z napisami „WindConnect” i „Windows ⇄ Android”.
Ikona robocza jest własnym zasobem projektu na licencji MIT. Aplikacja nie wymaga
uprawnień systemowych. Instalację na telefonie oraz pobranie APK z CI trzeba
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
Połączenie z `wind-core` powstanie w WC-17.

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
