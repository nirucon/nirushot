#!/bin/sh
set -eu
BIN=${1:?usage: cli-smoke.sh BIN}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT INT TERM
export HOME=$T/home NIRUSHOT_FIXTURE=$T PATH=$T/bin:$PATH TMPDIR=$T/tmp
mkdir -p "$HOME" "$T/bin" "$TMPDIR"
printf '0\n' > "$T/notified"
cat > "$T/bin/slop" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" > "$NIRUSHOT_FIXTURE/slop"
printf '100x50+20+30\n'
EOF
cat > "$T/bin/maim" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" > "$NIRUSHOT_FIXTURE/maim"
for last; do :; done
python3 - "$last" <<'PY'
import struct, sys, zlib
w, h = 4, 3
raw = b''.join(b'\x00' + bytes((20, 40, 60, 255)) * w for _ in range(h))
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
with open(sys.argv[1], 'wb') as out:
    out.write(b'\x89PNG\r\n\x1a\n')
    out.write(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)))
    out.write(chunk(b'IDAT', zlib.compress(raw)))
    out.write(chunk(b'IEND', b''))
PY
EOF
cat > "$T/bin/xclip" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" > "$NIRUSHOT_FIXTURE/xclip"
EOF
cat > "$T/bin/notify-send" <<'EOF'
#!/bin/sh
printf '1\n' > "$NIRUSHOT_FIXTURE/notified"
EOF
chmod +x "$T/bin/"*
"$BIN" --version | grep -q "NIRUSHOT 0.7.0"
if "$BIN" --delay nope >/dev/null 2>&1; then echo 'FAIL invalid delay accepted' >&2; exit 1; fi
if "$BIN" --save >/dev/null 2>&1; then echo 'FAIL missing save path accepted' >&2; exit 1; fi
# The colour picker parses its options before it touches the display.
"$BIN" pick --help | grep -q 'nirushot pick'
set +e
"$BIN" pick --format cmyk >/dev/null 2>&1; rc=$?
env -u DISPLAY "$BIN" pick --no-notify >/dev/null 2>"$T/pick.err"; rc2=$?
set -e
[ "$rc" = 2 ] || { echo "FAIL pick accepted an unknown format (rc=$rc)" >&2; exit 1; }
[ "$rc2" = 2 ] && grep -q 'cannot open the display' "$T/pick.err" || { echo "FAIL pick without a display (rc=$rc2)" >&2; exit 1; }
[ ! -e "$HOME/.local/state/nirushot/colors.json" ] || { echo 'FAIL failed pick wrote history' >&2; exit 1; }
"$BIN" --region --save "$T/region.png" > "$T/out"
grep -q -- "-b 4 -c 1.0,0.72,0.15,1.0" "$T/slop"
grep -q -- "--quiet -g 100x50+20+30 " "$T/maim"
grep -q 'nirushot-.*\.png$' "$T/maim"
[ -e "$T/xclip" ]
grep -qxF -- "-selection clipboard -t image/png -i" "$T/xclip"
[ -f "$T/region.png" ]
file "$T/region.png" | grep -q 'PNG image data' || { echo 'FAIL saved file is not a PNG' >&2; exit 1; }
for _ in $(seq 1 100); do grep -qxF 1 "$T/notified" && break; sleep .02; done
grep -qxF 1 "$T/notified"
rm -f "$T/notified"
"$BIN" --full --copy --no-notify
for _ in $(seq 1 100); do [ -e "$T/xclip" ] && break; sleep .02; done
grep -qxF -- "-selection clipboard -t image/png -i" "$T/xclip"
[ ! -e "$T/notified" ]
[ ! -f "/tmp/nirushot-$$.png" ]
# A clipboard helper that starts but exits unsuccessfully must make --copy fail.
cat > "$T/bin/xclip" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$T/bin/xclip"
if "$BIN" --full --copy --no-notify >"$T/copy-fail.out" 2>"$T/copy-fail.err"; then
  echo 'FAIL copy reported success when xclip failed' >&2
  exit 1
fi
grep -q 'xclip exited with' "$T/copy-fail.err" || {
  cat "$T/copy-fail.err" >&2
  echo 'FAIL copy failure did not explain xclip' >&2
  exit 1
}
# No private capture directory may outlive a run, including a failed one.
if ls "$TMPDIR"/nirushot-* >/dev/null 2>&1; then echo 'FAIL capture directory left behind' >&2; exit 1; fi
# The shutter sound is one line to a running NIRUNOTIFY, and nothing at all
# without one.
export XDG_RUNTIME_DIR="$T/run"
mkdir -p "$XDG_RUNTIME_DIR"
printf '#!/bin/sh\nexit 0\n' > "$T/bin/xclip"
"$BIN" --full --copy --no-notify >/dev/null
SOCK="$XDG_RUNTIME_DIR/nirunotify-$(id -u).sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1]); s.listen(1); s.settimeout(10)
c, _ = s.accept()
data = c.recv(64)
c.sendall(b"ok\n"); c.close()
open(sys.argv[2], "wb").write(data)
' "$SOCK" "$T/sound" &
for _ in $(seq 1 100); do [ -S "$SOCK" ] && break; sleep .02; done
"$BIN" --full --copy --no-notify >/dev/null
wait
grep -qxF 'sound screenshot' "$T/sound" || { echo 'FAIL no shutter sound request' >&2; exit 1; }
# OCR parses its options first, and without tesseract it says how to get it.
"$BIN" ocr --help | grep -q 'nirushot ocr'
printf '0\n' > "$T/notified"
set +e
"$BIN" ocr --psm 99 >/dev/null 2>&1; rc=$?
PATH="$T/bin" "$BIN" ocr >/dev/null 2>"$T/ocr.err"; rc2=$?
set -e
[ "$rc" = 2 ] || { echo "FAIL ocr accepted --psm 99 (rc=$rc)" >&2; exit 1; }
[ "$rc2" = 2 ] && grep -q 'tesseract is not installed' "$T/ocr.err" && grep -Eq 'tesseract-(ocr|data)-swe' "$T/ocr.err" || { cat "$T/ocr.err" >&2; echo "FAIL ocr without tesseract (rc=$rc2)" >&2; exit 1; }
for _ in $(seq 1 100); do grep -qxF 1 "$T/notified" && break; sleep .02; done
grep -qxF 1 "$T/notified" || { echo 'FAIL ocr without tesseract did not notify' >&2; exit 1; }
# A fake tesseract with English only: the text is tidied, copied and printed,
# and the missing Swedish data is named.
cat > "$T/bin/tesseract" <<'TESS'
#!/bin/sh
case "$1" in
  --list-langs) printf 'List of available languages in "/x/" (2):\neng\nosd\n'; exit 0 ;;
  --version) echo 'tesseract 5.5.0'; exit 0 ;;
esac
printf '%s\n' "$*" > "$NIRUSHOT_FIXTURE/tesseract.args"
printf 'Hello  \n\n\n\nworld\n\f'
TESS
cat > "$T/bin/xclip" <<'CLIP'
#!/bin/sh
printf '%s\n' "$*" > "$NIRUSHOT_FIXTURE/xclip"
cat > "$NIRUSHOT_FIXTURE/clip.txt"
CLIP
chmod +x "$T/bin/tesseract" "$T/bin/xclip"
"$T/bin/maim" x "$T/ocr.png"
"$BIN" ocr --image "$T/ocr.png" --no-notify > "$T/ocr.out" 2>"$T/ocr.err"
printf 'Hello\n\nworld\n' | cmp -s - "$T/ocr.out" || { cat "$T/ocr.out" >&2; echo 'FAIL ocr output' >&2; exit 1; }
printf 'Hello\n\nworld' | cmp -s - "$T/clip.txt" || { echo 'FAIL ocr clipboard' >&2; exit 1; }
grep -qxF -- "-selection clipboard -i" "$T/xclip"
grep -q -- '-l eng --dpi 288' "$T/tesseract.args" || { cat "$T/tesseract.args" >&2; echo 'FAIL tesseract arguments' >&2; exit 1; }
grep -q 'Missing language data: swe' "$T/ocr.err" || { echo 'FAIL missing language not named' >&2; exit 1; }
# The runner may set XDG_CONFIG_HOME; the config must come from this home.
export XDG_CONFIG_HOME="$HOME/.config"
mkdir -p "$HOME/.config/nirushot"
printf '[ocr]\nlanguages = "eng"\npsm = 6\nkeep_newlines = false\n' > "$HOME/.config/nirushot/config.toml"
"$BIN" ocr --image "$T/ocr.png" --no-notify > "$T/ocr.out" 2>"$T/ocr.err"
grep -qxF 'Hello world' "$T/ocr.out" || { echo 'FAIL keep_newlines = false' >&2; exit 1; }
grep -q -- '--psm 6' "$T/tesseract.args" || { echo 'FAIL psm from the config' >&2; exit 1; }
[ ! -s "$T/ocr.err" ] || { cat "$T/ocr.err" >&2; echo 'FAIL configured eng still reported missing languages' >&2; exit 1; }
"$BIN" ocr --check > "$T/check.out"
grep -q 'using: eng' "$T/check.out"
set +e
"$BIN" ocr --check --languages swe+eng > "$T/check.out"; rc=$?
set -e
[ "$rc" = 1 ] && grep -q 'Missing language data: swe' "$T/check.out" || { cat "$T/check.out" >&2; echo "FAIL ocr --check (rc=$rc)" >&2; exit 1; }
if ls "$TMPDIR"/nirushot-ocr-* >/dev/null 2>&1; then echo 'FAIL OCR directory left behind' >&2; exit 1; fi
echo 'PASS: pick options, region/full capture, save, clipboard, failed-copy detection, cleanup, notification, shutter sound and OCR'
