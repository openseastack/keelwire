# Installing Keelwire

Download the file for your computer from the project's **Releases** page. Nothing else needs
to be installed: the analyzer, its web interface and the NMEA 2000 message database are all
inside the one download.

## Mac (macOS 11 or later, Apple Silicon or Intel)

1. Open `Keelwire_<version>_universal.dmg`.
2. Drag **Keelwire** onto the **Applications** folder.
3. Open it from Applications or Launchpad.

If the release isn't signed yet, macOS says the app "can't be opened because Apple cannot check
it". Open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** (or
right-click the app → **Open**). This is needed once.

## Windows 10 / 11

1. Run `Keelwire_<version>_x64-setup.exe`. It installs for your user only, so no
   administrator rights are needed.
2. Start **Keelwire** from the Start menu.

If the release isn't signed yet, Windows SmartScreen says "Windows protected your PC": click
**More info → Run anyway**. IT departments can use the `.msi` instead.

The app uses Microsoft Edge WebView2, which is part of Windows 11 and current Windows 10. On an
older Windows 10 that lacks it, the installer adds it automatically (this one step needs internet).

## Linux

- **Any distribution:** download `Keelwire_<version>_amd64.AppImage`, mark it executable
  (Properties → Permissions, or `chmod +x`), then double-click it.
- **Debian / Ubuntu:** `sudo apt install ./Keelwire_<version>_amd64.deb`
- **Fedora:** `sudo dnf install ./Keelwire-<version>-1.x86_64.rpm`

USB gateways (Actisense NGT-1, Yacht Devices USB) appear as serial ports. To use them without
sudo, add yourself to the serial group once and log in again: `sudo usermod -aG dialout $USER`.

## Raspberry Pi aboard (no screen needed)

Run the analyzer on a Pi with a CAN HAT on the backbone, then open it from any phone, tablet or
laptop on the boat's Wi-Fi.

1. Download `keelwire-<version>-raspberry-pi-64bit.tar.gz` (or `-32bit` for older
   Raspberry Pi OS) and unpack it: `tar xzf keelwire-*.tar.gz`.
2. Try it: `./keelwire --headless --interface socketcan:can0`
3. Open `http://<the Pi's address>:8080` on your phone.
4. To start it at boot, copy the program to `/usr/local/bin/` and install the included
   `keelwire.service` (see the comments inside it).

Add `--token <secret>` on shared networks so only people with the token can connect, and
`--allow-tx` if changes and switching should be possible from the boat's network.

## No install at all (portable)

The `keelwire-<version>-<platform>` archives hold a single program. Run it and the
interface opens in your web browser. Handy on a borrowed laptop or a USB stick.

## First start

With no gateway chosen, the first screen lists the USB gateways it finds and asks for an
IPG100 or Wi-Fi gateway address. **Try the simulated boat** shows everything without hardware.

Changes to devices and switching are locked until you press **Changes locked** at the top of
the window. They lock again after 15 minutes without changes. The desktop app can be made
read-only for good by starting it with `--read-only`.
