#!/usr/bin/env bash
set -u

failures=0

check_command() {
  if command -v "$1" >/dev/null 2>&1; then
    printf 'ok   %-18s %s\n' "$1" "$(command -v "$1")"
  else
    printf 'miss %-18s %s\n' "$1" "$2"
    failures=$((failures + 1))
  fi
}

check_pkg() {
  if pkg-config --exists "$1"; then
    printf 'ok   pkg-config:%-7s %s\n' "$1" "$(pkg-config --modversion "$1")"
  else
    printf 'miss pkg-config:%-7s %s\n' "$1" "$2"
    failures=$((failures + 1))
  fi
}

check_command cargo "install Rust with rustup"
check_command rustc "install Rust with rustup"
check_command node "install Node.js 20 or newer"
check_command npm "install npm"
check_command gcc "install build-essential"
check_command make "install build-essential"
check_command pkg-config "install pkg-config"
check_command cmake "install cmake"

if command -v pkg-config >/dev/null 2>&1; then
  check_pkg gtk+-3.0 "install libgtk-3-dev"
  check_pkg webkit2gtk-4.1 "install libwebkit2gtk-4.1-dev"
  check_pkg alsa "install libasound2-dev"
fi

if [[ $failures -ne 0 ]]; then
  printf '\n%d required development dependency check(s) failed.\n' "$failures"
  exit 1
fi

printf '\nDesktop development environment is ready.\n'
