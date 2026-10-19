#region License Information (GPL v3)

/*
    ShareX - A program that allows you to take screenshots and share any file type
    Copyright (c) 2007-2026 ShareX Team

    This program is free software; you can redistribute it and/or
    modify it under the terms of the GNU General Public License
    as published by the Free Software Foundation; either version 2
    of the License, or (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program; if not, write to the Free Software
    Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301, USA.

    Optionally you can also view the license at <http://www.gnu.org/licenses/>.
*/

#endregion License Information (GPL v3)

// GENERATED FILE - do not edit.
//
// Written by Tools/generate-skins.py from the web panel's lib/themes.ts and
// messages/{en,pl}/profile.json. Regenerate with:
//
//     python Tools/generate-skins.py

// The .g.cs suffix makes Roslyn treat this as generated and switch the nullable
// context off, which turns every `string?` below into a warning. Say it explicitly.
#nullable enable

using Avalonia.Styling;
using System.Collections.Generic;

namespace ShareX.AvaloniaUI.Theming;

/// <summary>One skin, as the picker lists it.</summary>
/// <param name="Id">The panel's id for it - the value written to and read from /api/theme.</param>
/// <param name="NameEnglish">The panel's own label.</param>
/// <param name="NamePolish">The panel's Polish label.</param>
/// <param name="Light">The theme variant for this skin on a light ground.</param>
/// <param name="Dark">The theme variant for this skin on a dark ground.</param>
public sealed record SkinDefinition(
    string Id,
    string NameEnglish,
    string NamePolish,
    ThemeVariant Light,
    ThemeVariant Dark);

/// <summary>
/// The theme variants the generated Skins.axaml dictionaries are keyed by.
/// </summary>
/// <remarks>
/// Each one inherits the plain <see cref="ThemeVariant.Dark"/> or
/// <see cref="ThemeVariant.Light"/> rather than ShareXDark/ShareXLight, so Fluent's
/// own resources resolve through exactly the one-level fallback ShareXDark has
/// always used. The dictionaries are complete - every ShareX.Color.* and
/// ShareX.Brush.* key is present in all of them - so nothing depends on a longer
/// chain that this codebase has never exercised.
/// </remarks>
public static class SkinVariants
{

    public static readonly ThemeVariant Default_Light = new("Skin.default.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Default_Dark = new("Skin.default.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Glass_Light = new("Skin.glass.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Glass_Dark = new("Skin.glass.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Synthwave_Light = new("Skin.synthwave.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Synthwave_Dark = new("Skin.synthwave.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Terminal_Light = new("Skin.terminal.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Terminal_Dark = new("Skin.terminal.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Paper_Light = new("Skin.paper.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Paper_Dark = new("Skin.paper.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Win98_Light = new("Skin.win98.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Win98_Dark = new("Skin.win98.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Nebula_Light = new("Skin.nebula.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Nebula_Dark = new("Skin.nebula.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Grimoire_Light = new("Skin.grimoire.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Grimoire_Dark = new("Skin.grimoire.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Cartoon_Light = new("Skin.cartoon.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Cartoon_Dark = new("Skin.cartoon.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Mono_Light = new("Skin.mono.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Mono_Dark = new("Skin.mono.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Deco_Light = new("Skin.deco.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Deco_Dark = new("Skin.deco.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Riso_Light = new("Skin.riso.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Riso_Dark = new("Skin.riso.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Discord_Light = new("Skin.discord.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Discord_Dark = new("Skin.discord.Dark", ThemeVariant.Dark);
    public static readonly ThemeVariant Spotify_Light = new("Skin.spotify.Light", ThemeVariant.Light);
    public static readonly ThemeVariant Spotify_Dark = new("Skin.spotify.Dark", ThemeVariant.Dark);

    /// <summary>What an account gets before it has chosen - the panel's own default.</summary>
    public const string DefaultSkinId = "discord";

    /// <summary>Every skin, in the order the panel's picker shows them.</summary>
    public static readonly IReadOnlyList<SkinDefinition> All = new SkinDefinition[]
    {
        new("default", "Classic", "Klasyczny", Default_Light, Default_Dark),
        new("glass", "Liquid Glass", "Liquid Glass", Glass_Light, Glass_Dark),
        new("synthwave", "Synthwave", "Synthwave", Synthwave_Light, Synthwave_Dark),
        new("terminal", "Terminal", "Terminal", Terminal_Light, Terminal_Dark),
        new("paper", "Paper", "Papier", Paper_Light, Paper_Dark),
        new("win98", "Windows 98", "Windows 98", Win98_Light, Win98_Dark),
        new("nebula", "Nebula", "Mg\u0142awica", Nebula_Light, Nebula_Dark),
        new("grimoire", "Grimoire", "Grimuar", Grimoire_Light, Grimoire_Dark),
        new("cartoon", "Cartoon", "Kresk\u00F3wka", Cartoon_Light, Cartoon_Dark),
        new("mono", "Monochrome", "Monochrom", Mono_Light, Mono_Dark),
        new("deco", "Art Deco", "Art d\u00E9co", Deco_Light, Deco_Dark),
        new("riso", "Riso Press", "Riso Press", Riso_Light, Riso_Dark),
        new("discord", "Late Night", "P\u00F3\u017Ana noc", Discord_Light, Discord_Dark),
        new("spotify", "Night Player", "Nocny odtwarzacz", Spotify_Light, Spotify_Dark),
    };

    /// <summary>The skin with this id, or the default when the id is unknown.</summary>
    /// <remarks>
    /// Unknown rather than invalid: the website can be a release ahead of this build,
    /// and a skin it has that we do not is a reason to fall back for the moment, never a
    /// reason to fail. The stored id is left alone so the next build picks it up.
    /// </remarks>
    public static SkinDefinition Resolve(string? id)
    {
        foreach (SkinDefinition skin in All)
        {
            if (string.Equals(skin.Id, id, System.StringComparison.OrdinalIgnoreCase))
            {
                return skin;
            }
        }

        foreach (SkinDefinition skin in All)
        {
            if (skin.Id == DefaultSkinId)
            {
                return skin;
            }
        }

        return All[0];
    }

    /// <summary>Whether this id names a skin this build can paint.</summary>
    public static bool IsKnown(string? id)
    {
        foreach (SkinDefinition skin in All)
        {
            if (string.Equals(skin.Id, id, System.StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        return false;
    }
}
