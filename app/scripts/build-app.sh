#!/bin/sh
# Build Modlight.app: a release build of the app, wrapped in a bundle with its
# Info.plist and an icon converted from upstream's ShareX/Modlight_Icon.ico, signed with
# the Keychain's Apple Development certificate when there is one (ad hoc otherwise). The
# script never starts the app.
#
#   app/scripts/build-app.sh [output-dir]     (default: app/dist)
#
# Needs: the Rust toolchain, and the tools every Mac has (od, dd, sips, iconutil,
# codesign, plutil). Nothing is downloaded and nothing outside the output directory
# and cargo's target directory is written.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
app_root=$(cd "$here/.." && pwd)
out_dir=${1:-"$app_root/dist"}
mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)

name=Modlight
bundle="$out_dir/$name.app"
identifier=org.modlight.Modlight
icon_source="$app_root/assets/upstream/icons/Modlight_Icon.ico"

# The version the workspace builds, e.g. 0.7.7.
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$app_root/Cargo.toml" | head -n 1)
if [ -z "$version" ]; then
    echo "build-app: no version in $app_root/Cargo.toml" >&2
    exit 1
fi

echo "build-app: cargo build --release ($version)"
(cd "$app_root" && cargo build --release -p modlight-app)
binary="$app_root/target/release/$name"
if [ ! -x "$binary" ]; then
    echo "build-app: $binary was not built" >&2
    exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/modlight-build-app.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

# --- The icon -------------------------------------------------------------------
# Every frame of the upstream ICO is a PNG (16, 24, 32, 48, 64, 128 and 256 px);
# They are cut out of the file as they are, no resampling, and filed under the
# names iconutil expects; sizes the ICO does not have are left to the system.
iconset="$work/$name.iconset"
mkdir -p "$iconset" "$work/frames"
u16() { od -An -tu2 -j "$2" -N 2 "$1" | tr -d ' '; }
u32() { od -An -tu4 -j "$2" -N 4 "$1" | tr -d ' '; }
u8() { od -An -tu1 -j "$2" -N 1 "$1" | tr -d ' '; }
count=$(u16 "$icon_source" 4)
i=0
while [ "$i" -lt "$count" ]; do
    entry=$((6 + 16 * i))
    width=$(u8 "$icon_source" "$entry")
    [ "$width" -eq 0 ] && width=256
    size=$(u32 "$icon_source" $((entry + 8)))
    offset=$(u32 "$icon_source" $((entry + 12)))
    dd if="$icon_source" of="$work/frames/$width.png" bs=1 skip="$offset" count="$size" 2>/dev/null
    i=$((i + 1))
done
# The Windows frames fill their whole square, which on a Mac makes the icon look
# bigger than every other app. Apple's grid puts the artwork in 824 of 1024 px with a
# transparent margin, so each size is made from the largest frame: scaled to that
# share of the size, then padded back to the full size.
largest=$(ls "$work/frames" | sed 's/\.png$//' | sort -n | tail -1)
put() {
    # put <size px> <iconset name>
    inner=$(( ($1 * 824 + 512) / 1024 ))
    sips -Z "$inner" "$work/frames/$largest.png" --out "$work/inner.png" >/dev/null
    sips --padToHeightWidth "$1" "$1" "$work/inner.png" --out "$iconset/$2.png" >/dev/null
}
put 16 icon_16x16
put 32 icon_16x16@2x
put 32 icon_32x32
put 64 icon_32x32@2x
put 128 icon_128x128
put 256 icon_128x128@2x
put 256 icon_256x256
for png in "$iconset"/*.png; do
    # A frame that is not a PNG (an old BMP entry) would make iconutil fail later
    # with a message about the whole set; say which one it is instead.
    sips -g pixelWidth "$png" >/dev/null 2>&1 || {
        echo "build-app: $(basename "$png") is not a PNG frame" >&2
        exit 1
    }
done
iconutil --convert icns --output "$work/$name.icns" "$iconset"

# --- The bundle -----------------------------------------------------------------
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$binary" "$bundle/Contents/MacOS/$name"
cp "$work/$name.icns" "$bundle/Contents/Resources/$name.icns"
printf 'APPL????' > "$bundle/Contents/PkgInfo"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>$name</string>
    <key>CFBundleExecutable</key>
    <string>$name</string>
    <key>CFBundleIconFile</key>
    <string>$name</string>
    <key>CFBundleIdentifier</key>
    <string>$identifier</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleLocalizations</key>
    <array>
        <string>en</string>
        <string>pl</string>
    </array>
    <key>CFBundleName</key>
    <string>$name</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.productivity</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <!-- ffmpeg records cameras and microphones through avfoundation as this app's child,
         so the questions are asked in this app's name. -->
    <key>NSCameraUsageDescription</key>
    <string>Modlight records the camera you choose as a screen recording source.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Modlight records the microphone you choose with a screen recording.</string>
    <!-- Finder's "Upload with Modlight" (the Explorer context menu's counterpart). -->
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>Upload with Modlight</string>
            </dict>
            <key>NSMessage</key>
            <string>uploadFiles</string>
            <key>NSPortName</key>
            <string>$name</string>
            <key>NSRequiredContext</key>
            <dict/>
            <key>NSSendFileTypes</key>
            <array>
                <string>public.item</string>
            </array>
        </dict>
    </array>
    <key>NSHumanReadableCopyright</key>
    <string>GPL-3.0-or-later. Based on ShareX.</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist" >/dev/null

# The service's name in Polish (the menu item's key is its English text).
mkdir -p "$bundle/Contents/Resources/en.lproj" "$bundle/Contents/Resources/pl.lproj"
printf '"Upload with Modlight" = "Upload with Modlight";\n' > "$bundle/Contents/Resources/en.lproj/ServicesMenu.strings"
printf '"Upload with Modlight" = "Prześlij za pomocą Modlight";\n' > "$bundle/Contents/Resources/pl.lproj/ServicesMenu.strings"

# --- Signature ------------------------------------------------------------------
# macOS keeps the Screen Recording permission (and the others) for one signature. An ad
# hoc signature is new with every build, so each rebuild lost the permission and
# captures failed until it was given again; an Apple Development certificate gives every
# build the same signature. MODLIGHT_SIGN_IDENTITY picks another identity, or "-" for ad
# hoc. The Keychain is reached through /usr/bin/security, so a new signature does not
# orphan the stored token.
identity=${MODLIGHT_SIGN_IDENTITY:-}
if [ -z "$identity" ]; then
    identity=$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/"Apple Development: / { print $2; exit }')
    [ -n "$identity" ] || identity=-
fi
if [ "$identity" = - ]; then
    echo "build-app: signing ad hoc (macOS asks for its permissions again after each build)"
else
    echo "build-app: signing with the identity $identity"
fi
codesign --force --sign "$identity" --timestamp=none "$bundle"
codesign --verify --strict --verbose=2 "$bundle"

echo "build-app: $bundle"
