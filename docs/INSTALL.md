# Installing SkyLink

## Requirements

* x86_64 Linux with a desktop session (GNOME, KDE, XFCE, …)
* **GTK 4** runtime (Ubuntu 22.04+, Debian 12+, Fedora 36+, Arch)
* **Wine** (any recent version; 8+ recommended)
* **polkit** with an authentication agent (every mainstream desktop has one), for the password prompt when connecting
* `iproute2` (`ip`), which is always present

### Debian / Ubuntu / Mint
```bash
sudo apt install wine libgtk-4-1 policykit-1
```
### Fedora
```bash
sudo dnf install wine gtk4 polkit
```
### Arch / Manjaro
```bash
sudo pacman -S wine gtk4 polkit
```
### openSUSE
```bash
sudo zypper install wine gtk4 polkit
```

## Install the AppImage

```bash
mkdir -p ~/Applications
mv ~/Downloads/SkyLink-x86_64.AppImage ~/Applications/
chmod +x ~/Applications/SkyLink-x86_64.AppImage
~/Applications/SkyLink-x86_64.AppImage
```

### Add it to the application menu (optional)

```bash
mkdir -p ~/.local/share/applications ~/.local/share/icons/hicolor/64x64/apps
cd /tmp && ~/Applications/SkyLink-x86_64.AppImage --appimage-extract nthlink.png >/dev/null
cp /tmp/squashfs-root/nthlink.png ~/.local/share/icons/hicolor/64x64/apps/nthlink.png
cat > ~/.local/share/applications/skylink.desktop <<EOT
[Desktop Entry]
Type=Application
Name=SkyLink (nthLink)
Exec=$HOME/Applications/SkyLink-x86_64.AppImage
Icon=nthlink
Categories=Network;
Terminal=false
EOT
```
Or install [AppImageLauncher](https://github.com/TheAssassin/AppImageLauncher), which does this automatically.

## Using it

1. Start SkyLink and press **Connect**.
2. The first time, a private Wine environment is created (~20 s).
3. Enter your password in the system dialog. It is needed to create the network device.
4. When the status says **Connected**, all traffic goes through nthLink. The landing page opens in your browser (you can turn this off on the Home page).
5. Press **Disconnect**, or close the window, to restore your normal connection.

### Without admin rights: proxy mode
```bash
./SkyLink-x86_64.AppImage proxy            # SOCKS5 127.0.0.1:1080, HTTP 127.0.0.1:1087
./SkyLink-x86_64.AppImage proxy 9050 8118  # custom ports
```
Then set your browser's proxy to SOCKS5 `127.0.0.1:1080` with "proxy DNS through SOCKS" enabled, or HTTP `127.0.0.1:1087`.

### From a terminal / headless
```bash
./SkyLink-x86_64.AppImage connect   # asks for your sudo password, Ctrl+C disconnects
```

## Where things are stored

| Path | Contents |
|---|---|
| `~/.config/nthlink/` | client id, session data, settings.json |
| `~/.local/share/nthlink/wineprefix` | private Wine environment (about 300 MB, safe to delete) |
| `~/.cache/nthlink/` | generated config, last server list, logs, staged helper |
| `/run/nthlink/` | runtime state of the tunnel (root, cleared on reboot) |

`settings.json` options: `open_landing_page` (true/false) and `log_level` (`error`, `warn`, `info`, `debug`).

## Troubleshooting

| Problem | Fix |
|---|---|
| "Wine is required…" | Install wine (see above). |
| "no polkit authentication agent" | Use a desktop session, or run `connect` from a terminal (uses sudo). |
| "Authorization was cancelled" | You closed the password dialog. Try again. |
| Stuck at "Contacting the directory service" | Wait (up to 2.5 min, retried 3×). If it still fails, the last known servers are used. *About → Reset Wine environment* recreates Wine. |
| Connected but the first pages fail | Normal for the first 30–60 s while working servers are found. |
| Internet broken after a crash or forced kill | `./SkyLink-x86_64.AppImage cleanup` (reboot works too). |
| Want logs | *About → Open logs folder*, set `"log_level": "debug"` in settings.json. |
| AppImage won't start: "FUSE" error | `./SkyLink-x86_64.AppImage --appimage-extract-and-run` |

## Uninstall
```bash
./SkyLink-x86_64.AppImage cleanup
rm -rf ~/Applications/SkyLink-x86_64.AppImage ~/.config/nthlink ~/.local/share/nthlink ~/.cache/nthlink \
       ~/.local/share/applications/skylink.desktop
```
