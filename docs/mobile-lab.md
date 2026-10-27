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
| GitHub `ubuntu-24.04-arm` | no | the SDK index (`repository2-3.xml`, read 2026-10-27) ships the emulator for `linux x64`, `macosx x64`, `macosx aarch64` and `windows x64` only, so no `linux-aarch64` build exists (issuetracker 242699119); container Android (redroid, Waydroid) needs binder/ashmem modules a hosted runner will not load |
| GitHub `macos-14`/`macos-15` (Apple Silicon) | image installs, hypervisor does not | the runner is an Apple M1 VM without nested virtualization, so QEMU cannot create the guest: `HVF error: HV_UNSUPPORTED` within 5 s, six image and flag variants (below) |
| Self-hosted Apple Silicon Mac (M-series, bare metal) | yes | HVF works on real hardware and the emulator runs `arm64-v8a` natively; this is the only hosted-free host that can boot the image |
| Device farm (BrowserStack, Firebase Test Lab) | real arm64 phones | runs the APK as shipped, but no proxy interception and no Frida, so it is a smoke test only |

## Why the hosted macOS path is closed

Measured 2026-10-27 on `macos-15` (image `macos-15-arm64/20260907.0337`,
`Apple M1 (Virtual)`, emulator 37.1.11 and 37.2.12):

- `system-images;android-33;google_apis;arm64-v8a` installs and the AVD is created,
  then the emulator exits inside 5 s with `HVF error: HV_UNSUPPORTED`,
  `qemu-system-aarch64-headless: failed to initialize HVF: Invalid argument` and
  `QEMU main loop exits abnormally with code 1`, while `adb devices` stays empty.
  The same happens for api 30, 33, 34 and 35 `google_apis`, for
  `-gpu swiftshader_indirect` and `-gpu host`, and with and without
  `--device pixel_6` at `hw.ramSize` 2048 and 4096 (run 40432400695, eight jobs).
- The host cannot virtualise: `hw.optional.arm.FEAT_EL2` is absent and
  `kern.hv_support` is an unknown oid. `emulator -accel-check` still prints
  `accel: 0 Hypervisor.Framework OS X Version 15.7`, which is why the failure only
  appears at `hv_vm_create` and shows up as a ten-minute `sys.boot_completed` poll.
- Software acceleration is not a fallback: `-accel off` is ignored for the arm64
  guest, and `-accel off -qemu -accel tcg` still initialises HVF and dies the same
  way, with a segfault 11 (run 40435003804).
- GitHub states the limit itself: "Nested-virtualization is not supported due to the
  limitation of Apple's Virtualization Framework" (GitHub-hosted runners reference,
  arm64 macOS limitations), and `actions/runner-images#9472` is closed with that
  explanation.

So `.github/workflows/mobile-re.yml` keeps running on `macos-15` as the probe that
would notice a change: it installs api 33 `google_apis` `arm64-v8a`, boots in two
attempts (HVF, then HVF with the TCG accelerator), prints the emulator's own log the
moment the process dies, and fails in about two minutes instead of polling for ten.
Every run uploads `host.txt`, `emulator-*.log` and `result.txt`, so a future
host that does boot records its own `arm64-v8a` ABI without a code change.

To get the payload running today the job needs a host with a working hypervisor:
a self-hosted Apple Silicon Mac with `runs-on: [self-hosted, macOS, ARM64]`, or the
device farm row for an install-and-launch smoke test. Nothing else in the job
changes: `ro.product.cpu.abi`, `pm list packages` and `dumpsys activity` are the
acceptance output either way.

## The payload is not in this repository

The repository is public and the vendor binaries are not ours to publish, so the
job fetches its payload at run time and verifies it by hash:

- default: Termux's arm64 build, which proves the arm64 path end to end (ABI,
  install, package manager, launch) without touching vendor code;
- `vendor_archive_url` + `vendor_archive_sha256` (workflow_dispatch inputs): a zip
  holding `com.proview.cctv.apk` and `config.arm64_v8a.apk`, installed with
  `adb install-multiple`. Host it somewhere the owner controls; it never enters git.

## Next steps once the vendor payload runs

1. On the self-hosted host, adding `-writable-system` to the boot flags plus
   `adb root` lets a mitmproxy CA go into the system store, so TLS is decrypted
   without repackaging the APK and without tripping its signature check.
2. `frida-server` arm64 on the same emulator hooks
   `XMAccountAPI::IXMAccount::XM_Video_Login_XMAccount` (0xf3fc2c) and
   `GetServerDSNAndPort`, which yields the ALC host, the encrypted query and the
   `Set-Cookie` from inside the process. No pinning to defeat, no TLS to break.
3. That supersedes the qemu-bionic harness of `notes/08` for anything that needs
   the network, and it is reproducible in CI instead of a manual phone capture.
