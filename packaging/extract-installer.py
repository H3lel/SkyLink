#!/usr/bin/env python3
"""Pull what the Linux build needs out of the official Windows installer.

    extract-installer.py <nthlink-win64-*.exe> <outdir>

Writes <outdir>/rootGo.dll (the directory-service client) and <outdir>/apikey
(the key the NSIS script hands to DataProtector.exe, which on Windows stores
it DPAPI-encrypted as apikey.dat). Needs 7z for the file payload.
"""
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
import lzma


def nsis_header(exe: bytes) -> bytes:
    """Decompress the NSIS header block (holds the install script strings)."""
    i = exe.find(b"\xef\xbe\xad\xdeNullsoftInst")
    if i < 4:
        raise SystemExit("not an NSIS installer")
    pos = i - 4
    hdr_len = struct.unpack_from("<i", exe, pos + 20)[0]
    body = exe[pos + 28:]
    attempts = []
    for off in (0, 4):
        attempts.append(lambda o=off: zlib.decompressobj(-15).decompress(body[o:o + 32 * 1024 * 1024]))
        attempts.append(
            lambda o=off: lzma.LZMADecompressor(
                lzma.FORMAT_RAW,
                filters=[lzma._decode_filter_properties(lzma.FILTER_LZMA1, body[o:o + 5])],
            ).decompress(body[o + 5:o + 5 + 32 * 1024 * 1024], max_length=hdr_len + 8)
        )
    for attempt in attempts:
        try:
            out = attempt()
        except Exception:
            continue
        if len(out) >= min(hdr_len, 1024):
            return out
    raise SystemExit("could not decompress the NSIS header")


def api_key(header: bytes) -> str:
    for enc in ("utf-16-le", "latin-1"):
        m = re.search(r"--key:apikey --value:(\S+)", header.decode(enc, "replace"))
        if m:
            return m.group(1)
    raise SystemExit("API key not found in the installer script")


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    exe_path, outdir = sys.argv[1], sys.argv[2]
    os.makedirs(outdir, exist_ok=True)

    with open(exe_path, "rb") as f:
        head = f.read(64 * 1024 * 1024)
    key = api_key(nsis_header(head))
    key_path = os.path.join(outdir, "apikey")
    fd = os.open(key_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(key)

    seven = shutil.which("7z") or shutil.which("7zz") or shutil.which("7za")
    if not seven:
        raise SystemExit("7z is required (sudo apt install p7zip-full)")
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run([seven, "e", "-y", f"-o{tmp}", exe_path, "rootGo.dll"], check=True, stdout=subprocess.DEVNULL)
        shutil.copyfile(os.path.join(tmp, "rootGo.dll"), os.path.join(outdir, "rootGo.dll"))
    print(f"extracted rootGo.dll and apikey ({len(key)} chars) to {outdir}")


if __name__ == "__main__":
    main()
