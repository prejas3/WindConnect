# Współpraca

Ogranicz zmianę do zakresu zadania. Opisz jej cel i sposób sprawdzenia.
Nowy kod i dokumentacja projektu są udostępniane na licencji MIT.

## Rust

Używaj stabilnego toolchaina Rust oraz formatowania rustfmt.
Przed przekazaniem zmiany do recenzji uruchom z katalogu głównego:

```sh
cargo fmt --all -- --check
cargo build --locked
```

Gdy dodajesz zachowanie, dodaj odpowiednie testy i uruchom `cargo test --locked`.
Aktualizuj `Cargo.lock` przy zmianach zależności i dołącz go do commita.

## Aplikacje

Instrukcje klienta Windows: [apps/windows/README.md](apps/windows/README.md).
Instrukcje klienta Android: [apps/android/README.md](apps/android/README.md).

## Wersje i tagi (WN-114)

Jedna wersja `X.Y.Z` dla całego repozytorium. Tag zawsze nazywa wersję,
nigdy kartę: numer karty jest w treści commita, nie w tagu.

- Karta z nową funkcją lub zmianą zachowania kończy się jednym commitem
  na `master` o tytule `WN-N (vX.Y.Z): opis`. Ten commit podnosi wersję
  (zwykle `Y`; poprawka bez nowej funkcji może podnieść `Z`) i dostaje
  lokalny tag `vX.Y.Z` (bez wypychania, dopóki zdalne repozytorium nie jest
  publiczne).
- Commity bez zmiany wersji (testy, porządki, poprawki wewnątrz karty)
  nie dostają tagu i nie mają wersji w tytule.
- Karta nie wskazuje numeru wersji ani tagu z góry: wersję wybiera ten,
  kto robi commit, jako kolejną po bieżącej na `master`. Prośba w karcie
  o konkretny tag jest nieaktualna, jeśli wersja już go przekroczyła.
- Wersję zmieniaj naraz we wszystkich miejscach: `Cargo.toml`
  (`workspace.package`) i `Cargo.lock`, `apps/android/app/build.gradle.kts`
  (`versionName` oraz `versionCode` + 1), `apps/android/wind-core/build.gradle.kts`,
  oba `tauri.conf.json` w `apps/windows/src-tauri`, `relay/package.json`
  i `relay/package-lock.json`.
- Przed commitem i przed tagiem uruchom `node scripts/check-version.cjs`
  (wydanie: `--tag vX.Y.Z`). Sprawdza zgodność wszystkich miejsc, tag na
  `HEAD` i wzrost `versionCode`; uruchamia go też Core CI. Dopóki tagi są
  tylko lokalne, CI (płytki checkout bez tagów) sprawdza jedynie zgodność
  plików i pisze to wprost („No previous v* tag here…”); tag i `versionCode`
  sprawdza lokalne uruchomienie przed commitem.
- Istniejących tagów nie przepisuje się i wersji się nie obniża. Wyjątek
  sprzed tej zasady: `v0.32.0` wskazuje commit z wersją 0.40.0 (WN-32).

## Praca agentów: miejsce na dysku (WN-95)

Każdy slot i każda kopia recenzji buduje od zera, więc foldery robocze
szybko rosną. Zasady:

- Po zakończonej karcie (Done) i po każdej recenzji uruchom sprzątanie:
  `pwsh scripts/clean-build.ps1` (podgląd: `-WhatIf`, cały `target`:
  `-Full`). Skrypt usuwa tylko wyniki budowania: pamięć przyrostową Cargo,
  paczki MSI starszych wersji, buildy Gradle i `target` fuzzingu. Kodu ani
  plików śledzonych przez Git nie rusza; działa też na czystym slocie.
- Nie trzymaj kopii MSI ani APK z poprzednich wersji (także poza `target`).
- Pełne logi testów i budowania trzymaj w `%TEMP%` (folder roboczy agenta),
  tylko do końca karty, nigdy w repozytorium.
- Android: buildy debug i testy kompilują jedno ABI, domyślnie `x86_64`
  (emulator); na telefon `-PwindAbi=arm64-v8a`. Release i CI budują oba.
- Profil `dev`/`test` Cargo ma tylko tablice linii dla naszych crate'ów i
  bez danych debug dla zależności; do debuggera z pełnymi danymi ustaw
  na chwilę `CARGO_PROFILE_DEV_DEBUG=true`.
