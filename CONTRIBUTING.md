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
