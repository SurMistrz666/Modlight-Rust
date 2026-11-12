# Bug hunt, 2026-11-11 and 2026-11-12

The whole code base (server, web panel, desktop app) was split into ten areas. Each area
was read file by file by its own reviewer; every bug was confirmed before it was fixed,
and every fix came with a test that failed before the fix wherever a test was possible.
A final pass then fixed what crossed area boundaries. Each area's report lists the files
read, the findings with `file:line`, what was left open and why, and the checks run.

| Area | Report | Fixed | Highlights |
|---|---|---|---|
| Server: accounts, sign-in, permissions | [srv-auth.md](srv-auth.md) | 9 | racing first sign-ups on a closed install; rate limiter and throttle memory growth; 1 MiB body limit on `/api/auth/*`; bans through `admin/update-user` end sessions; releases refuse files apps cannot fetch |
| Server: uploads, storage, vaults | [srv-files.md](srv-files.md) | 17 | encrypted file lost when encryption is switched off mid-upload; stack overflow on deep folder chains; tus path traversal and free-space checks; double finalize; trash sweep and restore race; quota double refunds |
| Server: media processing and serving | [srv-media.md](srv-media.md) | 3 | HEIC/AVIF box walk that looped forever or exhausted memory; quota refunded twice on concurrent deletes |
| Web panel | [web.md](web.md) | 11 | open redirect after sign-in; admin dialogs saving to the wrong account; forms frozen after a failed request; Polish plurals |
| App: capture, recording, hotkeys | [app-capture.md](app-capture.md) | 11 | "Perform actions" deleting its own output on case-insensitive volumes; quick task menu closing at once; hotkey repeat limit; options of a second hotkey saved into the first |
| App: sign-in, sync, uploads, library | [app-net.md](app-net.md) | 11 | downloads and direct uploads hanging for ever on a dead connection; Upload from URL destroying the target first; queued confirmations |
| App: main window, settings, config | [app-ui.md](app-ui.md) | 8 | crash on notices with pictures over the texture limit; settings files with a byte order mark; windows re-placed every frame; egui's global zoom and quit shortcuts |
| App: image editor | [app-editor.md](app-editor.md) | 12 | Rotate 90 moved annotations the wrong way (a pixelation could end up beside what it hid); paste into text fields also pasting on the canvas; huge shadow layers |
| App: image effects | [app-effects.md](app-effects.md) | 4 | blurred shapes blurring the whole canvas each (28 s to 0.1 s); "No overlap" particles wiping the picture; text watermark panic |
| App: tools | [app-tools.md](app-tools.md) | 8 | EXIF rotation ignoring its setting; error window for panics already caught; history windows |
| App: cross-area final pass | [final.md](final.md) | 9 | large-picture crashes in four dialogs and the scrolling result; one size limit for effect results; backup import racing a save |

About 103 fixes in all. No database migrations. Items left open on purpose (decisions for
the owner, behaviour kept identical to the originals, or changes too large for this pass)
are listed in each report under "Not fixed".
