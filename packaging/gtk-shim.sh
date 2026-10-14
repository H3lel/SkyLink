#!/usr/bin/env bash
# Generates pkg-config files + unversioned .so symlinks for the GTK4 stack so
# gtk4-rs can link on machines that only have the runtime libraries installed
# (no -dev packages). Usage: gtk-shim.sh <outdir>; then export
# PKG_CONFIG_PATH=<outdir>/pkgconfig.
set -euo pipefail
out=$(realpath -m "$1")
libdir=${LIBDIR:-/usr/lib/x86_64-linux-gnu}
mkdir -p "$out/pkgconfig" "$out/lib"

# pc-name  version  lib-name  requires
while read -r pc ver lib req; do
  so=$(ls "$libdir"/lib"$lib".so.* 2>/dev/null | head -1 || true)
  if [[ -z "$so" ]]; then echo "missing runtime library lib$lib" >&2; exit 1; fi
  ln -sf "$so" "$out/lib/lib$lib.so"
  [[ "$req" == "-" ]] && req=""
  cat > "$out/pkgconfig/$pc.pc" <<PC
Name: $pc
Description: runtime shim for $pc
Version: $ver
Requires: ${req//,/ }
Libs: -L$out/lib -l$lib
Cflags:
PC
done <<'LIST'
glib-2.0 2.80.0 glib-2.0 -
gobject-2.0 2.80.0 gobject-2.0 glib-2.0
gio-2.0 2.80.0 gio-2.0 gobject-2.0
gmodule-2.0 2.80.0 gmodule-2.0 glib-2.0
cairo 1.18.0 cairo -
cairo-gobject 1.18.0 cairo-gobject cairo,gobject-2.0
pango 1.52.1 pango-1.0 gobject-2.0
pangocairo 1.52.1 pangocairo-1.0 pango,cairo
gdk-pixbuf-2.0 2.42.10 gdk_pixbuf-2.0 gio-2.0
graphene-gobject-1.0 1.10.8 graphene-1.0 gobject-2.0
graphene-1.0 1.10.8 graphene-1.0 -
harfbuzz 8.3.0 harfbuzz -
gtk4 4.14.5 gtk-4 gio-2.0,pango,cairo,gdk-pixbuf-2.0,graphene-gobject-1.0
LIST
echo "$out/pkgconfig"
