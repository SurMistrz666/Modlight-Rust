# Audyt bezpieczeństwa: serwer Modlight w Rust (port)

Zakres: `/Users/marcinasdfg/Desktop/modlight/server` (axum + sqlx + Postgres, ~90 tys. linii),
gałąź `main` (w chwili audytu HEAD `cfe7411`, kod serwera bez zmian od merge `0933c6d`).
Oryginał TypeScript (`upstream/Modlight`) traktowany jako specyfikacja; audyty oryginału w
`docs/audyt/serwer-konta-api.md` i `docs/audyt/serwer-pliki-wyciek.md`, poprawki do wdrożenia w
`server/docs/PORT.md` sekcja 12 oraz `server/docs/SKELETON.md` sekcja 9 (pozycje 26-40).

Metoda: analiza statyczna całego drzewa `src/` (auth i sesje, tokeny/zakresy, framework RPC i
autoryzacja każdej akcji, loadery stron, uploady ShareX/tus, sterowniki magazynu, serwowanie
mediów i bramki, transformacje ffmpeg/libvips, poczta, admin, docker) plus testy dynamiczne na
**wyrzucanym** stosie Docker (`modlight-secreview`, 127.0.0.1:8449, własny `.env`, własna baza i
wolumeny, usunięty `down -v` po pracy). Stosów właściciela (8430, 8420, baza dev 55421) nie
dotykano. Sekrety nie były wypisywane.

Data: 2026-10-20.

## Werdykt skrótowy

**Nie znaleziono luki krytycznej ani wysokiej wprowadzonej w tłumaczeniu.** Port jest wierny i
nietypowo starannie zahartowany; autoryzacja, zawężanie po właścicielu, zakresy tokenów i bramki
widoków działają tak, jak zaprojektowano, i zostały potwierdzone dynamicznie. Wszystkie 7 poprawek
z audytu oryginału jest obecnych i **skutecznych** (6 z 7 odtworzonych na żywym stosie; poprawka 3
potwierdzona statycznie i testem). Kod **nie wysyła danych na zewnątrz** poza usługami, które
konfiguruje administrator (SMTP, S3, Postgres). Ustalenia to wyłącznie sprawy niskie/informacyjne,
w większości będące wiernym odwzorowaniem zachowania oryginału (parytet), a nie regresją portu.

---

## Czy kod wysyła dane na zewnątrz

Pełny przegląd połączeń wychodzących (grep: `reqwest`, `TcpStream`, `UdpSocket`, `tokio::net`,
`std::net::Tcp`, `hyper*client`, `aws_sdk`, `lettre`, `rustls`, `to_socket_addrs`, `lookup_host`,
`curl`, `fetch`). Zależności: `Cargo.toml` nie zawiera `reqwest`; jedyne biblioteki sieciowe to
`aws-sdk-s3`, `lettre`, `sqlx`.

| Miejsce | Dokąd | Jakie dane | Decyduje administrator? |
|---|---|---|---|
| `mail/smtp.rs`, `mail.rs` (lettre, rustls) | serwer SMTP z konfiguracji | maile weryfikacji/resetu/zaproszeń do użytkowników | TAK (host z env/panelu; brak hosta = brak wysyłki) |
| `storage/s3.rs`, `storage/s3/lib_storage.rs`, `upload/tus_server/s3_store.rs` (aws-sdk-s3) | `S3_ENDPOINT` z konfiguracji | pliki użytkowników | TAK (tylko gdy `STORAGE_DRIVER=s3`) |
| `db/*`, `main.rs` (sqlx) | `DATABASE_URL` | dane aplikacji | TAK |
| `main.rs::health()` | 127.0.0.1:PORT (pętla zwrotna) | jedno żądanie `GET /api/health` dla healthchecku kontenera | — (lokalne) |

- **Brak SSRF.** `routes/api/link_info.rs` + `link_info/parse.rs` odpowiadają wyłącznie z wiersza
  bazy (`get_media`/`SELECT`), nigdy nie pobierają opisywanego adresu, i odrzucają hosty spoza
  wdrożenia (`LinkHosts::is_known_host`) — potwierdzone na żywo (obcy host → `ForeignHost`).
- `ffmpeg/ffprobe`: `-protocol_whitelist file,pipe`, wymuszony `-f`, `-use_absolute_path 0` dla
  mov, `env_clear()` (poza `PATH`), `shell:false`, własna grupa procesów z kill po deadline
  (`media/ffmpeg.ts` → `media/ffmpeg.rs`). Brak wstrzyknięcia argumentów i pobierania po sieci.
- `libvips`: loadery z allowlisty sharp-libvips (SVG ładowany, ale serwowany jako `text/plain`;
  bez loadera sieciowego).
- Front (`server/web`): `fetch` wyłącznie na własny origin (`/_page`, `/_rpc`, `/api/*`) i na
  `http://127.0.0.1:<port>` dla handoffu desktopowego (RFC 8252, osobne CSP). Jedyne zewnętrzne
  URL-e to statyczne linki do GitHuba na stronie lądowania. Brak telemetrii/beaconów/analityki.
  `index.html`: skrypt inline to bootstrap motywu (`localStorage`), bez sieci.

**Werdykt: serwer nie wykrada danych. Brak backdoora, ukrytego konta, hasła-wytrychu, telemetrii
ani „phone home”. Jedyne wyjścia sieciowe to SMTP/S3/Postgres konfigurowane przez administratora
oraz lokalny healthcheck.**

---

## Poprawki z audytu oryginału (PORT.md sekcja 12)

| # | Poprawka | Status | Dowód |
|---|---|---|---|
| 1 | Limit wyświetleń (maxViews) nieomijalny `Range`/`Sec-Fetch-Dest`/`w,h,motion` | **Skuteczna (odtworzona)** | patrz niżej |
| 2 | ShareX: autoryzacja przed ciałem, kwota/miejsce na ciałach chunked | **Skuteczna (odtworzona)** | patrz niżej |
| 3 | Pierwszy admin w transakcji z blokadą | **Skuteczna (statycznie + test)** | `auth/adapter.rs:251` |
| 4 | `POST /api/releases/app`: limit ciała + Content-Type | **Skuteczna (statycznie)** | `routes/api/releases_app.rs:165` |
| 5 | Token usuwania porównywany w stałym czasie | **Skuteczna (statycznie)** | `routes/media_delete.rs:38` |
| 6 | `/oembed` pliku z limitem bez bajtów | **Skuteczna (odtworzona)** | patrz niżej |
| 7 | `X-Real-IP` tylko przy `TRUST_PROXY` | **Skuteczna (statycznie + test + żywo)** | `auth/ip.rs:134`, `password.rs:11` |

### Poprawka 1 (limit wyświetleń / „spal po obejrzeniu”) — `media/serve.rs`, `routes/media_action.rs`, `pages/media.rs`
- Rdzeń: `lookup_media` rozróżnia `Live`/`Spent`/`Missing`; `view_budget()` (`media/serve.rs:336`)
  wymusza, że dla pliku z `max_views` każde żądanie niosące treść płatne jest widokiem, chyba że
  niesie podpisaną przepustkę `ml_view_<id>` (HMAC-SHA256 z `AUTH_SECRET`, httpOnly, SameSite=Lax,
  `constant_time_eq` przy weryfikacji) bitej przez wcześniejszy zliczony widok. `consume_view`
  liczy atomowo (`UPDATE ... WHERE views < max_views RETURNING`).
- **Odtworzono** na stosie 8449: plik `maxViews=1`, atak `GET /raw` z `Range: bytes=1-` →
  pierwsze żądanie 206 i **zużyło widok** (`views` 0→1), kolejne `Range: bytes=1-` i
  `Sec-Fetch-Dest: image` → 404. Zwykłe `GET /raw` innego pliku: pierwsze 200, drugie 404.
  W oryginale ta sama sekwencja dawała nieskończone 206 bez zliczania (A1 audytu).

### Poprawka 2 (ShareX DoS dyskowy / anonimowy zapis) — `upload/sharex.rs`
- Token nagłówkowy (`x-api-key`/`Authorization`) uwierzytelniany **przed** czytaniem ciała
  (`:185`); bez tokena strumień ucinany po `ANON_MAX_BYTES` (8 MiB, nadpisywalne
  `LEGACY_UPLOAD_MAX_SIZE`); kwota i wolne miejsce sprawdzane co 8 MiB podczas streamowania
  (`:451-479`). `Content-Length > 0` i tak sprawdzany wstępnie (`:196`).
- **Odtworzono**: anonimowy upload 10 MB → `413` „File too large for an unauthenticated upload”;
  anonimowy bez tokena → `400 Token not specified`. W oryginale do ~512 MB lądowało na dysku przed
  odrzuceniem, a ciało chunked omijało wstępny check.

### Poprawka 3 (pierwszy admin TOCTOU) — `auth/adapter.rs`
- `create_user` otwiera transakcję, bierze `SELECT pg_advisory_xact_lock($1)`
  (`USER_CREATE_LOCK_KEY`), liczy `count(*)` i decyduje o roli w tej samej transakcji (`:250-264`).
  Wstrzyknięcie `role:"admin"` w `/sign-up/email` odrzucone (`FIELD_NOT_ALLOWED`) — **potwierdzone
  na żywo** (`HTTP 400 "role is not allowed to be set"`). Test: `security_fixes_db.rs`
  `fix3_concurrent_first_sign_ups_make_exactly_one_admin`.

### Poprawka 4 (publikacja wydania) — `routes/api/releases_app.rs`
- `post_release` czyta ciało przez `read_json_object(..., JSON_BODY_MAX=16 KiB)` z wymogiem
  `application/json` (415) i limitem (413), po bramce `require_admin_caller` (zakres `admin` tokenu
  ORAZ rola `admin` konta — dwie bramki, `:131-170`).

### Poprawka 5 (token usuwania w stałym czasie) — `routes/media_delete.rs:38`
- `constant_time_eq(token, media.delete_token)` (bez wczesnego wyjścia), fallback na token API
  zawężony do właściciela. Błędny token → `404` (brak oracle) — potwierdzone na żywo.

### Poprawka 6 (`/oembed` pliku z limitem) — `routes/oembed.rs:96`
- Plik z `max_views` zwraca `{version,title,provider_name,provider_url,cache_age,type:"link"}`
  bez `url`/wymiarów/HTML. **Odtworzono**: ten sam plik bez limitu → z `url`; po ustawieniu
  `maxViews` → `type:"link"` bez `url`.

### Poprawka 7 (`X-Real-IP` tylko za zaufanym proxy) — `auth/ip.rs`, `password.rs`
- `get_ip`/`client_ip` ufają `X-Real-IP` tylko gdy `TRUST_PROXY=true`, inaczej biorą adres gniazda
  (`to_canonical()`). `setup-local.sh` ustawia `TRUST_PROXY=false` dla stosu pętli zwrotnej.
  Testy: `password::tests::honours_x_real_ip_only_behind_a_trusted_proxy`,
  `auth::ip::tests::real_ip_only_behind_a_trusted_proxy`.

### Zamierzone odchylenia (SKELETON.md 9) — nic nie otwierają
Sprawdzono pozycje 26-42 i 15 (zanieczyszczenie prototypu w `pastes`), 20 (loadery libvips),
31/37 (lookupy po kluczach dosłownych zamiast prototypowych), 38 (tus finished na S3/lokalnie),
39 (kasowanie starej awatarki/tła tylko własnych kluczy). Poprawki 38 i 39 zamykają realne luki
oryginału:
- **38** (potwierdzone na żywo): po zakończeniu uploadu tus `GET`/`HEAD` → `404`, `DELETE` nie
  usuwa wiersza mediów (plik nadal `200`). W oryginale na S3 `GET` strumieniował obiekt
  każdemu z URL-em tus niezależnie od prywatności, a `DELETE` kasował obiekt spod wiersza.
- **39**: `discard()` (`avatar_store.rs:140`) kasuje tylko klucze `avatars/<userId>-...` tego konta,
  bo `user.image` da się też ustawić przez Better Auth `update-user` — stąd twarda bramka
  prefiksu, która chroni przed skasowaniem cudzego obiektu.

---

## Błędy logiczne

Nie znaleziono błędów logicznych skutkujących utratą danych, obejściem autoryzacji ani złą
kolejnością ponad wymienione niżej drobiazgi. Sprawdzone i poprawne (odnotowane, bo w zakresie):

- `/api/uploads` PATCH/DELETE zawężone do właściciela także dla admina (brak przywileju admina na
  tej trasie — parytet). **Odtworzono**: token `full` admina na pliku user2 → `affected:0`.
- Zakresy tokenów (`upload`/`read`/`write`/`full`) egzekwowane przy każdym wywołaniu
  (`api_scopes.rs`, `api_token.rs`). **Odtworzono**: token `read`→upload `403`,
  `upload`→`GET /api/uploads` `403 insufficient_scope`, `upload`→`/api/me` OK.
- Admin: samo-demotowanie roli i samo-ban odrzucane (`actions/admin.rs:568-575`), usunięcie
  własnego konta odrzucane (`admin_library.rs`). Zawężanie tokenu to test podzbioru pojemności z
  WHERE na starym zakresie (`is_narrowing`, `set_scope_if`).
- Framework RPC: origin = host żądania (`rpc/mod.rs:318`), `X-Modlight-Rpc` ∈ {1,form}, nieznana
  akcja 404, ciało > 1 MiB → 413. **Odtworzono**: brak/zły Origin → 403, brak nagłówka → 403,
  ciało 1,2 MB → 413.
- Parser JSON (`js_json.rs`) iteracyjny — brak przepełnienia stosu przy głębokim zagnieżdżeniu;
  stronicowanie z pułapem (`MAX_PAGE`), `parse_range` na `f64` z saturacją (brak przepełnienia
  w zakresach/kwotach).

### [Niska] Zapis od danych w handlerze budowany konkatenacją (admin list i filtry) — `auth/admin.rs:381-414, 426-490`
- `list_users` buduje `WHERE`/`ORDER BY` z `format!`. Pewność: **sprawdzone, bez wstrzyknięcia**.
  Kolumny przechodzą przez `user_field_column()` (allowlista nazw → stałe identyfikatory),
  wartości idą jako parametry (`$n::text::ty` / `LIKE $n`), kierunek sortowania to literał
  `ASC/DESC`, `LIMIT/OFFSET` castowane do `i64`. Trasa chroniona pluginem admina. Analogicznie
  bezpieczne są pozostałe `format!`-SQL: `uploads.rs`, `data/uploads.rs`, `data/folders.rs`,
  `folders.rs` (identyfikatory ze stałych, wartości parametrami). Zostawione jako obserwacja:
  wzorzec „kolumna z allowlisty + wartość parametrem” wymaga, by każda przyszła kolumna dalej
  szła przez `user_field_column`/stałą.

### [Niska/parytet] Pamięciowy licznik limitów Better Auth bez zamiatania — `auth/rate_limit.rs`
- `memory()` (HashMap) usuwa wpis tylko przy kolejnym `get` tego samego klucza; nie ma okresowego
  czyszczenia. Przy `TRUST_PROXY=true` i proxy przepuszczającym dowolny `X-Real-IP` liczba
  kluczy mogłaby rosnąć. To **wierne odwzorowanie** magazynu `memory` better-auth 1.6.13 (też
  bez zamiatania). Przy wdrożeniu pętli zwrotnej (`TRUST_PROXY=false`) klucz = adres gniazda, więc
  ograniczony. Dla kontrastu własny licznik plików (`password.rs`) MA zamiatanie (soft/hard limit).
  Propozycja: rozważyć okresowe czyszczenie `memory` (bez zmiany zachowania widocznego dla klienta).

### [Informacyjna/parytet] Porównanie kodu TOTP niestałoczasowe — `auth/crypto.rs:243`
- `verify_totp` używa `hotp(...) == code` (String). Identycznie jak oryginał
  (`@better-auth/utils` `otp.mjs`: `otp === generatedOTP`). Kod 6-cyfrowy, okno ±1, krótkożyjący,
  trasa throttlowana (`/two-factor/*` 3/10 s). Kody zapasowe i tokeny (unlock/render/view-grant,
  hasła plików, hasła kont) używają `constant_time_eq`. Nic do naprawy; odnotowane dla pełności.

**Stan po przeglądzie (2026-10-20):** naprawione. `verify_totp` liczy wszystkie trzy kroki i porównuje kod przez `constant_time_eq` (świadome odstępstwo od oryginału, opisane w komentarzu funkcji).

### [Informacyjna] Handoff tokenu desktopowego bez PKCE — `authorize/loopback.rs`, `actions/authorize.rs`
- `callback` budowany wyłącznie z **sparsowanego numeru portu** (`loopback_callback`, 1024-65535),
  nigdy z ciągu klienta; `name` sanityzowane; token mintowany w akcji wymagającej sesji, o zakresie
  `write` (desktop nie opublikuje wydania). Zgodne z RFC 8252, jak oryginał. Bez zmian.

---

## Rzeczy sprawdzone i w porządku

- **Sesje / Better Auth**: cookie `better-auth.session_token` (prefiks `__Secure-` na https),
  wartość `token.base64(HMAC)`, weryfikacja podpisu, polityki odświeżania (DbAndCookie/DbOnly/Skip)
  jak w kontekstach Next; konto zbanowane odrzucane przez `resolve_api_owner`/sign-in, nie przez
  samo `getSession` (parytet). Podrobiony podpis odrzucony (test skeletonu).
- **Hasła**: scrypt w formacie Better Auth (N=2^14), NFKC przed hashem, legacy bcrypt
  `$2a/$2b/$2y`, `constant_time_eq`. `burn_hash` wyrównuje czas na ścieżkach „nie ma użytkownika”.
  Hashowanie na `spawn_blocking`.
- **2FA**: sekret i kody zapasowe szyfrowane XChaCha20-Poly1305 (klucz = SHA-256 z AUTH_SECRET),
  TOTP HMAC-SHA1, trust-device jako HMAC + wiersz `verification`, aktualizacja kodu zapasowego
  atomowa (`UPDATE ... WHERE backup_codes = $old RETURNING`).
- **Autoryzacja API**: token przed sesją; token obecny-lecz-nieważny = odmowa (bez fallbacku do
  sesji) — potwierdzone na żywo; mapowanie zakres→pojemność zgodne z `token-scope-coverage`.
- **Path traversal / dowolny odczyt-zapis**: `LocalDriver::full()` wymusza prefiks korzenia
  (`resolve_lexically` + `starts_with`); klucze `userCode/code.ext` z CSPRNG; derived-cache z
  allowlisty wariantów; tus `file_id` dekodowany i odrzuca segment ścieżki serwera; nazwy w
  ZIP albumu spłaszczane (`sanitize_entry_name`, `../` → `_..`). Test: `..%2f..%2fetc%2fpasswd`
  na żywo → 404.
- **Stored XSS**: `serve_decision` to allowlista — HTML/SVG/XML/JSON wychodzą jako
  `text/plain; charset=utf-8` + `nosniff`; prawdziwy render tylko przez `/render` z biletem HMAC,
  wymogiem `Sec-Fetch-Dest: iframe` i CSP `sandbox allow-scripts; default-src 'none'`. Strona
  usuwania: `esc()` + `default-src 'none'`. Payload SPA: `json_for_html` escapuje `<`/`>`/`&`/
  U+2028/9; atrybuty `<html>` przez `escape_attr`.
- **Bomby dekompresyjne**: `media/zip_guard.rs` realnie rozpakowuje każdy wpis z
  `max_output_length`, odrzuca ZIP64/nieznane metody przed mammoth; obrazy ograniczone
  `MAX_TRANSFORM_BYTES` (32 MB) i `MAX_THUMBNAIL_PIXELS` (64 Mpx) sprawdzane z metadanych przed
  dekodowaniem; `limit_input_pixels` dla tła; wszystko przez wspólny limiter z wagą i timeoutem.
- **Unsafe / FFI (libvips)**: `unsafe` skupione w `media/vips/*`; `Image`/`Blob` z poprawnym
  licznikiem referencji GObject (`g_object_ref`/`unref`, `vips_area_unref`), wejście kopiowane do
  `vips_tracked_malloc` i zwalniane przez `free_tracked`, callback timeoutu z `Box` zwalnianym raz
  (`drop_timeout_state`). Poza vips: `libc::statvfs` (zeroed + sprawdzenie rc), `kill(-pid)` w
  grupie procesów ffmpeg, FFI `vips_foreign_is_a_buffer`. `set_var("PANGOCAIRO_BACKEND")` raz w
  `OnceLock` przed startem libvips (bezpieczne).
- **Panika z żądania**: `App::handle` łapie `catch_unwind` → 500; przegląd `.unwrap()` w
  `routes/`, `actions/`, `pages/`, `rpc/`, `upload/`, `auth/routes/` — wszystkie trafienia poza
  `lock().unwrap()` są w `#[cfg(test)]`. `parse_metadata` `tokens[0]` bezpieczne (split daje ≥1).
- **Nagłówki / CSP**: HSTS, nosniff, `X-Frame-Options: SAMEORIGIN`, `Permissions-Policy`,
  `frame-ancestors 'self'`, brak `X-Powered-By`. Osobne restrykcyjne CSP dla `/raw`, `/render`,
  `/authorize`. `'unsafe-inline'` na app-CSP to świadoma decyzja oryginału (treści użytkownika w
  osobnych politykach).
- **Sekrety w logach**: błędy SMTP przez `redact_secrets`, `S3Config` Debug ukrywa
  `secret_access_key`, dev-mail loguje treść tylko poza produkcją (parytet). `config.rs` odmawia
  startu produkcyjnego przy braku/placeholderze/za krótkim `AUTH_SECRET` i domyślnych danych bazy.
- **Docker/compose**: publikacja tylko `127.0.0.1`, Postgres bez publikacji, `user 1000:1000`,
  `cap_drop: ALL`, `no-new-privileges`, limity mem/pids/cpu, ffmpeg z repo Debiana (nie z sieci),
  entrypoint tylko liczy cgroup i odpala migracje+serwer. `setup-local.sh`: sekrety przez
  `openssl rand`, `chmod 600`, nic nie pobiera ani nie wysyła, zachowuje istniejące sekrety.
- **Brak debug-endpointów, backdoorów, hardcodowanych danych** (grep: TODO/HACK/bypass/master_key
  oraz zewnętrzne URL-e — wszystkie trafienia w testach lub komentarzach).

---

## Metodyka testów dynamicznych (powtarzalne)

Stos wyrzucany: kopia `setup-local.sh` w `_scratch/sec-review/stack/.env` (APP_PORT=8449,
REGISTRATION_ENABLED=true), compose z `name: modlight-secreview` i portem 8449, `up -d --pull
never`, po pracy `down -v`. Potwierdzono: RPC CSRF/origin, wstrzyknięcie roli, zakresy tokenów,
IDOR między kontami, poprawka 1 (Range/Sec-Fetch-Dest), poprawka 2 (anon 10 MB), poprawka 6
(oembed), poprawka 38 (tus finished), limity ciała, path traversal, ukrycie trasy admina. Stosów
właściciela nie dotykano; stos wyrzucany usunięto z wolumenami.
