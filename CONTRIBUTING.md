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
