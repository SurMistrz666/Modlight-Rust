# Handoff: web panel as an SPA (branch `web-spa`)

Stopped on 2026-10-18 at the owner's request. Everything is committed on
`web-spa` (never pushed). The code builds and its tests pass; only the
documentation has unfinished parts (listed below).

## What is done

- `server/web/`: the upstream Next.js 16 panel as a client-side SPA (Vite 8,
  React 19, TypeScript strict, Tailwind 4 with upstream `app/globals.css`, `@/`
  alias). Upstream components, hooks, types and the client-side `lib/` modules
  are verbatim copies (`scripts/sync-upstream.mjs`, hashes in
  `upstream.lock.json`, `npm run sync:check`). 13 Server Components are ported
  under `src/overrides/` (listed in CONTRACT.md section 11).
- Shims (`src/shims/`): `next/link`, `next/navigation`, `next/dynamic`,
  `next-intl` (on `use-intl` 4.13.0), `client-only`, Geist fonts. Any other
  `next/*` or `server-only` import fails the build.
- RPC stubs for all 94 server actions (`scripts/gen-actions.mjs` ->
  `src/rpc/actions/*.ts`, `src/rpc/actions.manifest.json`), type mirrors of
  server-only modules (`scripts/gen-type-mirrors.mjs`).
- react-router 7 data router mirroring `app/`: 27 pages, 3 layouts
  (root, `(app)`, `(auth)`), `error.tsx` boundaries, dashboard `loading.tsx`,
  `not-found.tsx`. First render reads the inlined `__MODLIGHT_PAGE__` payload;
  navigations and refreshes fetch `/_page`.
- Aligned with the merged Rust skeleton (main 588a5a1, merged here as
  9a7ce62): `/_page?path=<encodeURIComponent(pathname)>&<query>`, envelopes
  `reload` / 501 `notImplemented` / 500 `error` / `notFound` with
  `layouts.root` + `meta`, translations from `layouts.root.messages` with
  `X-Modlight-Messages`, the document from `server/src/pages/shell.rs`,
  route ids as in `table.rs`. Tested live against the skeleton on
  127.0.0.1:8421 (document loads, client navigation through a redirect, back
  to a not-found page, the messages header, RPC refusals).
- Next 16's fallbacks: a URL no route matches, a route handler, and any
  `/_page` answer that is not a page payload are loaded as documents.
- Fixtures for every page (`fixtures/pages/*.json`, 41 files) and a fixture
  mode (`npm run dev:fixtures`).
- Tests: 61 (42 smoke = every fixture mounts, 16 runtime = wire formats and
  navigation, 3 route-table = the SPA's table against
  `server/src/pages/table.rs` and the server's own matching tests).
- `server/docs/CONTRACT.md`: sections 1 to 9 are current. Sections 5 and 6
  (common page rules, the 3 layouts, the 27 pages) are merged from the
  page-data work order a subagent wrote and verified against the reference.
  Section 7 covers the 94 actions.

## In progress: exact state

1. **CONTRACT.md sections 10, 11 and 12 are partly stale.**
   - Section 10: "Translations. Bundled ..." is wrong now (they come from
     `layouts.root.messages`); "Head and `<html>` ... replaces the
     `data-ml-head` elements" is wrong (the SPA sets the title and the
     description / application-name / robots metas, and removes the Open
     Graph, Twitter and alternate tags after the first navigation:
     `src/routes/root.tsx` `useHead`). The navigation bullet should name
     `SERVER_PATHS` in `src/contract/routes.ts` and the document-load
     fallbacks of section 4.2.
   - Section 11: drop the "Translations are bundled" bullet and change the
     intro ("Components and translations are byte-for-byte upstream": the
     translations are now `server/messages`, sent by the server); add the 501
     box (skeleton only, section 4.2).
   - Section 12 must be renumbered to this plan, which sections 5 and 6
     already reference (12.5 to 12.10): 1 `layouts.root.config` (the skeleton
     sends none; the SPA falls back to origin-derived values,
     `src/overrides/lib/config.ts`); 2 the page's own skin and wallpaper in
     the document (section 2.2); 3 `revalidate` on cookie writes (section
     7.1; the skeleton only sets it for revalidatePath/Tag); 4 `/dashboard`
     page redirects and notFound answer 200 upstream (section 2.1; the
     skeleton sends 307/404); 5 malformed percent-escapes (reference 500
     text/plain, skeleton 400 JSON; section 5.3); 6 to 15 the page-data
     questions Q12, Q2, Q1, Q3, Q4, Q5, Q6, Q7, Q8, Q9 in that order; 16 to 23
     the current items 3 to 10. Current items 1 (translations) and 2
     (shortener passthrough) are resolved by the skeleton: delete them. The
     Q texts are in `_scratch/web-spa/contract-pages.md` section 3; when
     copying, renumber their section references (1.N -> 5.(N+1), 2.N -> 6.N).
2. **Fixture `meta` blocks** do not all match the per-route metadata of
   CONTRACT.md section 6 (the page-data agent's Q10). To fix:
   `dashboard-links` (`Short links - Modlight`), `dashboard-paste` and
   `dashboard-upload` (their own descriptions, 6.16 and 6.17),
   `dashboard-admin-user-files*` (`User files - Modlight`), `user-gallery`
   (6.22), `album` (6.23), `shortlink-unlock*` (robots `noindex, nofollow`,
   6.24), `media*` and `tagless-media` (6.27: absolute title, `description`
   and `applicationName` null). Fixtures only feed fixture mode and the smoke
   test; nothing breaks meanwhile.

## What remains (besides the above)

- Backend work requested in CONTRACT.md section 12 (config in the root
  layout, owner skin/wallpaper in the shell, revalidate on cookie writes).
- After the doc fixes: `npm run verify`, and `npm run size` if the bundle
  changed (section 1.3 has the current numbers).

## Build and verify

```sh
export PATH=/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH
cd server/web
npx -y pnpm@10.33.0 install     # lock seeded from upstream's pnpm-lock.yaml
npm run verify                  # typecheck, 61 tests, build, bundle audit, sync:check
npm run dev:fixtures            # 127.0.0.1:5173, no backend
npm run build && ../scripts/dev.sh   # Rust server on 127.0.0.1:8421 serving web/dist
npm run dev                     # 127.0.0.1:5173, proxies to :8421
npm run size                    # first-load sizes vs the reference on :8420
```

## Gotchas

- Upstream (`upstream/Modlight`) is read-only: re-sync with
  `npm run sync:upstream`, `gen:actions`, `gen:mirrors`, then
  `node scripts/sync-upstream.mjs --accept` once the ports are updated.
- The Rust server always serves the production bundle (`web/dist`), so the
  501 box shows in every build; rebuild after SPA changes (the shell rereads
  `index.html` when its mtime changes).
- `vite.config.ts` `server.fs.allow` includes `../messages` (fixture
  dictionaries) and `../src/pages` (the route-table test reads `table.rs`
  with `?raw`, which vite only allows by directory).
- `tests/route-table.test.ts` parses `table.rs` with regexes; a format change
  there needs the regexes adjusted.
- A jsdom "Not implemented: navigation to another Document" line in test
  output means a test caused a document load: give the test a page source
  before it creates a router.
- `npm run size` measures dashboard rows only with `MODLIGHT_REF_COOKIE`
  set to a signed-in session of the reference; never print that value.
- The smoke test allow-lists one upstream console error
  (`OneClickShareX` passes a function as a message argument; CONTRACT.md 12).

## Outside the repository

- `_scratch/web-spa/`: drafts and reference material, not needed to build
  (`contract-actions*.md`, `contract-pages.md` = the merged sources of
  CONTRACT.md 5 to 7; `route-table.md` uses the old group-prefixed ids;
  `next/`, `next-intl/`, `geist/` = unpacked npm tarballs read for reference;
  `ref/` = reference HTML/CSS dumps; `stub/` = stub servers; logs).
- Reference server (127.0.0.1:8420): I signed in with the existing QA admin
  account recorded in `_wzorzec/qa-accounts.env` (`QA_ADMIN_EMAIL` /
  `QA_ADMIN_PASSWORD`, created by the app-1 agent). I created no account and
  no token and appended nothing there. My sessions are signed out and the
  cookie jar is deleted.
- No containers. The Rust dev server, vite servers and stub servers I started
  are stopped; the reference stack (8420) and the shared dev Postgres (55421)
  were left running.

## Commits on `web-spa` (newest last)

- 9a7ce62 merge of main (Rust skeleton)
- 48287b8 web: speak the Rust skeleton's /_page protocol
- fa4b3f2 web: fall back to document loads where Next does
- the commit adding this file: CONTRACT.md sections 1, 4, 5, 6, 7.1 updated
  (WIP: sections 10 to 12 as described above)
