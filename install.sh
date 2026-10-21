#!/bin/sh
set -eu
VERSION="0.1.0"
REPO="${NIRUSHOT_REPO:-nirucon/nirushot}"
PREFIX="${NIRUSHOT_PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
need(){ command -v "$1" >/dev/null 2>&1; }
say(){ printf '%s\n' "$*"; }
die(){ say "nirushot install: $*" >&2; exit 1; }
fetch(){ need curl && curl -fsSL "$1" -o "$2" || wget -qO "$2" "$1" || die "download failed"; }
case "${1:-}" in --build) need cargo || die 'cargo required'; (cd "$(dirname "$0")" && cargo build --release --locked); install -d "$BIN"; install -m755 "$(dirname "$0")/target/release/nirushot" "$BIN/nirushot" ;; --uninstall) rm -f "$BIN/nirushot" ;; "") d=$(mktemp -d); trap 'rm -rf "$d"' EXIT; fetch "https://github.com/$REPO/releases/download/v$VERSION/nirushot" "$d/nirushot"; fetch "https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS" "$d/SHA256SUMS"; sum=$(sha256sum "$d/nirushot"|cut -d' ' -f1); grep -q "^$sum  nirushot$" "$d/SHA256SUMS" || die 'checksum mismatch'; install -d "$BIN"; install -m755 "$d/nirushot" "$BIN/nirushot"; say "Installed NIRUSHOT $VERSION" ;; *) die "unknown option $1" ;; esac
