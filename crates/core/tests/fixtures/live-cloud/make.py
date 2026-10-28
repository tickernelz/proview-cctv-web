#!/usr/bin/env python3
import argparse
import hashlib
import os
import struct

KEY_BYTES = 128
MASK_WINDOW = 128
DHAV_HEADER = 16
SALT_MOD = 253
MATERIAL_BYTES = 40
MD5_STRIDE = 4
MD5_INPUT = 8
HEAD_BODY = 8192
AUDIO_BODY = 328


def derive(material):
    raw = material.encode()
    buf = bytearray(MATERIAL_BYTES)
    buf[: min(len(raw), MATERIAL_BYTES)] = raw[:MATERIAL_BYTES]
    key = bytearray(KEY_BYTES)
    for round_at in range(KEY_BYTES // 16):
        start = round_at * MD5_STRIDE
        key[round_at * 16 : round_at * 16 + 16] = hashlib.md5(
            bytes(buf[start : start + MD5_INPUT])
        ).digest()
    return bytes(key)


def mask(payload, declared, key):
    frame_len = (declared + DHAV_HEADER) & 0xFFFFFFFF
    limit = min(frame_len, MASK_WINDOW, len(payload))
    salt = frame_len % SALT_MOD
    shift = frame_len % MASK_WINDOW
    out = bytearray(payload)
    for i in range(limit):
        source = ((key[i] % MASK_WINDOW) + shift) % MASK_WINDOW
        out[i] ^= key[source] ^ salt
    return bytes(out)


def nal_starts(es):
    at = 0
    while True:
        found = es.find(b"\x00\x00\x01", at)
        if found < 0:
            return
        if found > 0 and es[found - 1] == 0:
            yield found - 1, (es[found + 3] >> 1) & 0x3F
        else:
            yield found, (es[found + 3] >> 1) & 0x3F
        at = found + 3


def split_access_units(es):
    starts = list(nal_starts(es))
    boundaries = [offset for offset, nal_type in starts if nal_type == 35]
    units = []
    for index, start in enumerate(boundaries):
        end = boundaries[index + 1] if index + 1 < len(boundaries) else len(es)
        unit = es[start:end]
        units.append((unit, is_irap(unit)))
    return units


def is_irap(unit):
    return any(16 <= nal_type <= 21 for _, nal_type in nal_starts(unit))


def head(cls, declared, stamp):
    if cls == 0xFC:
        return (
            b"\x00\x00\x01\xfc"
            + b"\x53\x0c\x20\x44"
            + struct.pack("<I", stamp)
            + struct.pack("<I", declared)
        )
    return b"\x00\x00\x01\xfd" + struct.pack("<I", declared)


def marker():
    return b"\x00\x00\x01\xf9" + bytes(12)


def audio_body():
    return b"\x00\x00\x01\xfa" + bytes(AUDIO_BODY - 4)


def chunked(prefix, payload, first_capacity, rest_capacity):
    bodies = [prefix + payload[:first_capacity]]
    rest = payload[first_capacity:]
    while rest:
        bodies.append(rest[:rest_capacity])
        rest = rest[rest_capacity:]
    return bodies


def frame_bodies(unit, keyframe, stamp, key):
    declared = len(unit)
    if keyframe:
        payload = unit if key is None else mask(unit, declared, key)
        bodies = [marker()]
        bodies.extend(
            chunked(head(0xFC, declared, stamp), payload, HEAD_BODY - DHAV_HEADER, HEAD_BODY)
        )
        return bodies
    bodies = chunked(head(0xFD, declared, 0), unit, HEAD_BODY - 8, HEAD_BODY)
    bodies.append(audio_body())
    return bodies


def write_bodies(path, bodies):
    with open(path, "wb") as handle:
        for entry in bodies:
            handle.write(struct.pack("<I", len(entry)))
            handle.write(entry)
    return sum(len(entry) for entry in bodies)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, help="HEVC Annex-B elementary stream")
    parser.add_argument("--out", required=True, help="fixture directory")
    parser.add_argument("--material", default="zha4084-live-cloud-fixture-material-0001")
    parser.add_argument("--plain-frames", type=int, default=5)
    parser.add_argument("--label", default="1280x720")
    args = parser.parse_args()

    es = open(args.source, "rb").read()
    units = split_access_units(es)
    keyframes = sum(1 for _, irap in units if irap)
    key = derive(args.material)
    stamp = 0x6A9F2C00

    keyed = []
    plain = []
    for index, (unit, irap) in enumerate(units):
        keyed.extend(frame_bodies(unit, irap, stamp + index, key))
        if index < args.plain_frames:
            plain.extend(frame_bodies(unit, irap, stamp + index, None))

    last_head = [entry for entry in keyed if entry[3] == 0xFC][-1]
    declared = struct.unpack_from("<I", last_head, 12)[0]
    keyed.append(marker())
    keyed.append(head(0xFC, declared, stamp + 99) + last_head[DHAV_HEADER : DHAV_HEADER + 4096])

    os.makedirs(args.out, exist_ok=True)
    keyed_name = f"cloud-keyed-{args.label}.bodies"
    plain_name = f"cloud-plain-{args.label}.bodies"
    keyed_bytes = write_bodies(os.path.join(args.out, keyed_name), keyed)
    plain_bytes = write_bodies(os.path.join(args.out, plain_name), plain)
    print(f"source {len(es)} B, {len(units)} access units, {keyframes} keyframes")
    print(f"keyed {len(keyed)} bodies {keyed_bytes} B -> {keyed_name}")
    print(f"plain {len(plain)} bodies {plain_bytes} B -> {plain_name}")
    print(f"es sha256 {hashlib.sha256(es).hexdigest()}")


if __name__ == "__main__":
    main()
