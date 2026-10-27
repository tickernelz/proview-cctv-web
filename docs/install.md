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

1. Download the `.msi` (or `.exe` NSIS installer) for your architecture.
2. Unblock the file before running it:
   ```powershell
   Unblock-File .\PROVIEW-CCTV_x64-setup.msi
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

1. Download the `.dmg` for your Mac (`aarch64` for Apple silicon,
   `x86_64` for Intel, or `universal`).
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
# AppImage
chmod +x PROVIEW-CCTV_0.2.0_amd64.AppImage
./PROVIEW-CCTV_0.2.0_amd64.AppImage

# Debian / Ubuntu
sudo apt install ./proview-cctv_0.2.0_amd64.deb

# Fedora / RHEL
sudo rpm -i proview-cctv-0.2.0-1.x86_64.rpm
```

The desktop build needs `webkit2gtk-4.1` at runtime (installed automatically by
the `.deb` and `.rpm`; the AppImage bundles it).

## Server binary

```sh
tar -xzf proview-server-x86_64-unknown-linux-gnu.tar.gz
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" ./proview-server --listen 127.0.0.1:8787
```

Put TLS in front of it (see `docs/deploy.md`); the binary itself speaks HTTP on
the loopback interface by default.
