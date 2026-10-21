# Audyt Modlight (serwer) - pliki, media, serwowanie, wyciek danych i lancuch dostaw

Zakres: `/path/to/modlight/upstream/Modlight` (serwer Next.js 16 + wlasny `server.ts`).
Metoda: wylacznie analiza statyczna (czytanie kodu, grep, `file`, `xxd`, wlasne skrypty Pythona w `_audyt/narzedzia`). Nic z repo nie bylo uruchamiane.
Data: 2026-10-18. Commit: `a59d03c` (jeden commit, autor Nimfek).

Rownolegly audytor bada konta/logowanie/autoryzacje API - ponizej tylko: pliki/media/serwowanie (zakres A) oraz wysylanie danych na zewnatrz i lancuch dostaw (zakres B).

Werdykt skrotowy: **nie znaleziono backdoora, telemetrii ani zadnego kodu wysylajacego dane do autora lub osob trzecich.** Kod jest nietypowo starannie napisany pod katem bezpieczenstwa (allowlisty typow, sandbox, bramki widokow). Jeden potwierdzony blad logiczny powaznie oslabia funkcje "spalenia po N wyswietleniach"; reszta to sprawy srednie/niskie.

---

## Najpowazniejsze ustalenia (lista do naprawy)

### A1. Obejscie limitu wyswietlen (maxViews / "spal po obejrzeniu") - Wysoka, potwierdzone
Plik: `app/[userCode]/[mediaCode]/[action]/route.ts:112-114, 150-158, 210-233` oraz `lib/media/serve.ts:95, 197-229`.

Licznik wyswietlen jest zwiekszany TYLKO dla zadania uznanego za "poczatkowe i nie-podglad":

```
const shouldCount = !privileged && isInitialRequest(rangeHeader) && !isPreviewFetch(req, reqUrl) && !botExempt;
```

- `isInitialRequest` zwraca `true` jedynie, gdy nie ma naglowka `Range` albo `Range` zaczyna sie od `bytes=0-` (`/^bytes=0-/`).
- `isPreviewFetch` zwraca `true`, gdy `Sec-Fetch-Dest: image|video` albo gdy w URL jest `w`/`h`/`motion`.
- `getMedia` (`serve.ts:95`) odrzuca plik dopiero, gdy `views >= maxViews`.

Skutek: ktokolwiek majacy link do pliku z limitem wyswietlen moze pobrac cala jego tresc, nie zuzywajac ani jednego widoku, na dwa trywialne sposoby:
1. `GET /{tag}/{code}/raw` z naglowkiem `Range: bytes=1-` (dowolny start != 0) -> `isInitialRequest=false` -> `shouldCount=false` -> `streamMedia` zwraca 206 z bajtami 1..koniec, `views` sie nie zmienia. Powtarzalne w nieskonczonosc.
2. Osadzenie `<img src=".../raw.png">` lub `<video src=".../raw">` -> przegladarka ustawia `Sec-Fetch-Dest: image|video` -> `isPreviewFetch=true` -> `shouldCount=false`. Pelny obraz/wideo widoczny dowolna liczbe razy.

Scenariusz: nadawca ustawia `maxViews=1` ("do jednorazowego obejrzenia") i wysyla link. Napastnik, ktory przechwyci link, pobiera cala zawartosc dowolna liczbe razy (bajty 1..koniec), a `views` pozostaje 0 - wiec zamierzony odbiorca dalej widzi dzialajacy link, a nadawca nie dowiaduje sie, ze tresc zostala odczytana. To rownoczesnie lamie gwarancje "spalenia" i jej wlasnosc wykrywania manipulacji. (Bajt 0 mozna dobrac jednym zliczonym zadaniem albo `Range: bytes=0-0`, ktore pasuje do `^bytes=0-`.)

To samo dotyczy `action === "download"` (ta sama bramka `shouldCount`, linie 216-221).

Zamierzone wylaczenia (seek odtwarzacza, kafelki siatki, unfurlery) sa opisane w komentarzach, ale efekt uboczny jest taki, ze limit jest nieegzekwowalny wobec napastnika. Poprawka: egzekwowac budzet wyswietlen jako twarda bramke na kazde zadanie serwujace pelne bajty pliku z `maxViews != null` (np. liczyc po (plik, sesja/IP) z deduplikacja zakresow tej samej sesji, albo odmawiac 206 dla plikow z limitem, albo wymuszac zliczenie takze dla `Range: bytes=N-` i `Sec-Fetch-Dest: image/video`, gdy `maxViews != null`). Dzis exempcje widokow i tak sa zawezone do `maxViews == null` tylko dla botow (`botExempt`) - te same zawezenie trzeba zastosowac do `isInitialRequest`/`isPreviewFetch`.

---

## Blędy logiczne (zachowanie niezgodne z intencja kodu/komentarzy)

### L1. Deklarowana intencja "tylko pierwsze zadanie liczy" vs. rzeczywistosc "zadne zadanie nie musi liczyc"
Patrz A1. Komentarz przy `isInitialRequest` mowi: "Only the opening request counts" - ale nic nie zmusza klienta do wyslania zadania poczatkowego, wiec dla pliku z `maxViews` tresc jest osiagalna bez zliczenia. To rozjazd intencji z implementacja, dlatego wymieniony tez tutaj. Waga jak w A1 (Wysoka).

### L2. `oembed` nie sprawdza limitu wyswietlen - Niska, potwierdzone
Plik: `app/oembed/route.ts:49-52`. Endpoint odmawia dla `!published` i `passwordHash`, ale nie sprawdza `maxViews`. Zwraca `filename` (jako `title`) oraz `width/height` dla opublikowanego pliku z limitem. Nie jest to realny wyciek tresci (zwraca tylko URL/metadane, ktore i tak sa na stronie podgladu dla pliku opublikowanego bez hasla), ale jest niespojne z `lib/media/embed.ts`, ktore dla plikow z limitem celowo nie emituje `og:image`. Warto wyrownac polityke. Bajtow nie serwuje i widoku nie zuzywa.

---

## Pozostale ustalenia (niskie / obserwacje)

### N1. Skracacz linkow jest celowo otwartym przekierowaniem (http/https) - Niska/by-design
`lib/shortlinks.ts:105-133, 285-302`. `normalizeTarget` poprawnie odrzuca `javascript:`, `data:` i wszystko poza `http/https` (brak XSS typu open-redirect do `javascript:`). Przekierowanie `Response.redirect(link.target, 302)` na dowolny adres http(s) jest natura skracacza; bramka hasla jest respektowana przed odczytaniem celu. Zostawione jako informacja, nie luka.

### N2. CSP `script-src/style-src 'unsafe-inline'` na calej aplikacji - Niska/by-design
`next.config.ts:63-64`. Swiadoma decyzja (brak nonce, bo `proxy.ts` nie moze buforowac cial). Tresci uzytkownika trafiaja do osobnych, bardzo restrykcyjnych polityk (`rawCsp`, `renderCsp`), wiec nie tworzy to stored-XSS z plikow. Osobna polityka `authorizeCsp` dopuszcza `connect-src http://127.0.0.1:*` tylko dla `/authorize` (RFC 8252 loopback) i jest poprawnie zawezona do tej jednej sciezki.

### N3. Nagłowek `X-Real-IP` jako jedyne zrodlo IP do limitowania - Niska (operacyjna)
`lib/password.ts:375-377`, `docker/Caddyfile:36`, `docker/nginx.example.conf:25`. Jesli operator postawi aplikacje bez proxy nadpisujacego `X-Real-IP`, wszyscy dziela jeden kubelek (throttling za ostry, nie za luzny) - udokumentowane. Gdyby jednak proxy przepuszczalo kliencki `X-Real-IP`, limity sa omijalne. To konfiguracja wdrozenia, nie blad kodu; komentarze ostrzegaja wprost.

### N4. Strona usuwania budowana recznie z HTML - obserwacja, zabezpieczone
`app/[userCode]/[mediaCode]/delete/[token]/route.ts:43-73`. Nazwa pliku jest wstrzykiwana do HTML, ale wszystkie wartosci przechodza przez `esc()` (`& < > " '`), a odpowiedz ma restrykcyjny `Content-Security-Policy: default-src 'none'`. Autoryzacja: per-plik `deleteToken` albo token API ze scope `write` nalezacy do wlasciciela; 404 zamiast 403 (brak oracle). Wyglada poprawnie.

### N5. Avatar/tlo - klucze w storage, brak odczytu dowolnego pliku - obserwacja
`app/avatar/[userCode]/route.ts`, `app/bg/[userCode]/route.ts`, `lib/avatar.ts`, `lib/background.ts`. Klucz brany z kolumny `users.image`/`backgroundKey` (nie z URL), walidowany `isAvatarKey`/prefiks `backgrounds/`, serwowany z `nosniff` i staly `Content-Type: image/webp`. SVG odrzucone przy wgrywaniu tla (komentarz o loaderze SVG). Brak path traversal.

---

## Czy serwer wykrada dane - pelna lista polaczen wychodzacych i werdykt

Przeszukano caly kod serwera i frontu: `fetch(`, `http/https.request`, `net/tls.connect`, `dns`, `nodemailer`, klient S3, `WebSocket`, `sendBeacon`, `<script src>`/`<link>`/`<img>` z obcych domen, czcionki z CDN oraz kazdy zakodowany na sztywno URL/IP (`grep -rniE "https?://"`). Node_modules nie istnieje (zgodnie z zalozeniem).

Polaczenia sieciowe wychodzace inicjowane przez serwer:

| Miejsce | Dokad | Jakie dane | Kiedy | Decyduje administrator? |
|---|---|---|---|---|
| `lib/mail.ts` / `lib/mail-settings.ts` (nodemailer) | serwer SMTP z konfiguracji (`SMTP_HOST`) | maile weryfikacyjne/reset do uzytkownikow | wysylka maila | TAK (konfiguracja, domyslnie wylaczone gdy brak `SMTP_HOST`) |
| `lib/storage/s3.ts` (`@aws-sdk/client-s3`) | `S3_ENDPOINT` z konfiguracji | wgrywanie/pobieranie plikow uzytkownikow | przy storage=s3 | TAK |
| `migrate.ts`, `lib/db` (`pg`) | `DATABASE_URL` | dane aplikacji | normalna praca | TAK |

Polaczenia po stronie przegladarki (front): wylacznie wlasny origin / sciezki wzgledne - `components/profile/*` do `/api/avatar`, `/api/background`; `components/upload/*` do `/api/uploads/check` i tus (`/api/files`); `components/viewer/decrypt-transfer.ts` do wlasnego `/raw` (`cache: no-store`). Jedyny wyjatek: `components/authorize/authorize-prompt.tsx` POST-uje token na `http://127.0.0.1:<port>` (lokalny desktop, RFC 8252) - to dzieje sie w przegladarce uzytkownika, do jego wlasnej maszyny, i jest jawnie obwarowane osobnym CSP.

Czcionki: self-hosted (`next/font/local`, `geist`, `public/fonts/Geist-Bold.ttf`), komentarz w `app/layout.tsx:2` "no Google Fonts fetch". Brak Google Fonts, brak CDN, brak analityki (Plausible/GA/Sentry/PostHog/itp.), brak beaconow, brak `NEXT_TELEMETRY` (wrecz `NEXT_TELEMETRY_DISABLED=1` w Dockerfile). Better Auth ma pakiet `@better-auth/telemetry`, ale nie znaleziono jego wlaczenia w kodzie.

Brak SSRF: `app/api/link-info/route.ts` i `lib/link-info/parse.ts` NIE pobieraja opisywanego adresu - odpowiadaja wylacznie z wiersza bazy (`getMedia`/`SELECT`), celowo nie wolaja `consumeView`, `streamMedia`, `redirectShortLink` ani niczego, co dotyka sieci/plikow. Nigdzie w serwerze nie ma `fetch`/`request` na adres pochodzacy od uzytkownika. Zadnego "pobierz podglad linku" po stronie serwera.

**Werdykt: serwer nie wysyla zadnych danych do autora ani osob trzecich. Brak telemetrii, beaconow i "phone home". Jedyne wyjscia sieciowe to uslugi skonfigurowane przez administratora (SMTP, S3, Postgres).**

---

## Lancuch dostaw

- `package.json` / `pnpm-lock.yaml`: `lockfileVersion 9.0`, wszystkie 1149 wpisow ma `resolution: {integrity: sha512-...}` z rejestru npm. **Brak zaleznosci z git/tarball/URL, brak lokalnych sciezek.** Nazwy pakietow przejrzane - standardowy ekosystem (Next, React, radix-ui, aws-sdk, @tus/*, sharp, mammoth, pdf-lib, drizzle, better-auth, nodemailer, archiver, busboy). `@shopify/semaphore` i dwa `@esbuild-kit/*` oznaczone jako deprecated (zwykle informacyjnie), nic podejrzanego.
- `pnpm-workspace.yaml`: `onlyBuiltDependencies` ograniczone do `sharp` i `esbuild`; skrypty instalacyjne dev-toolingu (`unrs-resolver`, `@parcel/watcher`, `@swc/core`, `msw`) jawnie na liscie `ignoredBuiltDependencies`. Override `kysely: ^0.28.17` z uzasadnieniem. Brak `.npmrc`/`.pnpmfile.cjs` (brak ukrytych hookow instalacji).
- `Dockerfile`: `pnpm install --frozen-lockfile`, build wieloetapowy, runtime jako `node:node` (uid 1000), `cap_drop: ALL`, `no-new-privileges`, limity cpu/mem/pids. ffmpeg z repozytoriow Debiana (`apt-get`, `--no-install-recommends`), **nie sciagany z internetu**. `entrypoint.sh` jedynie liczy limity cgroup i odpala migracje + serwer; nic nie pobiera ani nie wysyla.
- `docker-compose*.yml`: produkcyjnie aplikacja wystawiona przez `expose` (siec dockera) lub `127.0.0.1:${APP_PORT}` (wariant z wlasnym proxy) - nie na 0.0.0.0. Postgres bez publikacji portu. Dev `docker-compose.yml` publikuje Postgres na `5433` z haslem `modlight` (tylko do `pnpm dev`, nie produkcja).
- `docker/Caddyfile`, `docker/nginx.example.conf`: proxy, nadpisuja `X-Real-IP`, bez limitu ciala (swiadome, dla duzych uploadow).
- `setup.sh`: interaktywny generator `.env`; pyta z `/dev/tty` (komentarz: zabezpieczenie przed `curl | bash`), sekrety generuje lokalnie (`openssl rand` / `/dev/urandom`), `chmod 600 .env`. Nie pobiera ani nie wysyla niczego poza lokalnym `docker compose up`.
- `scripts/*`: narzedzia dev/CI (zrzuty Playwright na `localhost:8420`, mint tokenu, backfill). Zadne nie laczy sie na obcy adres; `publish-release.ts` i `fix-mime-types.ts` dzialaja na lokalnej bazie/storage.

## Zaciemniony kod / pliki binarne

- Skan niewidocznych znakow (`_audyt/narzedzia/nietypowe_znaki.py`): brak znakow BIDI/zero-width/cyrylicy w identyfikatorach. Dlugie linie to wylacznie klasy Tailwind w `components/ui/*`. Dwa "znaki sterujace" to literaly w kodzie: `0x00` jako separator w `lib/media/convert-verdict.ts:82` (klucz cache) i w `app/a/.../zip/route.ts:130,139` (regex sanityzacji nazw), oraz `U+200B` w `components/viewer/TextView.tsx:141` (placeholder pustej linii). Wszystkie nieszkodliwe.
- Brak `eval`, `new Function`, dynamicznego `require(<zmienna>)`, `import(<url>)`. `child_process` tylko w `lib/media/ffmpeg.ts` (spawn ffmpeg/ffprobe, `shell:false`) i `taskkill` na Windowsie. `i18n/request.ts` robi `import(\`../messages/${locale}/...\`)` gdzie `locale` jest wczesniej zawezone do `'pl'|'en'`.
- Pliki binarne (`_audyt/narzedzia/sprawdz_binarne.py` + `file`): `public/brand/*.png`, `app/apple-icon.png`, `public/fonts/Geist-Bold.ttf`, `docs/screenshots/*.webp` - wszystkie poprawne, bez danych doklejonych po `IEND`/poza deklarowanym rozmiarem RIFF/TTF, bez podejrzanych chunkow tekstowych (jedynie `Software: Adobe ImageReady` w `sharex.png`). `app/icon.svg`, `docs/logo.svg` - czysty SVG bez skryptu.

---

## Rzeczy sprawdzone i uznane za poprawne (zakres A)

- **Path traversal / dowolny odczyt-zapis**: `LocalDriver.full()` (`lib/storage/local.ts:21-27`) wymusza prefiks katalogu-korzenia; `deletePrefix` odmawia pustego/korzenia. Klucze storage budowane z `userCode/code.ext`; `codeFromMediaCode` tnie do pierwszej kropki, lookup po kolumnie `code`. Derived-cache: `derivedKey = derived/{uuid}/{hex}.{epoch}.{variant}.{ext}` gdzie `variant`/`ext` z allowlisty celow (`pdf/png/jpg/webp/avif`). Brak sciezki sterowanej uzytkownikiem.
- **Stored XSS (HTML/SVG/PDF/JS z domeny aplikacji)**: `serveDecision` (`lib/media/mime.ts`) to allowlista - HTML/SVG/XML/JSON wychodza jako `text/plain; charset=utf-8` z `nosniff`. Prawdziwy render HTML/SVG/docx tylko przez `/render` z: podpisanym ticketem HMAC (`render-ticket.ts`), wymogiem `Sec-Fetch-Dest: iframe`, polityka `Content-Security-Policy: sandbox [allow-scripts]` i ramka `sandbox=""` dla SVG (przeciecie flag). docx renderowany przez mammoth w tym samym sandboxie. Tresc odszyfrowana w przegladarce przechodzi przez te sama `serveDecision` (`decrypted-presentation.ts`), a blob do zapisu dostaje `application/octet-stream`.
- **ffmpeg/ffprobe**: `shell:false`, `-protocol_whitelist file,pipe`, wymuszony `-f <kontener>` z allowlisty, `-use_absolute_path 0` dla mov, srodowisko zredukowane do `PATH`, kill calej grupy procesow po deadline. Parametry `?t=`/`?d=` przechodza przez `seconds()` (skonczone 0..86400) i trafiaja jako `toFixed(3)`/`String(<liczba>)`; `scale=`/`fps=` to stale. Brak wstrzykniecia argumentow.
- **Bomby dekompresyjne**: `lib/media/zip-guard.ts` realnie rozpakowuje kazdy wpis przez `inflateRawSync` z `maxOutputLength`, odrzuca ZIP64 i nieznane metody - sprawdzane PRZED mammoth.
- **Limity sharp / DoS transformacji**: wejscie obrazu ograniczone `MAX_TRANSFORM_BYTES` (32 MB), piksele `MAX_THUMBNAIL_PIXELS` (64 Mpx) sprawdzane z metadanych PRZED dekodowaniem, `sharp.concurrency` przypiete do cgroup, wszystko przez wspolny limiter `imageTransforms` z waga i timeoutem (`transform-queue`, `transform-cost`). Poster wideo to seek, nie dekode.
- **Obejscie limitu miejsca / klamliwy rozmiar w tus**: `onUploadCreate` wymaga `Upload-Length` (odrzuca `Upload-Defer-Length`), sprawdza `hasRoomFor` i kwote; `maxSize` dynamiczny z pozostalej kwoty; rezerwacja kwoty atomowa (`reserveQuota` z predykatem w `WHERE`). ShareX: `ANON_MAX_BYTES` dla niezaautoryzowanych, `hasRoomFor` z `Content-Length` przed zapisem.
- **Linki z haslem/wygasajace/jednorazowe**: tokeny odblokowania to HMAC-SHA256 zwiazane ze scope+id+exp (`lib/password.ts`); cookie tylko podpisane; rate-limit prob (5/15 min) po IP+zasob; wygasniecie i haslo sprawdzane tez na `/raw`, `/render`, galeriach i w ZIP. (Wyjatek: egzekwowanie `maxViews` - patrz A1.)
- **Odgadywalnosc identyfikatorow**: `lib/ids.ts` i `lib/naming.ts` uzywaja `crypto.randomInt`/`randomBytes` (CSPRNG), minimalna dlugosc kodow wymuszona (RANDOM_LENGTH_MIN=5 nad alfabetem 56 znakow).
- **Kosz i przywracanie**: operacje zawezone do wlasciciela (lub admina z wpisem audytu), `purgeExpiredTrash` zwraca kwote, retired-tagi nieodtwarzalne przez inne konto (`user_code_aliases`).
- **Szyfrowanie E2E**: klucz tylko we fragmencie URL (`lib/crypto/link.ts`, `urlLeaksKey` jako asercja), AES-256-GCM, IV = 8B prefiks (per plik) || 32-bit licznik, odmowa przy przepelnieniu licznika, AAD wiaze ramke z plikiem/pozycja/flaga ostatniej ramki, metadane w ramce 0. Format koperty spojny (`envelope.ts`).

Narzedzia pomocnicze audytu: `_audyt/narzedzia/sprawdz_binarne.py`, `_audyt/narzedzia/nietypowe_znaki.py`.
