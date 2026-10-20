# Files taken from upstream Modlight App

Copied unchanged from `upstream/Modlight-App` (GPL-3.0, like this port). They are the
specification the Rust build reads at compile time; nothing here is edited by hand.

| Here | Upstream |
|---|---|
| `l10n/<Project>.resx`, `l10n/<Project>.pl.resx` (ShareX, ShareX.Avalonia, ShareX.UploadersLib, ShareX.HelpersLib, ShareX.ScreenCaptureLib) | `<Project>/Localization/Strings.resx`, `Strings.pl.resx` |
| `l10n/ShareX.ImageEditor.resx`, `l10n/ShareX.ImageEditor.pl.resx` | `ShareX.ImageEditor/Localization/Strings.resx`, `Strings.pl.resx` |
| `editor/emoji-catalog.json` | `ShareX.ImageEditor/Assets/emoji-catalog.json` |
| `theming/Skins.axaml`, `theming/SkinVariants.g.cs`, `theming/ShareXTheme.axaml` | `ShareX.Avalonia/Theming/` |
| `icons/lucide.ttf`, `icons/lucide-codepoints.json`, `icons/Lucide_license.txt` | `ShareX.Avalonia/Assets/`, `Licenses/` |
| `icons/Modlight_Icon.ico`, `icons/logo.svg` | `ShareX/Modlight_Icon.ico`, `docs/logo.svg` |
| `names/adjectives.txt`, `names/animals.txt` (the `%radjective` and `%ranimal` word lists), `names/emoji.txt` (the `%remoji` list: the strings of `Emoji.Emojis`, one per line in their order, the commented-out ones left out; the only file here written out rather than copied) | `ShareX.HelpersLib/Resources/adjectives.txt`, `animals.txt`, `ShareX.HelpersLib/Emoji.cs` |

To refresh them, copy the same files again and rebuild; `modlight-core/build.rs`
regenerates the string tables, skins and icon constants.
