# Installing PROVIEW CCTV web

Two families of download come from the same tag:

- **Desktop app** — a native window (Tauri 2) with the engine embedded. Nothing
  listens on your network.
- **Server binary** — a headless HTTP server you can host yourself and open in a
  browser.

Every release also ships `SHA256SUMS` covering all files. The server archives
are `proview-server-<target>.tar.gz` (`.zip` for Windows) for `x86_64` and
`aarch64` Linux (`-unknown-linux-gnu`, `-unknown-linux-musl`), macOS
(`-apple-darwin`) and Windows (`-pc-windows-msvc`).

## Verifying a download

```sh
sha256sum -c --ignore-missing SHA256SUMS
# macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
# Windows (PowerShell): Get-FileHash .\<file> -Algorithm SHA256
```

## Windows

The installers are **unsigned** (no Windows code-signing certificate). Windows
SmartScreen will show "Windows protected your PC" and the publisher appears as
unknown. That is expected, not a sign of tampering — verify the checksum above.

1. Download the `.msi` (or `.exe` NSIS installer) for your architecture:
   `PROVIEW.CCTV_0.5.0_x64_en-US.msi` / `PROVIEW.CCTV_0.5.0_x64-setup.exe`, or
   `PROVIEW.CCTV_0.5.0_arm64_en-US.msi` / `PROVIEW.CCTV_0.5.0_arm64-setup.exe`.
2. Unblock the file before running it:
   ```powershell
   Unblock-File .\PROVIEW.CCTV_0.5.0_x64_en-US.msi
   ```
   (Or: right-click the file → Properties → tick **Unblock** → OK.)
3. Run the installer. On the SmartScreen prompt choose **More info** →
   **Run anyway**.
4. The app opens its own window; there is no browser tab, and its engine
   listens only on an ephemeral loopback port.

Path to a trusted signature later: SignPath Foundation (free for open source,
signs under their name) or Certum Open Source Code Signing (paid, signs under
yours). Both need enrolment outside this repository. The `release` workflow
already signs the `.msi` and `.exe` with `signtool` once the
`WINDOWS_CERT_PFX_BASE64` and `WINDOWS_CERT_PASSWORD` secrets exist; until then
it logs `SIGNING: skipped, no certificate secret configured`.

## macOS

The app is **unsigned** and **not notarized**: no signing identity is
configured, so the only signature is the ad-hoc one the linker puts on an Apple
silicon binary, which Apple silicon requires to run at all. Gatekeeper will
refuse the first launch.

1. Download the `.dmg` for your Mac: `PROVIEW.CCTV_0.5.0_aarch64.dmg` for
   Apple silicon, `PROVIEW.CCTV_0.5.0_x64.dmg` for Intel, or
   `PROVIEW.CCTV_0.5.0_universal.dmg`.
2. Open the disk image and drag the app to `/Applications`.
3. Right-click the app → **Open** → **Open**. (Double-clicking only shows the
   refusal dialog the first time.) On macOS 15 and later that bypass is gone:
   after the refusal choose **System Settings → Privacy & Security → Open
   Anyway**.
4. If macOS still blocks it, clear the quarantine flag:
   ```sh
   xattr -dr com.apple.quarantine "/Applications/PROVIEW CCTV.app"
   ```

Path to a signed build later: Apple Developer ID certificate + notarization.

## Linux

```sh
# AppImage (x86_64; PROVIEW.CCTV_0.5.0_aarch64.AppImage on ARM)
chmod +x PROVIEW.CCTV_0.5.0_amd64.AppImage
./PROVIEW.CCTV_0.5.0_amd64.AppImage

# Debian / Ubuntu (PROVIEW.CCTV_0.5.0_arm64.deb on ARM)
sudo apt install ./PROVIEW.CCTV_0.5.0_amd64.deb

# Fedora / RHEL (PROVIEW.CCTV-0.5.0-1.aarch64.rpm on ARM)
sudo rpm -i PROVIEW.CCTV-0.5.0-1.x86_64.rpm
```

The desktop build needs `webkit2gtk-4.1` at runtime (installed automatically by
the `.deb` and `.rpm`; the AppImage bundles it).

## Server binary

```sh
tar -xzf proview-server-x86_64-unknown-linux-gnu.tar.gz
./proview-server --version
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" PROVIEW_DATA_DIR="$HOME/.proview-cctv" \
  ./proview-server --listen 127.0.0.1:8787
```

A release build needs `ROCKET_SECRET_KEY` to start at all: 32 random bytes in
base64 (`openssl rand -base64 32`) or 64 hex characters, kept stable so sessions
survive a restart. Everything else can wait until the app is open.

Then open <http://127.0.0.1:8787> and press **Open Settings** under the sign-in
form: the cloud address, the camera address and the camera password are set
there and saved to `settings.json` in the data directory (`PROVIEW_DATA_DIR`).
The server has no default data directory: without `PROVIEW_DATA_DIR` the view
says saved values apply to this run only, and they are gone after a restart.
Values read once at startup carry a **Restart** tag; restart the process after
saving those. The settings answer only a request from the same machine to a
loopback address. `docs/settings.md` lists every key.

Put TLS in front of it (see `docs/deploy.md`); the binary itself speaks HTTP on
the loopback interface by default.

## ffmpeg

Neither download needs `ffmpeg` for live view, playback and downloads when the
webview or browser decodes H.265: the camera's stream is then muxed into fMP4 in
process. Windows (WebView2 or Chrome, with the OS HEVC decoder) and macOS
(WKWebView) decode it. Elsewhere, install `ffmpeg` on `PATH` or set the
"ffmpeg program" setting. The server runs it to transcode to H.264 for a webview
that cannot decode H.265 or for a stream that reorders frames, to turn a
cloud-storage clip into MP4 for playback in the page, and to cut a playback
thumbnail from a clip's first keyframe; on Windows it never opens a console
window.
