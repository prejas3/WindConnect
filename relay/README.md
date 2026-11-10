# Przekaźnik WindConnect (WN-59)

Cloudflare Worker z Durable Object, który łączy dwa sparowane urządzenia w
różnych sieciach. Przenosi tylko pakiety QUIC, już zaszyfrowane end-to-end
TLS 1.3 z przypiętymi kluczami. Przekaźnik nie ma żadnego klucza sesji ani
urządzenia, więc nie może odczytać treści. Protokół opisuje
[PROTOCOL.md §13](../PROTOCOL.md).

Opcja jest wyłączona domyślnie (decyzja D3): aplikacje łączą się z
przekaźnikiem dopiero po włączeniu go w ustawieniach (WC-60).

## Co robi serwer

- Rendezvous po kluczach publicznych: Durable Object na każdą parę,
  nazwany `SHA-256("wc-relay-v1 pair" ‖ niższy klucz ‖ wyższy klucz)`. Oba
  urządzenia muszą wskazać siebie nawzajem.
- Każde urządzenie podpisuje losowe wyzwanie swoim kluczem Ed25519, więc
  nikt nie zajmie cudzego miejsca w parze.
- Przekazuje ramki binarne bez zmian i bez zapisu. Brak peera oznacza, że
  ramka przepada, jak w UDP.
- Limity:

  | limit | wartość |
  |---|---|
  | ramka | 2048 B |
  | przepustowość na urządzenie | 1 MiB/s, chwilowo 4 MiB (zmienne w `wrangler.jsonc`) |
  | gniazda przed uwierzytelnieniem | 4 na parę; nowe wypiera najstarsze (4004) |
  | uwierzytelnienie | 10 s |
  | nowe połączenia | 30 na minutę z jednego IP |

- WebSocket Hibernation: czekające urządzenie nie zużywa czasu CPU, a
  `{"t":"ping"}` dostaje odpowiedź bez budzenia obiektu.
- Workers Logs są wyłączone: adres żądania niesie oba klucze, a log dodałby
  IP. Operator nadal może widzieć IP, klucze, czas i objętość ruchu, ale nie
  treść (PROTOCOL.md §13.4).

## Lokalnie

```powershell
cd relay
npm ci
npm test            # testy w środowisku Workers (workerd): zasady §13 i wektory z wind-core
npm run typecheck
npm run dev         # wrangler dev na http://127.0.0.1:8787
```

Dowód, że przekaźnik widzi tylko szyfrogram: przy działającym
`npm run dev`, w katalogu głównym repozytorium uruchom

```powershell
$env:RELAY_URL = "ws://127.0.0.1:8787"
cargo test -p wind-core --test relay -- --ignored --nocapture
```

Test zestawia prawdziwą sesję QUIC przez przekaźnik i wysyła wiadomość
schowka. Każdą ramkę wysłaną do przekaźnika w obu kierunkach sprawdza pod
kątem tekstu wiadomości, jego fragmentu i nazw urządzeń. Żadna ich nie
zawiera, a druga strona odczytuje wiadomość. Pomiar z 2026-11-10 (Windows,
`wrangler dev`): 27–31 ramek, ok. 15 KB, RTT 6–9 ms.

## Wdrożenie (tylko właściciel)

Agenci nie wdrażają przekaźnika. Szymon uruchamia w Claude Code, w katalogu
głównym repozytorium:

```text
! cd relay && npm ci && npx wrangler login && npx wrangler deploy
```

`wrangler login` jest potrzebny tylko przy pierwszym wdrożeniu na danym
komputerze. Po wdrożeniu adres to
`wss://windconnect-relay.<subdomena>.workers.dev/v1/relay`; podaje się go w
ustawieniach aplikacji (WC-60). Usunięcie przekaźnika:
`! cd relay && npx wrangler delete`.
