# SkyLink — nthLink for Linux

An **unofficial** Linux client for the [nthLink](https://www.nthlink.com) censorship-circumvention service, shipped as a single AppImage.
It talks to the same nthLink directory service and runs the same proxy configuration as the official Windows client, so you get the same servers and protocols (trojan over WebSocket/TLS with ECH, multiplexing and automatic failover).

> Not affiliated with or endorsed by nthLink. "nthLink" and its logo belong to their owners.

## Features

| Windows client | SkyLink on Linux |
|---|---|
| One-click connect / disconnect | ✅ GTK4 window, or `connect` from a terminal |
| Whole-system VPN (NetFilter driver) | ✅ TUN device with policy routing, all TCP/UDP and DNS |
| Server list from the nthLink directory service | ✅ uses the vendor's own client library |
| Landing page after connecting | ✅ (can be turned off) |
| News & notifications | ✅ |
| Feedback form | ✅ |
| Connection health check | ✅ |
| — | ➕ Proxy-only mode (SOCKS5/HTTP, no admin rights) |
| — | ➕ Falls back to the last working server list if the directory is unreachable |
| Per-app bypass, diagnostics page | ❌ not ported |

## Quick start

```bash
# 1. dependencies (Debian/Ubuntu; see docs/INSTALL.md for other distros)
sudo apt install wine libgtk-4-1 policykit-1

# 2. download the AppImage from the Releases page, then
chmod +x SkyLink-x86_64.AppImage
./SkyLink-x86_64.AppImage
```

Click **Connect**, enter your password when the system asks, and you are online through nthLink.
The first connection takes ~20 s while a private Wine environment is created.

## Usage

```
SkyLink-x86_64.AppImage                     open the window
SkyLink-x86_64.AppImage connect             connect from a terminal (Ctrl+C disconnects)
SkyLink-x86_64.AppImage proxy [S] [H]       local proxy only, no root (SOCKS5 :1080, HTTP :1087)
SkyLink-x86_64.AppImage fetch               show what the directory service returns
SkyLink-x86_64.AppImage cleanup             remove routes/DNS left by a killed tunnel
SkyLink-x86_64.AppImage --version
```

More in [docs/INSTALL.md](docs/INSTALL.md) (installing, desktop menu entry, troubleshooting) and [docs/BUILD.md](docs/BUILD.md) (building from source).

## How it works

```
 GTK4 window / CLI  (your user)
   │  1. get servers ──► rootgo-bridge.exe ─(Wine)─► rootGo.dll ──► nthLink directory service
   │  2. write leaf config (same generator as the Windows client, NetFilter → TUN)
   │  3. pkexec
   ▼
 nthlink-core  (root)
   ├─ leaf (embedded)  ── TUN "nthlink0", fake-IP DNS, trojan/ws/tls/ech/amux outbounds
   ├─ routing: leaf's sockets carry fwmark 0x6e74 → main table;
   │           everything else → table 0x6e74 → default dev nthlink0
   └─ DNS: systemd-resolved per-link (~.) or a restored-on-exit resolv.conf
```

* The system default route is never modified. The only route into the tunnel lives on the TUN device, so if anything crashes the kernel removes it and your normal connection comes back by itself.
* Leaf's own traffic is marked and bypasses the tunnel, so there are no routing loops. IPv6 is routed into the tunnel too, so it can't leak.
* The core stops and cleans up when you disconnect, quit the window, or the window crashes.

## Repository layout

```
crates/common   directory-service model, leaf config generator, Wine bridge driver, state
crates/core     nthlink-core: privileged tunnel (leaf + TUN + routing + DNS)
crates/gui      nthlink: GTK4 UI and CLI
winhelper       rootgo-bridge: tiny .NET console app that calls rootGo.dll under Wine
third_party/leaf  vendored leaf (Apache-2.0) with a small SO_MARK patch
packaging       AppImage build script, desktop entry, icon, installer extractor
```

## License

SkyLink's own code is Apache-2.0 (see `LICENSE`). leaf is Apache-2.0 by eycorsican.
`rootGo.dll` and the nthLink API key are **not** in this repository: the build script extracts them from the official Windows installer you supply.
