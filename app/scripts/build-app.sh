#!/bin/sh
# Build Modlight.app: a release build of the app, wrapped in a bundle with its
# Info.plist and the icon converted from assets/icons/Modlight.ico, signed with
# the Keychain's Apple Development certificate when there is one (ad hoc otherwise), then
# zipped as Modlight-<version>-<build>-macos-<arch>.zip and published to the server's
# Downloads when a server and a publish token are configured. The script never starts
# the app.
#
#   app/scripts/build-app.sh [output-dir]       build (default output: app/dist)
#   app/scripts/build-app.sh --publish <zip>    publish a zip it made before, again
#
# The build is the commit (git rev-parse --short=8 HEAD, empty outside git) and the time
# it was built (UTC); both go into the app (MODLIGHT_BUILD, MODLIGHT_BUILT, read by
# crates/modlight-core/build.rs) and into the zip's name and its description beside it
# (<zip name>.json), so the app knows it is exactly the build the server lists.
#
# Publishing (app/docs/MACOS.md, "Updates and publishing"):
#   MODLIGHT_PUBLISH_SERVER   the server, e.g. https://files.example.com
#   MODLIGHT_PUBLISH_TOKEN    a publish token from the server's Downloads (never printed)
#   or both in ~/.config/modlight/publish.json ({"server": "...", "token": "..."}, mode 600)
#   MODLIGHT_PUBLISH=0        build without publishing
#   MODLIGHT_PUBLISH_NOTE     what changed, shown to people with the update
# A failed publish does not fail the build; it says how to try again.
#
# Needs: the Rust toolchain, and the tools every Mac has (od, dd, sips, iconutil,
# codesign, plutil, ditto, shasum, curl). Nothing is downloaded and nothing outside the
# output directory and cargo's target directory is written.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
app_root=$(cd "$here/.." && pwd)
name=Modlight

work=$(mktemp -d "${TMPDIR:-/tmp}/modlight-build-app.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

# --- Publishing -------------------------------------------------------------------
# POST /api/downloads/upload with the zip's description, PUT each chunk of at most the
# server's chunkSize (50 MiB), POST .../finish; Authorization: Bearer <publish token>,
# read by curl from a file of this run's own (never on a command line, never printed).

publish_config() {
    # Sets publish_server and publish_token, or says why not and fails.
    publish_server=${MODLIGHT_PUBLISH_SERVER:-}
    publish_token=${MODLIGHT_PUBLISH_TOKEN:-}
    config="$HOME/.config/modlight/publish.json"
    if [ -z "$publish_server" ] && [ -z "$publish_token" ] && [ -f "$config" ]; then
        mode=$(stat -f '%Lp' "$config")
        owner=$(stat -f '%u' "$config")
        if [ "$owner" != "$(id -u)" ] || { [ "$mode" != 600 ] && [ "$mode" != 400 ]; }; then
            echo "build-app: $config is readable by others (mode $mode); run chmod 600 on it" >&2
            return 1
        fi
        publish_server=$(plutil -extract server raw -o - "$config" 2>/dev/null || true)
        publish_token=$(plutil -extract token raw -o - "$config" 2>/dev/null || true)
    fi
    if [ -z "$publish_server" ] || [ -z "$publish_token" ]; then
        return 2
    fi
    publish_server=${publish_server%/}
    case $publish_server in
        https://?*) ;;
        http://127.0.0.1:* | http://127.0.0.1 | http://localhost:* | http://localhost | http://\[::1\]:*) ;;
        *)
            echo "build-app: the publish server must be https:// (plain http only on this computer)" >&2
            return 1
            ;;
    esac
    case $publish_token in
        *[!A-Za-z0-9._~+/=-]*)
            echo "build-app: the publish token has characters a token never has" >&2
            return 1
            ;;
    esac
    return 0
}

# json_field <file> <key>: a top-level value of a JSON file, or nothing.
json_field() {
    plutil -extract "$2" raw -o - "$1" 2>/dev/null || true
}

# http <method> <url> <response file> [curl arguments...]: the status code, or 000 when
# nothing answered (curl says why on standard error; it never shows the headers).
http() {
    method=$1 url=$2 response=$3
    shift 3
    code=$(curl -sS -X "$method" -H @"$work/auth" -H 'Expect:' --connect-timeout 20 \
        --speed-limit 1024 --speed-time 60 -o "$response" -w '%{http_code}' "$@" "$url") ||
        true
    echo "${code:-000}"
}

# answered <status>: how a status reads in a message.
answered() {
    if [ "$1" = 000 ]; then echo "got no answer"; else echo "answered $1"; fi
}

publish_failed() {
    echo "build-app: not published: $1" >&2
    if [ -s "$work/response" ]; then
        echo "build-app: the server said: $(head -c 300 "$work/response")" >&2
    fi
    echo "build-app: the build itself is fine; to publish it again:" >&2
    echo "    MODLIGHT_PUBLISH_SERVER=... MODLIGHT_PUBLISH_TOKEN=... $0 --publish $2" >&2
    echo "    (or with ~/.config/modlight/publish.json in place: $0 --publish $2)" >&2
}

# publish <zip>: the zip and its description (<zip without .zip>.json) to the server.
publish() {
    zip=$1
    description="${zip%.zip}.json"
    if [ "${MODLIGHT_PUBLISH:-1}" = 0 ]; then
        echo "build-app: not published (MODLIGHT_PUBLISH=0)"
        return 0
    fi
    if publish_config; then configured=0; else configured=$?; fi
    if [ "$configured" = 2 ]; then
        echo "build-app: not published (no MODLIGHT_PUBLISH_SERVER and MODLIGHT_PUBLISH_TOKEN, no ~/.config/modlight/publish.json)"
        return 0
    elif [ "$configured" != 0 ]; then
        publish_failed "no usable server and token" "$zip"
        return 1
    fi
    if [ ! -f "$zip" ] || [ ! -f "$description" ]; then
        publish_failed "$zip or its description $description is missing" "$zip"
        return 1
    fi
    size=$(wc -c < "$zip" | tr -d ' ')
    sha256=$(shasum -a 256 "$zip" | awk '{ print $1 }')
    if [ "$size" != "$(json_field "$description" size)" ] ||
        [ "$sha256" != "$(json_field "$description" sha256)" ]; then
        publish_failed "$zip is not the file its description describes" "$zip"
        return 1
    fi
    (umask 077 && printf 'Authorization: Bearer %s\n' "$publish_token" > "$work/auth")
    cp "$description" "$work/body.json"
    plutil -replace note -string "${MODLIGHT_PUBLISH_NOTE:-}" "$work/body.json"
    echo "build-app: publishing $(basename "$zip") to $publish_server"
    : > "$work/response"
    status=$(http POST "$publish_server/api/downloads/upload" "$work/response" \
        -H 'Content-Type: application/json' --data-binary @"$work/body.json")
    if [ "$status" != 200 ] && [ "$status" != 201 ]; then
        publish_failed "starting the upload $(answered "$status")" "$zip"
        return 1
    fi
    upload=$(json_field "$work/response" upload)
    chunk_size=$(json_field "$work/response" chunkSize)
    case $upload in '' | *[!A-Za-z0-9_-]*)
        publish_failed "the server named no upload" "$zip"
        return 1
        ;;
    esac
    case $chunk_size in '' | *[!0-9]*)
        publish_failed "the server named no chunk size" "$zip"
        return 1
        ;;
    esac
    [ "$chunk_size" -le 52428800 ] || chunk_size=52428800
    if [ "$chunk_size" -le 0 ]; then
        publish_failed "the server named no chunk size" "$zip"
        return 1
    fi
    chunks=$(( (size + chunk_size - 1) / chunk_size ))
    n=0
    while [ "$n" -lt "$chunks" ]; do
        dd if="$zip" of="$work/chunk" bs="$chunk_size" skip="$n" count=1 2>/dev/null
        attempt=1
        while :; do
            : > "$work/response"
            status=$(http PUT "$publish_server/api/downloads/upload/$upload/$n" "$work/response" \
                -H 'Content-Type: application/octet-stream' --data-binary @"$work/chunk")
            case $status in 2??) break ;; esac
            if [ "$attempt" -ge 3 ]; then
                publish_failed "chunk $((n + 1)) of $chunks $(answered "$status")" "$zip"
                return 1
            fi
            attempt=$((attempt + 1))
            sleep 2
        done
        n=$((n + 1))
        echo "build-app: sent $n of $chunks"
    done
    : > "$work/response"
    status=$(http POST "$publish_server/api/downloads/upload/$upload/finish" "$work/response")
    if [ "$status" != 200 ] && [ "$status" != 201 ]; then
        publish_failed "finishing the upload $(answered "$status")" "$zip"
        return 1
    fi
    echo "build-app: published as $(json_field "$work/response" id) ($(json_field "$work/response" url))"
    return 0
}

if [ "${1:-}" = --publish ]; then
    if [ -z "${2:-}" ]; then
        echo "usage: $0 --publish <Modlight-...-macos-<arch>.zip>" >&2
        exit 2
    fi
    zip=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
    if publish "$zip"; then exit 0; else exit 1; fi
fi

out_dir=${1:-"$app_root/dist"}
mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)

bundle="$out_dir/$name.app"
identifier=org.modlight.Modlight
icon_source="$app_root/assets/icons/Modlight.png"

# The version the workspace builds, e.g. 0.7.7.
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$app_root/Cargo.toml" | head -n 1)
if [ -z "$version" ]; then
    echo "build-app: no version in $app_root/Cargo.toml" >&2
    exit 1
fi
# The build: the commit and the time, the same in the app and in what is published.
build=$(git -C "$app_root" rev-parse --short=8 HEAD 2>/dev/null || true)
built=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo "build-app: cargo build --release ($version, build ${build:-outside git}, $built)"
(cd "$app_root" && MODLIGHT_BUILD="$build" MODLIGHT_BUILT="$built" cargo build --release -p modlight-app)
binary="$app_root/target/release/$name"
if [ ! -x "$binary" ]; then
    echo "build-app: $binary was not built" >&2
    exit 1
fi

# --- The icon -------------------------------------------------------------------
# assets/icons/Modlight.png is the artwork filling its whole square (as the Windows
# ICO's frames do). On a Mac that looks bigger than every other app: Apple's grid puts
# the artwork in 824 of 1024 px with a transparent margin, so each size is made from
# it scaled to that share of the size, then padded back to the full size.
iconset="$work/$name.iconset"
mkdir -p "$iconset"
put() {
    # put <size px> <iconset name>
    inner=$(( ($1 * 824 + 512) / 1024 ))
    sips -Z "$inner" "$icon_source" --out "$work/inner.png" >/dev/null
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
        echo "build-app: $(basename "$png") could not be made" >&2
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

# --- The zip for the server's Downloads ------------------------------------------
# ditto keeps what the signature seals; the app unpacks it with ditto too.
case $(lipo -archs "$binary" 2>/dev/null || uname -m) in
    *arm64*) arch=arm64 ;;
    *x86_64*) arch=x64 ;;
    *) arch=$(uname -m) ;;
esac
zip_name="$name-$version-$build-macos-$arch.zip"
zip="$out_dir/$zip_name"
description="$out_dir/${zip_name%.zip}.json"
rm -f "$zip" "$description"
ditto -c -k --keepParent "$bundle" "$zip"
size=$(wc -c < "$zip" | tr -d ' ')
sha256=$(shasum -a 256 "$zip" | awk '{ print $1 }')
# The upload's description, which `--publish` sends: written by plutil, so every value is
# escaped as JSON wants.
printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' \
    '<plist version="1.0"><dict/></plist>' > "$work/description.plist"
plutil -insert platform -string macos "$work/description.plist"
plutil -insert arch -string "$arch" "$work/description.plist"
plutil -insert version -string "$version" "$work/description.plist"
plutil -insert build -string "$build" "$work/description.plist"
plutil -insert built -string "$built" "$work/description.plist"
plutil -insert size -integer "$size" "$work/description.plist"
plutil -insert sha256 -string "$sha256" "$work/description.plist"
plutil -insert filename -string "$zip_name" "$work/description.plist"
plutil -insert note -string '' "$work/description.plist"
plutil -convert json -o "$description" "$work/description.plist"
echo "build-app: $zip"
echo "build-app: $size bytes, SHA-256 $sha256"

publish "$zip" || true
