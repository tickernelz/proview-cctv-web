# Mobile lab: running the shipped app on an emulator in CI

The app we reverse is arm64-only. The 1.0.4 XAPK carries one ABI split,
`config.arm64_v8a.apk`, holding `lib/arm64-v8a/libFunSDK.so` (33,983,560 B),
`libTargetTrack.so` and `libbarhopper_v3.so`. There is no `armeabi-v7a` and no
`x86_64`, so an x86_64 emulator would only run it through ARM translation, which is
not a path we accept for a 34 MB vendor SDK.

## Where an arm64 emulator can run

| Host | arm64 image | Verdict |
|---|---|---|
| GitHub `ubuntu-*` (x86_64) | no | Google's emulator refuses arm64 guests on x86_64 hosts |
| GitHub `ubuntu-24.04-arm` | no | Google publishes no `linux-aarch64` emulator build (issuetracker 242699119); container Android (redroid, Waydroid) needs binder/ashmem modules a hosted runner will not load |
| GitHub `macos-14`/`macos-15` (Apple Silicon) | yes | the emulator runs arm64-v8a images natively since 30.0.26, and `android-emulator-runner` takes `arch: arm64-v8a` (Apple Silicon support since v2.20.0) |
| Device farm (BrowserStack, Firebase Test Lab) | real arm64 phones | runs the APK as shipped, but no proxy interception and no Frida, so it is a smoke test only |

`.github/workflows/mobile-re.yml` is the CI form of the third row: `macos-15`,
`api-level: 33`, `target: google_apis`, `arch: arm64-v8a`, emulator options
`-no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect`, then
`adb root`, install and a printed `ro.product.cpu.abi`.

## The payload is not in this repository

The repository is public and the vendor binaries are not ours to publish, so the
job fetches its payload at run time and verifies it by hash:

- default: Termux's arm64 build, which proves the arm64 path end to end (ABI,
  install, package manager, launch) without touching vendor code;
- `vendor_archive_url` + `vendor_archive_sha256` (workflow_dispatch inputs): a zip
  holding `com.proview.cctv.apk` and `config.arm64_v8a.apk`, installed with
  `adb install-multiple`. Host it somewhere the owner controls; it never enters git.

## Next steps once the vendor payload runs

1. `adb root` plus a writable system partition (`-writable-system`, which the
   workflow does not pass yet) lets a mitmproxy CA go into the system store,
   so TLS is decrypted without repackaging the APK and without tripping its
   signature check.
2. `frida-server` arm64 on the same emulator hooks
   `XMAccountAPI::IXMAccount::XM_Video_Login_XMAccount` (0xf3fc2c) and
   `GetServerDSNAndPort`, which yields the ALC host, the encrypted query and the
   `Set-Cookie` from inside the process. No pinning to defeat, no TLS to break.
3. That complements the in-tree client: the account login and the device list
   already ship (`crates/core/src/cloud/envelope.rs`, `protocol.md` 3B), and so do
   live view, playback and PTZ over the cloud (`protocol.md` 4C, 9A-9C), so the
   emulator route is for what the app does that the client does not reproduce
   yet. The emulator route is reproducible in CI instead of a manual phone
   capture, and the out-of-tree qemu-bionic harness of `protocol.md` section 3A
   stays a reference for how the envelope was learned.
