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
| `app-1` | `_wt/app-1` | aplikacja na Maca, etap 1 prawie gotowy (3e022b7, testy zielone, test bez okna przeciw wzorcowi). Brakuje: testów zrzutów dla wszystkich ekranów (jasne skórki, PL), skryptu budującego Modlight.app, `app/docs/MACOS.md` i `STATUS.md`. Nic nie było uruchomione w prawdziwym oknie |
| `srv-auth` | scalona do `main` (bbde0a3) | konta, logowanie zgodne z Better Auth, 2FA, admin, poczta, wydania, strony auth i układ (app); skrypty porównawcze z oryginałem zielone, zgodność kont/sesji/2FA w obie strony 227/227; poprawki 3, 4, 7 wdrożone. Szczegóły i pozostałe różnice: `docs/handoff/srv-auth.md` |
| `srv-library` | `_wt/srv-library` | utworzona, NIE rozpoczęta: wysyłanie plików, tus, ShareX, magazyn, foldery, kosz, duplikaty, skróty |

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
