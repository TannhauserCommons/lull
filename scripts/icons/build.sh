#!/bin/sh
# Renders every packaged app icon from the masters in design/assets (mise run icons):
# the desktop, web and store sizes (build.mjs, pinned resvg), then the Android launcher icons with the
# Tauri CLI version app/package.json pins, background #F7F4FF (docs/context/android.md).
set -eu
root=$(cd "$(dirname "$0")/../.." && { pwd -W 2>/dev/null || pwd; })   # Git Bash: a Windows path, which node and npm understand
(cd "$root/scripts/icons" && npm ci --no-audit --no-fund --ignore-scripts >/dev/null)
node "$root/scripts/icons/build.mjs" "$root"
cli=$(node -p "require('$root/app/package.json').devDependencies['@tauri-apps/cli'] || require('$root/app/package.json').dependencies['@tauri-apps/cli']")
tmp=$(cd "$(mktemp -d)" && { pwd -W 2>/dev/null || pwd; })
npx -y "@tauri-apps/cli@$cli" icon "$root/design/assets/icons/icon-512.png" -o "$tmp" --ios-color '#F7F4FF' >/dev/null
cp -r "$tmp/android/." "$root/app/gen/android/app/src/main/res/"
rm -rf "$tmp"
echo "icons: Android launcher icons from icon-512.png"
