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
| `web-spa` | `_wt/web-spa` | panel WWW bez Next.js gotowy i sprawdzony na szkielecie (1145f1b, `npm run verify`: 61 testów, build, audyt paczki). Do dokończenia: sekcje 10-12 `server/docs/CONTRACT.md` (częściowo nieaktualne) i część danych testowych `meta`. Dla serwera: `layouts.root.config`, skórka właściciela strony na `<html>`, `revalidate` przy akcjach ustawiających ciasteczka, decyzje 200 kontra 307/404 i 400 kontra 500 (HANDOFF.md) |
| `app-1` | `_wt/app-1` | aplikacja na Maca, etap 1 prawie gotowy (3e022b7, testy zielone, test bez okna przeciw wzorcowi). Brakuje: testów zrzutów dla wszystkich ekranów (jasne skórki, PL), skryptu budującego Modlight.app, `app/docs/MACOS.md` i `STATUS.md`. Nic nie było uruchomione w prawdziwym oknie |
| `srv-auth` | `_wt/srv-auth` | konta i logowanie: dopiero fundament (7149927): kryptografia Better Auth (hasła scrypt i bcrypt, szyfrowanie 2FA, TOTP, linki weryfikacyjne), secret-box, ustawienia, rejestracja, poczta i szablony, 227 testów. Brakuje wszystkich endpointów `/api/auth/*`, tras, akcji, stron, testów zgodności i poprawek 3, 4, 7. Zapiski z badania zachowania oryginału w HANDOFF.md. Poza repo: pusta baza `modlight_srv_auth` na dev Postgres, zatrzymany stos `_scratch/srv-auth/ref` (porty 8424/55424) |
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
