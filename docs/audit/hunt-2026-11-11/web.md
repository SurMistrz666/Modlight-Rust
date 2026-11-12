# Bug hunt 2026-11-11: web panel (`server/web`) and its texts

Area: the React + TypeScript web panel in `server/web/src/**`, its tests, the PL/EN
dictionaries in `server/messages/{en,pl}/*.json`, and `server/docs/CONTRACT.md` where the
panel depends on it. Base: `dffba1a` plus the staged base patch of this worktree; the fixes
are the unstaged diff.

The verbatim upstream copies (`server/web/{app,components,hooks,lib,types}`) are checked by
`npm run sync:check` and must stay byte-identical, so they were read where `src/` uses them
but not edited. Bugs found there are under "Not fixed".

## 1. Files read

- [x] `server/web/src/`: env.d.ts, main.tsx, node-globals.d.ts
- [x] `server/web/src/assets/`: modlight-icon.png (binary, imported only)
- [x] `server/web/src/contract/`: db.ts, layouts.ts, routes.ts
- [x] `server/web/src/downloads/`: DownloadsScreen.tsx, api.ts
- [x] `server/web/src/encryption/`: EncryptionSettings.tsx, VaultBanner.tsx, VaultGate.tsx, actions.ts
- [x] `server/web/src/fixtures/`: install.ts, source.ts
- [x] `server/web/src/fonts/`: geist.css, LICENSE.txt (the two .woff2 files are binary copies)
- [x] `server/web/src/overrides/`: components/admin/ServerDisk.tsx, components/admin/UserManager.tsx, components/app-shell.tsx, components/auth/brand-mark.tsx, components/auth/forgot-form.tsx, components/auth/invite-form.tsx, components/auth/login-form.tsx, components/auth/register-form.tsx, components/authorize/authorize-prompt.tsx, components/brand/ModlightMark.tsx, components/dashboard/EmptyState.tsx, components/duplicates/DuplicateReminderSlot.tsx, components/landing/cta.tsx, components/landing/features.tsx, components/landing/hero.tsx, components/landing/large-files.tsx, components/landing/sharex.tsx, components/landing/site-footer.tsx, components/landing/site-nav.tsx, components/profile/QuotaCard.tsx, components/tokens/AppCard.tsx, components/viewer/MediaViewer.tsx, lib/api-token.ts, lib/auth.ts, lib/avatar-store.ts, lib/config.ts, lib/data/admin-disk.ts, lib/data/admin-files.ts, lib/data/admin.ts, lib/data/duplicates.ts, lib/data/folders.ts, lib/data/sessions.ts, lib/data/uploads.ts, lib/data/user-prefs.ts, lib/folders.ts, lib/mail-settings.ts, lib/maintenance/trash.ts, lib/media/avatar.ts, lib/media/render-ticket.ts, lib/naming.ts, lib/password.ts, lib/registration.ts, lib/releases-core.ts, lib/releases.ts, lib/shortlinks.ts, lib/themes.ts, lib/upload/disk.ts (each port also diffed against its upstream file)
- [x] `server/web/src/pages/`: (new: _lib/safe-next.ts) (app)/dashboard/admin/page.tsx, (app)/dashboard/admin/users/[id]/files/page.tsx, (app)/dashboard/downloads/page.tsx, (app)/dashboard/duplicates/page.tsx, (app)/dashboard/folders/page.tsx, (app)/dashboard/links/page.tsx, (app)/dashboard/loading.tsx, (app)/dashboard/page.tsx, (app)/dashboard/paste/page.tsx, (app)/dashboard/profile/page.tsx, (app)/dashboard/tokens/page.tsx, (app)/dashboard/trash/page.tsx, (app)/dashboard/upload/page.tsx, (app)/layout.tsx, (auth)/2fa/page.tsx, (auth)/authorize/page.tsx, (auth)/forgot/page.tsx, (auth)/invite/[token]/page.tsx, (auth)/layout.tsx, (auth)/login/page.tsx, (auth)/register/page.tsx, (auth)/reset/page.tsx, [userCode]/[mediaCode]/page.tsx, [userCode]/page.tsx, _lib/search-params.ts, a/[userCode]/[slug]/page.tsx, download/page.tsx, layout.tsx, not-found.tsx, page.tsx, s/[code]/[name]/unlock/page.tsx, s/[code]/unlock/page.tsx, u/[userCode]/page.tsx, welcome/page.tsx (JSX diffed against upstream)
- [x] `server/web/src/routes/`: boundary.tsx, loader.ts, modules.ts, not-implemented.tsx, root.tsx, segments.tsx, tree.tsx
- [x] `server/web/src/rpc/`: actions.manifest.json, actions/*.ts (18 generated stubs; every export checked to call its own module and name with `arguments`), authorize-pickup.ts
- [x] `server/web/src/runtime/`: errors.ts, global-handlers.ts, messages.ts, navigation.ts, payload.ts, root-data.ts, rpc.ts, shell.ts, tagged-json.ts
- [x] `server/web/src/shims/`: empty.ts, geist/font.ts, next-intl.tsx, next/dynamic.tsx, next/link.tsx, next/navigation.ts
- [x] `server/web/src/strip-metadata/`: StripMetadataSettings.tsx, actions.ts
- [x] `server/web/src/styles/`: graphite.css
- [x] `server/web/src/turnstile/`: TurnstileSettings.tsx, actions.ts, widget.tsx
- [x] `server/web/tests/`: setup.ts, authorize-pickup, downloads, encryption, route-table, runtime, smoke, strip-metadata, turnstile
- [x] `server/web/{index.html, vite.config.ts, tsconfig.json, package.json}`, `scripts/sync-upstream.mjs` (head), `scripts/gen-type-mirrors.mjs` (head)
- [x] `server/messages/{en,pl}/*.json`: all 23 namespaces. Checked by script: same 2439 keys in both
  languages; every message parses as ICU; the same arguments in EN and PL; every Polish
  plural has one/few/many (four upstream messages do not, see "Not fixed"); every literal
  key the panel's code calls exists. The non-upstream namespaces (authorizePickup,
  downloads, encryption, stripMetadata, turnstile, the nav/profile additions) read line by line.
- [x] `server/docs/PORT.md`; `server/docs/CONTRACT.md` sections on the SPA layout, page data and RPC
- [x] Earlier audits in `docs/audit/` (none covered the panel's own code except the
  authorize pickup, `rust-delta-2026-10-24.md` P1/P2, whose web side still holds:
  `tests/authorize-pickup.test.tsx` green).

## 2. Findings

| Id | Severity | Where | What goes wrong | Fix | Test |
|---|---|---|---|---|---|
| WEB-01 | Medium | `src/pages/(auth)/login/page.tsx:22`, `src/pages/(auth)/2fa/page.tsx:23` (`safeNext`, now `src/pages/_lib/safe-next.ts:18`) | Open redirect after sign-in. `safeNext` (upstream's) accepts any `?next=` that starts with one `/` and not `//`, but the URL parser reads `/\evil.example` as `//evil.example` (backslash is a slash in http URLs) and drops tabs and line breaks anywhere (`/%09/evil.example`). The forms pass `next` to `router.push`, which loads another origin as a document: a link to `/login?next=/%5Cevil.example/phish` signs the victim in on the real site and then lands them on the attacker's page. Same upstream. | `src/pages/_lib/safe-next.ts`: one `safeNext` for both pages that also refuses backslashes and control characters (as `server/src/vault/gate.rs` `safe_next` does) and anything that still resolves to another origin. | `tests/auth-next.test.tsx` (unit cases for the bypasses; a sign-in on `/login?next=%2F%5Cevil.example%2Fphish` must land on `/dashboard`: before the fix jsdom reported "navigation to another Document" and the router stayed on `/login`) |
| WEB-02 | Medium | `src/overrides/components/admin/UserManager.tsx:488` (`openEdit`), `:601` (`openRemove`) | Admin -> Users: the edit and delete dialogs read the account from the server when they open and apply whatever answer comes back. Open "Edit account" for Anna, close it while it loads, open it for Bob: when Anna's read answers after Bob's, Bob's dialog (titled with Bob's address) is filled with Anna's fields, and Save sends the change to Anna's id (`detail.id`). The delete dialog the same way shows Anna's file count and size while asking to delete Bob. Same upstream. | A request counter per dialog (`editRequest`, `removeRequest`): an answer for an earlier opening is dropped; closing the edit dialog invalidates its read. | `tests/user-manager.test.tsx` (answers in reverse order: the dialog shows Bob and saves to `u-bob`; the delete dialog shows Bob's numbers, not Anna's 3,000 files; both failed before the fix) |
| WEB-03 | Low | `src/overrides/components/auth/login-form.tsx:73`, `invite-form.tsx:78`, `register-form.tsx:95`, `forgot-form.tsx:57,84` | A sign-in, invitation, sign-up or reset request that fails outright (no connection, a proxy's error page while the server restarts, an error envelope) rejected inside `onSubmit` with nobody catching it: the form stayed on "Signing in..." with every field disabled until a reload, and nothing said why. The reset form also answered a 429 from the rate limit (3 a minute) with "If an account exists ... you'll receive a link", although nothing was sent. Same upstream. | Each call in try/catch: the failure shows `common.error` ("Something went wrong. Please try again.") and the form can be sent again; a 429 on the reset form shows `auth.login.tooManyAttempts` (the limiter refuses before the address is read, so it tells nothing about which accounts exist; a 5xx stays neutral as upstream). | `tests/auth-forms.test.tsx` (the four forms with a failing transport/fetch: before the fix four unhandled rejections and disabled buttons; the 429 case checked to fail without its branch) |
| WEB-04 | Low | `server/messages/{en,pl}/encryption.json` `settings.expires`, used by `src/encryption/EncryptionSettings.tsx:401` | The list of browsers that remember the file password put the grant's expiry, a relative time in the future ("in 29 days" / "za 29 dni"), after "until" / "do": every row read "until in 29 days" and "do za 29 dni". | `settings.expires` is now "expires {date}" / "wygasa {date}" ("expires in 29 days", "wygasa za 29 dni"). | `tests/encryption.test.tsx` "says when a grant expires in a sentence that reads" (EN and PL; both failed before) |
| WEB-05 | Low | `server/messages/pl/files.json` `menu.moveManyToFolder`, `menu.trashMany`; `pl/upload.json` `toast.many`, `results.copiedAll`, `encrypted.copiedAll`, `dock.liveDone`, `duplicates.bodyPartial`; `pl/profile.json` `duplicates.unchecksummed` | Polish counts printed with one fixed form: "Przenieś 2 plików do kosza", "Wysłano 4 plików", "Skopiowano 3 linków", "Zakończono wysyłanie 1 plików", "Wysyłanie zakończone, 2 nieudanych", "2 z 5 wybranych plików ma", and "3 pliki/plików nie ma" (a slash instead of a choice). Polish needs one/few/many (2-4, 22-24 vs 5-21). Same upstream (its translations). | ICU plurals with one/few/many/other; `admin.maintenance.scanOrphans` and `upload.duplicates.uploadAnyway` (text the same in every form) get the missing categories so the rule below holds everywhere. English unchanged. | `tests/messages.test.ts`: EN and PL have the same keys; every Polish plural has one/few/many/other; the messages above formatted with 1, 2-4, 5+, 12, 22 (11 of 19 cases failed before) |
| WEB-06 | Low | `src/overrides/components/app-shell.tsx:870` (`accountMenu`), `:798` (`Brand`), `:1040` | On a phone the port moved the account card (Profile, Admin, Sign out) and the app name into the drawer, without the `onNavigate` its navigation rows use to close it. Tapping Profile or Admin in the drawer's account menu, or the app name at its foot, navigated behind the modal drawer, which stayed open over the new page until closed by hand. (Upstream's account menu sat in the top bar, outside the drawer.) | `accountCard`/`accountMenu` and `Brand` take `onNavigate`; the drawer passes `() => setMobileOpen(false)`, the desktop rail nothing. | `tests/app-shell.test.tsx` (drawer open, Profile / Admin from the account menu and the app name: the page changes and no dialog is left; all three failed before) |

## 3. Not fixed

| Id | Severity | Where | What | Why not, and the recommendation |
|---|---|---|---|---|
| WEB-N1 (fixed in round 2 as WEB-07) | Low | `components/auth/two-factor-form.tsx:30`, `components/auth/reset-form.tsx` (verbatim upstream copies) | The same missing catch as WEB-03: a 2FA check or a new password whose request fails outright leaves the form disabled until a reload. | These two files are upstream copies that `npm run sync:check` keeps byte-identical. Fixing them means turning them into ports under `src/overrides/components/auth/` as the four other forms are; worth doing together if the owner wants WEB-03 everywhere. |
| WEB-N2 (fixed in round 2 as WEB-08) | Low | `src/turnstile/TurnstileSettings.tsx:83,100` (and upstream's `components/admin/MailSettings.tsx`, which it copies) | Save and "Use the environment settings" run inside `startTransition(async ...)` with no catch: when the request fails, React 19 hands the error to the nearest boundary, so the whole admin page turns into "Something went wrong / Try again" and the typed secret is lost, instead of a toast. | The card deliberately mirrors the SMTP card; changing one without the other makes them behave differently. Recommendation: catch in both (`toast.error(t("saveErrorTitle"))`) when MailSettings is ported. |
| WEB-N3 | Low | `src/encryption/EncryptionSettings.tsx:82` (`refresh`) | While the files are being encrypted the card polls every 2 s; a poll sent just before "Lock on this browser" that answers after the action's own reload shows "Unlocked" again for up to one poll interval. | Needs the poll to answer more slowly than a request sent after it; it corrects itself at the next poll. A sequence number in `refresh` would close it if it is ever seen. |
| WEB-N4 (fixed in round 2 as WEB-09) | Low (cosmetic) | `src/overrides/components/app-shell.tsx:843`, `UserManager.tsx:100`, `pages/(app)/dashboard/profile/page.tsx:180`, `pages/(app)/dashboard/admin/users/[id]/files/page.tsx:47` | Avatar initials are `name.slice(0, 2)`: a name that starts with an emoji or another character outside the BMP is cut in the middle of a surrogate pair and the circle shows a replacement character. No crash. Same upstream (and in vendored components). | Cosmetic; a fix belongs in one shared helper (`Array.from(name).slice(0, 2)`), which upstream does not have. |
| WEB-N5 (fixed in round 2 as WEB-10) | Low | `src/downloads/DownloadsScreen.tsx:506` (the `publish.sh` example shown to administrators) | The example script builds its JSON body with `"note":"${6:-}"`: a change note with `"`, `\` or a line break makes the start request invalid JSON (400) and the script stops. | Only the copy-paste example; the API itself is fine. Recommendation: escape the note in the script (or tell people to keep notes to one plain line). |
| WEB-N6 | Info | `src/runtime/tagged-json.ts:71` (`decode`) | A JSON object key `__proto__` is assigned with `out[k] = ...`, which sets the decoded object's prototype instead of adding a key. Only server answers are decoded, and no payload carries user-chosen object keys today, so nothing is reachable. | Hardening only: `Object.defineProperty` (or `Object.create(null)`) if a payload ever gets user-chosen keys. |
| WEB-N7 | Info | `server/messages/*/` upstream strings | Messages that interpolate a number without a plural are right for every value they can take today (constants: 30 days, 180 days, 8 characters, 5 builds kept, 10-second seek), but would read wrong in Polish for 2-4 if those constants changed. | Nothing to do now; `tests/messages.test.ts` guards every plural that exists. |
| WEB-N8 (fixed in round 2 as WEB-11) | Low | `src/routes/root.tsx:145` (`RouterError`) | When the router's loader itself throws (a page chunk that cannot be loaded), the SPA reloads the URL once; if it fails again it logs to the console and renders nothing, so the visitor sees an empty page with no message or retry. | Needs a chunk that keeps failing while the document loads (a broken deploy); the reload covers the usual case. Recommendation: render the same plain "This page could not be loaded / Try again" box as `NoRootLayout` on the second failure. |

## Round 2 (2026-11-12)

The coordinator asked for N1, N2, N4, N5 and N8 to be fixed; N3, N6 and N7 stay as they
are. Same rules; round 1 is in the index, round 2 is the unstaged diff. Every fix has a
test that failed before it.

| Id | Was | Severity | Where | Fix | Test |
|---|---|---|---|---|---|
| WEB-07 | N1 | Low | `src/overrides/components/auth/two-factor-form.tsx:46`, `src/overrides/components/auth/reset-form.tsx:56` (new ports) | The 2FA and new-password forms were verbatim upstream copies with the same missing catch as WEB-03 (a request that fails outright left the form disabled). Both are now ports under `src/overrides/` (header, `@upstream` tag; `node scripts/sync-upstream.mjs` removed the copies and records them as ports in `upstream.lock.json`), upstream's code line for line plus a catch marked "No answer" that shows `common.error` and lets the form be sent again. | `tests/auth-forms.test.tsx` "a two-factor check ..." and "a new password ..." (failed before: unhandled rejection, disabled button) |
| WEB-08 | N2 | Low | `src/turnstile/TurnstileSettings.tsx:91-130`, `src/overrides/components/admin/MailSettings.tsx:129-208` (new port) | The saves ran in `startTransition(async ...)` without a catch; a failed request was rethrown by React into the dashboard's error boundary, so the admin page became the error page and the typed secret or SMTP password was lost. Both cards now catch it: a toast plus an inline `role="alert"` line in the card (`saveErrorTitle` + `common.error`; for the address-confirmation switch, `common.error` in its own card), the fields keep what was typed, the buttons work again. Upstream's MailSettings became a port for this (same procedure as WEB-07). | `tests/admin-settings-cards.test.tsx` (Turnstile save, SMTP save, confirmation switch; all three failed before: the card was gone) |
| WEB-09 | N4 | Low | `src/avatar/initials.ts:10` (new), used by `app-shell.tsx`, `UserManager.tsx`, `pages/(app)/dashboard/profile/page.tsx`, `pages/(app)/dashboard/admin/users/[id]/files/page.tsx` | `initials(text)`: the first two grapheme clusters (`Intl.Segmenter`), code points where there is no Segmenter, upper-cased; replaces `slice(0, 2)` at the four places that drew initials. | `tests/initials.test.tsx` (unit cases: "B😀b", flags, skin tones, ZWJ families, the code-point fallback; the shell's account card with a name starting with a flag showed "🇵" before, "🇵🇱O" now) |
| WEB-10 | N5 | Low | `src/downloads/DownloadsScreen.tsx:505` (the `publish.sh` example) | The script now builds the start body with a `json()` shell function that escapes backslashes and quotes and joins lines with `\n`, for every string field (platform, arch, version, build, filename, note), and sends it with `--data "$BODY"`. | `tests/publish-script.test.tsx`: the script is taken from the rendered page and run by `/bin/sh` with a stand-in `curl` that records the body; a note with quotes, backslashes and blank lines parsed back unchanged (before: `JSON.parse` failed at the note) |
| WEB-11 | N8 | Low | `src/routes/root.tsx:149` (`RouterError`), `:181` (`LoadFailed`); `server/messages/{en,pl}/common.json` `loadFailed` | After the one automatic reload, a page whose code still fails to load now shows "This page could not be loaded", a line on what happened and "Try again" (reloads), instead of an empty page. The error element renders outside the root layout, so the words come from the dictionary the tab already holds (`common.loadFailed.{title,body,retry}`, new in EN and PL), and English when there is none. | `tests/router-error.test.tsx` (the reload is remembered; on the second failure the message and button appear, in Polish with a Polish dictionary held; both failed before: empty page) |

Notes on round 2:

- New ports (`two-factor-form`, `reset-form`, `MailSettings`) mean `server/web/components/`
  lost those three copies and `upstream.lock.json` tracks them under `ports`
  (`sync:check`: 158 copies verbatim, 101 ports current). If upstream changes them,
  `sync:check` will now say so, as for the other ports.
- `server/messages/{en,pl}/common.json` keep their CRLF line endings (the diff is only
  the five added lines each).
- `server/docs/CONTRACT.md` section 1.1 and 11 ("Bug hunt 2026-11-11", round 2) and the
  two Better Auth rows of section 8 name the new ports and fixes.

## 4. Outside my area

- Server, `server/src/routes/api/downloads.rs` / `server/src/downloads.rs` (`Refusal`): the
  refusals of `DELETE /api/downloads/{id}` carry an English `error` text, which the panel
  shows as the toast description under the translated title (`downloads.deleteFailed`) in
  the Polish UI too. Low; a stable `code` is already in the body, so the panel could map
  codes to `downloads.*` texts once the server's list of codes is fixed.
- No server bug was found that breaks the panel. The server parser accepts the changed
  Polish messages (`cargo test --lib i18n`: 14 passed, `every_shipped_message_parses`
  included).

## 5. Checks

Round 2 (final, on round 1 + round 2):

- `cd server/web && npm run verify`: typecheck clean; `vitest` **17 files, 160 tests
  passed**; `vite build` ok; bundle audit ok (174 chunks, 144 upstream modules, 119 SPA
  modules); `sync:check` ok (158 copies verbatim, 101 ports current).
- Round 2 tests: `tests/admin-settings-cards.test.tsx` (3), `tests/initials.test.tsx` (10),
  `tests/publish-script.test.tsx` (4), `tests/router-error.test.tsx` (3), two more in
  `tests/auth-forms.test.tsx`.
- Server: `cargo test --lib i18n` on the changed dictionaries (now with
  `common.loadFailed`): 14 passed. No Rust code changed.

Round 1 (as reported then):

- `cd server/web && npm run verify`: typecheck (app + node configs) clean; `vitest`
  13 files, **138 tests passed** (before the hunt: 8 files, 103 tests); `vite build` ok;
  bundle audit ok (173 chunks, 147 upstream modules, 115 SPA modules); `sync:check` ok
  (161 copies verbatim, 98 ports current).
- New test files: `tests/auth-next.test.tsx` (3), `tests/user-manager.test.tsx` (2),
  `tests/auth-forms.test.tsx` (5), `tests/messages.test.ts` (20), `tests/app-shell.test.tsx` (3);
  `tests/encryption.test.tsx` gained 2. The test of every fix was run against the code
  before the fix and failed there (WEB-03's 429 case by disabling just that branch; in
  `messages.test.ts` the cases of counts that were already right, and the key-parity
  check, passed before as well).
- Server: no Rust code changed. Because the dictionaries are the server's too,
  `cargo test --lib i18n` was run on the changed messages: 14 passed
  (`every_shipped_message_parses`, `locales_have_the_same_keys`, `polish_plurals` included).
  `cargo fmt`/`clippy` and the database tests were not needed.
- Scripts used for the dictionary review (not committed): every message parsed with
  `@formatjs/icu-messageformat-parser`, argument names compared EN/PL, Polish plural
  categories, and every literal `t("...")` key in `src/`, `components/`, `lib/`, `app/`
  resolved against `messages/en` (no key missing).
- No snapshots, no migrations.
