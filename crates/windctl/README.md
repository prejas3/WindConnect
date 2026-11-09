# windctl

CLI diagnostyczne do testów `wind-core`: tożsamość, mDNS, połączenie QUIC,
surowe wiadomości i pomiar RTT między węzłami. Wymaga jawnego wskazania pliku
tożsamości i zweryfikowanych kluczy publicznych peerów.

Z katalogu głównego: `cargo build --locked -p windctl`.
Program: `target/debug/windctl.exe` na Windows albo `target/debug/windctl`
na Linuxie/macOS. Można też użyć `cargo run --locked -p windctl -- <argumenty>`.
`windctl --help`, `windctl ping --help` i `windctl --version` nie tworzą klucza.

## Dwa komputery

Na każdym węźle wybierz osobny plik w prywatnym katalogu użytkownika.
Na Windows klucz jest szyfrowany przez DPAPI dla bieżącego użytkownika;
na Unixie `FileKeyStore` zapisuje PKCS#8 z uprawnieniami 0600. Istniejący
nieczytelny klucz oznacza błąd — nie jest zastępowany. Nie używaj jednego pliku
lub kopii klucza na obu węzłach.

1. Na A: `windctl --identity <plik_A> identity`.
2. Na B: `windctl --identity <plik_B> identity`.
3. Wymień i zweryfikuj `public_key` (64 znaki hex) innym kanałem.
   `fingerprint` to SHA-256 SPKI, a nie klucz publiczny; nie podawaj go jako
   `--peer-key`. Żadne ogłoszenie mDNS nie udziela zaufania.
4. Na B uruchom:

   ```text
   windctl --identity <plik_B> --name PC-B serve --bind 0.0.0.0:47820 --trust <public_key_A> --duration 300 --advertise --echo
   ```

5. Na A, podając rzeczywisty adres LAN B:

   ```text
   windctl --identity <plik_A> discover --duration 5
   windctl --identity <plik_A> connect --address 192.168.1.20:47820 --peer-key <public_key_B>
   windctl --identity <plik_A> ping --address 192.168.1.20:47820 --peer-key <public_key_B> --count 100 --warmup 5 --interval-ms 50
   ```

`serve` wypisuje rzeczywisty port (`--bind ...:0` wybiera wolny), akceptuje
wyłącznie klucze z powtarzalnego `--trust`, odpowiada na Ping/Pong i ogłasza
kanały diagnostyczne. Trwa domyślnie 60 s, maksymalnie 86400 s; Ctrl+C kończy
wcześniej. Obsługuje do 16 równoległych sesji. Na komputerze B zezwól w
zaporze na testowy port UDP; mDNS wymaga UDP 5353 i sieci przepuszczającej
multicast. Program nie zmienia reguł zapory.

Fallback broadcast działa na UDP 47821. Aby sprawdzić sieć bez multicastu,
uruchom B z `serve --advertise --broadcast-only` (pozostałe argumenty jak wyżej),
a A z `discover --duration 5 --broadcast-only`. Ten tryb w ogóle nie uruchamia
mDNS; znaleziony wpis `broadcast-<odcisk>` podaje IP nadawcy i jego rzeczywisty
port QUIC. Sprawdź potem `connect` lub `ping` ze zweryfikowanym kluczem B.
Nieodświeżone wpisy broadcast wygasają po 8 s. Fallback dotyczy lokalnego IPv4;
odkrywanie IPv6 nadal używa mDNS.

IPv6: `serve --bind [::]:47820`, po stronie klienta np.
`--address [2001:db8::20]:47820`. Domyślny serwer `0.0.0.0` jest IPv4;
wybór rodziny adresów nasłuchu jest jawny.

`discover` używa modułu mDNS rdzenia, wypisuje zmiany `DISCOVERED` i `REMOVED`
z odciskiem oraz adresami i kończy po zadanym czasie. Ogłasza także własny
tymczasowy endpoint z tym samym kluczem; własne wyniki są pomijane. Do
połączenia nadal wymagany jest zweryfikowany klucz publiczny.

## Pomiar latencji

Przykładowy format wyniku (wartości zależą od sieci):

```text
PING seq=1 rtt_ms=1.234
...
SUMMARY sent=100 received=100 lost=0 warmup=5 p50_ms=1.234 p95_ms=2.345 min_ms=0.987 max_ms=3.456
```

Mierzony jest RTT kontrolnego Ping/Pong po mTLS i wymianie Hello/HelloAck,
bez czasu zestawiania sesji. Rozgrzewka nie wchodzi do próbek. Każdy Ping
rdzenia ma limit 5 s. Nieudane próby wchodzą do `lost`, a nie do percentyli.
Najbliższa ranga: po posortowaniu wyników percentyl P to próbka o indeksie
`ceil(P*N/100)` (liczonym od 1). Jeden pomiar daje identyczne p50 i p95.
Program zwraca błąd, jeśli nie otrzyma żadnej poprawnej próbki; częściowa
utrata pozostaje widoczna w podsumowaniu. Liczba pomiarów: 1–10000.
`--timeout-ms` ogranicza zestawienie sesji (domyślnie 10000 ms) oraz oczekiwanie
na echo, nie zmienia wewnętrznego limitu Ping/Pong.

`connect` kończy się sukcesem po zweryfikowaniu obu kluczy, Hello/HelloAck
i jednym udanym Ping/Pong. Błędne argumenty, odmowa TLS, błąd klucza lub
brak wymaganej odpowiedzi oznaczają niezerowy kod wyjścia.

## Wiadomości diagnostyczne

Do węzła `windctl serve --echo`:

```text
windctl --identity <plik_A> send --address 192.168.1.20:47820 --peer-key <public_key_B> --channel notify --message-type 65000 --text "test węzła A" --expect-echo
windctl --identity <plik_A> send --address 192.168.1.20:47820 --peer-key <public_key_B> --channel media --message-type 65001 --body-hex a100182a --expect-echo
```

`--text` koduje ciało `{0: tekst}`. `--body-hex` przyjmuje samo ciało wiadomości
jako deterministyczną mapę CBOR z naturalnymi kluczami (`a100182a` = `{0: 42}`).
Kodowanie, ramkowanie, limity i wysyłanie strumieniem należą do rdzenia.
Kanały: `clipboard`, `notify`, `media`. To narzędzie surowych testów;
typy 65000/65001 nie są schematem funkcji schowka, powiadomień ani multimediów
i nie są dodawane do normatywnego protokołu.

`--expect-echo` potwierdza odebranie identycznej wiadomości zwrotnej, łącznie
z kanałem i typem. Bez tej opcji `SENT ... echo_verified=false` potwierdza
lokalne wysłanie i żywą sesję kontrolną, nie wykonanie funkcji na odbiorcy.
Serwer wypisuje `RECEIVED` z ciałem CBOR w hex; bez `--echo` nie odsyła go.

## Testy

```text
cargo test --locked -p windctl
cargo test --locked -p windctl --test cli -- --ignored --nocapture
```

Pierwsze polecenie sprawdza percentyle i parser, uruchamia dwa odrębne procesy
z osobnymi kluczami oraz testuje sesję, Ping/Pong, oba formaty wiadomości,
echo i odmowę obcego klucza. Drugie wymaga działającego multicastu i sprawdza
odkrywanie osobnego procesu przez mDNS. Każdy proces testowy jest ograniczony
czasowo i sprzątany po teście. Te testy lokalne nie zastępują pomiaru na dwóch
fizycznych komputerach w LAN według procedury powyżej.
