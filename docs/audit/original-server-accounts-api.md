# Audyt bezpieczeństwa — Modlight (serwer, konta, uwierzytelnianie, API)

Zakres: serwer Next.js 16 + własny `server.ts`, better-auth, drizzle/PostgreSQL. Przedmiot
audytu: `/path/to/modlight/upstream/Modlight`. Analiza wyłącznie statyczna
(czytanie kodu); nic z repozytorium nie było uruchamiane.

Commit: `a59d03c` (jeden commit, 2026-10-18).

## Podsumowanie

Kod jest nietypowo mocno zahartowany i gęsto skomentowany uzasadnieniami bezpieczeństwa.
Przeszedłem ścieżkami od wejścia do skutku dla: rejestracji i bootstrapu admina, logowania,
2FA, resetu hasła, zaproszeń, tokenów API i egzekwowania zakresów, każdego route handlera w
`app/api/**`, pozostałych `route.ts`, server actions w `app/actions/**`, przepływu autoryzacji
aplikacji desktopowej (loopback), publikacji wydań, migracji i skryptów.

Nie znalazłem:
- backdoora, ukrytego konta, hasła‑wytrychu ani magicznego nagłówka omijającego logowanie,
- ścieżki eskalacji użytkownik → admin na istniejącej instalacji,
- IDOR‑a (każdy dostęp do zasobu jest zawężony właścicielem w klauzuli WHERE),
- obejścia zakresów tokenów (upload/read/write/full są egzekwowane przy każdym wywołaniu),
- wycieku danych na zewnątrz (żaden kod z tego obszaru nie wysyła danych do obcych hostów —
  patrz sekcja „Wysyłanie danych na zewnątrz”),
- `eval`/`new Function`/`child_process` (poza `ffmpeg`/`ffprobe`, które mają whitelistę
  protokołów `file,pipe` i wymuszony demukser `-f`).

Najważniejsze ustalenia (szczegóły niżej):

| Waga | Miejsce | Zdanie |
|---|---|---|
| Średnia | `lib/upload/sharex.ts:206` (+ `server.ts`) | Nieuwierzytelniony rozmówca może wymusić zapis do ~512 MB na dysk na połączenie, zanim token zostanie sprawdzony; przy braku Content-Length limit wstępny nie działa → DoS dyskowy. |
| Średnia | `lib/auth.ts:252-275` | Przydział roli „admin” pierwszemu kontu liczy użytkowników bez blokady (TOCTOU); dotyczy tylko pustej instalacji, nie daje eskalacji na istniejącej. |
| Niska | `app/api/releases/app/route.ts:79-84` | POST publikujący wydanie czyta `req.json()` bez limitu rozmiaru i bez kontroli Content-Type; CSRF łagodzone przez SameSite=Lax, ale niespójne z resztą API. |
| Niska | `app/[userCode]/[mediaCode]/delete/[token]/route.ts:28` | Porównanie tokenu usuwania zwykłym `===` (nie w stałym czasie); ryzyko znikome przy wysokiej entropii, ale niespójne z `timingSafeEqual` w innych miejscach. |
| Informacyjna | konfiguracja odwrotnego proxy | Cały model limitów ufa wyłącznie `x-real-ip`; proxy, które go nie nadpisze (albo bezpośrednie wystawienie portu), wyłącza throttling logowania/klucza/haseł plików. Udokumentowane, ale to największy realny foot-gun. |
| Informacyjna | `lib/authorize/loopback.ts`, `app/(auth)/authorize` | Handoff tokenu desktopowego (RFC 8252) bez PKCE; `state` tylko odbijany, nie wiązany po stronie serwera. Zgodne ze standardem; ryzyko rezydualne wymaga lokalnego nasłuchu. |

---

## Ustalenia szczegółowe

### [Średnia] Nieuwierzytelniony zapis dużego ciała na dysk przed sprawdzeniem tokenu (DoS)
- Plik: `lib/upload/sharex.ts` (meter: `:198-221`, wstępny check: `:126-134`), `server.ts:82-88`.
- Pewność: prawdopodobne (dokładne zachowanie zależy od FS i współbieżności).
- Ścieżka kodu: `POST /upload` i `POST /api/upload` są obsługiwane w surowej warstwie Node
  (przed Next). Uwierzytelnienie z nagłówka następuje przed IO tylko gdy nagłówek jest obecny
  (`bearerFromHeaders`). Jeśli klient nie poda nagłówka (legacy: token jako pole formularza),
  busboy strumieniuje plik do `tmpPath`, a limit anonimowy (`ANON_MAX_BYTES`, domyślnie
  **512 MB**, `LEGACY_UPLOAD_MAX_SIZE`) przerywa dopiero po przekroczeniu. Wstępny
  `hasRoomFor(declared, ...)` działa tylko dla skończonego `Content-Length > 0`
  (`:126-134`), więc żądanie **chunked bez Content-Length** go omija.
- Skutek: anonimowy rozmówca otwiera wiele połączeń chunked do `/upload`, w każdym wysyła do
  ~512 MB zanim finalny brak tokenu zwróci 400. Pliki tymczasowe są sprzątane w `finally`,
  ale w trakcie transferu zajmują `UPLOAD_TMP_PATH` (ten sam system plików co magazyn) —
  a kod sam gdzie indziej ostrzega, że pełny dysk „kładzie każdą aplikację na hoście”. Brak
  limitu szybkości na tej ścieżce dla anonima.
- Wejście → skutek: `POST /api/upload`, `Transfer-Encoding: chunked`, bez `x-api-key`, bez
  pola `token`, ciało > kilkaset MB, N równoległych połączeń → przejściowe wyczerpanie
  miejsca na dysku magazynu.
- Propozycja: wymuszać token/sesję zanim pojawi się choć bajt ciała (jak na ścieżce tus:
  `onUploadCreate` uwierzytelnia przed zapisem), albo twardo ograniczyć anonimowy bufor do
  kilku MB i throttlować beztokenowe `POST /upload` po `x-real-ip`. Rozważyć odrzucanie
  chunked bez `Content-Length` na tej ścieżce.

### [Średnia] Przydział roli admina pierwszemu kontu bez transakcyjnej blokady (TOCTOU)
- Plik: `lib/auth.ts:252-275` (`databaseHooks.user.create.before`), wsparcie:
  `lib/registration.ts:161-164`, `lib/auth.ts:206-239`.
- Pewność: potwierdzone (że liczenie jest bez blokady); skutek ograniczony.
- Ścieżka kodu: hook liczy `count()` użytkowników i jeśli 0, nadaje rolę `admin`. Dwie
  jednoczesne „pierwsze” rejestracje na pustej instalacji mogą obie odczytać `n=0` i obie
  dostać admina. Niezależnie bramka `evaluateSignUp` też liczy (`isFirstAccount`).
- Skutek: na **pustej** instalacji można uzyskać dwóch adminów zamiast jednego. Na
  **istniejącej** instalacji `count>0`, więc self‑signup zawsze dostaje rolę `user` — nie ma
  eskalacji. Jawnie podana rola `admin` jest honorowana tylko z `/admin/create-user`
  (chronione przez plugin admin better-auth), więc stronniczy stranger jej nie wstrzyknie.
- Wejście → skutek: dwa równoległe `POST /sign-up/email` w oknie bootstrapu → 2 konta admin.
- Propozycja: nadawać pierwszego admina w jednej transakcji z blokadą (np. `INSERT ...
  SELECT ... WHERE NOT EXISTS`, albo advisory lock), żeby „pierwszość” była rozstrzygana
  atomowo. Priorytet niski ze względu na wąskie okno i brak eskalacji na żywej instalacji.

### [Niska] POST publikacji wydania bez limitu ciała i bez kontroli Content-Type
- Plik: `app/api/releases/app/route.ts:75-88`.
- Pewność: potwierdzone.
- Ścieżka kodu: handler woła `await req.json()` bezpośrednio — brak górnego limitu rozmiaru
  (reszta API używa `readJsonObject`/`readCappedBody` z limitem) i brak wymogu
  `application/json`, który w `app/api/shortlinks/route.ts:233-241`,
  `app/api/client-settings` i `app/api/theme` jest świadomie użyty jako kontrola CSRF dla
  tras akceptujących ciasteczko sesji. Trasa jest admin-only (`requireAdminCaller`), a CSRF
  jest dodatkowo łagodzone domyślnym `SameSite=Lax` ciasteczka sesji better-auth (brak
  nadpisania w `lib/auth.ts`), więc żądanie cross-site i tak nie poniesie ciasteczka.
- Skutek: realne ryzyko minimalne (potrzebny admin‑ofiara i znajomość `code` istniejącego
  publicznego uploadu), ale rozbieżność z udokumentowaną w repo zasadą „Content-Type to
  kontrola CSRF” oraz brak limitu ciała (drobny DoS pamięci na trasie admina).
- Propozycja: przepuścić ciało przez `readJsonObject` (limit + wymóg `application/json`),
  tak jak pozostałe trasy zapisu.

### [Niska] Porównanie tokenu usuwania w czasie zależnym od danych
- Plik: `app/[userCode]/[mediaCode]/delete/[token]/route.ts:28`
  (`token === media.deleteToken`); podobnie dopieranie tokenów API na indeksowanym
  wyszukaniu hasza bez stałego czasu (`lib/api-token.ts:107-114`).
- Pewność: potwierdzone (niestałoczasowość); skutek praktyczny znikomy.
- Ścieżka kodu: `deleteToken` i tokeny API to sekrety o wysokiej entropii (`secureToken`,
  `randomBytes`), więc atak czasowy przez sieć na wydobycie 64‑znakowego sekretu jest
  niepraktyczny. Mimo to reszta kodu (tokeny odblokowania plików, bilety renderu) używa
  `timingSafeEqual`, więc to niespójność warta wyrównania.
- Propozycja: porównywać `deleteToken` przez `timingSafeEqual` nad buforami równej długości
  (jak w `lib/password.ts:249` / `lib/media/render-ticket.ts:146`).

### [Informacyjna] Model limitów całkowicie zależny od `x-real-ip`
- Plik: `lib/password.ts:375-377`, `lib/auth.ts:100-104`, `docker/Caddyfile:36`,
  `docker/nginx.example.conf:25`, `docker/docker-compose.expose.yml`.
- Ścieżka kodu: `clientIp()` ufa wyłącznie `x-real-ip`; better-auth ma `ipAddressHeaders:
  ["x-real-ip"]`. To słuszny wybór (X-Forwarded-For jest dopisywalny przez klienta). Ryzyko
  jest konfiguracyjne: jeśli odwrotne proxy **nie nadpisze** `x-real-ip` (albo aplikacja
  zostanie wystawiona wprost — przed czym `docker-compose.expose.yml` ostrzega), to każdy
  klient nadaje sobie dowolne `x-real-ip` i dostaje świeży kubełek na każde żądanie —
  wyłączając throttling logowania, klucza rejestracji, odblokowania haseł plików i resendu
  potwierdzeń. Udokumentowane obszernie (`docs/deployment.md`), ale to największy realny
  foot-gun wdrożeniowy.
- Propozycja: pozostawić jak jest w kodzie; w dokumentacji/healthchecku rozważyć twarde
  ostrzeżenie startowe, gdy `TRUST_PROXY=true`, a nagłówek nie dociera.

### [Informacyjna] Handoff tokenu desktopowego (loopback) bez PKCE
- Plik: `lib/authorize/loopback.ts`, `app/(auth)/authorize/page.tsx`,
  `app/actions/authorize.ts`, `components/authorize/authorize-prompt.tsx`, `next.config.ts:68-94`.
- Ocena: implementacja jest poprawna i ostrożna. `callback` budowany jest wyłącznie z
  **sparsowanego numeru portu** (`loopbackCallback`), nigdy z ciągu od klienta — otwarte
  przekierowanie poza loopback jest niemożliwe (tylko `http://127.0.0.1:<port>/`, port
  1024–65535). `name` sanityzowane (usuwa znaki sterujące i bidi‑override). Mennica tokenu to
  Server Action (POST) wymagający sesji i kliknięcia człowieka; zakres tokenu to `write`
  (nie `full`), więc desktop nie może publikować wydań. CSP dla `/authorize` rozszerza
  `connect-src` o `http://127.0.0.1:*` wyłącznie dla tej ścieżki (reszta aplikacji nie).
- Ryzyko rezydualne: brak PKCE i brak serwerowego wiązania `state` oznacza, że `state` służy
  tylko aplikacji do rozpoznania własnej odpowiedzi. Przechwycenie tokenu wymaga procesu
  nasłuchującego na porcie loopback ofiary (lokalny dostęp) — zgodnie ze standardem RFC 8252
  i akceptowalne. Nic do naprawy; odnotowane, bo było w zakresie pytań.

---

## Błędy logiczne

Nie znalazłem błędów logicznych skutkujących utratą danych, złą kolejnością czy złymi
warunkami ponad to, co wymieniono wyżej. Kilka miejsc sprawdzonych pod tym kątem i uznanych
za **poprawne** (odnotowuję, bo były w zakresie):

- Kolejność w `updateUserAccount` (`app/actions/admin.ts:331-550`): walidacja całości przed
  zapisem, tag jako pierwszy (bo jego unikalność rozstrzyga sam zapis), samo‑demotowanie i
  samo‑ban odrzucane — spójne, bez stanu „pół konta zmienione”.
- `app/api/uploads/route.ts` `reachableBy()` (`:529-533`): tryb `permanent` celowo widzi
  wiersze z `deleted_at` i bez — wcześniejszy bug (permanent nie czyścił kosza) jest opisany
  i naprawiony; `trash`/`restore` mają rozłączne zakresy. OK.
- Rotacja domyślnego hasła plików (`app/actions/account.ts:464-510`): bump wersji + przepis
  istniejących w jednej transakcji; pliki z ręcznym hasłem (`passwordVersion=null`) i bez
  hasła nietknięte. OK.
- `isNarrowing` (`lib/api-scopes.ts:109-112`) i `narrowApiTokenScope` (`lib/api-token.ts:248-281`):
  zawężenie to test podzbioru pojemności (nie ranga), a stary zakres wpięty w WHERE UPDATE —
  dwa równoległe zawężenia nie mogą przeplatać się w rozszerzenie. OK.
- Bramka rejestracji (`lib/registration.ts:85-103`): bootstrap > rate‑limit > „zamknięte” >
  klucz; nieodczytywalny klucz = zamknięte (fail‑closed). OK.

---

## Wysyłanie danych na zewnątrz (mój zakres)

W obszarze serwer/konta/uwierzytelnianie/API **nie ma** kodu wysyłającego dane do obcych
hostów. Konkretnie:

- Jedyny wychodzący ruch sieciowy serwera to **SMTP** przez nodemailer (`lib/mail.ts`), do
  hosta skonfigurowanego przez administratora (panel lub env). Treść: maile weryfikacji,
  resetu hasła, zaproszeń, „zmieniono hasło/e‑mail”, test SMTP. Błędy SMTP są redagowane z
  sekretów (`redactSecrets`, `smtpSecretForms`) przed trafieniem do logu/audytu. Hasło SMTP
  nigdy nie wraca do przeglądarki (`toMailStatus`), przechowywane szyfrowane (AES‑256‑GCM,
  `lib/secret-box.ts`).
- Serwer S3 (`lib/storage/s3.ts`) tylko gdy `STORAGE_DRIVER=s3`, do endpointu operatora.
- `ffmpeg/ffprobe` mają whitelistę protokołów `file,pipe` i wymuszony `-f`
  (`lib/media/ffmpeg.ts`), więc nie pobiorą niczego po sieci (ochrona SSRF).
- Fetch po stronie klienta idzie wyłącznie do własnych tras (`/api/avatar`,
  `/api/background`, `/api/uploads/check`) oraz do loopbacku (`127.0.0.1:<port>`). Jedyne
  twarde zewnętrzne URL‑e to linki do GitHuba na stronie lądowania i `oembed.com` (tekst spec).
- `userEmail` konta (kontekst harnessu) nie trafia do żadnego żądania wychodzącego.

Nie znalazłem żadnej eksfiltracji tokenów, haseł ani sekretów.

---

## Mapa API (metoda, ścieżka, uwierzytelnienie / zakres)

Publiczne (bez uwierzytelnienia):
- `GET /api/health` — sonda; ujawnia tylko status podsystemów.
- `GET /api/releases/app` — manifest wydania (celowo publiczny, cache 300 s).
- `GET /download/app` — 302 do bieżącego wydania.
- `GET /avatar/{tag}`, `GET /bg/{tag}` — obrazy (zawsze przekodowany WebP; `nosniff`).
- `GET /{tag}/{code}` (strona), `/raw[.ext]`, `/download`, `/render`, `/timeline*`,
  `GET|POST /{tag}/{code}/delete/{token}` — token usuwania **lub** token API o zakresie
  `write` należący do właściciela; `/render` wymaga `Sec-Fetch-Dest: iframe` + ważnego biletu.
- `GET /a/{tag}/{slug}` i `/a/{tag}/{slug}/zip` — tylko folder `isPublic`; pozycje filtrowane
  (opublikowane, nie w koszu, bez hasła, nie wygasłe, w limicie odsłon) w
  `lib/data/folders.ts:299-333`.
- `GET /u/{tag}`, `GET /s/{code}[/..]`, `GET /oembed`, `GET /embed/{v}/{card}`.
- `POST /upload`, `POST /api/upload` — token API w nagłówku `x-api-key`/`Authorization` lub
  (legacy) pole `token`; zakres **upload**.
- `/api/files*` (tus: POST/PATCH/HEAD/…) — token API (zakres **upload**) **lub** sesja;
  kolejne PATCH‑e autoryzuje posiadanie URL‑a uploadu.

Token API (pierwszeństwo) lub sesja; zakres wymagany:
- `GET /api/me` — **identify** (każdy zakres).
- `GET /api/folders` — read; `POST/PATCH/DELETE /api/folders` — **write**.
- `GET /api/uploads` — read; `PATCH/DELETE /api/uploads` — **write** (zawężone do właściciela,
  bez przywileju admina na tej trasie).
- `POST /api/uploads/check` — **read**.
- `GET /api/duplicates` — read; `POST /api/duplicates` — **write**.
- `GET /api/shortlinks` — read; `POST /api/shortlinks` — **write** (+ limit szybkości).
- `GET /api/client-settings` — read; `PUT /api/client-settings` — **write**.
- `GET /api/theme` — read; `PUT /api/theme` — **write**.
- `GET /api/link-info` — **read**, ale z trybem anonimowym (zły/pusty token = odpowiedź
  publiczna, nie 403).

Admin (token o zakresie **full** + rola `admin`, albo sesja admina):
- `POST /api/releases/app` — publikacja wydania (dwie bramki: zakres `full` i rola admin).
- `GET /api/admin/disk` — raport dysku (nie‑admin dostaje 404 bajt‑w‑bajt jak brak trasy).

Tylko sesja przeglądarki:
- `POST|DELETE /api/avatar`, `POST|DELETE /api/background` — konto dzwoniącego (własne).
- `ALL /api/auth/[...all]` — better-auth (logowanie, 2FA, reset, weryfikacja, plugin admin
  chroniony rolą).
- Wszystkie server actions w `app/actions/**` (każda sprawdza `requireUser`/`requireAdmin`
  albo zawęża po właścicielu w WHERE; brak martwego kodu‑endpointów).
