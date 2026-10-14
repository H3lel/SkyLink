# Source this before `cargo build` on a machine without -dev packages:
#   . ./build-env.sh <scratch-dir>
# It provides a pkg-config shim for GTK4 and the libclang bits leaf's lwip
# netstack needs for bindgen.
_scratch=${1:-${TMPDIR:-/tmp}/nthlink-build}
mkdir -p "$_scratch"
export PKG_CONFIG_PATH="$("$(dirname "${BASH_SOURCE[0]:-$0}")"/packaging/gtk-shim.sh "$_scratch/gtkshim")${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
if [ -e /usr/lib/llvm-18/lib/libclang-18.so.1 ] && ldd /usr/lib/llvm-18/lib/libclang-18.so.1 | grep -q 'not found'; then
  if [ ! -e "$_scratch/llvm/usr/lib/llvm-18/lib/libLLVM.so.1" ]; then
    (cd "$_scratch" && apt download libllvm18 >/dev/null && dpkg-deb -x libllvm18_*.deb llvm)
  fi
  _l="$_scratch/llvm/usr/lib/llvm-18/lib"
  ln -sf libLLVM.so.1 "$_l/libLLVM-18.so.18.1"
  ln -sf libLLVM.so.1 "$_l/libLLVM.so.18.1"
  export LD_LIBRARY_PATH="$_l${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
export LIBCLANG_PATH=${LIBCLANG_PATH:-/usr/lib/llvm-18/lib}
_gccinc=$(gcc -print-file-name=include 2>/dev/null)
[ -n "$_gccinc" ] && export BINDGEN_EXTRA_CLANG_ARGS="-I$_gccinc"
unset _scratch _l _gccinc
