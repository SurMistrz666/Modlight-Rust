# Modlight (Rust)

Modlight przepisany na Rust: serwer (własny hosting plików i zrzutów ekranu) oraz aplikacja
na macOS. Oryginał: [Nimfek/Modlight](https://github.com/Nimfek/Modlight) (Next.js + PostgreSQL)
i [Nimfek/Modlight-App](https://github.com/Nimfek/Modlight-App) (klient Windows, fork ShareX).
Działanie 1:1 z oryginałem i ta sama baza danych. Świadome różnice to poprawki bezpieczeństwa
i błędów oryginału (niżej).

## Co tu jest

| Katalog | Zawartość |
|---|---|
| `server/` | serwer w Rust (axum + PostgreSQL), panel WWW (te same komponenty React, bez Next.js), obraz Dockera. Instrukcja: [`server/README.md`](server/README.md) |
| `app/` | aplikacja Modlight na macOS w Rust (egui): biblioteka, wysyłanie, zrzuty, nagrywanie, pobieranie wideo, edytor obrazów, 230 efektów i klasyczne efekty ShareX, narzędzia ShareX. Stan ekranów: [`app/docs/STATUS.md`](app/docs/STATUS.md), różnice względem Windows: [`app/docs/MACOS.md`](app/docs/MACOS.md) |
| `docs/audyt/` | audyty: oryginału (serwer, klient) i wersji Rust (serwer, aplikacja) |
| `docs/handoff/` | notatki z kolejnych etapów pracy |
| `docs/STAN.md` | stan prac i decyzje |

## Uruchomienie

Serwer (tylko na tym Macu, port 127.0.0.1:8430):

```sh
colima start --profile modlight
server/docker/setup-local.sh          # tylko za pierwszym razem: tworzy server/docker/.env
docker compose -f server/docker/docker-compose.yml up -d --build
```

Pierwsze konto założone na http://127.0.0.1:8430/register zostaje administratorem.

Aplikacja:

```sh
app/scripts/build-app.sh               # buduje app/dist/Modlight.app (nie uruchamia jej)
```

## Bezpieczeństwo

Audyt oryginału (`docs/audyt/`): nic nie wykradało danych, brak backdoorów. W wersji Rust
naprawione: obejście limitu wyświetleń („spal po obejrzeniu”), zapis na dysk przed sprawdzeniem
tokenu ShareX, dwóch adminów przy jednoczesnej rejestracji, limity zapytań oparte na
podrabialnym nagłówku, porównania tokenów w stałym czasie, oEmbed pliku z limitem, CORS logowania
aplikacji, pobieranie bez znanego rozmiaru, dostęp do ukończonych plików przez adres tus na S3,
niekasowane stare awatary, niedziałający zip albumu. Przegląd wersji Rust
(`docs/audyt/rust-serwer.md`, `docs/audyt/rust-aplikacja.md`): po poprawkach bez luk krytycznych
i wysokich, żadnego wysyłania danych poza serwer właściciela.

## Licencja

To przeróbka oryginałów, więc licencje zostają: serwer AGPL-3.0 (`server/LICENSE`, jak Modlight),
aplikacja GPL-3.0 (`app/LICENSE.txt`, jak ShareX i Modlight App; `app/Licenses/`). Udostępniając
serwer innym ludziom, trzeba im udostępnić jego kod.
