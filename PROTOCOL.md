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
├── strumień uni   input        (jw., przyciski)              ramki CBOR
├── strumień bidi  files × N    (jeden na transfer)           ramki CBOR
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
- Aktualizacja rozwiązanego ogłoszenia zastępuje jego poprzednie adresy;
  goodbye lub wygaśnięcie rekordów usuwa usługę. Własny odcisk jest pomijany.

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
od serwera, przykład w sekcji 7.1) i `files` (odpowiedzi odbiorcy pliku).

| kanał | nazwa | rodzaj strumienia | otwiera | priorytet wysyłki |
|---|---|---|---|---|
| `00` | `control` | bidi | klient; dokładnie 1 na połączenie | 0 (najwyższy) |
| `01` | `clipboard` | uni | nadawca wiadomości | 2 |
| `02` | `notify` | uni | nadawca wiadomości | 3 |
| `03` | `media` | uni | nadawca wiadomości | 4 |
| `04` | `input` | uni + datagramy | nadawca zdarzeń | 1 |
| `05` | `files` | bidi | nadawca pliku; 1 na transfer | 7 (najniższy) |
| `06`–`3f` | zarezerwowane dla przyszłych kanałów | | | |
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

Kanały `clipboard`, `notify`, `media` i `input` mają w każdym kierunku
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
      do FIN, a jego wiadomości przekazane modułowi, albo porzucony
      (`RESET_STREAM`), albo odrzucony (`STOP_SENDING`). Strumień, który
      nadawca już zakończył, a którego wiadomości czekają na wolny moduł,
      nadal zajmuje limit. Strona MOŻE przyznawać strumienie partiami
      (np. gdy zwolni się ponad 1/8 limitu).
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
      na każdą wiadomość; inaczej wolny moduł jednego kanału może na
      chwilę wstrzymać strumienie innych.
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
| `clipboard/1` | strona obsługuje kanał `clipboard` |
| `notify/1` | strona obsługuje kanał `notify` |
| `media/1` | strona obsługuje kanał `media` |
| `input/1` | strona przyjmuje zdarzenia wejścia (kanał `input`, sekcja 8) |
| `files/1` | strona obsługuje kanał `files` |

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

## 9. Kanały funkcji i zmiany specyfikacji

Kanały `clipboard`, `notify`, `media` i `files` mają w v1.0 tylko numer,
możliwość i zasady strumieni (sekcja 3). Typy ich wiadomości dopisują
karty funkcji (WC-35, WC-38, WC-42, WC-46 i dalsze) jako zmiany tego
dokumentu:

1. Nowy typ lub pole opcjonalne: kolejna wersja minor (1.1, 1.2, …),
   nowy wpis w tabeli typów kanału, przykład binarny jak w sekcjach 7–8.
2. Zmiana znaczenia istniejącego typu lub pola: nowa wersja możliwości
   (`clipboard/2`), a dla ramkowania nowa wersja główna.
3. Numery typów i kluczy nie są nigdy używane ponownie, także po wycofaniu.
4. Zasady wspólne: wiadomości kanału uni są jednokierunkowe, a odpowiedzi
   podlegają sekcji 6, zasada 4. Strumień `files` jest dwukierunkowy:
   otwiera go nadawca pliku (nagłówek `01 05`, potem metadane i dane),
   odbiorca pisze w kierunku powrotnym potwierdzenia bez nagłówka, a koniec
   transferu to FIN obu stron. Strumień `files` przyjmuje tylko strona,
   która ogłosiła `files/1`.

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

Kody `0e`–`ff` są zarezerwowane dla v1; nieznany kod odbiorca traktuje jak
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
zamknięciu okna (ogłaszanie wdraża WC-27; do tego czasu adres podaje
aplikacja). Ogłoszenie jest niezaufaną wskazówką, jak całe mDNS.

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
da się odczytać, zgłasza je i nigdy nie przepisuje pliku. Usuwanie i
porządkowanie należą do listy urządzeń (WC-28).

Serwer zapisuje klienta w kroku 2 (QR) albo po obu zgodach (SAS), zanim
wyśle wynik. Jeśli połączenie zerwie się później, serwer ma zapisany klucz,
a klient nie. Taki klucz nic nie otwiera (klient go nie przypiął), a
ponowne parowanie lub rozparowanie (WC-28) porządkuje stan.

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
