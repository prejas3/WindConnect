# Protokół WindConnect v1

Status: wersja 1.0 (WN-5, v0.5.0). Dokument jest normatywny dla `wind-core`.
Słowa MUSI, NIE MOŻE, POWINIEN i MOŻE mają znaczenie jak MUST, MUST NOT,
SHOULD i MAY w RFC 2119.

Zakres v1.0: transport, ramkowanie, wersjonowanie, negocjacja możliwości,
kody błędów, kanał `control` i kanał `input`. Wiadomości funkcji (schowek,
powiadomienia, multimedia, pliki) dopisują karty tych funkcji jako zmiany
tego dokumentu według zasad z sekcji 9; do tego czasu ich kanały mają
zarezerwowane numery i zasady strumieni, ale nie mają typów wiadomości.

## 1. Przegląd

```text
QUIC v1 (RFC 9000) + TLS 1.3, ALPN "windconnect/1"
├── strumień bidi  control      (jeden, otwiera klient)       ramki CBOR
├── strumień uni   clipboard    (1 aktywny na kierunek, §3.1) ramki CBOR
├── strumień uni   notify       (jw.)                         ramki CBOR
├── strumień uni   media        (jw.)                         ramki CBOR
├── strumień uni   notify-action (jw., §9.8)                  ramki CBOR
├── strumień uni   volume       (jw., §9.9)                   ramki CBOR
├── strumień uni   call         (jw., §9.10)                  ramki CBOR
├── strumień uni   input        (jw., przyciski)              ramki CBOR
├── strumień bidi  files × N    (1–8 na transfer, §9.1)       ramki CBOR
├── strumień bidi  actions × N  (1 na prośbę, §9.2)           ramki CBOR
└── datagramy QUIC input        (ruch wskaźnika, przewijanie) CBOR bez długości
```

- **Klient** to strona, która zestawia połączenie QUIC; **serwer** to strona,
  która je przyjmuje. Obie role ma każde urządzenie; po połączeniu protokół
  jest symetryczny poza otwarciem kanału `control`.
- Bajty na łączu są w kolejności sieciowej (big-endian).
- Liczby w przykładach binarnych są szesnastkowe.

## 2. Transport i bezpieczeństwo

- Transport: QUIC v1 (RFC 9000) z TLS 1.3 (RFC 9001). Domyślny port UDP
  serwera: **47820**; odkrywanie (mDNS, broadcast) ogłasza rzeczywisty port.
- ALPN: `windconnect/1`, gdzie `1` to **główna** wersja protokołu (sekcja 5).
  Ustanowienie połączenia bez zgodnego ALPN kończy się alertem TLS
  `no_application_protocol`.
- ALPN `windconnect-pair/1` należy do protokołu parowania (sekcja 12), na
  osobnym endpoincie; główny endpoint go nie przyjmuje.
- Uwierzytelnienie jest wzajemne (mTLS): każda strona przedstawia certyfikat
  self-signed z kluczem Ed25519 urządzenia (WC-07). Strona MUSI zaakceptować
  certyfikat drugiej strony wyłącznie wtedy, gdy jego klucz publiczny jest
  przypiętym kluczem sparowanego urządzenia (WC-10). Łańcuch, nazwa i daty
  certyfikatu nie są podstawą zaufania.
- Odmowa następuje w uzgadnianiu TLS, zanim popłyną dane aplikacji: strona
  odrzucająca kończy je alertem TLS `access_denied` (kod QUIC `0x131`).
  Podpisy uzgadniania są wyłącznie Ed25519, więc certyfikat z innym kluczem
  kończy się już brakiem wspólnego schematu podpisu. Klient TLS 1.3 wysyła
  swój certyfikat w ostatnim locie, więc odrzucony klient może uznać
  uzgadnianie za zakończone i dopiero potem dostać zamknięcie z tym
  alertem; serwer takiego połączenia nie przyjmuje. Kod `UNTRUSTED_PEER`
  (sekcja 10) służy do zamknięcia już zestawionego połączenia, gdy klucz
  przestał być przypięty (np. rozparowanie).
- Tożsamością urządzenia jest wyłącznie jego klucz TLS. Pola wiadomości
  (np. nazwa urządzenia w `Hello`) są danymi do wyświetlania, nie dowodem
  tożsamości.
- Datagramy QUIC (RFC 9221): strona, która ogłasza `input/1`, POWINNA
  ogłosić parametr `max_datagram_frame_size` ≥ 1200. Gdy w danym połączeniu
  datagramy nie są dostępne, zdarzenia ciągłe idą strumieniem `input`
  (sekcja 8.7).
- Dane 0-RTT (wznawianie, WC-12) MOGĄ zawierać wyłącznie nagłówek strumienia
  `control`, ramkę `Hello` i ramki `Ping`. Wszystkie pozostałe dane MUSZĄ
  czekać na zakończenie uzgadniania 1-RTT. Obie wiadomości są odporne na
  powtórzenie (replay).
- Utrzymanie i zmiana sieci (WC-12): strona wysyła keep-alive QUIC (ramka
  `PING`) co 15 s bezczynności i ogłasza `max_idle_timeout` 45 s. Migracja
  ścieżki (RFC 9000 §9) jest dozwolona: strona NIE MOŻE ogłaszać
  `disable_active_migration`, a po zmianie sieci klient przenosi istniejące
  połączenie na nowe gniazdo zamiast zestawiać nowe.
- Wznowienie TLS: klient pamięta bilety sesji osobno dla każdego urządzenia
  i oferuje bilet tylko temu, które go wydało; nazwa serwera (SNI) jest dla
  wszystkich urządzeń ta sama (`windconnect`), więc nie zdradza, z kim klient
  się łączy. Wznowiona sesja pomija weryfikację certyfikatu, dlatego po
  zakończeniu uzgadniania strona MUSI ponownie sprawdzić, że klucz drugiej
  strony jest nadal przypięty, a jeśli nie jest, zamknąć połączenie kodem
  `UNTRUSTED_PEER`.
- Ponowne łączenie po utracie połączenia: odstępy rosnące wykładniczo
  (od 100 ms do 30 s, z losowym rozrzutem 50–100 %), z natychmiastową próbą
  po wykryciu zmiany sieci (WC-16). Odmowa z powodu przypięcia
  (`access_denied`, `UNTRUSTED_PEER`) kończy ponawianie.

### 2.0 Odkrywanie mDNS (WN-14)

- Usługa DNS-SD: `_windconnect._udp.local.`. SRV ogłasza rzeczywisty,
  niezerowy port nasłuchu QUIC, a A/AAAA adresy urządzenia.
- TXT `fingerprint` zawiera pełny SHA-256 SPKI z §2.1: 64 znaki hex.
  Nadawca używa małych liter; odbiorca akceptuje obie wielkości liter.
  Brak lub błędny odcisk oznacza pominięcie ogłoszenia.
- Nazwy instancji i hosta mają postać `wc-<pierwsze 40 znaków odcisku>`
  (host z `.local.`); mDNS może zmienić nazwę przy konflikcie.
- Ogłoszenie jest niezaufaną wskazówką adresową. Odcisk TXT NIE MOŻE
  przypinać klucza ani omijać uwierzytelnienia TLS z §2.
- TXT `pair` (WN-82, §12.4): port endpointu parowania, tylko gdy otwarte
  jest okno SAS; znika po jego zamknięciu (sparowanie, wygaśnięcie,
  zamknięcie, blokada po próbach, zastąpienie oknem QR). Wartość: 1–5 cyfr
  dziesiętnych, 1–65535; inna (0, nie liczba, za duża) oznacza brak portu,
  a reszta ogłoszenia zostaje. Zmiana sieci nie usuwa pola.
- Aktualizacja rozwiązanego ogłoszenia zastępuje jego poprzednie adresy;
  goodbye lub wygaśnięcie rekordów usuwa usługę. Własny odcisk jest pomijany.
- Ponowne ogłoszenie tej samej nazwy (np. po zmianie sieci) nadawca wysyła
  nie wcześniej niż 1 s po swoim goodbye, czyli po czasie, przez który odbiorca
  trzyma goodbye (RFC 6762 §10.1). Odbiorca nie musi wtedy rozróżniać
  „ratowania” rekordu od nowego urządzenia (WN-104).

### 2.0.1 Zapasowe odkrywanie IPv4 broadcast (WN-15)

- Port odkrywania: UDP **47821**, niezależny od portu QUIC. Pakiet ma
  dokładnie 39 bajtów: ASCII `WCD1` (4 B), rodzaj (1 B: `1` zapytanie,
  `2` odpowiedź), rzeczywisty port QUIC (2 B big-endian, niezerowy),
  pełny odcisk SHA-256 SPKI (32 B, jak w §2.1).
- Zapytanie ogłasza nadawcę i prosi o odpowiedź. Jest wysyłane na adresy
  broadcast interfejsów IPv4 co 2 s i bezpośrednio po zmianie sieci;
  gdy brak takich adresów, używane jest `255.255.255.255:47821`.
- Odpowiedź jest unicastem do źródła zapytania. Ma ten sam rozmiar i nie
  wywołuje kolejnej odpowiedzi. Limit: jedna odpowiedź na IP źródłowe na
  sekundę, maksymalnie 128 śledzonych źródeł.
- Adres peera pochodzi z IP źródłowego datagramu, a port z jego treści.
  Port źródłowy musi być portem odkrywania. Pakiety obce, niepoprawne,
  nadmiarowe i własny odcisk są pomijane. Nieodświeżony wpis wygasa po 8 s;
  bufor broadcastu mieści najwyżej 128 odcisków.
- Pełny bufor robi miejsce zamiast odrzucać nowe wpisy (WN-87), tak aby
  jeden nadawca zmyślonych odcisków nie wyłączył odkrywania:
  - jedno IP źródłowe ma najwyżej jeden niesparowany odcisk; nowszy odcisk
    z tego IP zastępuje poprzedni (jedno urządzenie ma jeden odcisk);
  - odcisk sparowanego urządzenia (klucz przypięty, §2.1) jest przyjmowany
    zawsze i nie liczy się do limitu jednego odcisku na IP;
  - gdy bufor jest pełny, nowy wpis wypiera najdawniej odświeżony wpis
    niesparowany; niesparowany odcisk nigdy nie wypiera sparowanego. Gdy
    bufor jest pełen samych sparowanych, niesparowany jest pomijany, a
    sparowany wypiera najdawniej odświeżony sparowany.
  Pierwszeństwo sparowanych to tylko kolejność w buforze wskazówek; nie
  daje zaufania i niczego nie przypina.
- Broadcast działa równolegle z mDNS, również gdy multicast jest niedostępny.
  Wyniki są łączone według odcisku, z usunięciem powtórzonych adresów.
  To niezaufane wskazówki; NIE MOGĄ przypinać kluczy ani omijać TLS.

### 2.1 Klucz, certyfikat i odcisk

- Klucz urządzenia to para Ed25519 (RFC 8032), tworzona przy pierwszym
  uruchomieniu i trwała między uruchomieniami. Nieczytelny zapisany klucz
  jest błędem; implementacja NIE MOŻE wtedy po cichu utworzyć nowego, bo
  nowy klucz oznacza utratę wszystkich parowań.
- Certyfikat TLS to self-signed X.509 v3 podpisany tym kluczem, z
  `subjectAltName` DNS `windconnect`. Pola certyfikatu poza kluczem
  publicznym nie niosą znaczenia (sekcja 2).
- **Odcisk** klucza to SHA-256 z DER `SubjectPublicKeyInfo` klucza
  (RFC 8410), czyli `302a300506032b6570032100 ‖ klucz (32 B)`. Daje się go
  policzyć standardowymi narzędziami, np.
  `openssl pkey -pubout -outform DER | sha256sum`.
- Postać do wyświetlania: pierwsze 160 bitów odcisku w base32 Crockforda
  (alfabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, bez `I L O U`), 32 znaki
  w ośmiu grupach po cztery, rozdzielone spacją. Przykład dla klucza
  publicznego z RFC 8032 §7.1, test 1
  (`d75a9801…f707511a`): odcisk `06e3fd8f…1b792fa9`, wyświetlany jako
  `0VHZ V3YT 56XP 1ATS ANYY C7PV 1BPD P8RH`.
- Odcisk służy ludziom do porównania i do odkrywania; zaufanie opiera się
  na pełnym kluczu publicznym (sekcja 2).

## 3. Strumienie i nagłówek strumienia

Strona, która **otwiera** strumień, zaczyna go dwubajtowym nagłówkiem:

| bajt | pole | wartość |
|---|---|---|
| 0 | `version` | główna wersja protokołu, w v1 `01` |
| 1 | `channel` | numer kanału z tabeli niżej |

Po nagłówku następuje ciąg ramek (sekcja 4) aż do końca strumienia (FIN).
Nagłówek pisze **tylko otwierający**. W strumieniu bidi kierunek powrotny
(od strony, która strumień przyjęła) nie ma nagłówka: zaczyna się od razu
pierwszą ramką. Dotyczy to `control` (`HelloAck` to pierwsze bajty
od serwera, przykład w sekcji 7.1), `files` (odpowiedzi odbiorcy pliku) i
`actions` (odpowiedź na prośbę).

| kanał | nazwa | rodzaj strumienia | otwiera | priorytet wysyłki |
|---|---|---|---|---|
| `00` | `control` | bidi | klient; dokładnie 1 na połączenie | 0 (najwyższy) |
| `01` | `clipboard` | uni | nadawca wiadomości | 2 |
| `02` | `notify` | uni | nadawca wiadomości | 3 |
| `03` | `media` | uni | nadawca wiadomości | 4 |
| `04` | `input` | uni + datagramy | nadawca zdarzeń | 1 |
| `05` | `files` | bidi | nadawca pliku; 1–8 na transfer (§9.1) | 7 (najniższy) |
| `06` | `actions` | bidi | proszący o akcję; 1 na prośbę (§9.2) | 5 |
| `07` | `battery` | uni | nadawca stanu baterii (§9.4) | 6 |
| `08` | `find` | bidi | proszący o dzwonienie; 1 na dzwonienie (§9.5) | 5 |
| `09` | `media-control` | uni | nadawca poleceń dla odtwarzacza drugiej strony (§9.6) | 4 |
| `0a` | `notify-action` | uni | strona pokazująca powiadomienia drugiej strony: odrzucenie, odpowiedź (§9.8) | 3 |
| `0b` | `volume` | uni | strona, której głośność się zmienia (`Level`), albo strona sterująca (`Set`) (§9.9) | 4 |
| `0c` | `call` | uni | telefon: stan połączenia (§9.10) | 1 |
| `0d` | `sms` | bidi | komputer; 1 na prośbę o wątki, wiadomości, zdjęcie albo obserwację (§9.11) | 5 |
| `0e` | `call-action` | uni | komputer: odrzuć lub wycisz dzwoniące połączenie telefonu (§9.12) | 1 |
| `0f`–`3f` | zarezerwowane dla przyszłych kanałów | | | |
| `40`–`ff` | niedozwolone w v1 | | | |

Zasady:

1. Strona MUSI odrzucić strumień z nieznanym `version`: `STOP_SENDING`
   z kodem `UNSUPPORTED_VERSION`. Strumień `control` z nieznanym `version`
   zamyka całe połączenie tym kodem.
2. Strumień z nieznanym lub niedozwolonym `channel` strona odrzuca przez
   `STOP_SENDING` z kodem `UNKNOWN_CHANNEL`; połączenie trwa dalej.
3. Kanał `control` nie wymaga możliwości. Strumień każdego innego kanału
   strona przyjmuje tylko wtedy, gdy **ona sama** ogłosiła (w ostatnim
   wysłanym `Hello` / `HelloAck` / `Capabilities`) możliwość tego kanału,
   bo możliwość oznacza „przyjmuję” (sekcja 6). W przeciwnym razie
   odrzuca strumień przez `STOP_SENDING` z kodem `CAPABILITY_MISSING`;
   połączenie trwa dalej.
4. Drugi strumień `control`, strumień `control` otwarty przez serwer albo
   strumień o niewłaściwym rodzaju (uni zamiast bidi i odwrotnie) to
   `PROTOCOL_VIOLATION` dla całego połączenia.
5. Nagłówek przychodzi w całości przed pierwszą ramką; strumień zakończony
   (FIN) przed pełnym nagłówkiem to `MALFORMED_FRAME` dla tego strumienia.
6. Priorytet wysyłki: nadawca POWINIEN obsługiwać strumienie o niższej
   liczbie przed wyższą, tak by ruch `files` nie opóźniał `control` ani
   `input`. Strumienie `files` o równym priorytecie dzielą łącze po równo.

Przykład: nagłówek strumienia `control` to `01 00`, strumienia `input` to `01 04`.

### 3.1 Cykl życia strumieni uni

Kanały `clipboard`, `notify`, `notify-action`, `media`, `media-control`, `volume`, `call`, `call-action`, `input` i `battery` mają w każdym kierunku
**kolejkę wiadomości**: ciąg strumieni uni jednego nadawcy dla tego kanału.

1. Nadawca ma naraz najwyżej **jeden otwarty do zapisu** strumień kanału.
   Nowy strumień tego kanału otwiera dopiero po zakończeniu poprzedniego
   po swojej stronie: FIN (zwykłe zamknięcie) albo `RESET_STREAM`
   (porzucenie). Limit dotyczy strumieni aktywnych, nie wszystkich
   w historii połączenia.
2. Nadawca MUSI zapisać nagłówek od razu przy otwarciu strumienia,
   najpóźniej razem z pierwszą ramką; nie otwiera strumienia „na zapas”.
3. **Kolejność w kanale.** Odbiorca przekazuje modułowi wiadomości
   strumienia uni `S` kanału `C` od nadawcy `N` dopiero wtedy, gdy:
   1. każdy strumień uni od `N` o niższym numerze QUIC jest
      **rozpoznany** (doszedł jego nagłówek) albo **zakończony** (FIN lub
      `RESET_STREAM`), oraz
   2. każdy z nich, rozpoznany jako kanał `C`, jest zakończony i jego
      wiadomości zostały już przekazane.

   Strumienie innych kanałów blokują tylko do chwili dojścia ich nagłówka
   (zasada 2), nie do końca. Strumień porzucony przez `RESET_STREAM`, zanim
   doszedł jego nagłówek, liczy się jako zakończony. Numery strumieni QUIC
   rosną w kolejności otwierania (RFC 9000 §2.1), a zasada 1 gwarantuje, że
   poprzedni strumień kanału ma niższy numer, więc kolejność wiadomości
   jednego kanału jest zachowana także przy wymianie strumienia.
4. Strumień uni, którego nagłówek nie doszedł w 2 s od dojścia strumienia
   o wyższym numerze od tego samego nadawcy, odbiorca odrzuca
   (`STOP_SENDING`, `PROTOCOL_VIOLATION`) i traktuje jako zakończony.
5. **Limit strumieni.** Przed nadawcą, który otwiera strumienie wbrew
   zasadzie 1, odbiorcę chroni limit strumieni QUIC i kontrola przepływu,
   nie odrzucanie:
   1. Strona ogłasza `initial_max_streams_uni` = **100** (zalecane; nie
      mniej niż 16). Limit jest wspólny dla wszystkich kanałów uni.
   2. Nowe strumienie (ramka `MAX_STREAMS`) strona przyznaje dopiero wtedy,
      gdy strumień uni drugiej strony jest u niej **obsłużony**: przeczytany
      do FIN, a jego wiadomości przekazane modułowi albo przyjęte do bufora
      odczytu z wyprzedzeniem (punkt 6), albo porzucony (`RESET_STREAM`),
      albo odrzucony (`STOP_SENDING`). Strumień, którego wiadomości nie
      zmieściły się ani w kolejce modułu, ani w buforze, nadal zajmuje
      limit. Strona MOŻE przyznawać strumienie partiami (np. gdy zwolni się
      ponad 1/8 limitu).
   3. Nadawca, który wyczerpał limit, czeka na `MAX_STREAMS`; żadna
      wiadomość nie przepada. Ilość danych buforowanych przez odbiorcę
      ograniczają okna kontroli przepływu QUIC (strumieni i połączenia).
   4. Odbiorca NIE MOŻE odrzucić strumienia (np. kodem `RATE_LIMITED`)
      tylko z powodu liczby otwartych lub nieobsłużonych strumieni kanału:
      nie odróżni nadawcy łamiącego zasadę 1 od szybkiego, poprawnego
      nadawcy, którego zakończone strumienie czekają na wolny moduł.
   5. Ponieważ limit jest wspólny, nadawca POWINIEN wysyłać kolejne
      wiadomości kanału tym samym strumieniem i kończyć go dopiero w
      przerwie albo przy wycofaniu możliwości, a nie otwierać strumienia
      na każdą wiadomość.
   6. **Odczyt z wyprzedzeniem (WN-81).** Odbiorca czyta strumienie kanału
      dalej, niż jego moduł odbiera: wiadomości trafiają do bufora kanału,
      w kolejności, a strumień przeczytany do FIN jest od razu obsłużony
      (punkt 2), choć moduł jeszcze ich nie dostał. Bufor MUSI być
      ograniczony; zalecane **1 MiB na kanał**, licząc ładunek każdej
      wiadomości plus stały narzut (w `wind-core` 64 B), tak by ograniczać
      też wiele drobnych wiadomości. Pełny bufor zatrzymuje czytanie
      bieżącego strumienia (kontrola przepływu wstrzymuje nadawcę); ten i
      dalsze strumienie kanału zajmują wtedy limit jak w punkcie 2.
      Wiadomość większa niż cały bufor czeka, aż bufor się opróżni, i
      wchodzi sama. Wycofanie możliwości (sekcja 6, zasada 5) usuwa z bufora
      wiadomości sprzed wycofania.

   Skutek: wolny moduł jednego kanału **nie wstrzymuje** innych kanałów,
   dopóki jego zaległość mieści się w buforze. Inne kanały mogą czekać tylko
   wtedy, gdy (a) zaległość kanału przekracza bufor i nadawca dalej otwiera
   nowe strumienie tego kanału, aż wyczerpie limit, albo (b) nadawca łamie
   zasadę 1 i trzyma strumienie bez FIN. Oba przypadki dotyczą
   sparowanego urządzenia i są błędem nadawcy, nie utratą danych: nic nie
   przepada, a nadawca czeka na `MAX_STREAMS`.
6. Wiadomości z porzuconego strumienia (`RESET_STREAM`) mogą przepaść;
   kanały, w których to szkodzi, opisują w swojej specyfikacji ponowienie.

Przykład: klient (strumienie uni 2, 6, 10, …) wysyła na strumieniu 2
schowek A z FIN, potem na strumieniu 6 schowek B z FIN. Do odbiorcy cały
strumień 6 dochodzi pierwszy:

```text
strumień 6: 01 01 + B + FIN  → kanał clipboard; strumień 2 jeszcze
                               nierozpoznany → B czeka (zasada 3, pkt 1)
strumień 2: 01 01 + A + FIN  → clipboard, nic niższego nie brakuje →
                               przekaż A; strumień 2 zakończony → przekaż B
```

Gdyby strumień 2 był kanałem `notify`, B zostałby przekazany zaraz po
dojściu nagłówka `01 02` strumienia 2. Gdyby strumień 2 został porzucony
(`RESET_STREAM`) przed dojściem nagłówka, B zostałby przekazany zaraz po
dojściu ramki `RESET_STREAM`.

Wycofanie i ponowne włączenie możliwości (sekcja 6, zasada 5) korzysta
z tych samych reguł: wycofanie kończy strumień, ponowne włączenie otwiera
nowy, o wyższym numerze.

## 4. Ramka

```text
ramka   = długość ładunek
długość = liczba zmiennej długości QUIC (RFC 9000 §16), 1–8 bajtów
ładunek = dokładnie `długość` bajtów: jeden element CBOR
```

Ładunek to tablica CBOR o dwóch elementach `[type, body]`:

- `type`: liczba naturalna, typ wiadomości. Numeracja jest **osobna dla
  każdego kanału** (kanał wynika ze strumienia), więc router wybiera moduł
  po parze (`channel`, `type`).
- `body`: mapa CBOR o kluczach będących liczbami naturalnymi. Wiadomość
  bez pól ma pustą mapę `a0`.

Kodowanie długości: `06` (6), `41 2c` (300), `80 01 11 70` (70 000).
Nadawca MUSI użyć najkrótszego kodowania długości; odbiorca MUSI odrzucić
dłuższe niż potrzebne kodowanie (`MALFORMED_FRAME`).

### 4.1 Podzbiór CBOR

Nadawca MUSI kodować deterministycznie (RFC 8949 §4.2.1): najkrótsze
kodowanie liczb i długości, tylko długości określone, klucze map
posortowane bajtowo po ich zakodowanej postaci. Odbiorca MUSI przyjmować
wyłącznie:

| typ główny | dozwolone |
|---|---|
| 0, 1 | liczby całkowite w zakresie i64 / u64 |
| 2 | ciągi bajtów o określonej długości |
| 3 | tekst UTF-8 o określonej długości, poprawny UTF-8 |
| 4 | tablice o określonej długości |
| 5 | mapy o określonej długości, bez powtórzonych kluczy |
| 7 | tylko `false` (`f4`), `true` (`f5`), `null` (`f6`) |

Wszystko inne (znaczniki, liczby zmiennoprzecinkowe, długości nieokreślone,
`undefined`, niekanoniczne kodowanie) jest błędem `MALFORMED_FRAME`.
Zagnieżdżenie (tablice i mapy) nie przekracza 16 poziomów. Ładunek musi
zawierać dokładnie jeden element CBOR, bez bajtów nadmiarowych.

### 4.2 Limity rozmiaru

| miejsce | największy `długość` / datagram |
|---|---|
| ramka `Hello` i `HelloAck` | 4 096 B |
| inne ramki na `control` | 16 384 B |
| ramki na `actions` (§9.2) | 16 384 B |
| ramki na pozostałych strumieniach | 1 048 576 B (1 MiB) |
| datagram | mniejszy z: `max_datagram_frame_size` drugiej strony, 1 200 B |

Ramka o długości 0 albo ponad limit to błąd: dla `control` zamknięcie
połączenia `FRAME_TOO_LARGE` (lub `MALFORMED_FRAME` dla 0), dla innych
strumieni `STOP_SENDING` z tym kodem. Odbiorca MUSI sprawdzić długość
**przed** buforowaniem ładunku.

### 4.3 Pola i rozszerzalność

- Pola w `body` są opisane przy każdej wiadomości jako
  `klucz nazwa: typ` z oznaczeniem *wymagane* albo *opcjonalne*.
- Brak pola wymaganego albo pole o złym typie to `MALFORMED_FRAME`.
- Nieznane klucze w `body` odbiorca MUSI pominąć (zgodność w przód).
- Teksty mają podane limity długości w bajtach UTF-8; przekroczenie to
  `MALFORMED_FRAME`.

### 4.4 Nieznane typy wiadomości

Nieznany `type` na znanym kanale nie zrywa połączenia. Odbiorca pomija
ramkę i MOŻE odesłać `Error` z kodem `UNKNOWN_MESSAGE` (sekcja 7.6), nie
częściej niż raz na sekundę na kanał.

## 5. Wersjonowanie

Wersja protokołu to para `major.minor`.

- **major** zmienia się przy zmianie niezgodnej wstecz (ramkowanie,
  znaczenie istniejących pól). Jest w ALPN (`windconnect/1`) i w bajcie
  `version` nagłówka strumienia oraz datagramu. Strony bez wspólnej wersji
  głównej nie zestawią połączenia (alert ALPN).
- **minor** zmienia się przy zmianie zgodnej: nowe typy wiadomości, nowe pola
  opcjonalne, nowe kanały w puli zarezerwowanej. Strona ogłasza swoją wersję
  w `Hello` / `HelloAck`; obie strony używają **mniejszej** z dwóch wersji
  minor i nie wysyłają typów wiadomości dodanych później.
- Pole `version` w `Hello` z inną wersją główną niż w ALPN to
  `PROTOCOL_VIOLATION`.
- Klient MOŻE oferować kilka wersji głównych w ALPN (od najnowszej); serwer
  wybiera najnowszą wspólną.

Ten dokument opisuje wersję **1.0**.

## 6. Negocjacja możliwości

Możliwość to tekst `nazwa/N`: `nazwa` z małych liter ASCII, cyfr i `-`
(1–24 bajty), `N` to dodatnia liczba dziesiętna, wersja możliwości. Lista
ma najwyżej 64 pozycje, bez powtórzeń.

Możliwości v1.0:

| możliwość | znaczenie |
|---|---|
| `clipboard/1` | strona obsługuje kanał `clipboard` (tekst) |
| `clipboard/2` | jak `clipboard/1` oraz obrazy PNG (§9.3.1, WN-36) |
| `notify/1` | strona pokazuje powiadomienia drugiej strony: przyjmuje `Posted`, `Icon`, `Removed`, `Replied` na kanale `notify` (§9.7, WN-42) |
| `notify-action/1` | powiadomienia strony można odrzucać i na nie odpowiadać: przyjmuje `Dismiss`, `Reply` na kanale `notify-action` (§9.8, WN-44) |
| `volume/1` | głośność strony można zmieniać, a strona pokazuje głośność drugiej: przyjmuje `Set` i `Level` na kanale `volume` (§9.9, WN-48) |
| `call/1` | strona wstrzymuje swoje odtwarzacze przy połączeniu drugiej strony albo pokazuje jej połączenia: przyjmuje `State` i `Missed` na kanale `call` (§9.10, WN-48, WN-57) |
| `call-action/1` | dzwoniące połączenie strony można odrzucić lub wyciszyć: przyjmuje `Reject` i `Silence` na kanale `call-action` (§9.12, WN-57) |
| `media/1` | strona pokazuje odtwarzacz drugiej strony: przyjmuje `Now`, `Art`, `Gone` na kanale `media` (§9.6, WN-46) |
| `media-control/1` | odtwarzaczem strony można sterować: przyjmuje `Command` na kanale `media-control` (§9.6, WN-46) |
| `input/1` | strona przyjmuje zdarzenia wejścia (kanał `input`, sekcja 8) |
| `input/2` | jak input/1 oraz niezawodne bariery gestów Windows (sekcja 8.9); odbiorca ogłasza również input/1 |
| `files/1` | strona obsługuje kanał `files` |
| `actions/1` | strona przyjmuje prośby o akcje (kanał `actions`, §9.2) |
| `battery/1` | strona przyjmuje stan baterii drugiej strony (kanał `battery`, §9.4, WN-49) |
| `find/1` | strona potrafi zadzwonić na prośbę drugiej (kanał `find`, §9.5, WN-50) |
| `sms/1` | strona udostępnia drugiej do odczytu swoje wątki SMS/MMS (WN-55) i, za osobną zgodą, wysyłanie przez siebie (WN-56): przyjmuje prośby na kanale `sms` (§9.11) |

Zasady:

1. Możliwość oznacza **odbiór**: strona ogłasza w `Hello` / `HelloAck`
   kanały, których wiadomości **przyjmuje**. Wysyłanie nie wymaga ogłoszenia
   niczego: strona MOŻE wysyłać wiadomości kanału, gdy druga strona ogłosiła
   jego możliwość, a ona sama potrafi je wysyłać. Każdy kierunek jest
   niezależny. Kanał `control` działa zawsze, bez możliwości.
2. Wybór wersji, osobno dla każdego kierunku: nadawca używa najwyższej
   wersji `N`, dla której odbiorca ogłosił `nazwa/N`, a nadawca umie wysyłać
   wiadomości w tej wersji. Gdy takiej wersji nie ma, nadawca nie wysyła
   wiadomości tego kanału. Lista może zawierać kilka wersji tej samej nazwy
   (`clipboard/1`, `clipboard/2`).
3. Nieznane możliwości odbiorca pomija.
4. Funkcja, w której odbiorca odpowiada nadawcy (np. potwierdzenie),
   umieszcza odpowiedź w tym samym strumieniu bidi albo wymaga w swojej
   specyfikacji, by obie strony ogłosiły możliwość.
5. Zmiana w trakcie połączenia (np. użytkownik wyłączył funkcję): wiadomość
   `Capabilities` z pełną nową listą tego, co strona przyjmuje.
   - **Wycofanie:** strona, która wycofuje możliwość, od chwili wysłania
     `Capabilities` pomija przychodzące wiadomości tego kanału i wysyła
     `STOP_SENDING` (`CANCELLED`) na jego strumieniach. Druga strona po
     odebraniu `Capabilities` przestaje wysyłać wiadomości tego kanału
     i kończy swój otwarty strumień przez `RESET_STREAM` (`CANCELLED`).
     Wiadomości w drodze w chwili wycofania przepadają. Strumień, który
     druga strona otworzy, zanim odbierze `Capabilities`, strona wycofująca
     odrzuca kodem `CAPABILITY_MISSING` (sekcja 3, zasada 3). Strumienia
     przyjętego przed wycofaniem nie dostarcza już nigdy, także gdy
     możliwość zostanie ponownie włączona, zanim przyszła jego kolej
     (sekcja 3.1, zasada 3).
   - **Ponowne włączenie:** druga strona po odebraniu `Capabilities` z tą
     możliwością otwiera **nowy** strumień kanału (sekcja 3.1); zakończony
     wcześniej strumień nie jest wznawiany.
   - Kanał `input` przy wycofaniu i ponownym włączeniu zaczyna nową sesję
     z wyzerowanym stanem, a odbiorca zwalnia wciśnięte przyciski
     (sekcje 8.2 i 8.4); zdarzenia z czasu wyłączenia nie trafiają
     do systemu.
   - Kanał `files` (WN-86): wycofanie zatrzymuje (`STOP_SENDING`,
     `CANCELLED`) strumienie wszystkich trwających transferów do tej
     strony; odbiorca kończy je i usuwa częściowe pliki, a nadawca widzi
     transfer jako anulowany przez drugą stronę. Po ponownym włączeniu
     nadawca zaczyna transfer od nowa (sekcja 9.1).
   - Kanał `actions` (WN-54): wycofanie działa jak dla `files`; prośba
     przyjęta przed wycofaniem, której odczyt jeszcze trwa, jest
     zatrzymana (`CANCELLED`), a nowe strumienie dostają
     `CAPABILITY_MISSING`.

Przykład asymetryczny: komputer ogłasza `["clipboard/1", "input/1"]`,
telefon `["notify/1"]`.

| kierunek | dozwolone kanały |
|---|---|
| telefon → komputer | `clipboard`, `input` (komputer je przyjmuje) |
| komputer → telefon | `notify` (telefon go przyjmuje) |

Telefon wysyła więc zdarzenia `input`, choć sam nie ogłasza `input/1`.
Gdyby komputer otworzył strumień `input` do telefonu, telefon odrzuci go
z kodem `CAPABILITY_MISSING`, bo sam nie ogłosił `input/1`.

## 7. Kanał `control` (`00`)

### 7.1 Przebieg połączenia

```text
klient                                          serwer
  ── QUIC + TLS 1.3 (mTLS, ALPN windconnect/1) ──
  open bidi: 01 00  Hello  ─────────────────────▶
                         ◀─────────────────────  HelloAck
  (od tej chwili obie strony otwierają kanały według możliwości)
  Ping ──────────────────▶  ◀────────────────── Pong
  ...
  Goodbye ───────────────▶  CONNECTION_CLOSE(GOING_AWAY)
```

1. Klient otwiera strumień `control` jako pierwszy strumień i wysyła `Hello`
   jako pierwszą ramkę.
2. Serwer odpowiada `HelloAck` jako pierwszą ramką na tym strumieniu.
3. Przed wysłaniem `HelloAck` serwer nie otwiera strumieni ani nie wysyła
   datagramów. Klient nie otwiera innych strumieni ani nie wysyła datagramów
   przed odebraniem `HelloAck`. Strumienie i datagramy, które przyjdą przed
   zakończeniem tej wymiany, strona MOŻE buforować do 64 KiB łącznie albo
   odrzucić (`STOP_SENDING`, `PROTOCOL_VIOLATION`).
4. Brak `Hello` w 5 s od ustanowienia połączenia (lub `HelloAck` w 5 s od
   wysłania `Hello`): zamknięcie z kodem `HELLO_TIMEOUT`.
5. Inna wiadomość niż `Hello` / `HelloAck` jako pierwsza ramka, drugi `Hello`
   albo `HelloAck` to `PROTOCOL_VIOLATION`.
6. Zakończenie strumienia `control` przez którąkolwiek stronę kończy
   połączenie (`PROTOCOL_VIOLATION`, o ile nie poprzedził go `Goodbye`).

Dokładne bajty początku strumienia `control` (stream ID 0) dla przykładów
z sekcji 7.3:

```text
klient → serwer:  01 00                      nagłówek: version 1, kanał control
                  31 82 00 a5 …              ramka Hello (50 B)
serwer → klient:  2c 82 01 a5 …              ramka HelloAck (45 B), bez nagłówka
```

### 7.2 Typy wiadomości `control`

| type | nazwa | kierunek |
|---|---|---|
| `0` | `Hello` | klient → serwer |
| `1` | `HelloAck` | serwer → klient |
| `2` | `Ping` | obie strony |
| `3` | `Pong` | obie strony |
| `4` | `Capabilities` | obie strony |
| `5` | `Error` | obie strony |
| `6` | `Goodbye` | obie strony |

### 7.3 `Hello` (0) i `HelloAck` (1)

Oba mają ten sam zestaw pól:

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `version` | tablica `[major, minor]` liczb naturalnych | wymagane |
| 1 | `name` | tekst, 1–64 B | wymagane; nazwa do wyświetlania |
| 2 | `kind` | liczba: 0 komputer, 1 telefon, 2 tablet | wymagane; nieznana wartość = 0 |
| 3 | `caps` | tablica tekstów (sekcja 6) | wymagane; może być pusta |
| 4 | `app` | tekst, 1–32 B | opcjonalne; wersja aplikacji |

Przykład `Hello` od komputera `SZYMON-PC` (wersja 1.0, możliwości
`clipboard/1` i `input/1`, aplikacja 0.5.0), 50 B:

```text
31 82 00 a5 00 82 01 00 01 69 53 5a 59 4d 4f 4e
2d 50 43 02 00 03 82 6b 63 6c 69 70 62 6f 61 72
64 2f 31 67 69 6e 70 75 74 2f 31 04 65 30 2e 35
2e 30
```

Rozbiór:

```text
31                         długość 49
82                         tablica [type, body]
   00                      type 0 = Hello
   a5                      mapa, 5 par
      00 82 01 00          0 version: [1, 0]
      01 69 53 5a…43       1 name: "SZYMON-PC"
      02 00                2 kind: 0 (komputer)
      03 82                3 caps: tablica 2
         6b 63 6c…2f 31       "clipboard/1"
         67 69 6e…2f 31       "input/1"
      04 65 30 2e 35 2e 30 4 app: "0.5.0"
```

Przykład `HelloAck` od telefonu `Pixel 9` (możliwości `input/1`,
`notify/1`), 45 B:

```text
2c 82 01 a5 00 82 01 00 01 67 50 69 78 65 6c 20
39 02 01 03 82 67 69 6e 70 75 74 2f 31 68 6e 6f
74 69 66 79 2f 31 04 65 30 2e 35 2e 30
```

### 7.4 `Ping` (2) i `Pong` (3)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `nonce` | liczba naturalna u64 | wymagane |

Na `Ping` odbiorca odpowiada niezwłocznie `Pong` z tym samym `nonce`.
`Ping` służy do pomiaru opóźnienia na poziomie aplikacji (np. `windctl ping`)
i nie zastępuje keep-alive QUIC (WC-12). Strona MOŻE zignorować `Ping`
ponad 10 na sekundę.

`Ping` z `nonce` 42, 7 B: `06 82 02 a1 00 18 2a`

`Pong` z `nonce` 42, 7 B: `06 82 03 a1 00 18 2a`

### 7.5 `Capabilities` (4)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `caps` | tablica tekstów (sekcja 6) | wymagane; pełna nowa lista |

Przykład (zostaje tylko `input/1`), 14 B:

```text
0d 82 04 a1 00 81 67 69 6e 70 75 74 2f 31
```

### 7.6 `Error` (5)

Błąd, który **nie** zamyka połączenia (np. nieznany typ wiadomości).
Błędy kończące połączenie lub strumień idą kodem QUIC (sekcja 10).

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `code` | liczba, kod z sekcji 10 | wymagane |
| 1 | `message` | tekst, ≤ 256 B | opcjonalne; dla logów, nie dla użytkownika |
| 2 | `channel` | liczba, kanał wiadomości, której dotyczy | opcjonalne |
| 3 | `type` | liczba, typ wiadomości, której dotyczy | opcjonalne |

Przykład: `UNKNOWN_MESSAGE` dla typu 9 na kanale `control`, 24 B:

```text
17 82 05 a4 00 06 01 6c 75 6e 6b 6e 6f 77 6e 20
74 79 70 65 02 00 03 09
```

Odebranie `Error` nie wywołuje odpowiedzi `Error` (brak pętli).

### 7.7 `Goodbye` (6)

Zapowiedź zamknięcia połączenia (wyjście z aplikacji, uśpienie,
rozparowanie). Po wysłaniu `Goodbye` strona zamyka połączenie ramką
`CONNECTION_CLOSE` z tym samym kodem; druga strona nie próbuje wtedy
natychmiast ponownie się połączyć.

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `code` | liczba, kod z sekcji 10 | wymagane |
| 1 | `message` | tekst, ≤ 256 B | opcjonalne |

Przykład: `GOING_AWAY`, 6 B: `05 82 06 a1 00 0d`

## 8. Kanał `input` (`04`)

Wymaga możliwości `input/1` po stronie **odbiorcy** zdarzeń. Zdarzenia
ciągłe (ruch wskaźnika, przewijanie) idą datagramami, bo retransmisja tylko
opóźniłaby obraz. Przejścia przycisków idą strumieniem `input`, bo ich
zgubienie zostawiłoby wciśnięty przycisk. Sekcje 8.2–8.6 opisują, jak
odbiorca łączy obie drogi w jeden spójny ciąg zdarzeń; sekcja 8.6 zbiera
wszystkie reguły w tabeli stanów.

### 8.1 Format datagramu

```text
datagram = version(1 B, 01) channel(1 B, 04) ładunek
```

`ładunek` to jeden element CBOR `[type, body]` jak w ramce, ale **bez** pola
długości (granicę wyznacza datagram). Datagram z innym `version`,
z kanałem innym niż `04` albo z błędnym ładunkiem odbiorca po cichu
odrzuca (nie zamyka połączenia).

### 8.2 Sesja wejścia

**Sesja** to czas życia jednego strumienia `input` nadawcy. Wszystkie
liczniki z sekcji 8.3 i 8.4 należą do sesji i zaczynają się od zera razem
z nią.

Nadawca:

- Pierwszą ramką każdego strumienia `input` MUSI być `InputSession`
  z numerem sesji `s`. Pierwsza sesja w połączeniu ma `s` = 0, każda
  następna o 1 więcej.
- Nową sesję (nowy strumień, sekcja 3.1) zaczyna tylko wtedy, gdy poprzedni
  strumień `input` zakończył, np. po wycofaniu i ponownym włączeniu
  `input/1` przez odbiorcę. Migracja ścieżki **nie** zmienia sesji:
  strumień i liczniki trwają dalej.
- Każdy datagram niesie numer sesji `s`.
- `InputSession` niesie też `g`: numer ogłoszenia możliwości odbiorcy, na
  podstawie którego nadawca otworzył sesję. Ogłoszenia odbiorcy są
  numerowane po kolei w strumieniu `control`: jego `Hello` / `HelloAck`
  ma numer 0, każda jego wiadomość `Capabilities` o 1 więcej. Nadawca
  liczy je przy odbiorze (strumień `control` jest uporządkowany) i podaje
  numer ostatniego odebranego ogłoszenia, które zawierało `input/1`.

Odbiorca prowadzi dla nadawcy numer bieżącej sesji `cur` (na początku
połączenia: brak) i jej stan:

| stan | znaczenie |
|---|---|
| **brak** | jeszcze żadna sesja nie została przyjęta |
| **żywa** | `InputSession` z `cur` przyjęta, strumień trwa, odbiorca ogłasza `input/1` |
| **zamknięta** | sesja `cur` się skończyła: koniec jej strumienia (FIN, `RESET_STREAM`), `STOP_SENDING` odbiorcy na nim, wycofanie `input/1` przez odbiorcę |

Do systemu trafiają wyłącznie zdarzenia **żywej** sesji `cur`. Zamknięcie
sesji jest ostateczne: żadne zdarzenie z `s` ≤ `cur` nie zostanie już
zastosowane, także w czasie przed przyjęciem następnej sesji. Datagramy
z `s` > `cur` (lub dowolnym `s`, gdy stan to „brak”) to sesja przyszła:
odbiorca je odkłada (sekcja 8.5), o ile ogłasza `input/1`; inaczej pomija.

Odbiorca pamięta `g_min`: numer swojego ogłoszenia, które ostatnio
**włączyło** `input/1` (0, gdy `input/1` było już w `Hello` / `HelloAck`).

Ramka `InputSession` (`s`, `g`) musi mieć `s` > `cur` (gdy `cur` istnieje);
inaczej to `PROTOCOL_VIOLATION` dla strumienia (`STOP_SENDING`). Poza tym
odbiorca ustawia `cur` = `s` i zeruje stan z sekcji 8.3 i 8.4
(`pointer` = `scroll` = (0, 0), brak `pointer_seq` i `scroll_seq`,
`epoch` = 0), a dalej:

- gdy ogłasza `input/1` i `g` ≥ `g_min`: sesja żywa, przegląd odłożonych
  datagramów (sekcja 8.5);
- w przeciwnym razie sesja jest **od razu zamknięta**, a odbiorca wysyła
  na strumieniu `STOP_SENDING` (`CANCELLED`). Tak kończy się sesja, którą
  nadawca otworzył przed wycofaniem `input/1`, a której ramki doszły
  dopiero po ponownym włączeniu: jej `g` jest mniejsze od `g_min`.

Ramki strumienia sesji zamkniętej odbiorca pomija.

### 8.3 Stan skumulowany zamiast przesunięć

Ruch i przewijanie nie są przesyłane jako przesunięcia (zgubienie `dx`
oznaczałoby trwałą utratę ruchu), tylko jako **stan skumulowany**: suma
wszystkich przesunięć od początku sesji.

- Nadawca prowadzi w sesji dwa liczniki par i64: `pointer` = (`x`, `y`)
  i `scroll` = (`x`, `y`). Każde zdarzenie ruchu lub przewijania niesie
  bieżącą wartość swojego licznika; `PointerButton` niesie oba.
- `seq` to licznik u64 nadawcy, wspólny dla wszystkich zdarzeń `input`
  sesji (datagramów i ramek strumienia oprócz `InputSession`): pierwsze
  zdarzenie sesji ma `seq` 1, każde nowe o 1 więcej.
- Odbiorca pamięta osobno dla ruchu i dla przewijania ostatnio zastosowany
  stan i jego `seq`: (`pointer_seq`, `pointer`) i (`scroll_seq`, `scroll`).
- **Zastosowanie** stanu ruchu z (`seq`, `x`, `y`): jeżeli `seq` ≤
  `pointer_seq`, stan jest spóźniony lub powtórzony i odbiorca go pomija.
  W przeciwnym razie przesuwa wskaźnik o (`x` − `pointer.x`,
  `y` − `pointer.y`) i zapisuje nowy stan. Przewijanie analogicznie,
  z (`scroll_seq`, `scroll`).
- Zgubiony datagram nie powoduje utraty ruchu: następny niesie sumę, więc
  odbiorca nadrabia całą różnicę naraz.
- Po ostatniej zmianie licznika (koniec gestu) nadawca POWINIEN powtórzyć
  ostatni datagram ruchu lub przewijania **bez zmian** (ten sam `seq`)
  dwa razy: po ok. 20 ms i ok. 100 ms. Jeżeli oryginał dotarł, powtórzenie
  zostanie pominięte; jeżeli zginął, powtórzenie dostarczy końcowy stan.
- Jednostka: 1/100 dp palca na ekranie nadawcy; przyspieszenie
  i przeliczenie na piksele robi odbiorca. Przewijanie: `y` dodatnie w dół,
  `x` dodatnie w prawo.
- `t` to znacznik czasu nadawcy w mikrosekundach z zegara monotonicznego,
  od dowolnego punktu; służy tylko do interpolacji ruchu. Zegary stron nie
  są synchronizowane: odbiorca porównuje wyłącznie różnice `t`.

Przykład utraty i przestawienia (bez przycisków; ruch w osi `x`, nadawca
przesuwa o +100, potem +1, potem przewija):

```text
nadawca wysyła:   seq 10 ruch x=100   seq 11 ruch x=101   seq 12 przewijanie y=240
odbiorca dostaje: seq 12 przewijanie y=240 → przewija o 240   (scroll_seq=12)
                  seq 11 ruch x=101        → wskaźnik +101    (pointer_seq=11)
                  seq 10 ruch x=100        → pominięte, 10 ≤ 11
wynik: wskaźnik +101, przewinięcie 240; gdyby datagram seq 10 zginął,
wynik byłby ten sam.
```

### 8.4 Przyciski i epoka

Przejścia przycisków idą strumieniem `input` (niezawodnie i po kolei),
a datagramy mogą je wyprzedzić albo się za nimi spóźnić. Dlatego każde
zdarzenie niesie **epokę** `b`: liczbę przejść przycisków wysłanych w tej
sesji do chwili wysłania zdarzenia, a każde przejście niesie **pełny** stan
ciągły nadawcy (`pointer` i `scroll`) z chwili przejścia.

1. `PointerButton` niesie `b` po swoim przejściu: pierwsze przejście w sesji
   ma `b` = 1, każde następne o 1 więcej. Odbiorca stosuje je w kolejności
   strumienia i sprawdza, że `b` = jego `epoch` + 1; inaczej to
   `PROTOCOL_VIOLATION` dla strumienia (`STOP_SENDING`).
2. Odbiorca stosuje `PointerButton` tak: najpierw stan `pointer` (`x`, `y`)
   i stan `scroll` (`sx`, `sy`) z `seq` przycisku (sekcja 8.3), potem
   wciska lub zwalnia przycisk, potem ustawia `epoch` = `b`, na końcu
   przegląda odłożone datagramy (sekcja 8.5).
3. Datagram ruchu lub przewijania żywej sesji z `b`:
   - `b` = `epoch`: stosowany od razu (sekcja 8.3),
   - `b` < `epoch`: pominięty. To bezpieczne dla obu klas: przejście
     z epoką > `b` zostało wysłane później i przyniosło pełny stan
     `pointer` i `scroll`, który zawiera ten datagram,
   - `b` > `epoch`: odkładany (sekcja 8.5), bo strumień nie dostarczył
     jeszcze przejść, które go poprzedzają.
4. Przy zamknięciu sesji (sekcja 8.2) odbiorca MUSI zwolnić wszystkie
   przyciski wciśnięte w tej sesji.

Przykład: przeciąganie, w którym datagram wyprzedza przyciski (lewy
przycisk, oś `x`, bez przewijania):

```text
nadawca wysyła:   seq 10 down x=100 b=1   (strumień)
                  seq 11 ruch x=200 b=1   (datagram)
                  seq 12 up   x=200 b=2   (strumień)
                  seq 13 ruch x=300 b=2   (datagram)
odbiorca dostaje: seq 11 ruch b=1 → epoch 0 < 1: odłożony
                  seq 13 ruch b=2 → zastępuje odłożony seq 11 (nowszy)
                  seq 10 down x=100 → wskaźnik na x=100, wciśnij, epoch=1;
                                      odłożony b=2 > 1: czeka dalej
                  seq 12 up x=200   → wskaźnik na x=200, puść, epoch=2;
                                      odłożony seq 13 b=2: wskaźnik na x=300
wynik: wciśnięcie w x=100, przeciągnięcie do x=200, puszczenie w x=200,
potem ruch do x=300 bez przycisku.
```

Przykład: przewijanie wyprzedzone przez przycisk:

```text
nadawca wysyła:   seq 1 przewijanie y=240 b=0          (datagram)
                  seq 2 down x=0 y=0 sx=0 sy=240 b=1   (strumień)
odbiorca dostaje: seq 2 down → scroll na sy=240 (przewija o 240), wciśnij,
                               epoch=1
                  seq 1 przewijanie b=0 → pominięte, 0 < 1; jego stan już
                               przyniósł przycisk
wynik: przewinięcie o 240 przed wciśnięciem, jak u nadawcy.
```

### 8.5 Odkładanie datagramów

Odbiorca trzyma na klasę (ruch, przewijanie) najwyżej jeden odłożony
datagram żywej sesji `cur` (z `b` > `epoch`) i najwyżej jeden na każdą
sesję przyszłą, dla co najwyżej 4 sesji przyszłych (przy piątej pomija
odłożone najstarszej z nich). W każdym miejscu zostaje datagram
o większym `seq`. Razem to najwyżej 10 datagramów. Jeden na miejsce
wystarcza, bo stan jest skumulowany, a stan z chwili każdego przejścia
przycisku niesie sam `PointerButton`.

**Przegląd** odłożonych datagramów (po przyjęciu `InputSession` i po każdym
zastosowanym `PointerButton`), dla każdego odłożonego datagramu:

- `s` < `cur`: pomiń,
- `s` = `cur` i `b` < `epoch`: pomiń,
- `s` = `cur` i `b` = `epoch`: zastosuj (sekcja 8.3),
- `s` = `cur` i `b` > `epoch`: zostaw w miejscu żywej sesji,
- `s` > `cur`: zostaw w miejscu sesji przyszłej.

Przy zamknięciu sesji odbiorca pomija odłożone datagramy z `s` ≤ `cur`;
odłożone z sesji przyszłej zostają (o ile odbiorca nadal ogłasza
`input/1`). Gdy odbiorca wycofuje `input/1`, pomija wszystkie odłożone
datagramy. Strumień `input` jest niezawodny, więc odłożony datagram czeka
najwyżej tyle, ile dojście brakujących ramek.

Przykład: datagram nowej sesji przed jej `InputSession` (sesja 0 zamknięta,
`cur` = 0):

```text
nadawca: nowy strumień 01 04 + InputSession s=1 g=2, potem datagram ruchu
         s=1 seq 1 x=1 b=0 i jego dwa powtórzenia
odbiorca: datagram s=1 (i powtórzenia) → 1 > cur: odłożony (sesja przyszła)
          InputSession s=1 g=2 → g ≥ g_min=2: cur=1, żywa, stan zerowy; przegląd:
                             s=1, b=0 = epoch → wskaźnik +1
wynik: ruch +1 dochodzi, choć wszystkie kopie datagramu wyprzedziły
ramkę InputSession.
```

Przykład: datagram zamkniętej sesji po ponownym włączeniu, przed nową
`InputSession`:

```text
sesja 0 żywa (g=0): odbiorca zastosował pointer x=100 (seq 7)
odbiorca wycofuje input/1 (Capabilities nr 1) → sesja 0 zamknięta;
  nadawca wysłał już x=150 (s=0, seq 8, b=0)
odbiorca włącza input/1 (Capabilities nr 2) → g_min=2;
  InputSession s=1 jeszcze nie doszła
  datagram x=150 (s=0) dochodzi → s=0 ≤ cur=0, sesja zamknięta: pominięty
  (tak samo, gdyby doszedł przy wyłączonym input/1 albo po InputSession s=1)
nadawca: InputSession s=1 g=2, użytkownik przesuwa o +1 → datagram x=1 (s=1)
  odbiorca: stan zerowy, x=1 → wskaźnik +1
wynik: do systemu trafia tylko +1; +50 z czasu wyłączenia przepadło.
```

Przykład: sesja otwarta przed wycofaniem, której ramki doszły dopiero po
ponownym włączeniu (`cur` brak):

```text
nadawca otwiera sesję 0 (g=0) i wysyła ruch; ramki i datagramy się opóźniają
odbiorca wycofuje input/1 (nr 1), potem włącza (nr 2) → g_min=2
  datagramy s=0 dochodzą → stan „brak”: odłożone jako sesja przyszła
  InputSession s=0 g=0 dochodzi → g=0 < 2: cur=0, zamknięta, STOP_SENDING;
                                   odłożone s=0 pominięte
nadawca, po odebraniu ogłoszenia nr 2, otwiera sesję 1 (g=2): żywa
wynik: nic z sesji 0 nie trafia do systemu po ponownym włączeniu.
```

### 8.6 Tabela stanów odbiorcy

Stan odbiorcy dla jednego nadawcy: `cur` i stan sesji (sekcja 8.2),
`epoch`, (`pointer_seq`, `pointer`), (`scroll_seq`, `scroll`), odłożone
datagramy (sekcja 8.5), wciśnięte przyciski. „Ogłasza” znaczy: odbiorca
ogłasza `input/1`.

| zdarzenie | warunek | działanie |
|---|---|---|
| datagram (`s`, `b`) | nie ogłasza | pomiń |
| | stan „brak” albo `s` > `cur` | odłóż jako sesję przyszłą |
| | `s` < `cur`, albo `s` = `cur` i sesja zamknięta | pomiń |
| | `s` = `cur`, żywa, `b` < `epoch` | pomiń |
| | `s` = `cur`, żywa, `b` = `epoch` | zastosuj (8.3) |
| | `s` = `cur`, żywa, `b` > `epoch` | odłóż w miejscu żywej sesji |
| ramka `InputSession` (`s`, `g`) | `cur` istnieje i `s` ≤ `cur` | `STOP_SENDING` `PROTOCOL_VIOLATION` |
| | nie ogłasza albo `g` < `g_min` | `cur` = `s`, stan zerowy, sesja zamknięta, `STOP_SENDING` `CANCELLED` |
| | poza tym | `cur` = `s`, żywa, stan zerowy, przegląd (8.5) |
| ramka strumienia sesji zamkniętej | | pomiń |
| ramka `PointerButton` `b` | `b` ≠ `epoch` + 1 | `STOP_SENDING` `PROTOCOL_VIOLATION` |
| | poza tym | zastosuj `pointer` i `scroll`, przejście, `epoch` = `b`, przegląd |
| ramka `PointerMotion` / `Scroll` (strumień, 8.7) | | zastosuj (8.3) |
| koniec strumienia, `STOP_SENDING` odbiorcy | sesja żywa | zamknij: zwolnij przyciski, pomiń odłożone z `s` ≤ `cur` |
| odbiorca wycofuje `input/1` | | zamknij jak wyżej i pomiń wszystkie odłożone |
| odbiorca włącza `input/1` | | `g_min` = numer tego ogłoszenia |
| koniec połączenia | | zwolnij przyciski, wyczyść wszystko |

Ramki ze strumienia `input` dotyczą zawsze jego sesji, która jest żywa,
dopóki strumień trwa (sekcja 3.1 gwarantuje, że ramki nowego strumienia
przychodzą po końcu poprzedniego).

### 8.7 Datagramy albo strumień

Droga zdarzeń ciągłych jest ustalona **raz na połączenie**:

- Gdy w połączeniu datagramy są dostępne (obie strony ogłosiły
  `max_datagram_frame_size`), `PointerMotion` i `Scroll` idą **wyłącznie**
  datagramami.
- Gdy nie są dostępne, idą **wyłącznie** strumieniem `input`; wtedy
  wszystkie zdarzenia są w kolejności strumienia, epoka zawsze się zgadza
  i nic nie jest odkładane.

### 8.8 Typy wiadomości `input`

| type | nazwa | droga |
|---|---|---|
| `0` | `PointerMotion` | datagram albo strumień (sekcja 8.7) |
| `1` | `Scroll` | datagram albo strumień (sekcja 8.7) |
| `2` | `PointerButton` | strumień |
| `3` | `InputSession` | strumień, pierwsza ramka |
| `4` | `Gesture` | strumień; tylko gdy odbiorca ogłasza input/2 |

#### `PointerMotion` (0)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `seq` | u64 | wymagane |
| 1 | `x` | i64, skumulowany | wymagane |
| 2 | `y` | i64, skumulowany | wymagane |
| 3 | `t` | u64, µs | opcjonalne |
| 4 | `s` | u32, sesja | wymagane |
| 5 | `b` | u64, epoka | wymagane |

Przykład datagramu: `seq` 1, `x` −350, `y` 120, `t` 5 000 123, sesja 0,
epoka 0, 24 B:

```text
01 04 82 00 a6 00 01 01 39 01 5d 02 18 78 03 1a
00 4c 4b bb 04 00 05 00
```

Ruch w trakcie przeciągania (po `PointerButton` z przykładu niżej): `seq` 3,
`x` −310, `y` 95, sesja 0, epoka 1, bez `t`, 18 B:

```text
01 04 82 00 a5 00 03 01 39 01 35 02 18 5f 04 00
05 01
```

#### `Scroll` (1)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `seq` | u64 | wymagane |
| 1 | `x` | i64, skumulowany | wymagane; w poziomie |
| 2 | `y` | i64, skumulowany | wymagane; w pionie, dodatnie w dół |
| 3 | `t` | u64, µs | opcjonalne |
| 4 | `s` | u32, sesja | wymagane |
| 5 | `b` | u64, epoka | wymagane |

Przykład datagramu: `seq` 4, `x` 0, `y` 2400, sesja 0, epoka 1, 17 B:

```text
01 04 82 01 a5 00 04 01 00 02 19 09 60 04 00 05
01
```

#### `PointerButton` (2)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `seq` | u64 | wymagane |
| 1 | `button` | liczba: 0 lewy, 1 prawy, 2 środkowy | wymagane; nieznany: tylko stan i epoka |
| 2 | `down` | bool | wymagane; `true` wciśnięty |
| 3 | `x` | i64, stan `pointer` nadawcy | wymagane |
| 4 | `y` | i64, stan `pointer` nadawcy | wymagane |
| 5 | `b` | u64, epoka po tym przejściu | wymagane |
| 6 | `sx` | i64, stan `scroll` nadawcy | wymagane |
| 7 | `sy` | i64, stan `scroll` nadawcy | wymagane |

Przykład ramki: `seq` 2, lewy wciśnięty w punkcie `x` −350, `y` 120,
epoka 1, przewijanie (0, 0), 23 B:

```text
16 82 02 a8 00 02 01 00 02 f5 03 39 01 5d 04 18
78 05 01 06 00 07 00
```

#### `InputSession` (3)

| klucz | nazwa | typ | |
|---|---|---|---|
| 0 | `s` | u32, numer sesji | wymagane |
| 1 | `g` | u64, numer ogłoszenia odbiorcy (sekcja 8.2) | wymagane |

Przykład: początek pierwszego strumienia `input` w połączeniu: nagłówek
i `InputSession` z `s` 0, `g` 0 (`input/1` z `HelloAck`), razem 10 B:

```text
01 04 07 82 03 a2 00 00 01 00
```

### 8.9 Gesty Windows (`input/2`, WN-51)

CCS/1 COMPAT:input/2 rozszerza input/1 na tym samym kanale 04; format datagramów pozostaje identyczny. Samo input/1 pozwala na ruch, przewijanie i przyciski. Nadawca MUSI sprawdzić bieżące input/2 przed wysłaniem Gesture; wycofanie input wycofuje obie wersje i kończy sesję, zwalniając przyciski.

`Gesture` (type 4) ma wymagane pola CBOR:

| klucz | nazwa | typ |
|---|---|---|
| 0 | seq | u64 |
| 1 | gesture | u64: 0 TaskView (Win+Tab), 1 ShowDesktop (Win+D), 2 PreviousApp (Alt+Shift+Tab), 3 NextApp (Alt+Tab) |
| 2 | x | i64, skumulowany pointer |
| 3 | y | i64, skumulowany pointer |
| 4 | b | u64, epoka po geście |
| 5 | sx | i64, skumulowany scroll |
| 6 | sy | i64, skumulowany scroll |

CCS/1 RULE:Gesture jest barierą jak PointerButton (§8.4): następna epoka dokładnie b+1, najpierw pełny stan pointer/scroll, potem pojedynczy gest, potem odłożone datagramy nowej epoki. Nieznany kod gesture aktualizuje stan i epokę bez działania systemowego. Znany type 4 bez obsługi input/2 lub błędna epoka powoduje PROTOCOL_VIOLATION. Starszy program, który nie zna type 4, pomija nieznaną ramkę zgodnie z §4.4; nie potrafi jednak zaktualizować epoki, dlatego nadawca NIE MOŻE wysyłać gestu do input/1. Gesty nie są dozwolone w datagramach ani w actions/1.

CCS/1 EXT:WN-52 może dodać klawisze/tekst przez osobną capability (np. keys/1) lub input/3 na kanale 04, z tymi samymi regułami seq/epoch i niezawodnych barier; ta zmiana nie rezerwuje nowego numeru kanału. Tekst/klawisze nie należą do input/2.

CCS/1 IO:odbiorca PC łączy ruch przez najwyżej jedną klatkę (8,333 ms), bez przewidywania ruchu; bariera opróżnia wcześniejszy ruch. Koniec sesji usuwa oczekujący ruch i zwalnia przyciski. SendInput ogranicza ruch do ±4096 px i koło do ±1200 jednostek/klatkę; gest jest jedną tablicą wszystkich key-down/key-up. Częściowa odmowa zwalnia klawisze, wycofuje input i wymaga ponownego lokalnego włączenia. Secure desktop/UIPI nie są omijane.

## 9. Kanały funkcji i zmiany specyfikacji

Kanały `clipboard`, `notify`, `media` i `files` mają w v1.0 tylko numer,
możliwość i zasady strumieni (sekcja 3). Typy ich wiadomości dopisują
karty funkcji (WC-35, WC-38, WC-42 (§9.7), WC-46 i dalsze) jako zmiany tego
dokumentu:

1. Nowy typ lub pole opcjonalne: kolejna wersja minor (1.1, 1.2, …),
   nowy wpis w tabeli typów kanału, przykład binarny jak w sekcjach 7–8.
2. Zmiana znaczenia istniejącego typu lub pola: nowa wersja możliwości
   (`clipboard/2`), a dla ramkowania nowa wersja główna.
3. Numery typów i kluczy nie są nigdy używane ponownie, także po wycofaniu.
4. Zasady wspólne: wiadomości kanału uni są jednokierunkowe, a odpowiedzi
   podlegają sekcji 6, zasada 4. Strumień `files` jest dwukierunkowy:
   otwiera go nadawca pliku (nagłówek `01 05`, potem wiadomości sekcji
   9.1), odbiorca pisze w kierunku powrotnym odpowiedzi bez nagłówka, a
   koniec transferu to FIN obu stron. Strumień `files` przyjmuje tylko
   strona, która ogłosiła `files/1`. Tak samo `actions` (§9.2, nagłówek
   `01 06`), który przyjmuje tylko strona z `actions/1`, i `sms` (§9.11,
   nagłówek `01 0d`), który przyjmuje tylko strona z `sms/1`.

### 9.1 Kanał `files` (`05`, WC-38)

Wiadomości `files/1`. Kanał nie miał wcześniej typów wiadomości, więc
wersjonuje je sama możliwość `files/1` (sekcja 9, zasada 2), bez zmiany
wersji minor protokołu.

**Transfer** to jeden plik. Nadawca wybiera losowy `id` (16 B) i dzieli
plik na **fragmenty** o stałym rozmiarze `chunk` (ostatni może być krótszy).
Fragment `i` zaczyna się w bajcie `i × chunk`. Transfer używa od 1 do
`streams` strumieni `files` naraz; każdy fragment może pójść dowolnym z nich.
Folder to osobny transfer dla każdego pliku, ze ścieżką względną.

Od nadawcy pliku (typy kanału w tym kierunku):

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Offer` | `0 id`: bajty, 16 B; `1 path`: tekst, 1–1024 B; `2 size`: uint; `3 chunk`: uint; `4 streams`: uint 1–8; `5 modified`: uint, opcjonalne (ms od epoki Uniksa) |
| 1 | `Join` | `0 id`: bajty, 16 B; `1 stream`: uint 1–7 |
| 2 | `Chunk` | `0 index`: uint; `1 data`: bajty; `2 sha256`: bajty, 32 B |

Od odbiorcy pliku (kierunek powrotny tych samych strumieni):

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Accept` | `0 have`: bajty, mapa bitowa fragmentów, które odbiorca już ma |
| 1 | `Ack` | `0 index`: uint; fragment sprawdzony i zapisany |
| 2 | `Bad` | `0 index`: uint; suma kontrolna się nie zgadza |
| 3 | `Complete` | pusta mapa; plik jest pod nazwą docelową |

Przebieg:

1. Pierwszy strumień transferu zaczyna się od `Offer`. `chunk` to
   wielokrotność 65 536 w zakresie 65 536–524 288 B, a liczba fragmentów
   `⌈size / chunk⌉` nie przekracza 2²² (mapa `have` ma najwyżej 512 KiB).
   Wartości spoza zakresu to `MALFORMED_FRAME`.
2. Odbiorca odpowiada `Accept`. W `have` fragment `i` to bajt `i / 8`,
   bit `0x80 >> (i mod 8)`; pusta mapa znaczy „nic nie mam”. Odmowa to
   `STOP_SENDING` i `RESET_STREAM` pierwszego strumienia: zła ścieżka
   `MALFORMED_FRAME`, za mało miejsca `NO_SPACE`, za dużo transferów
   `RATE_LIMITED`, odmowa użytkownika `CANCELLED`.
3. Po `Accept` nadawca może otworzyć do `streams − 1` dalszych strumieni;
   każdy zaczyna się od `Join` z `id` i numerem strumienia (każdy numer raz).
   `Join`, którego `Offer` nie doszedł w ciągu 5 s, odbiorca zatrzymuje
   kodem `CANCELLED`. Czekające strumienie liczą się do limitów odbiorcy
   (zalecane: 32 naraz, ponad limit od razu `RATE_LIMITED`).
4. Nadawca wysyła `Chunk` każdego fragmentu, którego nie ma w `have`.
   `data` ma dokładnie długość fragmentu, a `index` jest mniejszy niż
   liczba fragmentów; inaczej `MALFORMED_FRAME`. Odbiorca liczy SHA-256
   `data`: zgodny fragment zapisuje i odpowiada `Ack` na tym samym
   strumieniu, niezgodny odrzuca i odpowiada `Bad`. Nadawca wysyła wtedy
   fragment ponownie, najwyżej 3 razy, potem kończy transfer.
5. Gdy odbiorca ma wszystkie fragmenty, zapisuje plik trwale, nadaje mu
   nazwę docelową i dopiero wtedy wysyła `Complete` na pierwszym strumieniu
   oraz FIN na wszystkich. Nadawca uznaje transfer za skończony dopiero po
   `Complete` (nie po ostatnim `Ack`) i odpowiada FIN.
6. Anulowanie przez którąkolwiek stronę: `RESET_STREAM` i `STOP_SENDING`
   z kodem `CANCELLED` na wszystkich strumieniach transferu. Odbiorca usuwa
   wtedy częściowy plik.
7. Wycofanie `files/1` (sekcja 6, zasada 5) działa jak anulowanie przez
   odbiorcę każdego jego trwającego transferu; nowe strumienie `files`
   w czasie wycofania dostają `CAPABILITY_MISSING`. Taki transfer nie jest
   „anulowany przez użytkownika”, więc ponowiony (ten sam `id`) po ponownym
   włączeniu zostaje przyjęty.

**Wznowienie.** Zerwanie połączenia nie kończy transferu. Nadawca po
ponownym połączeniu wysyła ten sam `Offer` (ten sam `id`), a odbiorca w
`have` podaje fragmenty, które ma trwale zapisane; przesyłane są tylko
pozostałe. Odbiorca przechowuje częściowy plik osobno dla każdego
urządzenia, pod parą (odcisk nadawcy, `id`), bo `id` wybiera druga strona,
i to **poza** folderem docelowym (np. w katalogu stanu aplikacji), tak by
żadna `path` od drugiej strony nie sięgała do częściowych plików ani ich
stanu.
Bit fragmentu trafia do zapisanego stanu dopiero po trwałym zapisie jego
danych (najpierw synchronizacja pliku, potem atomowa podmiana stanu), więc
po awarii zasilania wznowienie nie pominie niezapisanego fragmentu. Stan
zawiera `size`, `chunk` i `modified` z `Offer`; wznowienie wymaga zgodności
wszystkich trzech, inaczej odbiorca zaczyna od nowa z pustym `have`.
Częściowe pliki porzucone (bez zmian przez 7 dni, zalecane) odbiorca usuwa.

**Brak skrótu całego pliku.** Każdy bajt pliku przychodzi w fragmencie,
którego SHA-256 odbiorca sprawdza przed zapisem i zaznaczeniem, a trwałość
stanu idzie po trwałości danych. Skrót całego pliku po stronie nadawcy
wymagałby drugiego odczytu i nie wykryłby zmiany źródła między odczytami.
Zamiast tego nadawca przed wysłaniem ostatniego fragmentu (i co 64
fragmenty) sprawdza, czy rozmiar i czas modyfikacji źródła są takie jak w
`Offer`; zmienione źródło kończy transfer kodem `CANCELLED`.

**Ścieżki.** `path` pochodzi od drugiej strony, więc odbiorca sprawdza ją
sam, niezależnie od systemu nadawcy, i niczego nie poprawia; ścieżka
łamiąca którąkolwiek regułę to `MALFORMED_FRAME`, bez tworzenia pliku:

- separatorem jest wyłącznie `/`; `\` w ścieżce jest niedozwolony;
- części puste, `.` i `..` są niedozwolone (także wiodący `/`, czyli
  ścieżka absolutna); najwyżej 32 części, każda do 255 B UTF-8;
- niedozwolone znaki: `:` (litery dysków, strumienie NTFS `plik:strumień`),
  `<>"|?*`, znaki sterujące (Cc) i znaki kierunku pisma (U+061C, U+200E,
  U+200F, U+202A–U+202E, U+2066–U+2069);
- niedozwolone nazwy urządzeń Windows: `CON`, `PRN`, `AUX`, `NUL`,
  `COM1`–`COM9`, `LPT1`–`LPT9` (także z cyframi ¹²³), bez względu na
  wielkość liter i rozszerzenie (`nul.txt`), oraz część kończąca się
  kropką lub spacją (Windows je obcina).

Odbiorca tworzy podfoldery w swoim folderze docelowym po jednym i odrzuca
każdy, który jest dowiązaniem lub punktem połączenia (junction); ścieżka
docelowa musi leżeć w folderze docelowym. Istniejącego pliku nigdy nie
nadpisuje: przy kolizji nazwy dodaje „ (1)”, „ (2)”… przed rozszerzeniem,
rezerwując nazwę atomowo (utworzenie nowego pliku tylko wtedy, gdy go nie
ma), więc dwa transfery kończące się naraz dostają różne nazwy.

**Znacznik pochodzenia (Windows, WN-96).** Plik od drugiego urządzenia
może być programem albo dokumentem z makrami, więc odbiorca w Windows
dopisuje mu strumień NTFS `<plik>:Zone.Identifier` (Mark-of-the-Web), tak
jak przeglądarka. Robi to przed nadaniem nazwy docelowej (zmiana nazwy w
NTFS zachowuje strumienie), więc plik nigdy nie leży w folderze docelowym
bez znacznika; po kopiowaniu z innego woluminu dopisuje go jeszcze raz:

```
[ZoneTransfer]
ZoneId=3
HostUrl=windconnect:<odcisk nadawcy, 64 znaki hex>
```

z końcami linii CRLF. `ZoneId=3` (strefa Internet) sprawia, że SmartScreen
i Widok chroniony pakietu Office ostrzegają przed otwarciem. Zapis jest
najlepszym staraniem: wolumin bez strumieni (FAT32, exFAT, część udziałów
sieciowych) albo inny błąd zapisu niczego nie zmienia, transfer kończy się
`Complete` jak zwykle. Znacznik nie należy do protokołu (nie idzie przez
sieć); w Androidzie go nie ma.

**Miejsce.** Przed `Accept` odbiorca sprawdza wolne miejsce na brakującą
część pliku z zapasem (zalecane 64 MiB); za mało to `NO_SPACE`. Nadawca nie
ponawia takiego transferu sam.

Przykłady (ramki z długością):

| wiadomość | bajty |
|---|---|
| `Offer` id `11`×16, `a/b.txt`, 300 000 B, chunk 262 144, 4 strumienie | `2c 82 00 a5 00 50 11…11 01 67 61 2f 62 2e 74 78 74 02 1a 00 04 93 e0 03 1a 00 04 00 00 04 04` |
| `Join` id `11`×16, strumień 1 | `17 82 01 a2 00 50 11…11 01 01` |
| `Chunk` 1, `hi`, sha256 zera (przykład kodowania) | `2c 82 02 a3 00 01 01 42 68 69 02 58 20 00…00` |
| `Accept` have `80` | `06 82 00 a1 00 41 80` |
| `Ack` 1 | `05 82 01 a1 00 01` |
| `Bad` 1 | `05 82 02 a1 00 01` |
| `Complete` | `03 82 03 a0` |

### 9.2 Kanał `actions` (`06`, WN-54)

Akcje zdalne: urządzenie prosi drugie o zablokowanie ekranu, uśpienie,
otwarcie linku albo uruchomienie akcji, którą użytkownik **drugiego**
urządzenia sam dodał do swojej listy dozwolonych. Wiadomości `actions/1`;
jak `files/1` wersjonuje je sama możliwość, bez zmiany wersji minor.

**Zasada nadrzędna.** Prośba wskazuje akcję wyłącznie identyfikatorem
(i adresem dla `open-url`). Protokół nie ma pola na polecenie, program,
ścieżkę ani argumenty: co robi akcja własna, ustala tylko użytkownik
urządzenia, które ją wykonuje, na tym urządzeniu. Strona NIE MOŻE wykonać
niczego, czego nie ma na jej własnej liście dozwolonych w chwili
wykonania.

**Strumień.** Jedna prośba to jeden strumień bidi otwarty przez
proszącego: nagłówek `01 06`, jedna wiadomość, FIN. Wykonujący odpowiada
jedną wiadomością w kierunku powrotnym (bez nagłówka) i FIN. Ramki na tym
kanale mają limit 16 384 B (§4.2).

Od proszącego:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Run` | `0 action`: tekst, identyfikator akcji; `1 url`: tekst 1–2048 B, opcjonalne, tylko dla `open-url` |
| 1 | `List` | pusta mapa; jakie akcje strona wykonałaby teraz |

Od wykonującego:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Result` | `0 status`: uint, wynik `Run` (tabela niżej) |
| 1 | `Actions` | `0 actions`: tablica do 64 par `[id, label]`; `label` tekst 1–64 B, do wyświetlania |

**Identyfikatory.** 1–64 bajtów z `a-z`, `0-9` i `-`; inny znak to
`MALFORMED_FRAME`. Wbudowane: `lock` (blokada ekranu), `sleep`
(uśpienie), `open-url` (otwarcie adresu). Akcje własne zaczynają się od
`x-`, więc nigdy nie zasłonią wbudowanej. Strona, która czegoś nie
potrafi (telefon nie usypia komputera), nie oferuje tego w `Actions` i
odpowiada na `Run` statusem `1`.

| `status` | znaczenie |
|---|---|
| 0 | wykonano (dla `sleep`: uśpienie zaplanowane, odpowiedź wychodzi przed nim) |
| 1 | odmowa: akcji nie ma na liście dozwolonych, jest wyłączona albo niedostępna |
| 2 | zły argument: brak lub niedozwolony `url`, albo `url` przy innej akcji niż `open-url` |
| 3 | dozwolona, ale wykonanie się nie udało |
| 4 | zajęte: za dużo próśb w krótkim czasie |

Nieznany `status` proszący traktuje jak `3`.

**Kolejność sprawdzeń u wykonującego:**

1. Strumień przyjmuje tylko strona, która ogłosiła `actions/1` (§3,
   zasada 3); połączenie istnieje tylko z przypiętym kluczem (§2).
2. Prośba musi przyjść w 5 s od otwarcia strumienia (inaczej
   `CANCELLED`). Błędna wiadomość: `MALFORMED_FRAME`, nieznany typ:
   `UNKNOWN_MESSAGE`; w obu przypadkach `STOP_SENDING` i `RESET_STREAM`,
   bez odpowiedzi i bez wykonania.
3. **Przed każdą odpowiedzią** strona ponownie sprawdza, że klucz
   drugiej strony jest nadal przypięty. Jeśli nie (rozparowanie w trakcie
   połączenia), zamyka połączenie kodem `UNTRUSTED_PEER` i niczego nie
   wykonuje.
4. Lista dozwolonych jest odczytywana w chwili wykonania, nie przy
   połączeniu: wyłączenie akcji działa od razu. Lista, której nie da się
   odczytać, oznacza, że nic nie jest dozwolone.
5. `open-url` przyjmuje tylko adres bezwzględny `http://` lub `https://`
   (wielkość liter schematu dowolna): drukowalne ASCII bez spacji i bez
   znaków `"` `<` `>` `\` `^` `` ` `` `{` `|` `}`, niepusty host, bez
   części `użytkownik@`, port (jeśli jest) 1–65535. Każdy inny adres
   (`file:`, `javascript:`, linki aplikacji, adresy względne, znaki spoza
   ASCII; nazwy międzynarodowe tylko w postaci `xn--`) to status `2`;
   strona go nie poprawia.
6. Naraz wykonuje się najwyżej jedna akcja na połączenie, najwyżej 10 w
   ciągu 10 s (ponad to status `4`); najwyżej 4 prośby czekają, kolejne
   strumienie dostają `RATE_LIMITED`.
7. Akcja własna uruchamia program wskazany przez użytkownika tego
   urządzenia z jego stałymi argumentami, bez powłoki systemowej (na
   Windowsie tylko plik `.exe`, nie skrypt `.bat`/`.cmd`).
8. **Telefon jako wykonujący (WN-100).** Telefon ogłasza `actions/1` i
   oferuje wyłącznie `open-url` (nie blokuje, nie usypia, nie ma akcji
   własnych), dopóki jego użytkownik ma włączone „Otwieraj linki z
   komputera”; wyłączone daje status `1`. Adres po sprawdzeniu z zasady 5
   otwiera się od razu, gdy aplikacja jest na ekranie; w tle Android nie
   pozwala uruchomić aktywności, więc telefon pokazuje powiadomienie do
   stuknięcia i odpowiada `0` (link podany osobie).

Przykłady (ramki z długością):

| wiadomość | bajty |
|---|---|
| `Run` `lock` | `09 82 00 a1 00 64 6c 6f 63 6b` |
| `Run` `open-url`, `https://a.pl/` | `1c 82 00 a2 00 68 6f 70 65 6e 2d 75 72 6c 01 6d 68 74 74 70 73 3a 2f 2f 61 2e 70 6c 2f` |
| `List` | `03 82 01 a0` |
| `Result` 0 (wykonano) | `05 82 00 a1 00 00` |
| `Result` 1 (odmowa) | `05 82 00 a1 00 01` |
| `Actions` `[["lock", "Zablokuj"]]` | `14 82 01 a1 00 81 82 64 6c 6f 63 6b 68 5a 61 62 6c 6f 6b 75 6a` |

### 9.3 Kanał `clipboard` (`01`, WN-35)

CCS/1
PURPOSE:tekst i linki jako zwykły UTF-8, w obie strony przez istniejącą sesję mTLS/E2E. Link nie jest poleceniem otwarcia URL. Pierwsze wiadomości możliwości `clipboard/1`, jak początkowe `files/1`, bez zmiany ALPN/ramkowania ani wersji minor.
RULE:obie strony ogłaszają `clipboard/1`; odpowiedź idzie osobnym strumieniem uni w przeciwnym kierunku, zgodnie z §6. Brak capability blokuje wysłanie.

| typ | nazwa | pola mapy body |
| --- | --- | --- |
| 0 | Set | `0`: losowy identyfikator 16 B (bytes); `1`: dodatni licznik Lamporta uint64; `2`: tekst UTF-8 (text), 0–262144 B |
| 1 | Applied | `0`: identyfikator Set 16 B; `1`: wynik uint: 0 applied, 1 refused, 2 failed |

Walidacja: odrzucić nieprawidłowy typ/rozmiar ID, zerowy/ujemny licznik,
NUL w tekście, nadmiarowy rozmiar, nieprawidłowe wartości i powtórzone klucze.
Nieznane opcjonalne pola są pomijane; nieznany typ/niepoprawna wiadomość
nie dotyka schowka i nie dostaje fałszywego potwierdzenia.
Odpowiedź **applied** powstaje dopiero po udanym zapisie w systemowym API,
**refused** przy wyłączonym odbiorze lub starszej/zdublowanej zmianie,
**failed** gdy zapis OS się nie udał. Nadawca ma do 4 oczekujących odpowiedzi,
każda z terminem 3 s i anulowaniem po zamknięciu sesji. Nieznane/stare ID
odpowiedzi nie zmieniają stanu. Brak odpowiedzi oznacza brak potwierdzenia,
nigdy sukces. Queue routera ma 4 wiadomości; nie ma historii ani kolejki offline.

Kolejność i echo: aktywna sesja utrzymuje w pamięci licznik i ostatni klucz
`(licznik, id)`, a proces SHA-256 ostatniej wysłanej/odebranej treści. Lokalna zmiana
zwiększa licznik; odbiór podnosi go do maksimum własnego i otrzymanego.
Klucze porównuje się leksykograficznie (id jako 16 bajtów bez znaku).
Set z kluczem mniejszym lub równym ostatniemu zostaje odrzucony. Przy
równoczesnych zmianach o tym samym liczniku wygrywa większe losowe id;
obie strony zbiegają do tej wartości bez porównywania zegarów ściennych.
Nieudany zapis też przesuwa klucz, ale nie hash zastosowanej treści.
Automat nie wysyła powtórnie tego samego hasha przy callbacku własnego
zapisu lub powrocie fokusu. Ręczna próba może ponowić taką samą treść.
Reconnect nie odtwarza starej zawartości. Adoptowana nowa sesja zaczyna licznik
od0 i usuwa stary klucz, aby restart drugiej strony nie zablokował jej zmian.
Hash pozostaje. Zmiana generacji kończy stary odbiór i oczekujące odpowiedzi.
Android dodatkowo wymaga timestamp schowka późniejszego niż przyjęcie sesji
(ClipDescription.timestamp, lokalny System.currentTimeMillis); zawartość
sprzed sesji/restartu procesu nie wychodzi po odzyskaniu fokusu. Nieznany,
równy granicy timestamp jest pomijany; po cofnięciu zegara
może być potrzebne ręczne wysłanie. Granica nie wymaga zapisu payloadu
ani porównywania zegarów urządzeń. Ręczna akcja omija wyłącznie tę granicę,
nie sprawdzanie treści wrażliwych.
Spóźnione Set ze starej sesji nie mogą zmieniać OS. Zapasowe połączenie
próbujące powrotu z relay na LAN nie jest adoptowane do tej funkcji przed
decyzją właściciela połączenia. Nie istnieje trwały log schowka.

Prywatność nadawcy (także ręcznego): Windows nie odczytuje/wysyła payloadu,
gdy jest format `ExcludeClipboardContentFromMonitorProcessing`, albo
`CanIncludeInClipboardHistory`/`CanUploadToCloudClipboard` zawiera DWORD 0
(uszkodzone dane opt-out traktuje zachowawczo jako prywatne). Android
nie wysyła treści z `ClipDescription.extras[android.content.extra.IS_SENSITIVE]`.
Windows usuwa terminator CF_UNICODETEXT przed walidacją; HTML, obrazy,
pliki, intenty i URI providerów nie są koercjonowane do tekstu.

Prywatność odbiorcy: Windows zapisuje przed tekstem formaty
`CanIncludeInClipboardHistory` = 0 i `CanUploadToCloudClipboard` = 0.
Android zawsze zapisuje odebrany tekst z `EXTRA_IS_SENSITIVE` = true,
co również maskuje podgląd w nakładce Androida 13+. W ten sposób odbiorca
nie rozsyła dalej treści przez własny automat/historię/chmurę. Żadne treści
nie trafiają do logów, powiadomień, frontendowego IPC ani plików aplikacji.

Polityka aplikacji: automat jest domyślny. Przełącznik wyłącza automatyczny
odczyt i odbiór/nadpisywanie; ręczne wysłanie nie omija flag wrażliwości.
Windows reaguje na WM_CLIPBOARDUPDATE. Android czyta tylko z fokusem:
listener i porównanie aktualnej treści przy powrocie do aplikacji. W tle
telefon wysyła po stuknięciu akcji powiadomienia otwierającej widoczną,
nieeksportowaną Activity z bezpośredniego immutable PendingIntent;
odczyt czeka na fokus/odblokowanie, bez READ_LOGS, Accessibility lub IME.
Odbiór PC→telefon działa w usłudze połączenia także bez Activity.

Przykład Set, id=16 zer, licznik=1, tekst `hi` (bez nagłówka/ramki):
`82 00 a3 00 50 00000000000000000000000000000000 01 01 02 62 6869`.
Applied dla tego ID: `82 01 a2 00 50 00000000000000000000000000000000 01 00`.

#### 9.3.1 Obrazy (`clipboard/2`, WN-36)

CCS/1
PURPOSE:obraz ze schowka (zrzut ekranu) wklejalny po drugiej stronie od razu: najpierw miniatura i rozmiar, pełny PNG w tle.
RULE:strona, która przyjmuje obrazy, ogłasza `clipboard/1` i `clipboard/2` (§6, zasada 2: wersja N obejmuje 1..N). Obraz idzie tylko do strony z `clipboard/2`; starszej strona mówi „nie obsługuje obrazów”, tekst działa dalej.

| typ | nazwa | pola mapy body |
| --- | --- | --- |
| 2 | ImageSet | `0`: id 16 B; `1`: licznik Lamporta (ten sam co dla tekstu); `2`: `"image/png"` (jedyny w v1); `3`: rozmiar PNG uint, 1 B–64 MiB; `4`, `5`: szerokość i wysokość uint, 1–16384; `6`: miniatura PNG (bytes), 1 B–128 KiB, dłuższy bok ≤ 512 px |
| 3 | ImageChunk | `0`: id obrazu; `1`: offset uint; `2`: bajty 1 B–256 KiB |
| 4 | ImageDone | `0`: id obrazu; `1`: 0 komplet, 2 nieudany |
| 1 | Applied | jak wyżej; przy `refused` obrazu ponad limit odbiorcy opcjonalne pole `2`: ten limit w bajtach |

Przebieg: nadawca wysyła `ImageSet`, a po `Applied` kolejne `ImageChunk`
po kolei (offset = suma poprzednich) na tym samym strumieniu kanału.
Odbiorca daje `Applied`, gdy obraz jest już w schowku OS (wklejalny), a
nie po ostatnim bajcie: Android zapisuje URI `content://` swojego
dostawcy, Windows formaty z opóźnionym renderowaniem. `ImageDone` idzie,
gdy przyszedł cały PNG (0) albo bajty były błędne (2: luka, nadmiar).
Nadawca pokazuje „wysłano” dopiero po `ImageDone` 0; po `Applied` tylko
„wysyłanie”. Czekanie na `ImageDone` kończy się po 10 s od ostatniego
fragmentu.

Kolejność i echo: obraz i tekst dzielą jeden „ostatni schowek” i klucz
`(licznik, id)` z §9.3. Nowsza zmiana po którejkolwiek stronie kończy
obraz w drodze: odbiorca porzuca go (czytelnik dostaje błąd, nigdy
mieszanki dwóch obrazów), fragmenty starego id pomija, a nadawca
przerywa wysyłanie (`replaced`). Hash zapamiętany przez nadawcę to SHA-256
PNG. Własnego zapisu odebranego obrazu strona nie odsyła: Windows pomija
schowek, którego właścicielem jest jego okno, Android pomija klip z URI
własnego dostawcy (ponowne kodowanie zmieniłoby bajty, więc nie hash).

Limit: ustawienie użytkownika 5 / 16 / 50 MB PNG (domyślnie 16 MB).
Nadawca nie wysyła obrazu ponad własny limit (`too-large`), także
ręcznie. Odbiorca odrzuca `ImageSet` ponad swój limit (`refused` z polem
`2`), zanim cokolwiek trafi do schowka; klucz nie przesuwa się, więc
późniejsza mniejsza zmiana nadal wygrywa.

Wklejanie przed końcem przesyłu czeka na bajty; brak nowych bajtów przez
10 s, przerwanie sesji albo nowszy schowek kończą je błędem (wklejenie
się nie udaje, bez częściowego obrazu). Miniatura służy tylko podglądom
(Android: `ContentResolver.EXTRA_SIZE`, np. nakładka schowka, klawiatura);
nigdy nie jest wklejana zamiast obrazu. Obraz żyje tylko w pamięci, jeden
naraz; po restarcie procesu stary URI daje `FileNotFoundException`.

Prywatność: nadawca stosuje te same flagi co dla tekstu (Windows: formaty
prywatności, Android: `IS_SENSITIVE`) i wtedy nie kopiuje obrazu z OS.
Windows zapisuje odebrany obraz z `CanIncludeInClipboardHistory` = 0 i
`CanUploadToCloudClipboard` = 0, jak tekst. Android **nie** oznacza
odebranego obrazu jako wrażliwego (inaczej niż tekst): obraz pochodzi z
własnego komputera użytkownika, który nie wysyła treści prywatnych, a
oznaczenie ukryłoby podgląd z miniatury w nakładce Androida 13+. Telefon
czyta obraz innej aplikacji tylko z fokusem (uprawnienie URI klipu), bez
innych uprawnień. Nic z treści nie trafia do logów, IPC ani plików.

Przykłady (z nagłówkiem długości ramki):

```text
ImageSet id 0×16, licznik 1, 2×1 px, rozmiar 3, miniatura [01 02]:
2c 82 02 a7 00 50 00…00 01 01 02 69 "image/png" 03 03 04 02 05 01 06 42 01 02
ImageChunk offset 0, bajty [07 08 09]:
1c 82 03 a3 00 50 00…00 01 00 02 43 07 08 09
ImageDone komplet:          17 82 04 a2 00 50 00…00 01 00
Applied refused, limit 16 000 000:
1d 82 01 a3 00 50 00…00 01 01 02 1a 00 f4 24 00
```

### 9.4 Kanał `battery` (`07`, WN-49)

Poziom baterii telefonu i ładowanie, pokazywane na komputerze. Wiadomości
`battery/1`; jak `actions/1` wersjonuje je sama możliwość. Komputer ogłasza
`battery/1`, telefon nie (jeden kierunek). Kanał uni, kolejka wiadomości
jak w §3.1; ramki do 16 384 B.

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Battery` | `0 level`: liczba 0–100, procent; `1 charging`: bool, podłączony do ładowania (także pełny na ładowarce) |

Zasady:

1. Nadawca wysyła jeden `Battery` zaraz po starcie sesji, jeśli druga
   strona ogłosiła `battery/1`, a potem **tylko przy zmianie** pary
   (`level`, `charging`). Bez timera i bez powtórek: brak wiadomości znaczy
   „bez zmian”.
2. Kolejne wiadomości idą jednym strumieniem (§3.1, zasada 5.5); nadawca
   kończy go przy wycofaniu możliwości albo końcu sesji.
3. Brak któregoś pola, powtórzony klucz, `level` > 100 albo pole złego
   typu: odbiorca pomija tę wiadomość, strumień trwa. Nieznane klucze
   pomija (§4.3).
4. Odbiorca trzyma ostatni stan na czas sesji i zapomina go po jej końcu;
   nic nie zapisuje. Alerty (niski poziom, pełne naładowanie) są sprawą
   aplikacji odbiorcy, nie protokołu; pierwsza wiadomość sesji ich nie
   wywołuje.

Przykład (z nagłówkiem długości ramki):

```text
nagłówek strumienia:            01 07
Battery 82 %, ładuje się:       08 82 00 a2 00 18 52 01 f5
```

### 9.5 Kanał `find` (`08`, WN-50)

Znajdź telefon: komputer prosi telefon o głośne dzwonienie, także w trybie
cichym, i obie strony mogą je zatrzymać. Wiadomości `find/1`; wersjonuje je
sama możliwość. Telefon ogłasza `find/1` (potrafi i pozwala dzwonić),
komputer nie. Użytkownik telefonu może to wyłączyć: wtedy telefon nie
ogłasza `find/1` (§6, zasada 5), zamiast odmawiać każdej prośbie.

**Strumień.** Jedno dzwonienie to jeden strumień bidi otwarty przez
proszącego: nagłówek `01 08`, `Ring`, potem ewentualnie `Stop`. Dzwoniący
odpowiada w kierunku powrotnym (bez nagłówka) stanem `0`, gdy dźwięk już
gra, a potem dokładnie jednym stanem końcowym i FIN; proszący kończy swój
kierunek FIN po stanie końcowym. Ramki do 16 384 B.

Od proszącego:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Ring` | pusta mapa |
| 1 | `Stop` | pusta mapa |

Od dzwoniącego:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `State` | `0 state`: uint (tabela niżej) |

| `state` | znaczenie |
|---|---|
| 0 | dzwoni: dźwięk już gra (wysyłane dopiero po starcie odtwarzania) |
| 1 | zatrzymane na tym urządzeniu przez jego użytkownika |
| 2 | zatrzymane na prośbę (`Stop`) albo przejęte przez nowszy `Ring` |
| 3 | minął limit 60 s |
| 4 | niedostępne: nie da się odtworzyć dźwięku |

Nieznany `state` proszący traktuje jak `4` (końcowy).

Zasady:

1. `Ring` musi przyjść w 5 s od otwarcia strumienia (inaczej `CANCELLED`).
   Nieznany typ to `UNKNOWN_MESSAGE`, `Stop` przed `Ring` to
   `PROTOCOL_VIOLATION`, błędna ramka `MALFORMED_FRAME`; w każdym
   przypadku `STOP_SENDING` i `RESET_STREAM`, bez dzwonienia.
2. Jedno dzwonienie naraz. `Ring` w czasie dzwonienia go nie zaczyna od
   nowa: nowy strumień przejmuje dzwonienie (od razu stan `0`), a
   poprzedni dostaje `2` i FIN. `Stop` z przejętego strumienia nic już nie
   zatrzymuje.
3. Dzwonienie trwa najwyżej 60 s. Koniec strumienia albo sesji proszącego
   go **nie** zatrzymuje (użytkownik wciąż szuka telefonu): dzwoni dalej do
   limitu albo zatrzymania na telefonie.
4. Dźwięk omija tryb cichy i wibracje (na Androidzie strumień alarmu z
   wygenerowanym tonem). Nic nie jest zapisywane w protokole; aplikacja
   przywraca głośność, którą zmieniła.

Przykłady (z nagłówkiem długości ramki):

```text
nagłówek strumienia:   01 08
Ring:                  03 82 00 a0
Stop:                  03 82 01 a0
State 0 (dzwoni):      05 82 00 a1 00 00
State 1 (na telefonie): 05 82 00 a1 00 01
```

### 9.6 Kanały `media` (`03`) i `media-control` (`09`, WN-46)

Odtwarzacz jednej strony pokazany i sterowany na drugiej. Dwie możliwości,
bo kierunki robią co innego: `media/1` (pokazuję odtwarzacz drugiej strony:
przyjmuję `Now`, `Art`, `Gone` na `media`) i `media-control/1` (moim
odtwarzaczem można sterować: przyjmuję `Command` na `media-control`). W
WN-46 komputer ogłasza `media/1`, telefon `media-control/1`; w kierunku
odwrotnym (WN-47: odtwarzacz komputera na telefonie) komputer ogłasza
`media-control/1`, a telefon `media/1`, z tymi samymi wiadomościami. Oba
kanały są uni z kolejką jak w §3.1.

Kanał `media` (od strony z odtwarzaczem):

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Now` | `0 title` tekst ≤ 256 B; `1 artist`? ≤ 256 B; `2 album`? ≤ 256 B; `3 app` tekst ≤ 64 B (nazwa odtwarzacza); `4 duration_ms`? uint; `5 position_ms` uint (w chwili wysłania); `6 playing` bool; `7 speed_permille` uint (1000 = normalnie; CBOR bez liczb zmiennoprzecinkowych); `8 actions` uint, bity: 1 play, 2 pause, 4 next, 8 previous, 16 seek; `9 art`? 8 B, klucz `Art` |
| 1 | `Art` | `0 key` 8 B (pierwsze 8 B SHA-256 obrazu); `1 image` JPEG albo PNG ≤ 96 KiB (nadawca: ≤ 256 px na bok) |
| 2 | `Gone` | pusta mapa: brak aktywnego odtwarzacza |

Kanał `media-control` (od strony sterującej):

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Command` | `0 action` uint: 0 play, 1 pause, 2 toggle, 3 next, 4 previous, 5 seek; `1 position_ms` uint, tylko i obowiązkowo przy seek |

Zasady:

1. `Now` na starcie sesji (gdy druga strona ogłosiła `media/1`) i przy
   każdej zmianie metadanych, odtwarzania, szybkości, `actions` albo pozycji
   innej niż wynikająca z upływu czasu (odchyłka > 2 s od ekstrapolacji).
   Nigdy okresowo: odbiorca liczy postęp sam,
   `position_ms + upływ × speed_permille / 1000`, do `duration_ms`.
2. `Art` przed pierwszym `Now`, które wskazuje nowy klucz; odbiorca trzyma
   tylko ostatni obraz. Typ obrazu odbiorca ustala z pierwszych bajtów
   (JPEG `FF D8 FF`, PNG `89 50 4E 47`), nigdy ze słowa nadawcy; inny albo
   większy obraz to błędna wiadomość.
3. Teksty i obrazy pochodzą z innych aplikacji nadawcy: odbiorca pokazuje
   je wyłącznie jako tekst (bez interpretacji znaczników), bez znaków
   sterujących i dwukierunkowych. Nic nie jest zapisywane ani logowane.
4. Błędna wiadomość (za długi tekst, zły typ pola, brak pola) jest
   pomijana; strumień trwa. `Command` z akcją, której ostatni wysłany `Now`
   nie dopuszcza w `actions`, strona pomija bez reakcji.
5. Strona ogłasza `media-control/1` tylko wtedy, gdy rzeczywiście może
   sterować odtwarzaczem (na Androidzie: dostęp do sesji multimediów i
   włączone ustawienie; na Windows: ustawienie „Udostępnij odtwarzanie
   telefonowi”), a `media/1` tylko, gdy chce pokazywać odtwarzacz drugiej
   strony; także w trakcie sesji (§6, zasada 5). Strona, która znów przyjmuje
   `media/1`, dostaje od nadawcy bieżący `Now` (i `Art`) od nowa.
6. **Zgoda.** Na Androidzie odczyt cudzych sesji multimediów wymaga
   dostępu do powiadomień. WindConnect używa go wyłącznie do odtwarzacza i
   nie czyta treści powiadomień. Funkcja czytająca powiadomienia (WN-42)
   NIE MOŻE po cichu rozszerzyć tej zgody: ma własny przełącznik (domyślnie
   wyłączony) i własne wyjaśnienie.
7. Strona nie odsyła drugiej stronie jej własnego odtwarzacza: sesja
   multimediów, którą sama utworzyła, by pokazać odtwarzacz drugiej strony,
   nie jest nigdy wysyłana jako `Now` (inaczej powstaje echo).

Przykłady (z nagłówkiem długości ramki):

```text
nagłówki strumieni:          01 03 (media), 01 09 (media-control)
Gone:                        03 82 02 a0
Command next:                05 82 00 a1 00 03
Command seek 61 000 ms:      09 82 00 a2 00 05 01 19 ee 48
```

### 9.7 Kanał `notify` (`02`, WN-42)

Powiadomienia jednej strony pokazane na drugiej. W WN-42 wysyła telefon;
komputer ogłasza `notify/1`, gdy potrafi je pokazać (toasty, WN-43). Kanał
uni z kolejką jak w §3.1. Kanał nie miał wcześniej typów wiadomości, więc
wersjonuje je sama możliwość `notify/1` (sekcja 9, zasada 2).

Od strony z powiadomieniami:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Posted` | `0 id` 16 B; `1 app` tekst ≤ 64 B (nazwa aplikacji); `2 package` tekst ≤ 128 B (identyfikator aplikacji, dla pamięci ikon i filtrów WN-45); `3 title`? tekst ≤ 256 B; `4 text`? tekst ≤ 1024 B; `5 time` uint (ms od epoki Uniksa, chwila opublikowania); `6 icon`? 8 B, klucz `Icon`; `7 reply`? tekst ≤ 64 B: etykieta akcji odpowiedzi, jest tylko wtedy, gdy na powiadomienie można odpowiedzieć (§9.8, WN-44); `8 quiet`? bool, tylko `true`: u nadawcy nie przerywa (tryb „Nie przeszkadzać”), odbiorca pokazuje je bez wyskakiwania i dźwięku (WN-45) |
| 1 | `Icon` | `0 key` 8 B (pierwsze 8 B SHA-256 obrazu); `1 png` PNG ≤ 16 KiB (nadawca: 64 px na bok) |
| 2 | `Removed` | `0 id` 16 B: powiadomienie zniknęło u nadawcy (odrzucone albo anulowane) |
| 3 | `Replied` | `0 id` 16 B; `1 ok` bool: czy odpowiedź z `Reply` (§9.8) poszła do aplikacji (WN-44) |

Zasady:

1. `id` jest stały dla jednego powiadomienia u nadawcy i nic nie znaczy
   poza nim: na Androidzie to pierwsze 16 B SHA-256 losowej soli instalacji
   i klucza powiadomienia, więc ze znanej aplikacji nie da się go odgadnąć.
   `Posted` ze znanym `id` zastępuje to powiadomienie.
2. Wysyłane są tylko powiadomienia opublikowane w trakcie sesji: na
   początku sesji nadawca NIE wysyła tych, które już są (bez zalewu
   powiadomień po każdym ponownym połączeniu). Lista bieżących, gdyby była
   potrzebna, to osobna wiadomość w przyszłej wersji.
3. Nadawca pomija powiadomienia stałe (trwające, usług pierwszoplanowych,
   nieusuwalne), podsumowania grup, odtwarzacze (§9.6), tajne
   (`VISIBILITY_SECRET`), własne i puste (bez tytułu i treści).
4. **Tempo.** Dla jednego `id` najwyżej jedno `Posted` na sekundę: zmiana w
   ciągu sekundy od poprzedniego czeka do jej końca i wychodzi tylko
   najnowsza. Powiadomienie z paskiem postępu wychodzi tylko na początku i
   na końcu (gdy pasek znika). `Posted` o treści takiej jak ostatnio wysłana
   (poza `time`) nie wychodzi.
5. `Icon` przed pierwszym `Posted`, które w tej sesji wskazuje nowy klucz;
   odbiorca trzyma ikony przynajmniej do końca sesji. Gdy odbiorca wycofa
   `notify/1` (§6, zasada 5), nadawca wysyła ikony od nowa po ponownym
   włączeniu. Typ obrazu odbiorca ustala z pierwszych bajtów (PNG
   `89 50 4E 47`); inny albo większy obraz to błędna wiadomość.
6. `Removed` tylko dla `id`, którego `Posted` wyszło w tej sesji (nie dla
   pominiętych ani sprzed sesji).
7. Teksty i ikony pochodzą z innych aplikacji nadawcy: nadawca usuwa
   formatowanie, znaki sterujące i dwukierunkowe oraz ciągi białych znaków;
   odbiorca pokazuje je wyłącznie jako tekst. Nic nie jest zapisywane ani
   logowane. Treść, którą system nadawcy ukrył (Android 15+: kody
   jednorazowe i inna treść wrażliwa), wychodzi w postaci, w jakiej dał ją
   system; nadawca tego nie obchodzi.
8. Błędna wiadomość (za długi tekst, zły typ pola, brak pola) jest
   pomijana; strumień trwa.
9. **Zgoda.** Na Androidzie odczyt wymaga osobnej zgody (§9.6, zasada 6):
   własny wpis w systemowym dostępie do powiadomień i ustawienie
   „Powiadomienia na komputerze”, domyślnie wyłączone.
10. `7 reply` i `Replied` (WN-44) to dodatki zgodne (sekcja 9, zasada 1):
    odbiorca bez nich pomija nieznane pole (§4.3) i nieznany typ (§4.4).
    Nadawca ustawia `reply` tylko dla akcji, która przyjmuje dowolny tekst
    (na Androidzie: `RemoteInput` z `getAllowFreeFormInput()`, nie
    kontekstowa; najpierw `SEMANTIC_ACTION_REPLY`, potem pierwsza taka).
11. **Filtr aplikacji (WN-45).** Nadawca może pomijać wybrane aplikacje;
    pominięta aplikacja nie wysyła nic (ani `Posted`, ani `Icon`). Gdy
    aplikacja zostaje pominięta w trakcie sesji, nadawca wysyła `Removed`
    dla jej powiadomień wysłanych w tej sesji. `8 quiet` to dodatek zgodny
    (zasada 10): nadawca ustawia go, gdy powiadomienie nie przechodzi przez
    jego tryb „Nie przeszkadzać” (Android: `Ranking.matchesInterruptionFilter()`
    = false). Odbiorca może też sam wyciszać (ciche godziny, tryb skupienia),
    ale nie ukrywa powiadomień i nie odrzuca ich u nadawcy.

Przykład (z nagłówkiem długości ramki): nagłówek strumienia `01 02`;
`Removed` z `id` = 16 × `07`, 22 B:

```text
15 82 02 a1 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07
```

`Replied` z tym samym `id`, `ok` = true, 24 B:

```text
17 82 03 a2 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 01 f5
```

### 9.8 Kanał `notify-action` (`0a`, WN-44)

Odrzucenie powiadomienia i odpowiedź na nie ze strony, która je pokazuje
(§9.7). Kanał uni z kolejką jak w §3.1; otwiera go strona pokazująca
powiadomienia, przyjmuje strona, która je wysłała i ogłosiła
`notify-action/1`.

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Dismiss` | `0 id` 16 B: odrzuć to powiadomienie |
| 1 | `Reply` | `0 id` 16 B; `1 text` tekst 1–1024 B: wyślij tę odpowiedź akcją odpowiedzi powiadomienia |

Zasady:

1. Strona ogłasza `notify-action/1` tylko wtedy, gdy wysyła powiadomienia
   (§9.7, zasada 9: zgoda i dostęp), także w trakcie sesji (§6, zasada 5).
2. Strona wykonuje akcję **tylko** dla `id` wysłanego przez nią w tej
   sesji i wciąż obecnego; każde inne `id` pomija (na `Reply` odsyła
   tylko `Replied` z `ok` = false).
3. `Dismiss` odrzuca powiadomienie u nadawcy tak, jak zrobiłby to
   użytkownik; nadawca odsyła potem zwykłe `Removed` (§9.7, zasada 6).
   Strona pokazująca usuwa je u siebie od razu i znosi `Removed` dla
   nieznanego już `id`.
4. `Reply` idzie do aplikacji tak, jakby tekst wpisano na urządzeniu
   nadawcy (Android: `RemoteInput.addResultsToIntent`, źródło
   `SOURCE_FREE_FORM_INPUT`). Po próbie nadawca odsyła `Replied` (§9.7);
   aplikacja zwykle publikuje też zmienione powiadomienie (`Posted`).
5. Tekst odpowiedzi nie jest zapisywany ani logowany po żadnej stronie.
   Tekst pusty albo dłuższy niż 1024 B to błędna wiadomość: odbiorca jej nie
   przycina, tylko pomija; strona pokazująca NIE MOŻE wysłać takiego
   tekstu (interfejs blokuje wysłanie z komunikatem).
6. Strona pokazująca wysyła `Dismiss` tylko na wyraźne zamknięcie przez
   użytkownika (nie po wygaśnięciu ani usunięciu przez siebie).

Przykłady (z nagłówkiem długości ramki): nagłówek strumienia `01 0a`;
`id` = 16 × `07`. `Dismiss`, 22 B, i `Reply` z tekstem `ok`, 26 B:

```text
15 82 00 a1 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07
19 82 01 a2 00 50 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 07 01 62 6f 6b
```

### 9.9 Kanał `volume` (`0b`, WN-48)

Głośność wyjścia każdej strony, pokazywana i zmieniana przez drugą (na
Windows: domyślne urządzenie odtwarzania, Core Audio; na Androidzie:
`STREAM_MUSIC`). Kanał uni z kolejką jak w §3.1, w obu kierunkach.

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Level` | `0 level` uint 0–100; `1 muted` bool: głośność nadawcy |
| 1 | `Set` | `0 level`? uint 0–100; `1 muted`? bool, co najmniej jedno: zmień głośność odbiorcy |

Zasady:

1. `volume/1` znaczy: „moją głośność można zmieniać i pokazuję twoją”.
   Strona wysyła `Level` tylko, gdy druga ogłosiła `volume/1` (i sama ma
   włączone ustawienie), na starcie, przy każdej zmianie (także z klawiszy
   czy miksera systemu) i ponownie, gdy druga strona zacznie przyjmować
   `volume/1`; najwyżej co 100 ms, wychodzi najnowsza.
2. `Set` zmienia tylko to, co podaje: sam `level` nigdy nie wyłącza
   wyciszenia. Odbiorca stosuje najwyżej 10 `Set` na sekundę (wygrywa
   najnowszy). `level` ponad 100, zły typ albo pusty `Set` to błędna
   wiadomość: odbiorca jej nie przycina, tylko pomija.
3. Urządzenie o stałej głośności (np. część wyjść HDMI/S/PDIF/USB, Android
   `isVolumeFixed`) pomija `level`; `Level` zgłasza to, co jest naprawdę.

Przykład (z nagłówkiem długości ramki): nagłówek strumienia `01 0b`;
`Level` 40 %, bez wyciszenia:

```text
08 82 00 a2 00 18 28 01 f4
```

### 9.10 Kanał `call` (`0c`, WN-48)

Stan połączenia telefonu, żeby komputer wstrzymał swoje odtwarzacze. Kanał
uni z kolejką jak w §3.1; wysyła telefon, przyjmuje komputer z `call/1`.

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `State` | `0 state` uint: 0 bezczynny, 1 dzwoni, 2 rozmowa; WN-57, opcjonalne: `1 call` uint, numer połączenia na telefonie; `2 number` tekst 1–64 B; `3 name` tekst 1–128 B; `4 actions` uint, bity: 1 można odrzucić, 2 można wyciszyć |
| 1 | `Missed` | `0 items`: tablica do 20 `[number, name, date]` (tekst 1–64 B, tekst 1–128 B albo `null`, uint ms), od najnowszego (WN-57) |

Zasady:

1. Bez ustawienia telefonu „Połączenia na komputerze” (WN-57) tylko stan:
   bez numeru, nazwy ani aplikacji. Rozmowa obejmuje też
   rozmowy w komunikatorach (Android: tryb `MODE_IN_COMMUNICATION`).
2. Telefon wysyła stan na starcie każdej sesji (także bezczynny), przy
   każdej zmianie i ponownie, gdy komputer zacznie przyjmować `call/1`.
   Dzwonienie i rozmowa wychodzą od razu; bezczynność po 1,5 s stałego
   spokoju.
3. Komputer przy „dzwoni” albo „rozmowa” wstrzymuje każdy grający
   odtwarzacz i pamięta, które; przy „bezczynny” wznawia tylko te, które
   wstrzymał, jeśli nadal są wstrzymane, i nie później niż 30 min potem.
   Koniec sesji nie czyści tej listy: rozstrzyga ją następny `State`.
4. Na Androidzie bez nowych uprawnień: tryb dźwięku (API 31+), stan
   telefonii bez numeru (API 29–30), a w trybie wibracji, gdy tryb się nie
   zmienia, powiadomienie kategorii połączenia widziane przez słuchacza
   powiadomień (tylko przy jego zgodzie, §9.7 zasada 9).
5. **Kto dzwoni (WN-57).** Z ustawieniem „Połączenia na komputerze” i
   zgodami (Android: `READ_PHONE_STATE`, `READ_CALL_LOG`; nazwa z kontaktów
   przy `READ_CONTACTS`) `State` dzwonienia i rozmowy przychodzącej niesie
   `1 call` i, gdy są, `2 number`, `3 name`, `4 actions`. `call` jest nowy
   dla każdego połączenia; ten sam `call` z nowymi polami (np. nazwa
   doszła po numerze) zastępuje poprzedni bez nowego powiadomienia.
   Bezczynność kończy połączenie. Strona bez WN-57 pomija nowe klucze
   (§4.3), a komputer bez nich pokazuje tylko stan.
6. `Missed`: cała lista nieodebranych z ostatnich 7 dni, przy starcie sesji,
   gdy komputer zacznie przyjmować `call/1`, i po każdym nieodebranym; z
   wyłączonym ustawieniem pusta. Komputer traktuje pierwszą listę w sesji
   jako stan wyjściowy i powiadamia tylko o nowszych pozycjach.
7. Komputer przyjmuje `call/1`, gdy ma włączone wstrzymywanie odtwarzania
   albo powiadomienia o połączeniach; stan wstrzymuje odtwarzacze tylko przy
   pierwszym ustawieniu.

Przykład (z nagłówkiem długości ramki): nagłówek strumienia `01 0c`;
`State` dzwoni:

```text
05 82 00 a1 00 01
```

### 9.11 Kanał `sms` (`0d`, WN-55)

Odczyt wątków SMS/MMS telefonu na komputerze. Wiadomości `sms/1`;
wersjonuje je sama możliwość. `sms/1` ogłasza telefon tylko wtedy, gdy
użytkownik włączył „SMS na komputerze” i system przyznał dostęp do SMS
(Android: `READ_SMS`); bez jednego z nich telefon wycofuje możliwość (§6,
zasada 5), zamiast odmawiać każdej prośbie. Komputer nie ogłasza `sms/1`.
WN-56 dodało typy 4–6 (karty SIM, wysyłanie, części MMS) i pola 8–9
wiadomości; zgodnie z §9 zasada 1 to rozszerzenie bez nowej wersji
możliwości: telefon z wcześniejszą wersją odrzuca nowe typy kodem
`UNKNOWN_MESSAGE`, a komputer pokazuje wtedy, że wysyłanie jest niedostępne.
Wysyłanie ma osobną zgodę na telefonie (ustawienie „Wysyłanie z komputera”
i `SEND_SMS`); bez niej `Sims` i `Send` dostają `End` 1.

**Strumień.** Jedna prośba to jeden strumień bidi otwarty przez komputer:
nagłówek `01 0d`, jedna wiadomość, FIN. Telefon odpowiada w kierunku
powrotnym (bez nagłówka) zerem lub więcej partii, potem dokładnie jednym
`End` i FIN. Wyjątek: `Watch` trwa, dopóki któraś strona nie zakończy
strumienia. Ramki do 1 MiB (§4.2); telefon dzieli odpowiedź na partie
około 256 KiB.

Od komputera:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Threads` | `0 limit`: uint 1–500; najnowsze wątki, od najnowszego |
| 1 | `Messages` | `0 thread`: uint; `1 before`: uint, opcjonalne; `2 since`: uint, opcjonalne; `3 limit`: uint 1–2000 |
| 2 | `Photo` | `0 address`: tekst 1–64 B, adres osoby z wątku |
| 3 | `Watch` | pusta mapa |
| 4 | `Sims` | pusta mapa; karty SIM do wysyłania (WN-56) |
| 5 | `Send` | `0 to`: tablica 1–20 adresów (tekst 1–64 B); `1 body`: tekst do 16 KiB; `2 sim`: uint, opcjonalne; `3 image`: bajty 1–786 432, opcjonalne; `4 mime`: tekst `image/jpeg` albo `image/png`, razem z `3` (WN-56) |
| 6 | `Part` | `0 message`: uint, `id` MMS; `1 part`: uint, `id` części (WN-56) |

Od telefonu:

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Threads` | `0 items`: tablica do 500 wątków |
| 1 | `Messages` | `0 items`: tablica do 2000 wiadomości |
| 2 | `Photo` | `0 png`: bajty 1–65 536, PNG |
| 3 | `Changed` | pusta mapa |
| 4 | `End` | `0 status`: uint (tabela niżej) |
| 5 | `Sims` | `0 items`: tablica do 4 kart `[id, label, slot, default]` (`id` uint, `label` tekst do 64 B, `slot` uint od 0, `default` bool) |
| 6 | `Sending` | `0 state`: uint: 0 przyjęta przez telefon, 1 wysłana, 2 dostarczona, 3 niewysłana; nieznana wartość to 3 |
| 7 | `Part` | `0 mime`: tekst 1–64 B; `1 data`: bajty 1–921 600 |

Wątek (mapa): `0 id` uint, `1 date` uint (najnowsza wiadomość, ms od
epoki), `2 snippet` tekst do 512 B, `3 people` tablica do 20 osób
`[address, name, photo]` (`address` tekst 1–64 B, `name` tekst do 128 B
albo `null`, `photo` bool: `Photo` dla tego adresu coś znajdzie),
`4 unread` uint, `5 count` uint.

Wiadomość (mapa): `0 id` uint (jednoznaczny w obrębie rodzaju), `1 mms`
bool, `2 date` uint (ms od epoki), `3 box` uint (0 odebrana, 1 wysłana,
2 w trakcie wysyłania, 3 niewysłana; nieznana wartość to 0), `4 body` tekst
do 16 KiB (może być pusty), `5 address` tekst 1–64 B, opcjonalne (nadawca
odebranej wiadomości), `6 read` bool, `7 attachments` uint, opcjonalne
(części MMS inne niż tekst), `8 parts` tablica do 16 par `[id, mime]`,
opcjonalne (części MMS do pobrania przez `Part`), `9 delivery` uint,
opcjonalne (raport doręczenia wysłanego SMS: 0 czeka, 1 dostarczona,
2 niedostarczona; nieznana wartość to 0).

| `status` | znaczenie |
|---|---|
| 0 | gotowe |
| 1 | odmowa: brak dostępu do SMS albo funkcja wyłączona |
| 2 | nie ma: wątku albo zdjęcia dla adresu |
| 3 | zajęte: za dużo obserwacji |
| 4 | błąd odczytu |

Nieznany `status` komputer traktuje jak `4`.

Zasady:

1. Prośba musi przyjść w 5 s od otwarcia strumienia (inaczej `CANCELLED`).
   Błędna wiadomość albo pole poza zakresem (np. `limit` 0 lub ponad
   maksimum): `MALFORMED_FRAME`; nieznany typ: `UNKNOWN_MESSAGE`; w obu
   przypadkach `STOP_SENDING` i `RESET_STREAM`, bez odpowiedzi. Wartości
   spoza zakresu odbiorca odrzuca, nie przycina.
2. `Messages` zwraca wiadomości wątku od najnowszej, najwyżej `limit`, o
   dacie nie późniejszej niż `before` i nie wcześniejszej niż `since`
   (obie granice włącznie). Szkice się nie liczą. **Synchronizacja
   przyrostowa:** komputer pyta z `since` = data najnowszej znanej
   wiadomości, a starsze strony z `before` = data najstarszej; granice
   włącznie powtarzają wiadomości z tej samej milisekundy, więc komputer
   scala je po parze (`mms`, `id`). Strona bez nowych wiadomości kończy
   przewijanie wstecz.
3. Odpowiedź z większą liczbą elementów niż `limit` prośby, albo partia
   innego typu niż prośba, to błąd: komputer zatrzymuje strumień
   (`CANCELLED`) i odrzuca całą odpowiedź.
4. `Watch`: telefon wysyła `Changed`, gdy zmienią się SMS lub MMS (nowa,
   usunięta, przeczytana), zebrane w 250 ms; komputer wtedy pyta od nowa
   o wątki i o otwarty wątek z `since`. Najwyżej 2 obserwacje naraz na
   sesję; kolejna dostaje od razu `End` 3. Wycofanie `sms/1` kończy
   obserwację (§6, zasada 5).
5. Najwyżej 4 prośby (bez `Watch`) obsługiwane naraz na sesję; kolejne
   czekają.
6. `Photo` to PNG (sprawdzany po pierwszych bajtach) do 64 KiB; telefon
   bez dostępu do kontaktów odpowiada `End` 2.
7. **Wysyłanie (WN-56).** `Send` z jednym adresem i bez obrazu to SMS
   (dzielony na części przez telefon); z obrazem albo kilkoma adresami to
   MMS. Pusta treść bez obrazu, obraz bez `mime` (lub odwrotnie), obraz
   niezgodny z `mime` po pierwszych bajtach (JPEG `ff d8 ff`, PNG
   `89 50 4e 47`) albo więcej niż 20 adresów to `MALFORMED_FRAME`. Telefon
   odpowiada `Sending` 0, gdy przekazał wiadomość systemowi, potem
   kolejnymi stanami, i kończy `End` 0: po stanie 2 albo 3, dla MMS po
   stanie 1 (MMS nie ma raportu doręczenia), po 60 s od stanu 1 bez raportu,
   albo po 120 s bez stanu 1 (wtedy najpierw `Sending` 3). Odmowa przed
   przekazaniem to sam `End` (1 wyłączone, 3 limit, 4 błąd). Najwyżej 10
   przyjętych wysyłek na minutę na telefon (kolejne `End` 3); wysyłka nie
   zajmuje miejsca z zasady 5. Wysłana wiadomość trafia do wątków telefonu
   (Android zapisuje ją sam), więc obserwacja ją zgłasza.
8. `Part` zwraca obraz z MMS jako JPEG albo PNG (telefon może go
   przeskalować i przekodować); części innych niż obraz telefon nie wydaje
   (`End` 2). Część musi należeć do podanej wiadomości.
9. `Sims` bez dostępu do listy kart (Android: `READ_PHONE_STATE`) zwraca
   jedną kartę domyślną; `Send` bez `sim` idzie kartą domyślną telefonu.
10. Prywatność: teksty i numery nie trafiają do logów żadnej strony.
   Komputer trzyma przeczytane wiadomości tylko w pamięci i usuwa je przy
   połączeniu innego telefonu albo rozparowaniu; pokazuje je jako tekst,
   bez znaków sterujących i kierunkowych.

Przykłady (z nagłówkiem długości ramki):

```text
nagłówek strumienia:                     01 0d
Threads limit 200:                       06 82 00 a1 00 18 c8
Messages thread 7, since 1000, limit 1000: 0d 82 01 a3 00 07 02 19 03 e8 03 19 03 e8
Watch:                                   03 82 03 a0
Changed:                                 03 82 03 a0
End 0 (gotowe):                          05 82 04 a1 00 00
Sims:                                    03 82 04 a0
Send do "123", "Hi":                     0d 82 05 a2 00 81 63 31 32 33 01 62 48 69
Sending 1 (wysłana):                     05 82 06 a1 00 01
```

### 9.12 Kanał `call-action` (`0e`, WN-57)

Komputer odrzuca albo wycisza dzwoniące połączenie telefonu. Kanał uni z
kolejką jak w §3.1; wysyła komputer, przyjmuje telefon z `call-action/1`.
Telefon ogłasza `call-action/1` tylko z ustawieniem „Połączenia na
komputerze” (§9.10 zasada 5) i wycofuje je razem z nim.

| typ | nazwa | pola `body` |
|---|---|---|
| 0 | `Reject` | `0 call` uint |
| 1 | `Silence` | `0 call` uint |

Zasady:

1. Telefon wykonuje akcję tylko wtedy, gdy połączenie o tym `call` właśnie
   dzwoni; inne (zakończone, odebrane, następne) pomija bez odpowiedzi, więc
   spóźnione kliknięcie nie odrzuci nowego połączenia.
2. `Reject` kończy połączenie (Android: `TelecomManager.endCall`, zgoda
   `ANSWER_PHONE_CALLS`); bez zgody telefon nie ustawia bitu 1 w `actions` i
   pomija `Reject`.
3. `Silence` wycisza dzwonek tego połączenia: system, jeśli pozwala, inaczej
   wyciszony strumień dzwonka, przywracany, gdy połączenie się skończy.
4. Wynik widać w `State` na kanale `call` (bezczynny po odrzuceniu).

Przykłady (z nagłówkiem długości ramki): nagłówek strumienia `01 0e`;
`Reject` połączenia 7: `05 82 00 a1 00 07`; `Silence` połączenia 7:
`05 82 01 a1 00 07`.

## 10. Kody błędów

Te same kody służą w polu `code` wiadomości `Error` / `Goodbye` oraz jako
kody aplikacji QUIC w `CONNECTION_CLOSE`, `RESET_STREAM` i `STOP_SENDING`.

| kod | nazwa | znaczenie | typowe użycie |
|---|---|---|---|
| `00` | `NO_ERROR` | zwykłe zakończenie | FIN, zamknięcie bez błędu |
| `01` | `PROTOCOL_VIOLATION` | naruszenie zasad przebiegu | połączenie |
| `02` | `UNSUPPORTED_VERSION` | nieznany bajt `version` | strumień, `control`: połączenie |
| `03` | `FRAME_TOO_LARGE` | ramka ponad limit | strumień, `control`: połączenie |
| `04` | `MALFORMED_FRAME` | błędny CBOR, pole lub długość | strumień, `control`: połączenie |
| `05` | `UNKNOWN_CHANNEL` | nieznany numer kanału | strumień |
| `06` | `UNKNOWN_MESSAGE` | nieznany typ na znanym kanale | `Error` |
| `07` | `CAPABILITY_MISSING` | kanał bez ogłoszonej możliwości | strumień, `Error` |
| `08` | `UNTRUSTED_PEER` | klucz nie jest przypięty | połączenie |
| `09` | `HELLO_TIMEOUT` | brak `Hello` / `HelloAck` w czasie | połączenie |
| `0a` | `INTERNAL_ERROR` | błąd implementacji | dowolne |
| `0b` | `RATE_LIMITED` | zbyt wiele wiadomości (nie liczba strumieni uni, §3.1 zasada 5) | strumień, `Error` |
| `0c` | `CANCELLED` | anulowane przez użytkownika lub stronę | strumień |
| `0d` | `GOING_AWAY` | wyjście, uśpienie, rozparowanie | `Goodbye`, połączenie |
| `0e` | `NO_SPACE` | brak miejsca na plik (§9.1) | strumień `files` |
| `0f` | `UNPAIRED` | nadawca usunął parowanie (§12.10) | `Goodbye`, połączenie |
| `10` | `BUSY` | nadawca ma sesję z innym urządzeniem i ją zachowuje (§12.10) | `Goodbye`, połączenie |

Kody `11`–`ff` są zarezerwowane dla v1; nieznany kod odbiorca traktuje jak
`INTERNAL_ERROR`. Błąd w ramce strumienia innego niż `control` zamyka tylko
ten strumień (`STOP_SENDING` po stronie odbiorcy), nie połączenie.

## 11. Zestawienie przykładów

| wiadomość | bajty |
|---|---|
| nagłówek `control` | `01 00` |
| nagłówek `input` | `01 04` |
| początek strumienia `control` | sekcja 7.1 |
| `Hello` | sekcja 7.3 (50 B) |
| `HelloAck` | sekcja 7.3 (45 B) |
| `Ping` nonce 42 | `06 82 02 a1 00 18 2a` |
| `Pong` nonce 42 | `06 82 03 a1 00 18 2a` |
| `Capabilities` | `0d 82 04 a1 00 81 67 69 6e 70 75 74 2f 31` |
| `Error` | sekcja 7.6 (24 B) |
| `Goodbye` GOING_AWAY | `05 82 06 a1 00 0d` |
| `PointerMotion` (datagram) | sekcja 8.8 (24 B) |
| `PointerMotion` 2 (datagram) | sekcja 8.8 (18 B) |
| `Scroll` (datagram) | sekcja 8.8 (17 B) |
| `PointerButton` | sekcja 8.8 (23 B) |
| `InputSession` s 0, g 0 | `07 82 03 a2 00 00 01 00` |
| wiadomości `files` | sekcja 9.1 |
| wiadomości `actions` | sekcja 9.2 |
| `Battery` 82 %, ładuje się | `08 82 00 a2 00 18 52 01 f5` (§9.4) |
| wiadomości `find` | sekcja 9.5 |
| wiadomości `media`, `media-control` | sekcja 9.6 |
| wiadomości `notify` | sekcja 9.7 |
| wiadomości `notify-action` | sekcja 9.8 |
| `Level` 40 %, bez wyciszenia | `08 82 00 a2 00 18 28 01 f4` (§9.9) |
| `State` ringing | `05 82 00 a1 00 01` (§9.10) |
| `Reject` call 7 | `05 82 00 a1 00 07` (§9.12) |
| wiadomości `sms` | sekcja 9.11 |

Przykłady są wektorami testowymi kodera ramek (WC-06): koder MUSI
produkować dokładnie te bajty, a dekoder odtwarzać z nich te wiadomości.

## 12. Parowanie (`windconnect-pair/1`, WC-19)

Parowanie to jedyna chwila, w której strona przyjmuje klucz, którego
jeszcze nie przypięła. Po udanym parowaniu obie strony zapisują i
przypinają klucz drugiej, a dalej łączą się wyłącznie przez §2.

Dwie ścieżki:

- **QR**: urządzenie z ekranem (zwykle PC) pokazuje kod QR z linkiem
  (§12.4), który niesie jego klucz i jednorazowy sekret 128 bit. Drugie
  urządzenie dowodzi znajomości sekretu (§12.5).
- **SAS**: bez aparatu. Obie strony pokazują ten sam 6-cyfrowy kod, a
  ludzie potwierdzają, że kody się zgadzają (§12.6).

Strona pokazująca QR albo czekająca na porównanie kodu to **serwer
parowania**, a druga to **klient**.

### 12.1 Transport

1. Serwer parowania nasłuchuje na **osobnym** endpoincie QUIC (port
   efemeryczny, ogłaszany w linku albo przez mDNS, §12.4), z ALPN
   wyłącznie `windconnect-pair/1`. Główny endpoint (§2) nigdy nie przyjmuje
   tego ALPN i nadal odrzuca nieprzypięte klucze w uzgadnianiu TLS.
2. TLS 1.3 z wzajemnym uwierzytelnieniem, jak w §2: każda strona
   przedstawia certyfikat urządzenia z kluczem Ed25519, a podpisy
   uzgadniania są wyłącznie Ed25519. Klient bez certyfikatu zostaje
   odrzucony w uzgadnianiu TLS. Serwer parowania przyjmuje dowolny klucz
   Ed25519. Klient w ścieżce QR przyjmuje wyłącznie klucz z linku i
   odrzuca każdy inny w TLS (alert `access_denied`); w ścieżce SAS
   przyjmuje dowolny klucz.
3. **Klucze tylko z uzgadniania.** `K_s` (serwera) i `K_c` (klienta) w
   każdym wzorze niżej oraz w zapisie sparowanego urządzenia to klucze z
   certyfikatów zweryfikowanych w **tym** połączeniu. Żadna wiadomość
   parowania nie niesie klucza.
4. Połączenie parowania to zawsze **pełne** uzgadnianie 1-RTT: serwer nie
   wydaje biletów sesji i nie prowadzi pamięci sesji, klient nie oferuje
   wznowienia, a 0-RTT jest zabronione w obie strony. Konfiguracja TLS
   endpointu parowania nie jest współdzielona z głównym.
5. Sesję TLS wiąże wartość eksportera (RFC 8446 §7.5):
   `E = TLS-Exporter("EXPORTER-windconnect-pair", "", 32)`. Obie strony
   liczą ją po uzgadnianiu; dwie różne sesje TLS (np. po obu stronach
   pośrednika) mają różne `E`.
6. Klient otwiera dokładnie jeden strumień bidi. Strumień **nie** ma
   nagłówka z §3 (protokół wyznacza ALPN); od razu niesie ramki §4.
   Endpoint parowania nie przyjmuje strumieni uni ani datagramów.

### 12.2 Okno parowania

Serwer odpowiada tylko wtedy, gdy ma otwarte **okno**: QR (z nowym
sekretem) albo SAS. Otwarcie nowego okna zamyka poprzednie.

1. Okno trwa najwyżej 5 min. Po upływie czasu zamyka się samo, a sekret
   jest kasowany od razu, bez czekania na połączenie.
2. Okno obsługuje **jedną próbę naraz**. Udana próba zamyka okno, a sekret
   jest zużyty: ten sam link nie zadziała drugi raz, także dla tego samego
   klienta.
3. **Nieudaną próbą** jest każde zakończenie po uzgadnianiu TLS bez
   sukcesu: zły dowód MAC, commit niezgodny z ujawnieniem, wiadomość poza
   kolejnością lub zły tryb, zła lub za duża ramka, rozłączenie albo reset
   przed wynikiem, upływ limitu czasu kroku (10 s) oraz odrzucony lub
   nierozstrzygnięty kod SAS (60 s na decyzję). Po **3** nieudanych
   próbach okno się zamyka, sekret jest kasowany, a kolejne połączenia
   dostają `RATE_LIMITED` do otwarcia nowego okna.
4. Połączenie, które przychodzi, gdy trwa inna próba, dostaje
   `RATE_LIMITED` i nie liczy się jako próba.
5. Stan okna serwer sprawdza **przed** odczytaniem jakiejkolwiek ramki i
   przed jakimkolwiek obliczeniem poza TLS: brak okna lub okno wygasłe to
   `GOING_AWAY`, okno zablokowane lub zajęte to `RATE_LIMITED`.

Uzgadnianie TLS nie liczy się jako próba (nieudane uzgadnianie nie
dochodzi do okna). Napastnik w sieci lokalnej może więc najwyżej zamknąć
okno trzema próbami; aplikacja pokazuje to i pozwala otworzyć nowe.

### 12.3 Wiadomości

Ramki jak w §4 (długość, potem CBOR `[type, body]`), z limitem
**1 024 B** ładunku. Dłuższa ramka to `FRAME_TOO_LARGE`, ramka o długości
0 to `MALFORMED_FRAME`; obie strona odrzuca po samej długości, przed
buforowaniem. Pola nieznane są pomijane (§4.3); znane muszą mieć swój typ.

| typ | nazwa | kierunek | pola |
|---|---|---|---|
| 0 | `PairHello` | klient → serwer | `0`: tryb (`0` QR, `1` SAS), `1`: nazwa (opcjonalnie), `2`: dowód (32 B) |
| 1 | `PairNonce` | serwer → klient (SAS) | `0`: `N_s` (32 B), `1`: nazwa (opcjonalnie) |
| 2 | `PairReveal` | klient → serwer (SAS) | `0`: `N_c` (32 B) |
| 3 | `PairResult` | serwer → klient (QR) | `0`: `mac_s` (32 B), `1`: nazwa (opcjonalnie) |
| 4 | `PairConfirm` | obie strony (SAS) | `0`: zgoda (bool) |

**Nazwa urządzenia** służy tylko do wyświetlania. Ma najwyżej 64 B UTF-8
(dłuższa to `MALFORMED_FRAME`). Przed pokazaniem i zapisem strona usuwa z
niej znaki sterujące (kategoria Cc) oraz znaki zmieniające kierunek tekstu
(U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069), a następnie
przycina białe znaki. Pusta nazwa liczy się jako brak nazwy. Te same
zasady dotyczą nazwy w linku (§12.4).

Przykłady:

```text
PairHello QR, nazwa "Pixel 9", dowód 11×32:
31 82 00 a3 00 00 01 67 50 69 78 65 6c 20 39 02 58 20 11…11

PairNonce N_s 22×32, nazwa "PC":
2a 82 01 a2 00 58 20 22…22 01 62 50 43

PairReveal N_c 33×32:   26 82 02 a1 00 58 20 33…33
PairResult mac 44×32:   26 82 03 a1 00 58 20 44…44
PairConfirm true:       05 82 04 a1 00 f5
```

### 12.4 Link parowania (QR)

```text
windconnect:pair?v=1&k=<K_s>&s=<sekret>&a=<host:port>[&a=…][&n=<nazwa>]
```

- `v`: wersja linku, `1`.
- `k`: klucz publiczny serwera (32 B), base64url bez dopełnienia.
- `s`: sekret (16 B z kryptograficznego generatora), base64url bez
  dopełnienia.
- `a`: adres endpointu parowania, 1–8 razy, w postaci `1.2.3.4:5` albo
  `[fe80::1]:5`, z kodowaniem procentowym.
- `n`: nazwa serwera, z kodowaniem procentowym (§12.3).

Odbiorca odrzuca link dłuższy niż 1 024 znaki, z inną wersją, bez `k`,
`s` lub `a`, z powtórzonym parametrem innym niż `a`, z niekanonicznym
base64url albo z kluczem, który nie jest punktem Ed25519. Nieznane
parametry pomija. Link to sekret: aplikacja nie zapisuje go ani nie
loguje.

Ścieżka SAS nie ma linku. Serwer z otwartym oknem SAS ogłasza port
endpointu parowania w mDNS (§2.0) w polu TXT `pair=<port>` i usuwa je po
zamknięciu okna (WN-82; aplikacja łączy `PairingListener` z odkrywaniem,
WC-27). Ogłoszenie jest niezaufaną wskazówką, jak całe mDNS: adres bierze
się z A/AAAA tego samego ogłoszenia, a uwierzytelnia §12.6.

### 12.5 Ścieżka QR

```text
mac_c = HMAC-SHA256(s, "wc-pair-v1 client" ‖ K_s ‖ K_c ‖ E)
mac_s = HMAC-SHA256(s, "wc-pair-v1 server" ‖ K_s ‖ K_c ‖ E)
```

1. Klient łączy się (TLS sprawdza `K_s` z linku) i wysyła
   `PairHello {0: 0, 1: nazwa, 2: mac_c}`.
2. Serwer sprawdza `mac_c` w czasie stałym. Jeśli MAC jest zły: zamyka
   połączenie kodem `UNTRUSTED_PEER` i liczy nieudaną próbę. Jeśli jest
   dobry: zamyka okno (sekret zużyty), zapisuje i przypina `K_c`, po czym
   wysyła `PairResult {0: mac_s, 1: nazwa}`.
3. Klient sprawdza `mac_s` w czasie stałym (serwer też zna sekret),
   zapisuje i przypina `K_s`, kończy strumień i zamyka połączenie kodem
   `NO_ERROR`. Zły `mac_s`: zamknięcie kodem `UNTRUSTED_PEER`, nic nie
   jest zapisane.

Bez sekretu napastnik nie policzy `mac_c`. MAC wiąże `E`, więc dowód z
innej sesji TLS (przekaźnik) nie pasuje. MAC wiąże też `K_c` z
uzgadniania, więc nie da się dowodu jednego klienta użyć dla innego
klucza. Podszycie się pod serwer kończy się już w TLS (§12.1, punkt 2).

### 12.6 Ścieżka SAS

```text
C   = SHA-256("wc-pair-v1 commit" ‖ K_c ‖ K_s ‖ N_c)
SAS = be_u32(SHA-256("wc-pair-v1 sas" ‖ E ‖ K_c ‖ K_s ‖ N_c ‖ N_s)[0..4]) mod 10^6
```

1. Klient losuje `N_c` (32 B) i wysyła `PairHello {0: 1, 1: nazwa, 2: C}`.
2. Serwer losuje `N_s` (32 B) i dopiero wtedy wysyła
   `PairNonce {0: N_s, 1: nazwa}`.
3. Klient wysyła `PairReveal {0: N_c}` dopiero po otrzymaniu `N_s`. Serwer
   sprawdza `C`; niezgodność to `PROTOCOL_VIOLATION` i nieudana próba.
4. Obie strony liczą `SAS` i pokazują go jako sześć cyfr w dwóch grupach
   (`020 627`) razem z nazwą drugiej strony.
5. Każda strona wysyła `PairConfirm {0: zgoda}`, gdy jej użytkownik
   zdecyduje. Odrzucenie wysyła `false` i zamyka połączenie kodem
   `CANCELLED`, a druga strona traktuje takie zamknięcie jak odrzucenie.
   Strona zapisuje i przypina klucz drugiej tylko wtedy, gdy zaakceptował
   jej użytkownik **i** druga strona przysłała `true`.
6. Zakończenie (WN-27): serwer po obu zgodach zapisuje i przypina klucz
   klienta, a dopiero potem kończy swój kierunek strumienia (FIN) i czeka,
   aż klient zamknie połączenie. Klient po obu zgodach kończy swój kierunek
   (FIN) i czeka na FIN serwera (limit kroku, 10 s); dopiero po nim zapisuje
   i przypina klucz serwera, po czym zamyka połączenie kodem `NO_ERROR`.
   Zamknięcie połączenia, dane po `PairConfirm` albo brak FIN w czasie to
   błąd: klient niczego nie zapisuje. Dzięki temu zamknięcie przez klienta
   nie porzuci jego `PairConfirm(true)` w drodze, a jedyna możliwa
   asymetria to serwer, który zapisał klienta, gdy jego FIN nie doszedł
   (jak w §12.8).

Kolejność jest wymuszona: wiadomość poza nią (np. `PairReveal` przed
`PairHello`, `PairConfirm` w miejscu `PairReveal`, druga `PairNonce`) to
`PROTOCOL_VIOLATION`. Napastnik pośrodku musi ustalić `N_c` (commitem)
albo `N_s` (wysyłając je), zanim pozna wartość drugiej strony, więc nie
dopasuje kodów po obu stronach inaczej niż z szansą 1 na 10^6 na próbę.
Limit prób (§12.2) ogranicza liczbę podejść. Bez commitu napastnik mógłby
po swojej stronie offline zmieniać udział klucza TLS, aż kody się zgodzą.

### 12.7 Wektory

Klucze to klucze publiczne Ed25519 z kluczy prywatnych `01×32`
(`K_s`) i `02×32` (`K_c`); `s = 11×16`, `E = 33×32`, `N_c = 44×32`,
`N_s = 55×32`.

| wartość | wynik |
|---|---|
| `mac_c` | `34061136437d946a87b158b60bfb316f361cd3ad9ea721ef346fa3cf4dc67f6f` |
| `mac_s` | `b434849080e57e61e837b556759352bf6d34d0e7d67f157a930f654fff8a37b3` |
| `C` | `649b9a0ba332e1b048f68641303bd9b699ada816d3079976c542d7caf4edac10` |
| `SAS` | `020 627` |

### 12.8 Zapis sparowanych urządzeń

Każda strona zapisuje sparowane urządzenie w pliku w folderze klucza
tożsamości (Windows: `%LOCALAPPDATA%\WindConnect\paired.txt`; w Unix prawa
`0600`). Do pliku tylko się dopisuje, jednym zapisem z synchronizacją,
rekord `\n<klucz hex> <sekundy Unix>[ <nazwa z kodowaniem procentowym>]`.
Każdy rekord **zaczyna się** od znaku nowej linii, więc rekord urwany przez
awarię zostaje sam w swojej linii, a następny zaczyna się czysto. Dwa
procesy dopisujące naraz nie gubią swoich rekordów. Późniejszy rekord
tego samego klucza zmienia nazwę i czas. Odczyt pomija linie, których nie
da się odczytać, zgłasza je i nigdy nie przepisuje pliku.

Usunięcie też jest dopisywane (WC-28): rekord `\n-<klucz hex> <sekundy>`
usuwa klucz i oznacza, że jego urządzeniu należy się powiadomienie
(§12.10); `\n=<klucz hex> <sekundy>` kończy sprawę (powiadomienie
doręczone albo to tamto urządzenie usunęło parowanie). Późniejszy zwykły
rekord tego klucza paruje go ponownie. Zmiana nazwy to zwykły rekord z tym
samym czasem parowania i nową nazwą; nazwa jest lokalna i nigdy nie jest
wysyłana. Starsza wersja aplikacji uzna `-`/`=` za uszkodzone linie i
zachowa klucz: wersje się nie cofają.

Serwer zapisuje klienta w kroku 2 (QR) albo po obu zgodach (SAS), zanim
wyśle wynik. Jeśli połączenie zerwie się później, serwer ma zapisany klucz,
a klient nie. Taki klucz nic nie otwiera (klient go nie przypiął), a
ponowne parowanie lub rozparowanie (§12.10) porządkuje stan.

### 12.9 Kody błędów

Zamknięcie połączenia parowania niesie tylko kod z §10, z pustym powodem.
Żadna wiadomość nie mówi, który krok zawiódł.

| sytuacja | kod |
|---|---|
| brak okna, okno wygasłe | `GOING_AWAY` |
| okno zablokowane po 3 próbach, trwa inna próba | `RATE_LIMITED` |
| zły `mac_c` lub `mac_s` | `UNTRUSTED_PEER` |
| zły tryb, kolejność, commit | `PROTOCOL_VIOLATION` |
| zła lub za duża ramka | `MALFORMED_FRAME`, `FRAME_TOO_LARGE` |
| brak kroku w 10 s | `HELLO_TIMEOUT` |
| kod odrzucony, brak decyzji, rozłączenie | `CANCELLED` |
| błąd zapisu po stronie serwera | `INTERNAL_ERROR` (nie liczy się jako próba) |
| sukces (zamyka klient) | `NO_ERROR` |

### 12.10 Rozparowanie i jedna sesja (WC-28)

Rozparowanie na jednym końcu usuwa klucz na drugim, także gdy drugi koniec
jest akurat poza siecią. Jedna droga dla obu przypadków:

1. Strona A zapisuje `-<klucz B>` (§12.8), odpina klucz B i oznacza go jako
   **odwołany**, a końcówka zapomina bilety TLS B (żadne wznowienie ani
   0-RTT z biletu sprzed rozparowania).
2. A wysyła `Goodbye` z kodem `UNPAIRED`: od razu w trwającej sesji z B
   albo przy najbliższym połączeniu B (punkt 4). Potem zamyka połączenie
   tym samym kodem.
3. B po `Goodbye UNPAIRED` (albo zamknięciu z tym kodem) usuwa **klucz
   rozmówcy z uzgadniania TLS** tego połączenia (zapis `=`), odpina go i
   nie łączy się ponownie. Wiadomość nie niesie żadnego klucza, a jej
   `message` jest ignorowane.
4. **Tryb powiadomienia.** Połączenie z kluczem odwołanym kończy
   uzgadnianie (dowód posiadania klucza), ale nie jest sesją: aplikacja
   widzi je dopiero po pełnym uzgadnianiu i sprawdzeniu klucza, więc nie
   czyta danych 0-RTT przed tą decyzją. Strona odwołująca nie instaluje
   routera, nie przyjmuje strumieni poza `control`, odpowiada na `Hello`
   swoją nazwą i pustą listą możliwości (nic nowego), wysyła
   `Goodbye UNPAIRED` i zamyka. Jeśli to ona dzwoni (telefon, który usunął
   komputer, gdy ten był poza siecią), robi pełne uzgadnianie bez 0-RTT
   po zobaczeniu komputera w odkrywaniu (LAN albo przekaźnik §13).
5. A zapisuje `=<klucz B>` dopiero, gdy B potwierdził odbiór całego
   `control` po jego końcu (potwierdzenie QUIC, nie lokalne wysłanie).
   Zerwanie wcześniej zostawia `-` i powiadomienie idzie ponownie.
   „Doręczone” znaczy więc: transport B przyjął te bajty, nie że aplikacja
   B je przetworzyła. Dlatego odbiorca MUSI obsłużyć `UNPAIRED` w każdej
   przyjętej sesji, także odrzuconej (`BUSY`) i takiej, która skończyła
   się przed użyciem: komputer, zanim odpowie `BUSY`, czeka do 500 ms na
   `Goodbye UNPAIRED` tej sesji, a telefon sprawdza kod zamknięcia
   połączenia, które nie zostało sesją. Powiadomienie spóźnione (wolny
   telefon, duże RTT, przekaźnik §13) może minąć się z `BUSY`: komputer
   po wysłaniu `Goodbye BUSY` sprawdza jeszcze raz, czy `UNPAIRED` nie
   doszło, i wtedy usuwa klucz i zamyka z `NO_ERROR` zamiast `BUSY`.
   A po potwierdzeniu czeka do 1 s na zamknięcie przez B: zamknięcie z
   `BUSY` znaczy, że aplikacja B go nie przeczytała, więc zostaje `-`.
6. Powiadomienie należy się przez **30 dni**. Potem, albo gdy B usunie
   klucz A w inny sposób, próby B kończą się odmową TLS (§2,
   `UNTRUSTED_PEER`); B pokazuje „Komputer odrzuca połączenie — sparuj
   ponownie” i przestaje ponawiać sam.

Kilka telefonów sparowanych z jednym komputerem: komputer ma jedną sesję
naraz. Ta sama strona (ten sam klucz) zastępuje swoje stare połączenie
(np. po zmianie sieci). Inny telefon dostaje `Goodbye BUSY` i nie ponawia
sam, aż zmieni się jego sieć albo użytkownik wybierze „Połącz”, więc dwa
telefony nie wyrzucają się nawzajem. Wyjątki są wyborem człowieka przy
komputerze: telefon sparowany na nim przed chwilą (2 min) przejmuje sesję,
a „Rozłącz” w panelu zamyka sesję `GOING_AWAY` i przez 10 min (albo do
sesji innego telefonu) odpowiada temu telefonowi `BUSY`, żeby inny mógł się
połączyć. LIMIT: telefon po `BUSY` nie wraca sam także wtedy, gdy komputer
się zwolni; czeka na zmianę sieci albo „Połącz”.

## 13. Przekaźnik poza domem (`/v1/relay`, WN-59)

Przekaźnik pozwala dwóm sparowanym urządzeniom w różnych sieciach rozmawiać
tym samym QUIC co w LAN (§2). Przenosi pakiety UDP sesji w ramkach
WebSocket i nie zna żadnego klucza sesji ani klucza urządzenia, więc nie
może odczytać ani zmienić treści: szyfrowanie i uwierzytelnienie końców
pozostają w TLS 1.3 z przypiętymi kluczami. Przekaźnik jest opcją wyłączoną
domyślnie (decyzja D3); klient włącza go w ustawieniach (WC-60). Serwer to
Cloudflare Worker z Durable Object w katalogu `relay/`.

### 13.1 Połączenie

- Urządzenie otwiera `wss://<host>/v1/relay?me=<klucz>&peer=<klucz>`, gdzie
  oba klucze to 64 małe znaki hex kluczy Ed25519 (§2.1): własny i
  sparowanego urządzenia. Inne kształty kończą się HTTP 400; brak
  nagłówka `Upgrade: websocket` daje 426; przekroczony limit nowych
  połączeń z jednego IP (30 na minutę) daje 429.
- Miejsce spotkania pary to
  `SHA-256("wc-relay-v1 pair" ‖ niższy klucz ‖ wyższy klucz)`, gdzie
  „niższy” oznacza mniejszy w porządku bajtów. Oba urządzenia muszą wskazać
  siebie nawzajem; urządzenie, które nikogo nie wskazało, jest dla innych
  nieosiągalne.
- Para trzyma najwyżej 4 gniazda przed uwierzytelnieniem. Nowe gniazdo przy
  pełnej puli zamyka najstarsze nieuwierzytelnione kodem 4004, zamiast być
  odrzucone, więc ktoś, kto zna oba klucze publiczne, nie zablokuje pary
  niepodpisanymi gniazdami. Gniazda uwierzytelnione się nie liczą: każde
  urządzenie ma najwyżej jedno (§13.2).

### 13.2 Uwierzytelnienie

Ramki tekstowe są obiektami JSON z polem `t`. Przekaźnik zaczyna od
`{"t":"challenge","nonce":"<64 hex>"}` z losowymi 32 bajtami. Urządzenie
odpowiada `{"t":"auth","sig":"<128 hex>"}`, podpisem Ed25519 swoim kluczem
urządzenia nad

```text
"wc-relay-v1 auth" ‖ nonce ‖ własny klucz ‖ klucz peera
```

Kontekst oddziela ten podpis od podpisów TLS tym samym kluczem. Podpis z
innym nonce, dla innego peera, złym kluczem albo spóźniony ponad 10 s
zamyka gniazdo kodem 4001. Po sukcesie przekaźnik wysyła
`{"t":"ready","peer":<bool>}` (czy peer już czeka), a później
`{"t":"peer","present":<bool>}`, gdy peer dołącza lub odchodzi. Podpis
dowodzi tylko, które urządzenie się łączy; zaufanie do peera daje wyłącznie
przypięty klucz w TLS.

Jedno urządzenie ma w parze najwyżej jedno uwierzytelnione gniazdo: nowe
zamyka poprzednie kodem 4003, a peer nie dostaje wtedy `present:false`.

### 13.3 Dane

- Po uwierzytelnieniu ramki binarne niosą po jednym pakiecie UDP QUIC i
  trafiają bez zmian do drugiego urządzenia, jeśli jest obecne. Bez peera
  ramka przepada jak zgubiony pakiet UDP.
- Ramka binarna ponad 2048 B zamyka gniazdo kodem 1009. Ramka binarna
  przed uwierzytelnieniem i ramka tekstowa po nim (poza pingiem) zamykają
  kodem 4002.
- `{"t":"ping"}` dostaje `{"t":"pong"}` (podtrzymanie bez budzenia obiektu).
- Każde urządzenie ma kubeł tokenów: 1 MiB/s, chwilowo do 4 MiB
  (zmienne `RATE_BYTES_PER_SEC`, `BURST_BYTES`). Ramka ponad limit przepada
  bez zamykania gniazda; QUIC dostosowuje tempo.
- Przekaźnik nie zapisuje ani nie loguje treści ramek.

| kod zamknięcia | znaczenie |
|---|---|
| 4001 | uwierzytelnienie nieudane lub spóźnione |
| 4002 | ramka niedozwolona w tym stanie |
| 4003 | zastąpione nowszym połączeniem tego urządzenia |
| 4004 | gniazdo nieuwierzytelnione wyparte przez nowsze (pełna pula) |
| 1009 | ramka danych ponad 2048 B |

### 13.4 Co widzi przekaźnik

Operator przekaźnika (Cloudflare i właściciel konta) widzi adresy IP obu
urządzeń, oba klucze publiczne i identyfikator pary w chwili łączenia, czas
połączeń oraz liczbę i rozmiar ramek. Nie widzi treści, nazw urządzeń ani
typów wiadomości: to wszystko jest wewnątrz QUIC. Worker nie zapisuje tych
metadanych: Workers Logs są wyłączone (`observability.enabled: false`), a kod
niczego nie loguje. Włączenie przekaźnika (D3) oznacza zgodę na to, że
operator może te metadane zobaczyć.

### 13.5 Klient (WN-60)

- Przekaźnik jest wyłączony, dopóki użytkownik nie poda jego adresu
  (`wss://…`; `ws://` tylko do adresu loopback lub prywatnego, dla
  lokalnego `wrangler dev`). Wyłączony przekaźnik nie jest nigdy używany.
- Klient łączy się najpierw pod adres urządzenia w LAN. Przy włączonym
  przekaźniku ta próba ma 2 s; potem łączy się przez przekaźnik. Odmowa
  zaufania (`UNTRUSTED_PEER`) nie przechodzi na przekaźnik.
- Przez przekaźnik działa ta sama sesja QUIC (§2): urządzenie trzyma
  lokalny adres UDP, który zastępuje peera. Pakiety własnego endpointu
  wysłane na ten adres idą do przekaźnika, a ramki z przekaźnika trafiają
  do endpointu jak od peera. Do przekaźnika trafiają tylko pakiety z
  aktualnego adresu własnego endpointu.
- PC z włączonym przekaźnikiem czeka na nim na każde sparowane urządzenie
  i łączy się ponownie z narastającym opóźnieniem (1–60 s), gdy połączenie
  się kończy. Bezczynne gniazdo co 25 s wysyła `{"t":"ping"}`.
- Po zmianie sieci w czasie sesji przez przekaźnik klient próbuje adresu
  w LAN (2 s). Jeśli odpowiada, nowa sesja w LAN zastępuje sesję przez
  przekaźnik.
