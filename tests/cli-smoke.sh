#!/bin/sh
set -eu
BIN=${1:?usage: cli-smoke.sh BIN}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT INT TERM
export HOME=$T/home NIRUSHOT_FIXTURE=$T PATH=$T/bin:$PATH
mkdir -p "$HOME" "$T/bin"
printf '0\n' > "$T/notified"
cat > "$T/bin/slop" <<'EOF'
#!/bin/sh
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
"$BIN" --version | grep -q "NIRUSHOT 0.4.3"
if "$BIN" --delay nope >/dev/null 2>&1; then echo 'FAIL invalid delay accepted' >&2; exit 1; fi
if "$BIN" --save >/dev/null 2>&1; then echo 'FAIL missing save path accepted' >&2; exit 1; fi
"$BIN" --region --save "$T/region.png" > "$T/out"
grep -q -- "--quiet -g 100x50+20+30 " "$T/maim"
grep -q 'nirushot-.*\.png$' "$T/maim"
[ ! -e "$T/xclip" ]
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
echo 'PASS: region/full capture, save, clipboard and notification'
