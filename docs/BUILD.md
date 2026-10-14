# Building SkyLink from source

## Tools
* Rust (stable, via rustup), `gcc`, `cmake`, `perl` (for aws-lc)
* .NET SDK 8 or newer (builds the small Windows bridge, cross-compiled; no Windows needed)
* `7z` (p7zip-full), `python3`, `curl`
* GTK 4 runtime libraries. **No `-dev` packages are needed**: `build-env.sh` generates a pkg-config shim and fetches the libLLVM that bindgen needs if it is missing.

## The official Windows installer

Download `nthlink-win64-current.exe` from nthlink.com. The build extracts `rootGo.dll` and the API key from it. Neither is committed to this repository.

## Build

```bash
packaging/build-appimage.sh /path/to/nthlink-win64-current.exe
# -> SkyLink-x86_64.AppImage
```

The script:
1. extracts `rootGo.dll` and the API key from the installer (`packaging/extract-installer.py`)
2. publishes `winhelper/` as a self-contained `rootgo-bridge.exe`
3. builds `nthlink` (GUI/CLI) and `nthlink-core` with cargo, and runs unit tests
4. assembles an AppDir and packs it with a pinned appimagetool

## Developing

```bash
. ./build-env.sh /tmp/skylink-build     # once per shell
cargo build --release
cargo test -p nthlink-common
NTHLINK_RESOURCES=build/AppDir/usr/lib/nthlink target/release/nthlink
```

Useful commands:
* `nthlink render [directory.json]` prints the leaf config that would be used
* `nthlink-core check --config leaf.conf` validates a leaf config
* `nthlink-core proxy --config leaf.conf` runs the tunnel as a local proxy without root

## The leaf patch
`third_party/leaf` is upstream [eycorsican/leaf](https://github.com/eycorsican/leaf) with one addition: when `LEAF_SOCKET_MARK` is set, every outbound socket gets that `SO_MARK` (`leaf/src/proxy/mod.rs`, `mark_socket`). Policy routing uses that mark to keep leaf's own traffic out of the tunnel.
