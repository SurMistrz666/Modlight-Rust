# Stan prac: Modlight w Rust

Ostatnia aktualizacja: 2026-10-18 (koniec dnia 1, praca wstrzymana na prośbę właściciela).

## Cel (decyzje właściciela z 2026-10-18)

- Pobrać Nimfek/Modlight (serwer) i Nimfek/Modlight-App (klient Windows), sprawdzić luki,
  błędy logiczne i czy kod nie wykrada danych.
- Przepisać **oba** na Rust z zachowaniem działania 1:1; aplikacja ma działać na Macu
  i mieć **wszystkie** narzędzia ShareX, robione etapami.
- Stawiać tylko lokalnie w Dockerze (127.0.0.1), nic nie wystawiać w świat.
- Na koniec wrzucić na prywatne repo właściciela na GitHubie (konto SurMistrz666).

## Zrobione

1. **Audyt** (3 agentów, analiza statyczna): raporty w `docs/audyt/`.
   Wynik: nic nie wykrada danych, brak backdoorów i ukrytych kont. Poprawki do zrobienia
   w wersji Rust: `server/docs/PORT.md` sekcja 12 i `app/docs/PORT.md` sekcja 5.
2. **Docker**: Colima z Homebrew, profil `modlight` (8 CPU, 16 GB, 120 GB dysku).
3. **Wzorzec**: oryginalny serwer TypeScript w `_wzorzec/` na http://127.0.0.1:8420
   (Postgres 127.0.0.1:55420). Sekrety w `_wzorzec/.env`, konta testowe w
   `_wzorzec/qa-accounts.env` (nie wypisywać wartości).
4. **Repozytorium** `~/Desktop/modlight` (git, autor commitów: noreply SurMistrz666).
   Zasady: `server/docs/PORT.md`, `app/docs/PORT.md`.
5. **Szkielet serwera w Rust** scalony do `main` (588a5a1): konfiguracja, migrator zgodny
   z drizzle (schemat identyczny z oryginałem), modele wszystkich tabel, RPC dla akcji
   (`/_rpc`), dane stron (`/_page`) i powłoka SPA, odczyt sesji zgodny z Better Auth
   (sprawdzony na sesji z oryginału), tokeny API i zakresy, `/api/health`, 178 testów.
   Opis: `server/docs/SKELETON.md`.

## W toku (wstrzymane, każda gałąź ma swój `HANDOFF.md`)

| Gałąź | Worktree | Co robi |
|---|---|---|
| `web-spa` | scalona do `main` | panel WWW gotowy (`npm run verify` zielone), CONTRACT.md poprawiony w całości (508eff7). Prace backendu z sekcji 12 kontraktu idą w gałęziach serwera |
| `app-1` | scalona do `main` (dff2386) | aplikacja na Maca, etapy 1 i 2: logowanie, biblioteka, wygląd (zrzuty 42 ekranów x jasny/ciemny x EN/PL), skrypt `Modlight.app`, wysyłanie (kolejka, tus ze wznawianiem, offline, historia, wklejanie, skracacz, foldery obserwowane). Tylko bez okna; prawdziwe okno do sprawdzenia z właścicielem |
| `app-3` | scalona do `main` (a42c49f) | etap 3: wszystkie rodzaje zrzutów (nakładka na każdy ekran), skróty (Cmd+Shift+1/2/7/8/9), nagrywanie wideo i GIF (ScreenCaptureKit + ffmpeg), komponenty z SHA-256, pobieranie yt-dlp, logowanie YouTube, zakładka Serwer, wszystkie strony ustawień, start z systemem, usługa Findera; 470 testów. Lista do sprawdzenia w prawdziwym oknie: `docs/handoff/app-3.md` |
| `srv-auth` | scalona do `main` (bbde0a3) | konta, logowanie zgodne z Better Auth, 2FA, admin, poczta, wydania, strony auth i układ (app); skrypty porównawcze z oryginałem zielone, zgodność kont/sesji/2FA w obie strony 227/227; poprawki 3, 4, 7 wdrożone. Szczegóły i pozostałe różnice: `docs/handoff/srv-auth.md` |
| `srv-library` | scalona do `main` (1be08bc) | wysyłanie (ShareX z poprawką 2, tus zgodny z Uppy i aplikacją), magazyn lokalny i S3, foldery, kosz, duplikaty, skróty, awatary i tapety, wiązanie libvips, strony biblioteki; 975 testów, porównania z oryginałem zielone. Otwarte pozycje dla mediów: `docs/handoff/srv-library.md` |
| `srv-media-a` | scalona do `main` (78eee7c) | konwersje, podglądy ruchome i osi czasu, okładki, docx (mammoth), karty OG, pdf; 1212 testów; przy ffmpeg 5.1 wyjścia bajt w bajt jak w oryginale, karty co do piksela |
| `srv-integration` | scalona do `main` (0933c6d) | serwer GOTOWY: obraz Dockera (libvips 8.17.3 jak w sharp, ffmpeg 5.1.9), stos `modlight-rust` na 127.0.0.1:8430 (działa, pusta baza: pierwsze konto = admin), wszystkie porównania z oryginałem zielone, e2e 12/15 jak na oryginale, wszystkie 7 poprawek z testami, `server/README.md` po polsku, `server/docs/COVERAGE.md` |
| `app-5-effects` | `_wt/app-5-effects` | W TOKU: biblioteka efektów obrazu (ImageEditor/Core/ImageEffects + ImageEffectsLib) na bazie edytora |
| przegląd bezpieczeństwa serwera | raport `docs/audyt/rust-serwer.md` | W TOKU |
| `app-4` | `_wt/app-4` | W TOKU: edytor obrazów (rdzeń, adnotacje, historia, interfejs, rejestr efektów) |
| `srv-media-b` | scalona do `main` (e549b53) | serwowanie plików, strony publiczne, osadzenia, oEmbed, albumy z zip, usuwanie tokenem; poprawki 1 (limit wyświetleń: przepustka `ml_view_<id>`), 5, 6; 1080 testów, 1064 porównań zgodnych. Czeka na podpięcie części A w `media/part_a.rs`. Znaleziony błąd oryginału: zip albumu zawsze 500 |

Gotowe polecenia dla kolejnych agentów: `_scratch/prompts/` (`common-server.md`,
`srv-library.md`, `srv-media.md`).

## Dalsza kolejność

1. Przejrzeć `HANDOFF.md` trzech gałęzi, dokończyć je (wznowić agentów z notatką),
   sprawdzić i scalić do `main`.
2. Serwer: `srv-library`, potem media (część A: przetwarzanie, część B: serwowanie,
   strony publiczne, osadzenia), potem integracja: obraz Dockera na 127.0.0.1:8430,
   testy Playwright z oryginału, porównania odpowiedzi z wzorcem, poprawki z audytu.
3. Aplikacja na Maca: etap 2 (wysyłanie, kolejka offline, wznawianie), etap 3 (zrzuty,
   nagrywanie, pobieranie yt-dlp, komponenty, ustawienia), etap 4 (edytor obrazów),
   etap 5 (efekty i pozostałe narzędzia ShareX).
4. Przegląd bezpieczeństwa kodu Rust, potem prywatne repo na GitHubie.

## Decyzje do potwierdzenia u właściciela

- Przekierowanie `/dashboard` i strony nie znalezione: oryginał odpowiada 200, szkielet 307/404.
- Zły znak `%` w adresie: oryginał 500, szkielet 400.
- Ramka „501: nie przeniesione” w panelu widoczna w każdym buildzie, do czasu ukończenia portu.
- Nazwa prywatnego repo na GitHubie (jedno repo `modlight` czy dwa).

## Przekazanie innej sesji

Pełna notatka dla kolejnej sesji Claude: `~/Desktop/claude/stan_agentow/modlight-rust.md`
(poza repo, z odnośnikami do kont testowych i sesji). Agentów z sesji 25670274 nie da się wznowić
z innej sesji: nowa sesja uruchamia świeżych agentów z `HANDOFF.md` danej gałęzi i poleceniami
z `_scratch/prompts/`.

## Wznowienie

```bash
colima start --profile modlight
docker compose -f ~/Desktop/modlight/_wzorzec/docker-compose.yml up -d
~/Desktop/modlight/server/scripts/dev.sh db    # podnosi dev Postgres na 127.0.0.1:55421
```

Porty (tylko 127.0.0.1): wzorzec 8420, Rust dev 8421 (agenci także 8423 i dalej),
dev Postgres 55421, docelowy Docker z wersją Rust 8430.

Zasady pracy agentów: najwyżej 3 naraz, każdy w swoim worktree, nic nie uruchamiać
z `upstream/`, aplikacji nie otwierać na ekranie właściciela (tylko tryb QA).
