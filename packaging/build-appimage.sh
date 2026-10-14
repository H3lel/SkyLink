#!/usr/bin/env bash
# Build SkyLink-x86_64.AppImage from source plus the official Windows installer.
#
#   packaging/build-appimage.sh [path/to/nthlink-win64-current.exe]
#
# Requires: cargo, dotnet SDK (8+), 7z, python3, curl, gcc. Works without any
# -dev packages (see build-env.sh).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
installer=${1:-$root/../nthlink-win64-current.exe}
build=$root/build
appdir=$build/AppDir
out=$root/SkyLink-x86_64.AppImage
appimagetool_version=1.9.0

[[ -f "$installer" ]] || { echo "installer not found: $installer" >&2; exit 1; }
for t in cargo dotnet 7z python3 curl gcc; do
  command -v "$t" >/dev/null || { echo "missing build tool: $t" >&2; exit 1; }
done

mkdir -p "$build"
echo "==> extracting rootGo.dll and the API key from $(basename "$installer")"
python3 -I "$root/packaging/extract-installer.py" "$installer" "$build/vendor"

echo "==> building rootgo-bridge.exe (runs under Wine)"
dotnet publish "$root/winhelper/rootgo-bridge.csproj" -c Release -o "$build/bridge" --nologo -v quiet

echo "==> building nthlink + nthlink-core"
# shellcheck source=/dev/null
. "$root/build-env.sh" "$build/env"
(cd "$root" && cargo build --release)
(cd "$root" && cargo test --release -p nthlink-common --quiet)

echo "==> assembling AppDir"
rm -rf "$appdir"
res=$appdir/usr/lib/nthlink
install -Dm755 "$root/target/release/nthlink" "$appdir/usr/bin/nthlink"
install -Dm755 "$root/target/release/nthlink-core" "$res/nthlink-core"
install -Dm644 "$build/bridge/rootgo-bridge.exe" "$res/rootgo-bridge.exe"
install -Dm644 "$build/vendor/rootGo.dll" "$res/rootGo.dll"
install -Dm644 "$build/vendor/apikey" "$res/apikey"
install -Dm644 "$root/packaging/nthlink.png" "$res/icons/nthlink.png"
install -Dm644 "$root/packaging/nthlink.png" "$appdir/usr/share/icons/hicolor/64x64/apps/nthlink.png"
install -Dm644 "$root/packaging/nthlink.desktop" "$appdir/usr/share/applications/nthlink.desktop"
install -Dm755 "$root/packaging/AppRun" "$appdir/AppRun"
cp "$root/packaging/nthlink.desktop" "$appdir/nthlink.desktop"
cp "$root/packaging/nthlink.png" "$appdir/nthlink.png"
ln -sf nthlink.png "$appdir/.DirIcon"

tool=$build/appimagetool-$appimagetool_version-x86_64.AppImage
if [[ ! -x "$tool" ]]; then
  echo "==> downloading appimagetool $appimagetool_version"
  curl -fsSL -o "$tool.part" "https://github.com/AppImage/appimagetool/releases/download/$appimagetool_version/appimagetool-x86_64.AppImage"
  chmod +x "$tool.part" && mv "$tool.part" "$tool"
fi

echo "==> packing $(basename "$out")"
# Extract-and-run avoids needing FUSE on the build machine.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream "$appdir" "$out" >/dev/null
chmod +x "$out"
ls -lh "$out"
