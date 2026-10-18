# Audyt bezpieczeństwa klienta Modlight-App

Data: 2026-10-18. Analiza wyłącznie statyczna (czytanie, grep, file, xxd, git, llvm-objdump). Nic z repozytorium nie było uruchamiane.

## Zakres i metoda

- Kopia klienta: `/path/to/modlight/upstream/Modlight-App` (jeden commit `b178a09` autora Nimfek, 2026-10-18).
- Przebadano CAŁY klient (wszystkie projekty), nie tylko `ShareX/Modlight/*`.
- Klient to fork ShareX. Żeby odróżnić zmiany autora od kodu upstream, sklonowałem oryginalny ShareX do `/path/to/modlight/_audyt/upstream-sharex` i ustaliłem commit bazowy: **`9c81fb01` ("Add configurable built-in toolbars for editor hosting", 2026-08-17)** — to commit ShareX z najmniejszą liczbą różnic w plikach wspólnych (biblioteki edytora/efektów/przechwytywania praktycznie bez zmian poza przemianowaniem). Wszystkie poniższe werdykty opierają się na porównaniu fork vs ten commit.
- Statystyka różnic: 164 pliki dodane, 726 usuniętych, 151 zmienionych. Autor usunął cały podsystem "uploaderów" ShareX (kilkadziesiąt usług z kluczami API, OAuth, FTP/SSH, host natywnego komunikatu przeglądarki, Steam, Microsoft Store) i zastąpił go jednym serwerem Modlight.

## WERDYKT GŁÓWNY: czy aplikacja wykrada dane

**Nie. Nie znalazłem żadnego backdoora, telemetrii ani wysyłania danych do strony trzeciej.** Kod jest napisany wyraźnie obronnie (token tylko w nagłówku, lista dozwolonych pól przy synchronizacji, pinowane SHA-256, walidacja hosta). Werdykt szczegółowy i pełna lista połączeń — w sekcji „Czy aplikacja wykrada dane". Nie ma ustaleń o wadze Krytyczna ani Wysoka. Ustalenia to głównie uwagi do przeniesienia na Rust i kilka drobiazgów Średnia/Niska.

---

## Co robi aplikacja (do przepisania na Rust)

Klient desktopowy Windows dla własnego serwera Modlight (Next.js + PostgreSQL). Logowanie przez przeglądarkę (RFC 8252 loopback), potem każdy zrzut/nagranie/plik ląduje w bibliotece konta.

### Okna (ShareX/Presentation/**)
- **MySpace** — biblioteka: foldery i pliki jako kafelki, podgląd animowany wideo/GIF, szuflada szczegółów, drag&drop, kosz.
- **SignIn** — logowanie: pole adresu serwera + logowanie przez przeglądarkę; awaryjnie wklejenie tokenu z panelu.
- **Download** — pobieranie przez yt-dlp (YouTube/TikTok/X), opcjonalne logowanie do YouTube w WebView2.
- **Admin** — podgląd dysku serwera (tylko dla roli admin).
- **Components** — pobieranie ffmpeg/yt-dlp/modelu ONNX/ExifTool.
- **Settings / AccountSettings**, **Upload (Paste)**, **Notification** (toasty), **FirstTimeConfig**, plus przeniesione z ShareX okna ustawień zadań, hotkeyów, edytora obrazu, efektów, przechwytywania.

### Endpointy serwera używane przez klienta (metoda, ścieżka, po co)
Wszystkie na host konta (`account.Host`), token w nagłówku `x-api-key`:
- `GET /api/me` — kto jest tokenem (tag, nazwa, e-mail, avatar, host linków, quota, rola) — `ModlightApi.cs:191`
- `POST /upload` — zwykły upload (plik/obraz/tekst), multipart `upload`, opcjonalnie `folderId` — `ShareX.UploadersLib/FileUploaders/Modlight.cs:125`
- `POST /api/files` (tus 1.0.0), `HEAD`, `PATCH`, `DELETE` — wznawialny upload offline — `ModlightResumableUpload.cs:98`
- `GET/PUT /api/client-settings?client=desktop` — synchronizacja ustawień (blob z wersją) — `ModlightSettingsSync.cs:574`
- `GET/PUT /api/theme` — synchronizacja skórki — `ModlightTheme.cs:53,61`
- `GET /api/folders`, `POST/PATCH/DELETE /api/folders` — foldery — `ModlightLibrary.cs`
- `GET /api/uploads`, `PATCH/DELETE /api/uploads` — lista/edycja/kosz plików — `ModlightLibrary.cs`
- `GET /api/duplicates`, `POST /api/duplicates` — duplikaty — `ModlightDuplicates.cs`
- `GET /api/admin/disk` — statystyki dysku (admin) — `ModlightServerDisk.cs:346`
- `POST /api/shortlinks` — skracanie linków — `ModlightURLShortener.cs:105`
- `GET /api/releases/app` — manifest aktualizacji (patrz niżej, host STAŁY, nie konta) — `ModlightReleaseClient.cs:94`
- `GET {host}/authorize?port=&name=&state=` — otwierane w przeglądarce przy logowaniu — `ModlightSignIn.cs:140`
- `GET .../download`, `.../convert`, thumbnails — pobieranie plików/konwersja/miniatury.

### Hosty zewnętrzne (poza serwerem konta)
- `github.com` / `api.github.com` — pobieranie komponentów (ffmpeg, yt-dlp, model) i auto-aktualizacja yt-dlp (adresy pinowane).
- `accounts.google.com` / `youtube.com` — tylko w oknie logowania YouTube w WebView2 (uruchamiane ręcznie).
- `api.openai.com` / `generativelanguage.googleapis.com` / `openrouter.ai` — tylko narzędzie „Analyze Image", tylko gdy użytkownik wpisze klucz i użyje funkcji.
- host aktualizacji = `ModlightDefaultHost` (w kodzie PUSTY; ustawiany przy budowaniu dystrybucji).

---

## Czy aplikacja wykrada dane (pełna lista, dowody)

### Połączenia wychodzące — dokąd, co, kiedy, czy użytkownik decyduje

| Dokąd | Co wysyłane | Kiedy | Decyzja użytkownika |
|---|---|---|---|
| `account.Host` (serwer konta) | token w `x-api-key`, pliki, nazwy, metadane folderów, blob ustawień/skórki | upload, przeglądanie biblioteki, sync | logowanie świadome; upload wg ustawień zadania |
| host aktualizacji (stały, build-time) | tylko `GET /api/releases/app` + pobranie instalatora | start + ręcznie | instalacja tylko po zgodzie i po zgodności SHA-256 |
| `github.com`, `api.github.com` | zwykłe GET (bez danych użytkownika) | pobieranie komponentów, auto-update yt-dlp co 24 h | komponenty wybierane w instalatorze/ustawieniach |
| Google/YouTube (WebView2) | logowanie Google użytkownika | tylko gdy klika „zaloguj do YouTube" | w pełni świadome |
| OpenAI/Gemini/OpenRouter | obraz + klucz użytkownika | tylko „Analyze Image" z własnym kluczem | w pełni świadome |

Dowody braku wycieku:
- **Token** trafia wyłącznie do nagłówka `x-api-key` na `account.Host`; nigdy do URL (żeby nie wpadł do logów proxy). Potwierdzone we wszystkich `Modlight*`. `ModlightSignIn.Save` (linie 419-451) i `Adopt` (359-385) nie nadpisują hosta/tokenu odpowiedzią z serwera — komentarz wprost: „letting a reply rewrite the credential it was authorised by is how a redirect becomes an account takeover".
- **Synchronizacja ustawień to lista dozwolonych** (`ModlightSettingsDocument.cs:96-208`). Nie wchodzą: token, klucze AI (`OpenAIAPIKey/GeminiAPIKey/OpenRouterAPIKey`), hasło proxy, ścieżki lokalne, pozycje okien, nazwa drukarki, ciasteczka YouTube (ich w ogóle nie ma w configu). Jawnie wykluczone w `NormalizeTaskSettings` (usuwa `AIOptions`, `CLIPath`, ścieżki).
- **Ciasteczka YouTube** (`YouTubeCookies.cs`): plik `YouTubeSignIn/cookies.txt` w folderze profilu, chroniony DPAPI-ACL (`Prepare`/`ProtectFile`), przekazywany TYLKO yt-dlp przez `--cookies` i TYLKO dla linków YouTube (`FileFor` zwraca null dla nie-YouTube). Nigdy nie jest uploadowany, nie trafia do configu, nie jest logowany (loguje się tylko liczbę ciasteczek). `NetscapeCookieFile.cs` to czyste IO plikowe, bez sieci. Używana własna profilowa instancja WebView2 (`userDataFolder` własny), NIE baza ciasteczek Chrome użytkownika, NIE `--cookies-from-browser`.
- **Brak telemetrii/analityki** — grep: żadnego Sentry/AppCenter/GA/Mixpanel/Plausible itp.
- **Brak wyłączonej walidacji TLS** — brak `ServerCertificateCustomValidationCallback`/`DangerousAccept...`. `HttpClientFactory` i własne handlery (`ModlightComponentDownload`, `ModlightResumableUpload`) używają domyślnej walidacji; `ResumableUpload` dodatkowo `AllowAutoRedirect=false` (nie przekazuje tokenu przez redirect) i `ResolveLocation` wymusza własny host/scheme.
- **Brak sprawdzania aktualizacji ShareX** — stary kanał GitHub `ShareX/ShareX` i dev-build AppVeyor zostały odcięte (`ShareXUpdateManager.cs` — komentarz wyjaśnia, że przy wersji 0.7.x każdy start widziałby „aktualizację" i wgrałby instalator ShareX). `getsharex.com` zostało tylko jako link WWW/docs (`Links.cs:30`, `BackgroundRemoverViewModel.cs:39`), nie jako źródło kodu.

### Dostęp do danych prywatnych (schowek, klawiatura, pliki, przeglądarka)
- **Schowek — odczyt**: tylko na żądanie — podgląd „Paste" wstępnie wypełnia treść (`PasteWindow.axaml.cs:65 ClipboardHelpers.GetText`) oraz ręczny „Clipboard upload". **Brak monitora schowka** (grep: brak `AddClipboardFormatListener`/`WM_CLIPBOARDUPDATE`). Zapis — kopiowanie linków/obrazów.
- **Klawiatura**: globalne hotkeye przez `RegisterHotKey` (nie keylogger). `KeyboardHook` (WH_KEYBOARD_LL) istnieje w `ShareX.HelpersLib/Input/KeyboardHook.cs` jako spadek po ShareX, ale **nigdzie nie jest tworzony** (grep `new KeyboardHook` = brak). Przy porcie na Rust nie przenosić.
- **Pliki/przeglądarki**: brak odczytu plików przeglądarek, historii, magazynów haseł. WatchFolder i AutoCapture działają tylko po świadomej konfiguracji przez użytkownika (`WatchFolderEnabled`, ręczne okno AutoCapture z timerem) — nie są ukryte.

---

## Ustalenia (waga, plik:linia, scenariusz, pewność, poprawka)

### [Średnia] Loopback logowania — ACAO:* + Allow-Private-Network; możliwy DoS logowania (NIE wstrzyknięcie tokenu)
`ShareX/Modlight/ModlightLoopbackListener.cs:172-178` — na preflight OPTIONS odpowiada `Access-Control-Allow-Origin: *`, `Access-Control-Allow-Private-Network: true`. Nasłuch na losowym porcie `127.0.0.1` (`:81`), przyjmuje pierwszy poprawny POST.
- **Wstrzyknięcie tokenu napastnika: NIE jest możliwe.** `ModlightSignIn.cs:138` generuje `state` = 32 znaki z `RandomCrypto` (CSPRNG, `RandomCrypto.cs:36 RandomNumberGenerator.Create`), a `NonceEquals` (`ModlightSignIn.cs:402-417`) porównuje go w stałym czasie; przy niezgodności logowanie jest **przerywane** (StateMismatch), nie kontynuowane. Obca strona/proces nie zna `state`, więc nie poda akceptowanego ciała.
- **Realne ryzyko (Niska/DoS)**: dowolny lokalny proces może połączyć się z portem i wysłać POST z błędnym `state` zanim zrobi to przeglądarka; listener zwraca to pierwsze ciało, a SignIn je odrzuca i KOŃCZY logowanie — da się tym uniemożliwić zalogowanie (nie przejąć konta). Okno 10 minut.
- **Scenariusz**: złośliwa strona WWW otwarta w oknie logowania robi `fetch` na `127.0.0.1:port` — bez `state` dostaje tylko przerwanie logowania.
- **Pewność: potwierdzone** (przeczytana cała ścieżka listener→SignIn).
- **Poprawka na Rust**: zachować obowiązkowy, stałoczasowy test `state`, pojedynczy strzał i losowy port. Rozważyć zawężenie ACAO do origin serwera zamiast `*` i niewychodzenie z pętli przy złym `state` (odporność na DoS). Nie opierać bezpieczeństwa na samym CORS.

### [Niska] `FileDownloader` uznaje pobranie bez Content-Length za kompletne
`ShareX.HelpersLib/FileDownloader.cs` (zmiana w forku, ~138-175): zwraca sukces gdy `DownloadedSize >= FileSize`; przy braku `Content-Length` `FileSize=0` i każde (nawet puste) pobranie = „kompletne".
- W wydanym kliencie ścieżkę tę wywołuje tylko aktualizator, który i tak weryfikuje `ExpectedSha256` przed uruchomieniem — plik niezgodny jest odrzucany (`DownloaderWindow.VerifyDownloadAsync`). Narzędzie build-time `ShareX.Setup/Program.cs` (pobranie ffmpeg/exiftool) hasha nie sprawdza, ale nie jest dostarczane użytkownikowi.
- **Pewność: potwierdzone.** **Poprawka na Rust**: nie uznawać pobrania za udane bez znanego rozmiaru/hasza; każdy pobierany wykonywalny plik musi mieć sprawdzony SHA-256.

### [Niska] Auto-aktualizacja yt-dlp pobiera i podmienia plik wykonywalny bez interakcji
`ShareX/Modlight/ModlightComponentUpdates.cs` — co 24 h (`CheckInterval`), domyślnie włączone (`ApplicationConfig.ExternalComponentsAutoUpdate = true`), pobiera najnowszy `yt-dlp.exe` z `yt-dlp/yt-dlp releases/latest` i podmienia go.
- Zabezpieczenia są solidne: `IsArtefactUrl` wymusza `https` i host dokładnie `github.com` (`ModlightToolUpdate.cs`), digest brany z pola `digest` API GitHuba i/lub pliku `SHA2-256SUMS`, przy obu musi być zgodność (`WithPublishedDigest`), inaczej odmowa. yt-dlp jest potem uruchamiany, więc to ścieżka „pobierz i wykonaj".
- **Pewność: potwierdzone.** To świadomy kompromis (yt-dlp starzeje się w tygodnie). **Poprawka na Rust**: zachować wymóg zgody dwóch źródeł digestu i pin na `github.com+https`; rozważyć pokazanie, że auto-update jest domyślnie włączony.

### [Niska] ExifTool — SHA-256 bez niezależnej publikacji
`ShareX/Modlight/ModlightComponent.cs:287` — digest ExifTool to własny hash autora; w komentarzu (`:157-161`) uczciwie zaznaczone, że wydanie `ShareX/ExifTool` nie publikuje sumy. Sprawdziłem: repo `ShareX/ExifTool` dziś nie istnieje/jest niedostępne przez API (404). Pozostałe trzy digesty **zweryfikowałem i zgadzają się**: ffmpeg x64 `1838932e...` i arm64 `7ac6c5db...` oraz yt-dlp x64 `66674953...` i arm64 `05b43899...` zgodne z polem `digest` wydań na GitHubie; model rembg `5b486f08...` zgodny z wartością w źródle rembg (`rembg/sessions/bria_rmbg.py`). **Poprawka na Rust**: przy braku publikacji trzymać pin na stałej w kodzie i nie dodawać dla ExifTool śledzenia „latest".

### [Info] Martwy kod po ShareX (nie przenosić)
- `TaskHelpers.DownloadAppVeyorBuild()` (`:2298`) i `AppVeyorUpdateChecker`/`GitHubUpdateChecker` — bez żadnego wywołania w forku (grep potwierdza brak callera). 
- `KeyboardHook` (opisany wyżej) — nieużywany.
- `Links.Website = https://getsharex.com` — otwierane w About/menu; kosmetyka, nie wyciek.

---

## Błędy logiczne

Przejrzałem kolejkę uploadu, synchronizację ustawień i pobieranie. Kod jest wyjątkowo starannie napisany pod kątem właśnie tych błędów; poniżej to, co znalazłem — głównie potwierdzenie braku błędów plus jedna uwaga.

1. **Upload na zły serwer po zmianie konta — NIE występuje.** `ModlightUploadQueue.Matches` (`:1066`) porównuje host (TrimEnd '/') i tag; element kolejki dla serwera A po zalogowaniu na B jest pomijany (`AccountChanged`) z 14-dniowym terminem, NIE wysyłany do B. `Verify` (`:653`) sprawdza, że plik źródłowy istnieje i ma ten sam rozmiar/czas zapisu, inaczej zaczyna od nowa. Pewność: potwierdzone.
2. **Utrata/podwójne wysłanie — zabezpieczone.** Element trzyma claim aż do zapisania wyniku (komentarz `:588-594`), więc Cancel tuż po sukcesie jest no-opem (nie tworzy drugiego „anulowano" dla wysłanego pliku). Zapis kolejki atomowy (tmp + `File.Move` over, `ModlightQueueStore.Save`). `IsWorthQueuing` (`:974`) kolejkuje tylko brak odpowiedzi i 408/429/502/503/504; 507 (brak miejsca) jest zgłaszane od razu, nie w pętli. Pewność: potwierdzone.
3. **Fresh-install guard w synchronizacji** (`ModlightSettingsPlan.cs:172`): instalacja bez potwierdzonego markera NIE może wypchnąć swoich ustawień — chmura wygrywa, więc świeży install nie nadpisze hotkeyów zbudowanych latami. Marker nie jest synchronizowany (słusznie). Schemat nieznany → nic nie robi. Pewność: potwierdzone, logika solidna.
4. **(Niska) `FileDownloader`** — opisane wyżej, jedyny realny błąd logiczny (sukces przy braku Content-Length), zneutralizowany przez weryfikację SHA aktualizatora.
5. **Kosz zamiast trwałego usunięcia** — wszystkie akcje usuwania z UI (MySpace, duplikaty) wołają tryb `"trash"` (`MainWindow.MySpace.Actions.cs:595`, `DuplicatesViewModel.cs:579`, `ModlightLibrary.TrashAsync:652`). Trwałe usuwanie nie jest podpięte pod UI. Domyślny wybór w duplikatach zostawia najstarszą kopię (`IsSelected = !IsOldest`). Pewność: potwierdzone, bezpieczne.
6. **Wstrzyknięcie argumentów do yt-dlp/ffmpeg z URL — NIE występuje.** `YtDlpRun` buduje argumenty jako listę (`ArgumentList`, nie string) i kończy `--` przed URL (`:505-506`), URL przechodzi przez whitelistę `DownloadLink.TryParse` (tylko absolutne http/https z hostem, `:63-100`). Brak `--exec`. yt-dlp dostaje `--ignore-config` tylko przy odpytaniu o wersję; przy pobieraniu config użytkownika mógłby coś dołożyć, ale to plik lokalny użytkownika, nie wejście z sieci. QuickJS podawany pełną ścieżką. Pewność: potwierdzone.

---

## Wykonanie kodu przy budowaniu/instalacji

- **Directory.build.props / .targets / Directory.Packages.props** — bez `Exec`, bez pobierania. `ModlightDefaultHost` w źródle PUSTY (jedyne `https://files.example.com` to przykład w komentarzu). `.targets` identyczne z upstream. Pakiety NuGet standardowe (Avalonia 12.1.1, Newtonsoft 13.0.4, SkiaSharp, ZXing, Microsoft.Web.WebView2 1.0.4129.50, ML.OnnxRuntime.DirectML, Vortice); usunięto `FluentFTP` i `SSH.NET`. Brak `nuget.config` (źródło = domyślny nuget.org). Centralne wersjonowanie. Nietypowych/podszywających się pakietów brak.
- **ShareX.UploadersLib.csproj** — `Target ModlightBuildInfo` (`:29-43`) generuje w czasie kompilacji stałą `ModlightBuildInfo.DefaultHost` z `ModlightDefaultHost`, z walidacją `https://` i braku końcowego `/`. Nieszkodliwe (WriteLinesToFile).
- **.github/workflows/build.yml** — `windows-latest`, `dotnet build ShareX/ShareX.csproj -c Release` + `dotnet run --project ShareX.Tests`. Uprawnienia `contents: read`. Bez sekretów, bez publikacji artefaktów, bez dociągania skryptów.
- **ShareX.Setup/Program.cs** — narzędzie build-time: pobiera ffmpeg/exiftool z `github.com/ShareX/...` (bez sprawdzania SHA — build-time, nie u użytkownika), woła lokalny ISCC.exe. Bez niespodzianek.
- **InnoSetup/Modlight-setup.iss** — osobny `AppId`/`AppMutex` (GUID `3030612E-...`, ≠ ShareX `82E6AC09-...`), więc nie przejmuje wpisu ShareX. Rejestr tylko pod `Software\Classes\...\Modlight`, `Software\Modlight\Setup` (SkipComponents) i `Control Panel\Keyboard` PrintScreen (zadanie opcjonalne). Import ShareX kopiuje **tylko** `ApplicationConfig.json` i `HotkeysConfig.json` — świadomie NIE `PersonalPath.cfg`, NIE `History.db`, NIE `UploadersConfig.json` (komentarz `:200-226` — ten ostatni zawiera klucze API usuniętych usług; kopiowanie go byłoby przeniesieniem sekretów). `taskkill` na ShareX bez `/f` (grzeczne zamknięcie). To naprawia realne pułapki, nie wprowadza nowych.
- **Tools/generate-skins.py**, **ValidateTranslations.ps1**, **Generate-LucideIcons.ps1** — narzędzia deweloperskie. Python importuje tylko stdlib (argparse/io/json/math/os/re/sys), czyta pliki panelu, pisze `Skins.axaml`/`SkinVariants.g.cs`; bez subprocess/eval/exec/sieci. PS1 bez `Invoke-WebRequest/Expression`, `DownloadString`, rejestru. Nie są częścią buildu klienta.
- **.resx** — żadnego zserializowanego obiektu binarnego poza standardowymi (ikony/bitmapy WinForms w `*.resx` edytora, niezmienione względem upstream; w zmienionych plikach różnice to tylko `ShareX.HelpersLib`→`Modlight.HelpersLib` w nazwach typów i teksty tytułów). `KnownTypesSerializationBinder` ogranicza deserializację efektów do podklas `ImageEffect`; brak `TypeNameHandling` ustawianego na danych z sieci.
- **Binaria w repo**:
  - `Tools/quickjs/qjs.exe` (SHA-256 `433a35a5...`) i `libwinpthread-1.dll` (`1933c3f0...`) — PE32+ x86-64, MinGW/Fedora, „QuickJS version 2026-06-04". Tabela importów (llvm-objdump): `qjs.exe` importuje tylko KERNEL32, msvcrt, i `MessageBoxW` z USER32 oraz pthready z libwinpthread — **brak ws2_32/wininet/winhttp** (żadnej sieci). `libwinpthread` tylko KERNEL32/msvcrt. Statycznie żadnej czerwonej flagi; pełnej analizy bajtowej nie da się zrobić statycznie, ale import table jest czysty. Reszta binariów (ikony .ico, .wav, .cur, .ttf lucide, .bmp kreatora, .webp zrzuty, docs/logo.svg) to zasoby — żaden .dll/.exe nie udaje innego pliku.
  - `ShareX.Setup/InnoSetup/puush` — **pusty plik (0 bajtów)**, odziedziczony z upstream (tam `-puush` branding puush.me). W forku tryb puush całkowicie usunięty z `Program.cs`/`MainForm.cs`, a plik nie jest już wymieniony w `.iss`. Nieszkodliwy, do usunięcia.

---

## Uwagi do przepisania na Rust (czego pilnować / nie przenosić)

1. Zachować: token tylko w nagłówku `x-api-key`; host aktualizacji STAŁY (build-time), nigdy z pola logowania; weryfikacja SHA-256 instalatora i komponentów przed uruchomieniem; lista dozwolonych pól przy synchronizacji (bez sekretów i ścieżek); ciasteczka YouTube tylko lokalnie i tylko do yt-dlp; argumenty podprocesów jako lista + `--`.
2. Loopback: obowiązkowy stałoczasowy `state` z CSPRNG, pojedynczy strzał, losowy port; zawęzić CORS i nie przerywać nasłuchu przy złym `state`.
3. Nie przenosić: `KeyboardHook`, `DownloadAppVeyorBuild`/AppVeyor/GitHubUpdateChecker, pozostałości puush, linki `getsharex.com`.
4. Pobieranie: nigdy nie uznawać pobrania za udane bez znanego rozmiaru/hasza.
5. Token przechowywany szyfrowany przez DPAPI (`[JsonEncrypt]` + `DPAPIEncryptedStringPropertyResolver`) — na Macu odpowiednik to Keychain; nie zapisywać tokenu/kluczy AI jawnym tekstem.
