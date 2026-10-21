# Modlight w Rust: serwer

To jest serwer Modlight przepisany z TypeScriptu (Next.js 16 + PostgreSQL 17,
oryginał w `upstream/Modlight`) na Rust, 1:1. Modlight to własny hosting plików
i zrzutów ekranu: panel WWW (biblioteka, foldery, kosz, duplikaty, skróty
linków, wklejki, profil, tokeny, administracja), wysyłanie przez ShareX
(`POST /upload`) i przez tus (wznawiane wysyłanie z panelu i z aplikacji),
publiczne strony plików, galerie i albumy, podglądy dla Discorda i innych
komunikatorów, konwersje obrazów, audio i wideo.

Ten sam adres URL, te same odpowiedzi, ta sama baza danych co oryginał:
panel WWW, ShareX i aplikacja Modlight nie odróżnią serwerów. Wyjątki są celowe
i opisane w `docs/SKELETON.md`, sekcja 9 (poprawki bezpieczeństwa z audytu
`docs/audyt/` i kilka błędów oryginału, których nie powtarzamy).

Dokumenty dla programistów: `docs/PORT.md` (zasady portu), `docs/SKELETON.md`
(budowa serwera, odstępstwa), `docs/CONTRACT.md` (umowa z panelem WWW),
`docs/COVERAGE.md` (co z oryginału jest gdzie i czym sprawdzone).

## Lokalny stos w Dockerze

Wszystko działa tylko na tym Macu. Panel jest pod **http://127.0.0.1:8430**,
port jest otwarty wyłącznie na adresie pętli zwrotnej (127.0.0.1): z innego
komputera ani z internetu nie da się do niego dostać. Baza danych nie ma
żadnego portu na zewnątrz.

Docker to Colima (profil `modlight`). Wszystkie polecenia niżej uruchamiaj
z katalogu repozytorium (`~/Desktop/modlight`):

```sh
colima start --profile modlight          # jeśli Docker nie działa (np. po restarcie Maca)
```

### Pierwsze uruchomienie

```sh
server/docker/setup-local.sh                                   # tworzy server/docker/.env (losowe hasła)
docker compose -f server/docker/docker-compose.yml up -d --build
```

`setup-local.sh` zapisuje `server/docker/.env` z prawami 600: klucz
podpisujący sesje (`AUTH_SECRET`), klucz do sekretów w bazie
(`SETTINGS_ENCRYPTION_KEY`) i hasło bazy (`POSTGRES_PASSWORD`). Plik nie trafia
do gita. **Nie zmieniaj tych wartości na działającej instalacji** (wylogowałoby
wszystkich, hasło SMTP w bazie przestałoby się dać odczytać, a aplikacja
straciłaby dostęp do bazy). Skrypt można uruchomić ponownie: istniejące sekrety
zostawia, starą wersję pliku zapisuje obok jako kopię.

Potem otwórz http://127.0.0.1:8430/register i załóż konto. **Pierwsze konto
na pustej instalacji zostaje administratorem** (rejestracja dla innych jest
wyłączona; włączysz ją w Admin → Ustawienia, a konta możesz też zakładać sam
w Admin → Użytkownicy).

### Codzienna obsługa

```sh
F=server/docker/docker-compose.yml
docker compose -f $F ps                 # stan (app powinna być "healthy")
docker compose -f $F logs -f app        # dziennik serwera
docker compose -f $F stop               # zatrzymanie (dane zostają)
docker compose -f $F start              # ponowne uruchomienie
docker compose -f $F restart app        # restart samego serwera
docker compose -f $F down               # usunięcie kontenerów (dane w wolumenach zostają)
```

Kontenery mają `restart: unless-stopped`, więc po starcie Colimy wstają same,
chyba że je zatrzymałeś.

### Aktualizacja

Po zmianach w kodzie (albo po scaleniu nowej wersji do `main`):

```sh
docker compose -f server/docker/docker-compose.yml up -d --build
```

To przebudowuje obraz i podmienia kontener aplikacji. Migracje bazy uruchamiają
się same przy każdym starcie aplikacji (najpierw sprawdzenie konfiguracji, potem
migracje, potem serwer). Pierwsze budowanie trwa ok. 10 minut (libvips i jego
biblioteki kompilują się od źródeł), kolejne są szybkie dzięki pamięci
podręcznej Dockera.

### Gdzie są dane

| Co | Gdzie |
|---|---|
| Wysłane pliki, miniatury, awatary, tapety, pliki w trakcie wysyłania | wolumen Dockera `modlight-rust_appdata` (w kontenerze `/app/data`) |
| Baza PostgreSQL 17 (konta, pliki, ustawienia) | wolumen Dockera `modlight-rust_pgdata` |
| Sekrety i konfiguracja | `server/docker/.env` (prawa 600) |

Wolumeny żyją w maszynie wirtualnej Colimy (profil `modlight`). `docker compose
down` ich nie usuwa; usunęłoby je dopiero `down -v` albo `colima delete`.

### Kopie zapasowe

Baza (spójny zrzut przy działającym serwerze):

```sh
mkdir -p ~/ModlightBackup
docker compose -f server/docker/docker-compose.yml exec -T postgres \
  pg_dump -U modlight -Fc modlight > ~/ModlightBackup/modlight-$(date +%F).dump
```

Pliki (wolumen `appdata`, spakowany obrazem aplikacji, który ma `tar`):

```sh
docker run --rm --entrypoint tar -v modlight-rust_appdata:/data:ro -v ~/ModlightBackup:/backup \
  modlight-rust:local -czf /backup/appdata-$(date +%F).tar.gz -C /data .
```

Zachowaj też kopię `server/docker/.env` (bez niej zaszyfrowane ustawienia, np.
hasło SMTP, i sesje są nie do odtworzenia). Trzymaj ją w bezpiecznym miejscu.

Przywracanie (na zatrzymanej aplikacji):

```sh
F=server/docker/docker-compose.yml
docker compose -f $F stop app
docker compose -f $F exec -T postgres pg_restore -U modlight -d modlight --clean --if-exists < ~/ModlightBackup/modlight-RRRR-MM-DD.dump
docker run --rm --entrypoint tar -v modlight-rust_appdata:/data -v ~/ModlightBackup:/backup \
  modlight-rust:local -xzf /backup/appdata-RRRR-MM-DD.tar.gz -C /data
docker compose -f $F start app
```

Baza jest zgodna z oryginałem w obie strony: zrzut z oryginalnego Modlight
(Next.js) można odtworzyć tutaj i odwrotnie.

### Porty

| Port | Co | Dostęp |
|---|---|---|
| 127.0.0.1:8430 | panel WWW, API, ShareX, tus | tylko z tego Maca |
| (brak) | PostgreSQL | tylko z kontenera aplikacji |

Port można zmienić w `server/docker/.env` (`APP_PORT` i `APP_URL` razem), potem
`up -d`.

## ShareX

ShareX działa tylko na Windowsie, a serwer słucha tylko na 127.0.0.1 tego Maca,
więc ShareX z innego komputera się nie połączy, dopóki serwer nie zostanie
udostępniony w sieci (to wymaga reverse proxy z HTTPS i nie jest zrobione).
Konfiguracja, gdy będzie potrzebna:

1. Panel → **Integracje** (`/dashboard/tokens`) → utwórz token.
2. „Pobierz konfigurację ShareX” (albo „Konfiguracja ShareX (.sxcu)” w oknie
   nowego tokenu) zapisuje gotowy plik `.sxcu`; otwórz go dwukrotnie w ShareX.
   Ręcznie: wysyłanie `POST <adres>/upload`, pole pliku `upload`, nagłówek
   `x-api-key: <token>`; adres pliku w odpowiedzi to `{json:url}`, adres
   usunięcia `{json:deletionUrl}`.

Na tym Macu ten sam kontrakt sprawdzisz poleceniem:

```sh
curl -F upload=@zrzut.png -H "x-api-key: <token>" http://127.0.0.1:8430/upload
```

## Aplikacja Modlight na Maca

Aplikacja (`app/` w tym repozytorium) łączy się z tym serwerem tak jak
z oryginałem:

1. W oknie logowania podaj adres serwera **http://127.0.0.1:8430**.
2. Logowanie przez przeglądarkę otwiera panel na stronie `/authorize`;
   zatwierdź tam urządzenie (aplikacja odbiera token przez lokalny port
   pętli zwrotnej). Albo wklej token utworzony w panelu w **Integracje**.
3. Token trafia do pęku kluczy macOS (usługa `Modlight`).

## Cloudflare Turnstile (sprawdzanie, czy to człowiek)

Opcjonalne i domyślnie wyłączone: bez kluczy wszystko działa dokładnie jak
w oryginale. Po włączeniu formularze logowania, rejestracji, prośby o reset
hasła i przyjęcia zaproszenia pokazują widżet Cloudflare, a serwer sprawdza
w Cloudflare każdy token, zanim cokolwiek zrobi. Gdy Cloudflare nie odpowiada,
serwer odmawia (lepiej chwilowo nie wpuścić nikogo, niż wpuścić boty).

**Klucze z panelu Cloudflare**

1. Zaloguj się na https://dash.cloudflare.com i otwórz **Turnstile**
   (w menu bocznym), potem **Add widget**.
2. Nazwa dowolna (np. `Modlight`). W **Hostname management** dodaj każdą
   nazwę hosta, pod którą otwierasz panel: host z `APP_URL` i każdy z
   `APP_DOMAINS` (sama nazwa, bez `https://` i bez portu, np.
   `pliki.example.com`). Na innym hoście widżet się nie pokaże.
3. **Widget Mode**: zostaw **Managed**. Pre-clearance: nie.
4. **Create**: Cloudflare pokaże **Site Key** (klucz witryny, jawny) i
   **Secret Key** (klucz tajny). Skopiuj oba.

**Gdzie je wpisać** (jedno z dwóch):

- Panel: **Administracja → Ustawienia → Cloudflare Turnstile**: klucz witryny,
  klucz tajny, przełącznik „Chroń formularze logowania”, **Zapisz**. Zapis
  z włączonym przełącznikiem najpierw pyta Cloudflare, czy przyjmuje klucz
  tajny (zły klucz zablokowałby każde logowanie, także Twoje); przycisk
  **Sprawdź** robi to samo bez zapisywania. Klucz tajny jest przechowywany
  zaszyfrowany i nigdy nie wraca do przeglądarki.
- Albo `server/docker/.env`: `TURNSTILE_SITE_KEY=...` i
  `TURNSTILE_SECRET_KEY=...` (oba, inaczej zostaje wyłączone), potem
  `docker compose -f server/docker/docker-compose.yml up -d`. Jeśli karta
  w panelu była kiedyś zapisana, panel ma pierwszeństwo w całości; przycisk
  „Wróć do ustawień środowiskowych” oddaje decyzję plikowi `.env`.

**Próba lokalnie, bez konta Cloudflare**: testowe klucze Cloudflare działają
na każdym hoście, także na 127.0.0.1. Klucz witryny `1x00000000000000000000AA`
z kluczem tajnym `1x0000000000000000000000000000000AA` przepuszcza wszystkich,
klucz tajny `2x0000000000000000000000000000000AA` odrzuca wszystkich. Widżet
pokazuje wtedy napis „For testing only”. Do prawdziwego użytku tylko klucze
własnego widżetu.

Zanim się wylogujesz po włączeniu, zaloguj się raz w oknie prywatnym. Gdyby
logowanie nie działało (np. host nie pasuje do widżetu), wyłącz to z bazy:

```sh
docker compose -f server/docker/docker-compose.yml exec postgres \
  psql -U modlight -d modlight -c "UPDATE settings SET value = 'off' WHERE key = 'turnstile_enabled'"
```

Zmiana działa w ciągu 30 sekund. Gdy Turnstile jest włączony, przeglądarka
pobiera skrypt z `challenges.cloudflare.com`, a serwer wysyła tam token
z formularza i adres IP odwiedzającego (więcej: `docs/SKELETON.md`, sekcja 9,
punkt 43).

## Co jest inaczej niż w oryginale (najważniejsze)

Pełna lista: `docs/SKELETON.md`, sekcja 9. W skrócie:

- **Poprawki bezpieczeństwa z audytu**: limit wyświetleń nie da się obejść
  nagłówkiem `Range` ani `Sec-Fetch-Dest`; ShareX sprawdza token, zanim
  przyjmie plik; pierwsze konto administratora jest ustalane w transakcji;
  porównanie tokenu usuwania w stałym czasie; `/oembed` nie zdradza plików
  z limitem wyświetleń; nagłówek `X-Real-IP` tylko przy `TRUST_PROXY`.
- **Poprawki z integracji**: zakończonego wysyłania tus nie da się pobrać ani
  skasować pod jego adresem tus (przy S3 omijało to prywatność pliku);
  zmiana awatara lub tapety usuwa stary plik (oryginał zostawiał go na zawsze).
- Album ZIP działa (w oryginale zawsze błąd 500).
- Nowość, której oryginał nie ma: opcjonalny Cloudflare Turnstile na
  formularzach logowania, rejestracji, resetu hasła i zaproszeń (wyżej).
- Zły znak `%` w adresie: 400 zamiast 500.

## Testy

```sh
cd server
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Testy z bazą danych uruchamiają się, gdy jest ustawione
`MODLIGHT_TEST_DATABASE_URL` (patrz `docs/SKELETON.md`, sekcja 1). Porównania
z oryginałem (`tests/parity`, `tests/parity_media`) i testy Playwright
z oryginału (`tests/e2e`) uruchamia się na osobnych, tymczasowych stosach,
nigdy na tym z portu 8430; jak, opisuje `docs/handoff/srv-integration.md`
w katalogu repozytorium.
