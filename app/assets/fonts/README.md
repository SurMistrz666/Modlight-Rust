# Interface font

Inter 4.001 (SIL Open Font License 1.1, see `OFL.txt`). Upstream Modlight App asks for
`Geist, Segoe UI Variable Text, Segoe UI, Inter`; none of the first three ships with
macOS, so the font Avalonia falls back to on a Mac, Inter (bundled there through
Avalonia.Fonts.Inter), is bundled here.

Static instances (400, 500, 600) of the Latin and Latin Extended subsets of
`@fontsource-variable/inter` 5.3.0. The Latin Extended subset carries the Polish
letters; both subsets of a weight are loaded into one egui font family, Latin first.
