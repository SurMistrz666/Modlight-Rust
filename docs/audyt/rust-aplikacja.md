# Audyt bezpieczeństwa aplikacji (Rust) Modlight App dla macOS

Data: 2026-10-20. Zakres: `app/` w `/Users/marcinasdfg/Desktop/modlight` (gałąź main, commit
`6eed920`), workspace Rust: `modlight-api`, `modlight-core`, `modlight-macos`,
`modlight-app`, `modlight-editor` (ok. 125 tys. linii). Metoda: czytanie statyczne +
grepy + budowa i uruchomienie całego pakietu testów oraz drobne testy-sondy w kopii
roboczej (`_scratch/sec-review-app`, poza głównym repo). Główny checkout tylko do odczytu.
Reguły, które port miał zachować: `app/docs/PORT.md` sekcja 5; podstawienia macOS:
`app/docs/MACOS.md`; audyt oryginału: `docs/audyt/klient.md`.

Testy: `cargo test --workspace` = 544 zdane, 0 niezdanych, 1 pominięty (ręczny),
łącznie z testami przeciw oryginalnemu serwerowi na `http://127.0.0.1:8420`. Konto QA
zalogowane i wyczyszczone po teście; nic nie zostawiono.

## WERDYKT GŁÓWNY: czy aplikacja wysyła dane na zewnątrz

**Nie ma backdoora, telemetrii ani ukrytej eksfiltracji.** Każde połączenie wychodzące
jest na stałe związane z hostem konta (token tylko w `x-api-key`, nigdy w URL, nigdy za
przekierowaniem), z `github.com` (komponenty z pinowanym SHA-256) albo z hostem
aktualizacji ustalonym przy budowaniu. Brak Sentry/GA/Mixpanel/Plausible itp. (grep
czysty). Brak wyłączenia walidacji TLS. Lista dozwolonych pól przy synchronizacji działa
(token, klucze AI, hasło proxy, ścieżki, pozycje okien, ciasteczka nie wychodzą).

Jest natomiast **jedna realna droga wyprowadzenia tokenu** — nie przez kod sieciowy, lecz
przez import kopii zapasowej ustawień: złośliwy plik `.sxb` może podmienić host konta,
zostawiając prawdziwy token w Keychain (ustalenie **W1** niżej). Wymaga, by użytkownik sam
zaimportował plik od napastnika, ale skutek to wysłanie tokenu na serwer napastnika.

---

## Czy aplikacja wysyła dane na zewnątrz (każde połączenie, werdykt)

| Dokąd | Co | Kiedy | Decyzja użytkownika | Werdykt |
|---|---|---|---|---|
| host konta (`Credential.host`) | token w `x-api-key`, pliki, nazwy, metadane, blob ustawień/skórki | upload, biblioteka, sync | logowanie świadome | OK — token tylko w nagłówku, host nienadpisywalny odpowiedzią (patrz niżej) |
| host konta — miniatury/konwersje/pobrania | token w nagłówku | przeglądanie/pobieranie | — | OK — każdy link z serwera przepuszczony przez `rewrite_to_host` (origin wymuszony na host konta) |
| `github.com` / `api.github.com` | zwykłe GET bez danych usera | komponenty, auto-update yt-dlp | wybór w Komponentach | OK — `net::is_artefact_url` wymusza https + dokładnie `github.com`; dwymóg zgody obu digestów |
| `ffmpeg.martin-riedl.de`, `downloads.sourceforge.net` (+ redirecty) | GET | pobranie ffmpeg/ExifTool | wybór w Komponentach | OK — tylko https, redirecty tylko na https, SHA-256 pinowany |
| host aktualizacji (`MODLIGHT_DEFAULT_HOST`, build-time) | GET `/api/releases/app` + pobranie `.dmg`/`.zip` | start + ręcznie | instalacja po zgodzie i zgodności SHA-256 | OK — domyślnie pusty = wyłączone; redirect none |
| `accounts.google.com` / `youtube.com` (WKWebView) | logowanie Google usera | tylko klik „zaloguj do YouTube" | świadome | OK — magazyn danych nieperswistentny, ciasteczka tylko lokalnie |
| adres wpisany/wklejony (Upload from URL, edytor „otwórz z URL") | brak poświadczeń | ręcznie | świadome | OK co do tokenu; patrz **S3** (brak limitu dekodowania obrazu) |

Dowody braku wycieku tokenu (potwierdzone czytaniem):
- `modlight-api/src/client.rs:72-83,221-229` — oba klienty `reqwest` mają
  `redirect::Policy::none()`; `request()` dokłada `x-api-key` i nic więcej;
  `absolute()` (`:231`) skleja tylko ścieżki względne z hostem konta, a wszystkie wywołania
  `send_json`/`get_string` używają literałów ścieżek (`/api/me`, `/api/uploads`, …).
- `modlight-api/src/host.rs:75-89` `rewrite_to_host` — z linku bierze tylko ścieżkę i
  query, origin zawsze z hosta konta; `:92` `same_origin`.
- `modlight-core/src/account.rs:216-237` `adopt` — odpowiedź `/api/me` nigdy nie nadpisuje
  hosta ani tokenu (komentarz i kod zgodne z oryginałem).
- tus: `modlight-api/src/tus.rs:799-819` `resolve_location` i `:351` — sesja zawsze
  przeniesiona na origin konta przed każdym żądaniem; token nie podąża za `Location`.
- synchronizacja: `modlight-core/src/sync/document.rs:117-184` buduje dokument wyłącznie z
  `APP_MEMBERS`, normalizuje `DefaultTaskSettings` (usuwa `AIOptions`, ścieżki,
  `CLIPath`/`OverrideCLIPath`, ścieżki dźwięków); `ProxySettings` nie ma na liście, więc
  hasło proxy nie wychodzi. Test `only_the_allowlist_travels` to potwierdza.

---

## Ustalenia (waga, plik:linia, scenariusz, pewność, poprawka)

### [Wysoka] W1 — import kopii zapasowej podmienia host konta i wysyła prawdziwy token na serwer napastnika
`modlight-core/src/backup.rs:52-80` (`import`), filtr `:63` + `is_plain_name` `:93`.

`import` wpisuje do folderu profilu każdy wpis zip o „zwykłej" nazwie z rozszerzeniem
`json`/`xml`. `UploadersConfig.json` taką nazwą jest. Na macOS token NIE leży w tym pliku
— leży w Keychain (`modlight-core/src/account.rs:17,134-144`), a w pliku jest tylko host
konta. Po imporcie, przy następnym starcie `AccountStore::load` (`main.rs:151`) czyta host
z podmienionego `UploadersConfig.json`, a token z Keychain (nienaruszony), i paruje je:
`credential()` zwraca `host = napastnik`, `token = prawdziwy`. Każde kolejne żądanie idzie
na host napastnika z `x-api-key: <token ofiary>`. `adopt` nie koryguje hosta.

- **Scenariusz**: napastnik podsyła plik „moje ustawienia Modlight.sxb"; ofiara robi
  Ustawienia → Aplikacja → Importuj kopię (`app_pages.rs:1951-1990`). Plik zawiera
  `UploadersConfig.json` = `{"ModlightAccount":{"Host":"https://zlo.example","Tag":"…"}}`.
  Po restartie token ofiary leci na `zlo.example`.
- **Regresja względem oryginału**: na Windowsie token jest w tym samym
  `UploadersConfig.json` (DPAPI), więc nadpisanie go kasuje też token — napastnik nie zna
  ważnego tokenu. Rozdzielenie „host w pliku / token w Keychain" (świadome podstawienie
  macOS) utworzyło dziurę, której oryginał nie miał. `docs/audyt/klient.md` wprost
  odnotowuje, że import ShareX w instalatorze celowo NIE kopiuje `UploadersConfig.json`,
  „bo to przeniesienie sekretów" — port tę zasadę porzucił.
- **Pewność: potwierdzone** (sonda w kopii roboczej: po imporcie żądanie `/api/me`
  poszło na host-pułapkę z nagłówkiem `x-api-key` = tokenem; przeczytana cała ścieżka
  import → load → credential).
- **Poprawka**: w `import` pominąć pliki stanu klienta, których `export`
  (`backup.rs:24-48`) i tak nigdy nie tworzy: `UploadersConfig.json`, `Queue.json`,
  oraz samo `History.db`. Eksport zapisuje tylko `ApplicationConfig.json`,
  `HotkeysConfig.json`, `History.db`, więc wykluczenie `UploadersConfig.json`/`Queue.json`
  nie zmienia poprawnego działania. Najlepiej: biała lista nazw dozwolonych do importu
  zamiast czarnej.

**Stan po poprawkach** (gałąź `app-secfix`, commit `735b6a4`):
- `backup::import` przyjmuje wyłącznie `ApplicationConfig.json` i `HotkeysConfig.json`
  (biała lista dokładnych nazw); `History.db` jest pomijany jak w oryginale. Każdy inny
  wpis (`UploadersConfig.json`, `Queue.json`, nazwy z folderem, `..`, kropką na początku,
  inna wielkość liter) jest odrzucany i wymieniany w komunikacie po imporcie (linia
  statusu + okno z ostrzeżeniem). Nic nie jest zapisywane, dopóki całe archiwum nie
  zostanie przeczytane.
- Głębia obrony: magazyn poświadczeń (Keychain / plik QA) trzyma token razem z originem,
  dla którego został wydany (`account.rs`, `BoundToken`). Host w `UploadersConfig.json`
  inny niż ten origin (edycja pliku, czyjaś kopia) daje stan „wylogowany”: token nie jest
  z nim parowany, dopóki użytkownik nie zaloguje się na tym hoście. Token zapisany przez
  wcześniejszą wersję (bez originu) wymaga jednorazowego ponownego logowania.
- Testy: `backup::tests::a_backup_cannot_repoint_the_account`,
  `backup::tests::import_takes_only_the_files_export_writes`,
  `account::tests::a_host_swapped_behind_the_token_never_gets_it`,
  `account::tests::a_token_that_does_not_name_its_host_is_not_used`.

### [Średnia] W2 — import kopii zapasowej (Queue.json) kasuje dowolny plik przy następnym starcie
`modlight-core/src/backup.rs:52` + `modlight-core/src/upload/store.rs:71` (`load`) +
`modlight-core/src/upload/queue.rs:963-979` (`expire`) → `fail` `:900-904` →
`release` `:983-985` → `modlight-core/src/upload/store.rs:172-182` (`discard_spool`).

`Queue.json` to też „zwykła" nazwa `.json`, więc import ją wpisuje. Pozycja kolejki z
`OwnsSource = true`, dowolną `SourcePath` i przeszłym `DeadlineUtc` jest przy starcie
wczytana (`load`), uznana za przeterminowaną (`expire`) i zwolniona; `discard_spool`
kasuje `source_path` bez sprawdzania, czy leży w folderze spool — a więc dowolny plik
użytkownika. Usunięcie jest trwałe (`std::fs::remove_file`, nie do Kosza).

- **Scenariusz**: złośliwy `.sxb` z `Queue.json` wskazującym np. `~/Documents/praca.docx`,
  `OwnsSource=true`. Po restarcie plik znika.
- **Pewność: potwierdzone** (sonda: plik-ofiara skasowany przez odtworzoną kolejkę; bez
  zalogowania i bez sieci).
- **Poprawka**: jak W1 — nie importować `Queue.json`. Dodatkowo (głębia obrony)
  `discard_spool` powinno kasować tylko pliki w folderze spool kolejki.

**Stan po poprawkach** (gałąź `app-secfix`, commity `735b6a4` i `ce46e09`):
- `Queue.json` nie jest już importowany (biała lista z W1, `735b6a4`).
- `QueueStore::discard_spool` (`ce46e09`) kasuje plik tylko wtedy, gdy ścieżka jest taka,
  jaką nadaje `QueueStore::spool`: bezwzględna, bez `..`, bezpośrednio w folderze spool
  i nazwana identyfikatorem pozycji (`{id}` + rozszerzenie); nigdy katalogu. Samo
  `OwnsSource = true` z `Queue.json` nie wystarcza; inne ścieżki są pomijane z wpisem w
  logu.
- Testy: `upload::queue::tests::a_queue_file_from_elsewhere_cannot_delete_the_persons_files`
  (restart z `Queue.json` wskazującym plik poza spoolem: plik zostaje, własny plik kolejki
  znika), `upload::store::tests::only_spooled_names_are_ever_deleted`,
  `backup::tests::a_backup_cannot_replace_the_upload_queue`.

### [Średnia] W3 — import kopii zapasowej (ApplicationConfig.json) może skonfigurować autostart programu zewnętrznego → wykonanie kodu
`modlight-core/src/backup.rs:52` → `app_pages.rs:1972` (`import`) →
`reload_settings_files` `:1802-1806` → `apply_settings_document` `:1787-1799`;
`modlight-core/src/tasks/worker.rs:773-790` (`PerformActions`) →
`modlight-core/src/tasks/actions.rs:104-136` (`run`).

`ApplicationConfig.json` jest legalnym plikiem kopii (eksport go tworzy), więc import go
nadpisuje w całości i od razu stosuje. `DefaultTaskSettings.ExternalPrograms` z
`IsActive=true`, `Path="/bin/sh"`, `Args="-c '…'"` plus `AfterCaptureJob` zawierający
`PerformActions` powoduje, że przy pierwszym przechwytywaniu/hotkeyu uruchamia się dowolne
polecenie. `actions.rs` nie używa powłoki, ale program może nią BYĆ (`/bin/sh -c`).
Watch folders z importowanego configu też uruchamiają zadania.

- **Pewność: potwierdzone czytaniem** (ścieżka import → reload → defaults_from →
  worker → actions kompletna).
- **Uwaga**: to zachowanie jest zgodne z oryginałem (przywracanie własnej kopii słusznie
  przywraca akcje zewnętrzne) i działa dopiero, gdy użytkownik świadomie zaimportuje obcy
  plik. To bardziej granica zaufania niż błąd portu, ale skutek (wykonanie kodu) uzasadnia
  wagę Średnia.
- **Poprawka**: po imporcie nie włączać milcząco akcji zewnętrznych/watch folderów bez
  potwierdzenia, albo ostrzec, że kopia może zawierać polecenia uruchamiane automatycznie.
  Minimalnie: skoro eksport i tak nie zapisuje tokenu, ograniczyć import do trzech nazw,
  które eksport wytwarza, i rozważyć prośbę o potwierdzenie, gdy config włącza
  `PerformActions`/watch folder.

**Stan po poprawkach** (gałąź `app-secfix`, commit `ece123a`):
- Import porównuje każde przywracane zadanie z tym samym zadaniem sprzed importu
  (`DefaultTaskSettings` z `ApplicationConfig.json`; hotkey z `HotkeysConfig.json` według
  pozycji na liście) i wyłącza to, co uruchamia się samo, a nie było już włączone w tym
  zadaniu: aktywny program zewnętrzny (zostaje włączony tylko, gdy identyczny program był
  już aktywny), krok `PerformActions` w `AfterCaptureJob` oraz watch foldery (zostają
  tylko, gdy każdy folder był już obserwowany). Reszta pliku jest przywracana bez zmian;
  własna kopia na tym samym Macu wraca bajt w bajt, na innym Macu te elementy czekają na
  ręczne włączenie.
- Komunikat po imporcie wymienia każdy wyłączony element (zadanie + akcja/ścieżka), żeby
  użytkownik włączał je świadomie. Odstępstwo od oryginału opisane w `app/docs/MACOS.md`
  („Behaviour that differs on purpose”). Plik ustawień, który nie jest obiektem JSON,
  przerywa import przed zapisem czegokolwiek.
- Testy: `backup::tests::programs_a_backup_switches_on_come_in_switched_off` (`/bin/sh -c`,
  własny hotkey z `PerformActions`, nowy watch folder),
  `backup::tests::ones_own_backup_comes_back_as_it_was`,
  `backup::tests::a_settings_file_that_is_not_json_fails_the_import`.

### [Średnia] S3 — brak limitów przy dekodowaniu obrazów z serwera i z adresów usera (DoS pamięcią)
`modlight-app/src/media.rs:75` (`decode_still`), `:100,:107` (animacje),
`modlight-app/src/uploads.rs:799` i `modlight-app/src/editor_window.rs:557-564`
(`download_image`), `modlight-api/src/web.rs:101-131`.

`image::load_from_memory` / dekodery wołane są na bajtach z serwera (miniatury, avatar) i z
adresów wklejonych przez użytkownika (Upload from URL, edytor „otwórz z URL") bez
ustawienia `image::Limits`. Pobranie jest ograniczone rozmiarem skompresowanym (8 MB
miniatura, 4 MB animacja, 50 MB podgląd), ale nie wymiarami. Spreparowany mały plik PNG z
ogromnymi wymiarami (np. 30000×30000) powoduje alokację rzędu GB i zabicie procesu.

- **Scenariusz**: wrogi serwer konta zwraca taką „miniaturę"; albo użytkownik wkleja URL do
  takiego obrazu. Skutek: zawieszenie/OOM aplikacji (DoS, nie RCE, nie wyciek).
- **Pewność: prawdopodobne** (brak jakiegokolwiek limitu w kodzie potwierdzony grepem;
  nie odpalono realnego PoC, bo wymaga dużej alokacji na maszynie właściciela).
- **Poprawka**: dekodować przez `image::ImageReader` z `.limits(Limits)` (limit alokacji i
  maks. wymiary) dla wszystkich bajtów z sieci. Audyt oryginału wymieniał „dekodowanie
  wrogich obrazów" jako punkt uwagi.

**Stan po poprawkach** (gałąź `app-secfix`, commit `efb151c`):
- Nowy moduł `modlight_core::image_decode`: `from_bytes`, `from_bytes_as`, `open` z
  `image::Limits` (maks. 32768 px na bok, 512 MiB na obraz), `fits()` dla dekoderów bez
  limitów, `animation_limits()` (128 MiB na klatkę). Uwaga: `ImageReader` miał już
  domyślnie 512 MiB alokacji, ale bez limitu wymiarów; dekodery GIF/WebP animacji nie
  miały żadnego limitu i trzymały do 60 pełnych klatek naraz.
- Przez ten moduł idą wszystkie dekodowania bajtów z zewnątrz: miniatury, podglądy i
  avatar (`media.rs`; animacje dekodowane klatka po klatce pod limitami i od razu
  zmniejszane), podgląd po uploadzie, okna zadań i powiadomienia, „kopiuj obraz”, upload
  pliku z przetwarzaniem obrazu (`worker.rs`), edytor: otwieranie/wstawianie/tło
  (`open_image`) i „otwórz z URL”. Schowek: obraz czytany jako bajty PNG/TIFF z
  `NSPasteboard` (`modlight_macos::pasteboard`) i dekodowany u nas, nie przez arboard.
  `NSImage` (HEIC itp. w edytorze) jest najpierw mierzony i odrzucany powyżej tych limitów.
- Błędy z tekstami oryginału: „The URL does not point to a valid image”, „Failed to
  download image: …”, „Failed to load image file. <ścieżka>”; pobieranie z adresu w
  edytorze przerywane powyżej 256 MiB.
- Testy: `image_decode::tests::a_header_claiming_an_enormous_picture_is_refused_by_its_size`
  (prawdziwe PNG i JPEG z nagłówkiem 30000×30000, 65535×65535, 32769×1),
  `media::tests::pictures_claiming_an_enormous_size_are_refused` (miniatura i animacja GIF
  z płótnem 30000/40000 px),
  `editor_window::tests::a_downloaded_picture_claiming_an_enormous_size_is_not_a_valid_image`.

### [Niska] S4 — link projektu ShareX w oknie „O programie"
`modlight-app/src/about_window.rs:266-282` — hiperłącza do `github.com/Jaex`,
`github.com/McoreD`, `github.com/ShareX/ShareX` (atrybucja zespołu). Otwierane wyłącznie
po kliknięciu przez `system::open_url`, który przepuszcza tylko http(s). Zgodne z PORT.md
(„About zachowuje zespół i stronę projektu"). Nie jest źródłem aktualizacji. Bez działania;
wymieniam dla kompletności, bo grep na „ShareX/ShareX" trafia tu.

---

## Zasady z audytu oryginału (PORT.md sekcja 5): zachowane czy nie

1. **Token tylko w `x-api-key`, na host konta, nigdy w URL/na inny host/za przekierowaniem**
   — **ZACHOWANE** w warstwie sieciowej (dowody w sekcji „Czy aplikacja wysyła dane"). Z
   zastrzeżeniem: host, z którym token jest parowany, da się podmienić przez import kopii
   (W1) — to nie obejście w kodzie sieciowym, lecz w imporcie.
2. **Host aktualizacji stały (build-time), nigdy z pola logowania** — **ZACHOWANE**.
   `modlight-core/src/update.rs:27-54` `RELEASE_HOST = option_env!("MODLIGHT_DEFAULT_HOST")`,
   walidacja `https://` i braku końcowego `/` w `const` (błąd kompilacji inaczej); domyślnie
   pusty = wyłączone; `fetch` z `redirect::none()`; `is_acceptable_url` https wszędzie, http
   tylko na loopbacku (dla testów).
3. **Każdy pobierany wykonywalny/model sprawdzony SHA-256 przed użyciem; brak rozmiaru/hasza
   = porażka** — **ZACHOWANE**. `components/verify.rs:65-98` `verify_or_delete` (kasuje przy
   niezgodności i przy braku digestu); `components/download.rs:94-101` wymaga
   `expected_size > 0`, nigdy nie ufa `Content-Length`; `components/mod.rs` pinuje URL,
   SHA-256 i rozmiar jako stałe; `tool_release.rs:296-316` `with_published_digest` wymaga
   zgody digestu z API GitHuba I z `SHA2-256SUMS` (ostrzej niż oryginał); update aplikacji
   wymaga rozmiaru i SHA (`update.rs:parse_manifest`, `download`).
4. **Synchronizacja wysyła tylko pola z białej listy (bez tokenu, kluczy AI, hasła proxy,
   ścieżek, pozycji, ciasteczek)** — **ZACHOWANE**. `sync/document.rs:19-184`; token nie jest
   nawet w pliku; `ProxySettings`/`AIOptions`/ścieżki poza listą lub usunięte przez
   `normalize_task_settings`. Test `only_the_allowlist_travels`.
5. **Loopback logowania: 127.0.0.1, losowy port, CSPRNG `state` w stałym czasie, jeden
   strzał, CORS tylko origin serwera (bez `*`, bez `Access-Control-Allow-Private-Network`),
   zły `state` odpowiedziany i zignorowany (nie przerywa)** — **ZACHOWANE w całości**.
   `modlight-api/src/signin.rs`: `bind(("127.0.0.1", 0))` `:112`; `new_state` z
   `rand::rng()` (CSPRNG) `:62-67`; `nonce_equals` stałoczasowe `:71-84`; zły state →
   log i pętla dalej `:142-147`; CORS tylko dla `allowed_origin`, bez private-network
   `:196-213,250-256`; pojedynczy strzał (`wait_for` zwraca na pierwszym zgodnym state).
6. **Procesy potomne: wektor argumentów, `--` przed URL/ścieżką usera, bez powłoki** —
   **ZACHOWANE**. yt-dlp: `download/run.rs:99-154` wektor, `--` przed URL (`:152`), bez
   `--exec`; `download/link.rs:27-50` biała lista URL (http/https, bez userinfo). ffmpeg:
   `recording/ffmpeg.rs:84-96` wektor, bez powłoki; szablon „własnych poleceń"
   `recording/args.rs:169-211` przez `split_command_line`, nie powłokę; wejście z potoku
   zawsze pierwsze. Programy zewnętrzne `tasks/actions.rs:84-119` — rozbicie na wektor, potem
   podstawienie `$input`/`$output` w całe argumenty (test `placeholders_fill_whole_arguments`).
   `process::command` `components/process.rs:89-98` bez powłoki, własna grupa procesów,
   stdin zamknięty.
   Pobrane binaria: weryfikacja SHA-256 w tej samej funkcji, która je stawia
   (`install.rs:201-273`), zapis atomowy przez staging `.incoming` + rename, prawa 0755/0644;
   kolejność download → verify → place → delete-part zamyka okno TOCTOU między weryfikacją a
   uruchomieniem (plik uruchamiany to ten zweryfikowany, przeniesiony atomowo).
7. **Ciasteczka YouTube: plik lokalny, tylko dla yt-dlp, tylko dla URL YouTube, nigdy nie
   wysyłane** — **ZACHOWANE**. `download/cookies.rs`: folder 0700 / plik 0600 (`write`
   `:106-128`), `file_for` zwraca Some tylko dla `is_youtube && is_signed_in` (`:244-246`),
   `--cookies` dokładane tylko gdy jest jar (`run.rs:120-123`), do configu nie trafia,
   logowana tylko liczba ciasteczek; WKWebView z magazynem nieperswistentnym
   (`webview.rs:379`). Test `file_for` dla TikToka = None.
8. **Nie przenosić martwego kodu ShareX (KeyboardHook, AppVeyor/GitHub ShareX update,
   puush, linki getsharex.com)** — **ZACHOWANE**. Brak `KeyboardHook`/keyloggera (żadnego
   globalnego monitora klawiatury; `keys.rs` to lokalny monitor tylko Cmd+V), brak AppVeyor,
   brak sprawdzania aktualizacji ShareX. `getsharex.com` tylko w komentarzach „pominięte".

Podstawienia z `MACOS.md` sprawdzone i zgodne: Keychain przez `/usr/bin/security`
(sekret przez stdin `security -i`, nie przez argv — `ps` pokazuje tylko `/usr/bin/security -i`;
token hex-kodowany, nazwy itemu walidowane `plain()`), `/usr/bin/open` tylko http(s)
i ze ścieżką po `--`, FSEvents dla watch folderów, loopback logowania jak w sekcji 5.

---

## Błędy logiczne (upload na zły serwer / kolejka / historia)

- **Upload na zły serwer po zmianie konta — NIE występuje.** `upload/queue.rs:246-255`
  `matches` porównuje host (bez końcowego `/`, bez rozróżniania wielkości) i tag; pozycja dla
  konta A po zalogowaniu na B jest pomijana z powodem `AccountChanged` (`:587-589`), nie
  wysyłana. `item.rs:is_usable` i porównanie rozmiaru/czasu zapisu przed wysyłką. Zgodne z
  oryginałem (`ModlightUploadQueue.Matches`).
- **Kolejka offline a nowe konto — zabezpieczone.** Slot konta aktualizowany co klatkę
  (`uploads.rs:221-239`), a `due` (`queue.rs:575-603`) i tak sprawdza `matches` przed każdą
  wysyłką.
- **Historia innego konta** — `History.db` jest wspólna dla profilu (kolumna `Host`), karta
  Upload łączy historię z sesją; to zgodne z oryginałem (jedna biblioteka lokalna na maszynę).
  Nie znalazłem wycieku linków między kontami w logice odczytu.
- **Podwójne wysłanie / utrata** — zapis kolejki atomowy (`store.rs` tmp+rename), claim do
  zapisania wyniku, `is_worth_queuing` kolejkuje tylko brak odpowiedzi i 408/429/502/503/504.
- **Fresh-install guard** w synchronizacji zachowany (`sync/plan.rs:90-96`): instalacja nigdy
  nie potwierdzona dla konta nie wypycha — chmura wygrywa. Schemat nowszy → nic nie robi.
- Uwaga: `discard_spool` (W2) kasuje `source_path` spoza folderu spool — opisane wyżej.

---

## Rzeczy sprawdzone i w porządku

- Brak telemetrii/analityki; brak wyłączenia TLS; brak `0.0.0.0`.
- Token: `Debug` dla `Credential`/`SignedIn`/`QueueAccount`/`CookieData` maskuje sekret
  (`client.rs:48-55`, `signin.rs:293-301`, `webview.rs:30-38`); logi zawierają tylko liczbę
  ciasteczek, nazwy itemów, hosty — nie token (grep logów). Teksty błędów zawierają nagłówki
  ODPOWIEDZI serwera, nie żądania, więc `x-api-key` tam nie trafia (`upload.rs:337-362`,
  `uploader.rs:106-162`).
- Keychain: sekret nie w argumentach procesu (stdin `security -i`); brak wstrzyknięcia
  (nazwy walidowane, token hex).
- Rozpakowywanie archiwów komponentów (`components/archive.rs`): wpisy wychodzące poza
  katalog pomijane (`safe_relative`), linki/urządzenia nigdy nie tworzone, sufit rozmiaru
  liczony i z deklaracji, i z realnie zapisanych bajtów, limit liczby wpisów. Wejście zawsze
  po weryfikacji SHA-256.
- Loopback, synchronizacja, pobrania komponentów/aktualizacji — jak w sekcji 5 wyżej.
- FFI/`unsafe` w `modlight-macos` (37 w `capture.rs`, 19 w `webview.rs`, resztówki gdzie
  indziej): przejrzane. Bloki to standardowe wywołania AppKit/ScreenCaptureKit/WebKit na
  wątku głównym, z komentarzami SAFETY, z retain/release przez `Retained`/`RcBlock`;
  `copy_frame` (`capture.rs:610-639`) sprawdza `width>=w && height>=h` i kopiuje wiersz po
  wierszu w granicach; `to_rgba` (`:293-350`) tnie wiersz do `min(bytes.len())` i kolumnę do
  `min(line.len()/4)`. Delegat WKWebView nie jest zwalniany we własnym callbacku (retire na
  następnej klatce) — poprawnie. Nie znalazłem UAF ani przepełnień.
- `unwrap`/`expect`/indeksowanie na danych zdalnych: parsowanie odpowiedzi idzie przez serde
  z `Default`/deserializerami tolerancyjnymi (`parse::<T>`, `null_default`, `lenient_time`);
  indeksy w `library/folders.rs`/`budget.rs`/`caches.rs` są po pozycjach z tej samej
  struktury (spójne wewnętrznie), nie z sieci. Nie znalazłem paniki z danych serwera.
- Testy: całość `cargo test --workspace` zielona (544/0/1).

---

## Podsumowanie wag

- Krytyczna: brak.
- Wysoka: **W1** — import kopii (`UploadersConfig.json`) rebinduje token na host napastnika.
- Średnia: **W2** — import kopii (`Queue.json`) kasuje dowolny plik; **W3** — import kopii
  (`ApplicationConfig.json`) może wstawić autouruchamiany program → wykonanie kodu;
  **S3** — brak limitów dekodowania obrazów (DoS pamięcią).
- Niska: **S4** — link do repo ShareX w „O programie" (bez działania).

Wspólny mianownik W1–W3: `backup::import` przyjmuje pliki, których `backup::export` nigdy
nie tworzy (`UploadersConfig.json`, `Queue.json`) i stosuje config w całości. Najprostsza
naprawa W1+W2: biała lista nazw dozwolonych do importu = dokładnie to, co eksport zapisuje.

Stan po poprawkach (gałąź `app-secfix`): W1 `735b6a4`, W2 `735b6a4` + `ce46e09`,
W3 `ece123a`, S3 `efb151c`; S4 bez zmian (bez działania). `cargo fmt`, `cargo clippy
--workspace --all-targets -- -D warnings` i `cargo test --workspace` zielone (557 zdanych,
0 niezdanych, 1 pominięty ręczny).
