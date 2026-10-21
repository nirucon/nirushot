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
printf 'PNG' > "$last"
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
"$BIN" --version | grep -q "NIRUSHOT 0.4.0"
"$BIN" --region --save "$T/region.png" > "$T/out"
grep -qxF -- "--quiet -g 100x50+20+30 $T/region.png" "$T/maim"
[ ! -e "$T/xclip" ]
[ -f "$T/region.png" ]
for _ in $(seq 1 100); do grep -qxF 1 "$T/notified" && break; sleep .02; done
grep -qxF 1 "$T/notified"
rm -f "$T/notified"
"$BIN" --full --copy --no-notify
for _ in $(seq 1 100); do [ -e "$T/xclip" ] && break; sleep .02; done
grep -qxF -- "-selection clipboard -t image/png -i" "$T/xclip"
[ ! -e "$T/notified" ]
[ ! -f "/tmp/nirushot-$$.png" ]
echo 'PASS: region/full capture, save, clipboard and notification'
