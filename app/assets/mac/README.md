# Texts of the macOS port

`l10n/Mac.resx` and `l10n/Mac.pl.resx` hold the few texts where upstream's own text
names Windows (Windows' keyboard settings, the Snipping Tool, Explorer, the Startup
folder) and a Mac needs a different sentence. They are written for this port, in
upstream's style, and generated into `modlight_core::l10n::mac` by
`modlight-core/build.rs` like the upstream tables. Every key is listed in
`app/docs/MACOS.md`.
