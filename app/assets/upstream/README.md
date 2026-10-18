# Files taken from upstream Modlight App

Copied unchanged from `upstream/Modlight-App` (GPL-3.0, like this port). They are the
specification the Rust build reads at compile time; nothing here is edited by hand.

| Here | Upstream |
|---|---|
| `l10n/<Project>.resx`, `l10n/<Project>.pl.resx` | `<Project>/Localization/Strings.resx`, `Strings.pl.resx` |
| `theming/Skins.axaml`, `theming/SkinVariants.g.cs`, `theming/ShareXTheme.axaml` | `ShareX.Avalonia/Theming/` |
| `icons/lucide.ttf`, `icons/lucide-codepoints.json`, `icons/Lucide_license.txt` | `ShareX.Avalonia/Assets/`, `Licenses/` |
| `icons/Modlight_Icon.ico`, `icons/logo.svg` | `ShareX/Modlight_Icon.ico`, `docs/logo.svg` |

To refresh them, copy the same files again and rebuild; `modlight-core/build.rs`
regenerates the string tables, skins and icon constants.
