#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

frames='[0:v]drawbox=x=250:y=140:w=140:h=80:c=0xE0C040@1:t=fill[top];[top][1:v]vstack=inputs=2,format=yuv420p[v]'

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "color=c=0x203040:s=640x360:r=10:d=2" \
  -f lavfi -i "testsrc2=s=640x360:r=10:d=2" \
  -filter_complex "$frames" -map "[v]" \
  -c:v libx264 -profile:v baseline -level 3.0 -g 10 -keyint_min 10 -sc_threshold 0 -crf 34 \
  -f h264 dual-lens-stacked-640x720.h264

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "color=c=0x203040:s=640x360:r=10:d=2" \
  -f lavfi -i "testsrc2=s=640x360:r=10:d=2" \
  -filter_complex "$frames" -map "[v]" \
  -c:v libx264 -profile:v baseline -level 3.0 -g 10 -keyint_min 10 -sc_threshold 0 -crf 34 \
  -movflags +empty_moov+default_base_moof+frag_keyframe+separate_moof+omit_tfhd_offset \
  -f mp4 dual-lens-stacked-640x720.mp4

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=s=1920x1080:r=5:d=0.4" \
  -c:v libx265 -x265-params log-level=none -f hevc "$tmp/clip.hevc"
python3 - "$tmp/clip.hevc" hevc-1920x1080.sps <<'PY'
import sys

raw = open(sys.argv[1], 'rb').read()
starts = []
index = 0
while index < len(raw) - 4:
    if raw[index:index + 4] == b'\x00\x00\x00\x01':
        starts.append((index, 4))
        index += 4
    elif raw[index:index + 3] == b'\x00\x00\x01':
        starts.append((index, 3))
        index += 3
    else:
        index += 1
for position, (offset, length) in enumerate(starts):
    end = starts[position + 1][0] if position + 1 < len(starts) else len(raw)
    nal = raw[offset + length:end]
    if ((nal[0] >> 1) & 0x3f) == 33:
        open(sys.argv[2], 'wb').write(nal)
        break
else:
    raise SystemExit('no SPS NAL in the generated clip')
PY

ffprobe -v error -show_entries stream=codec_name,width,height -of default=noprint_wrappers=1 dual-lens-stacked-640x720.mp4
