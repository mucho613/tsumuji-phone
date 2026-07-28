#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$root/vendor/pjproject"
prefix="$root/target/pjproject"
jobs="${JOBS:-$(nproc)}"

if [[ ! -f "$source_dir/aconfigure" ]]; then
  printf 'PJSIP submodule is missing. Run: git submodule update --init\n' >&2
  exit 1
fi

mkdir -p "$prefix"
cd "$source_dir"

./configure \
  --prefix="$prefix" \
  --disable-video \
  --disable-sdl \
  --disable-ffmpeg \
  --disable-v4l2 \
  --disable-openh264 \
  --disable-libyuv \
  --disable-opus \
  --disable-gsm-codec \
  --disable-g722-codec \
  --disable-g7221-codec \
  --disable-ilbc-codec \
  --disable-l16-codec

make dep
make -j"$jobs" lib
make install

printf '\nPJSIP installed under %s\n' "$prefix"
printf 'Cargo will discover this installation automatically.\n'
