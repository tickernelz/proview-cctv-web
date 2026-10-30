# Installing PROVIEW CCTV web

Two families of download come from the same tag:

- **Desktop app** — a native window (Tauri 2) with the engine embedded. Nothing
  listens on your network.
- **Server binary** — a headless HTTP server you can host yourself and open in a
  browser.

Every release also ships `SHA256SUMS` covering all files.

## Verifying a download

```sh
sha256sum -c SHA256SUMS --ignore-missing proview-server-x86_64-unknown-linux-gnu.tar.gz
# macOS: shasum -a 256 -c SHA256SUMS --ignore-missing <file>
# Windows (PowerShell): Get-FileHash .\<file> -Algorithm SHA256
```

## Windows

The installers are **unsigned** (no Windows code-signing certificate). Windows
SmartScreen will show "Windows protected your PC" and the publisher appears as
unknown. That is expected, not a sign of tampering — verify the checksum above.

1. Download the `.msi` (or `.exe` NSIS installer) for your architecture:
   `PROVIEW.CCTV_0.4.0_x64_en-US.msi` / `PROVIEW.CCTV_0.4.0_x64-setup.exe`, or
   `PROVIEW.CCTV_0.4.0_arm64_en-US.msi` / `PROVIEW.CCTV_0.4.0_arm64-setup.exe`.
2. Unblock the file before running it:
   ```powershell
   Unblock-File .\PROVIEW.CCTV_0.4.0_x64_en-US.msi
   ```
   (Or: right-click the file → Properties → tick **Unblock** → OK.)
3. Run the installer. On the SmartScreen prompt choose **More info** →
   **Run anyway**.
4. The app opens its own window; there is no browser tab and no open port.

Path to a trusted signature later: SignPath Foundation (free for open source,
signs under their name) or Certum Open Source Code Signing (paid, signs under
yours). Both need enrolment outside this repository.

## macOS

The app is **unsigned** and **not notarized**; the bundler applies only an
ad-hoc signature (`codesign -s -`), which Apple silicon requires to run at all.
Gatekeeper will refuse the first launch.

1. Download the `.dmg` for your Mac: `PROVIEW.CCTV_0.4.0_aarch64.dmg` for
   Apple silicon, `PROVIEW.CCTV_0.4.0_x64.dmg` for Intel, or
   `PROVIEW.CCTV_0.4.0_universal.dmg`.
2. Open the disk image and drag the app to `/Applications`.
3. Right-click the app → **Open** → **Open**. (Double-clicking only shows the
   refusal dialog the first time.)
4. If macOS still blocks it, clear the quarantine flag:
   ```sh
   xattr -dr com.apple.quarantine "/Applications/PROVIEW CCTV.app"
   ```

Path to a signed build later: Apple Developer ID certificate + notarization.

## Linux

```sh
# AppImage (x86_64; PROVIEW.CCTV_0.4.0_aarch64.AppImage on ARM)
chmod +x PROVIEW.CCTV_0.4.0_amd64.AppImage
./PROVIEW.CCTV_0.4.0_amd64.AppImage

# Debian / Ubuntu (PROVIEW.CCTV_0.4.0_arm64.deb on ARM)
sudo apt install ./PROVIEW.CCTV_0.4.0_amd64.deb

# Fedora / RHEL (PROVIEW.CCTV-0.4.0-1.aarch64.rpm on ARM)
sudo rpm -i PROVIEW.CCTV-0.4.0-1.x86_64.rpm
```

The desktop build needs `webkit2gtk-4.1` at runtime (installed automatically by
the `.deb` and `.rpm`; the AppImage bundles it).

## Server binary

```sh
tar -xzf proview-server-x86_64-unknown-linux-gnu.tar.gz
./proview-server --version
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" ./proview-server --listen 127.0.0.1:8787
```

A release build needs `ROCKET_SECRET_KEY` to start at all: 32 random bytes in
base64 (`openssl rand -base64 32`) or 64 hex characters, kept stable so sessions
survive a restart. Everything else can wait until the app is open.

Then open <http://127.0.0.1:8787> and press **Settings** under the sign-in form:
the cloud address, the camera address and the camera password are set there and
saved to `settings.json` in the data directory (`PROVIEW_DATA_DIR`). Values read
once at startup are marked "needs restart"; restart the process after saving
those. `docs/settings.md` lists every key.

Put TLS in front of it (see `docs/deploy.md`); the binary itself speaks HTTP on
the loopback interface by default.

## ffmpeg

Neither download needs `ffmpeg` when the webview or browser decodes H.265: the
camera's stream is then muxed into fMP4 in process for live view, playback and
downloads. Windows (WebView2 or Chrome, with the OS HEVC decoder) and macOS
(WKWebView) decode it. Elsewhere, install `ffmpeg` on `PATH` or set the
"ffmpeg program" setting; the server uses it only to transcode H.265 to H.264
for a webview that cannot decode H.265, and on Windows it never opens a console
window.
