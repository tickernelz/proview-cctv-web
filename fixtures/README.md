# Fixtures

The fixture provider exists so the web layer can be exercised without a real
account or camera. It is not a camera emulator: it serves recorded device
metadata and, if present, recorded HLS playlists.

- `account.json` — the local fixture credentials (not a vendor account).
- `devices.json` — what the device list returns.
- `cloud-envelope.json` — vectors for the signed v3 envelope: scramble rows,
  `tsid`/`sig`/`key16` triples, and one AES body pair. The identity inside is a
  demo one; the outputs were produced by the reference implementation that
  reproduces the 27 `GetEncryptStr` rows captured from `libFunSDK.so`
  (ZHA-3940). No account, password, or vendor key material is stored here.
- `<device-id>_ch<N>_<main|sub>.m3u8` — optional recorded playlist for a channel.
  Recording stays outside this repository; drop files here at run time.

Fixture mode is the default. `PROVIEW_PROVIDER=proview` switches to the vendor
cloud provider, which is where the unimplemented protocol pieces live.
