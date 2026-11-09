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

