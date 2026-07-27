#!/bin/sh
set -eu

if test -z "${ANDROID_NDK_WINDOWS_ROOT:-}"; then
  echo "ANDROID_NDK_WINDOWS_ROOT is not set" >&2
  exit 1
fi

exec "${ANDROID_NDK_WINDOWS_ROOT}/ndk-build.cmd" "$@"
