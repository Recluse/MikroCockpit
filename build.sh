#!/bin/sh
set -eu
cd "$(dirname "$0")"
umask 077
# This Mac's cargo is linked into Homebrew while rustc is not on PATH.
if ! command -v rustc >/dev/null 2>&1; then
    RUSTC="$(rustup which rustc)"
    PATH="$(dirname "$RUSTC"):$PATH"
    export RUSTC PATH
fi
CARGO_HOME="$PWD/.cargo-local"
export CARGO_HOME
cargo build --release --locked
bundle_stage=$(mktemp -d "$PWD/target/bundle.XXXXXX")
app="$bundle_stage/MikroCockpit.app"
mkdir -p "$app/Contents/MacOS"
mkdir -p "$app/Contents/Resources" target/AppIcon.iconset
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/logo.png --out "target/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" assets/logo.png --out "target/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
cp -R assets/lucide "$app/Contents/Resources/"
cp assets/tray/*-large@2x.png "$app/Contents/Resources/"
iconutil -c icns target/AppIcon.iconset -o "$app/Contents/Resources/AppIcon.icns"
# 18-point menu bar artwork, with separate 1x and 2x representations.
# Prepared artwork; status-driven switching is separate from asset generation.
mkdir -p target/tray-preview
for state in online offline vpn; do
    sips -z 18 18 "assets/tray/${state}-source.png" --out "target/tray-preview/${state}.png" >/dev/null
    sips -z 36 36 "assets/tray/${state}-source.png" --out "target/tray-preview/${state}@2x.png" >/dev/null
done
cp target/release/mikrocockpit "$app/Contents/MacOS/MikroCockpit"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>MikroCockpit</string>
<key>CFBundleDisplayName</key><string>MikroCockpit</string>
<key>CFBundleIdentifier</key><string>ru.recluse.mikrocockpit</string>
<key>CFBundleExecutable</key><string>MikroCockpit</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleDevelopmentRegion</key><string>en</string>
<key>CFBundleLocalizations</key><array><string>en</string></array>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
mkdir -p dist
if [ ! -f dist/settings.json ]; then
    if [ -f settings.json ]; then cp settings.json dist/settings.json
    else cp settings.example.json dist/settings.json; fi
fi
codesign --force --sign - "$app"
codesign --verify --strict "$app"
# Do not overwrite a signed executable while a running app has it mapped.
if [ -d dist/MikroCockpit.app ]; then mv dist/MikroCockpit.app "$bundle_stage/previous.app"; fi
mv "$app" dist/MikroCockpit.app
app="$PWD/dist/MikroCockpit.app"
cp "$app/Contents/Resources/AppIcon.icns" dist/MikroCockpit.icns
# Notify Finder/Launch Services when rebuilding the same bundle in place.
touch "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
printf 'Built: %s\n' "$app"
