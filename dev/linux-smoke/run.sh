#!/usr/bin/env bash
# Build pgscope for Linux, run it under a virtual display, and capture a
# screenshot so the non-macOS window chrome can be inspected.
set -euo pipefail

OUT=/out
mkdir -p "$OUT"

# The repo is mounted read-only at /src. Copy it to a scratch dir so the Linux
# build never touches the host's node_modules or target/ — building straight
# from a bind mount would let pnpm purge the macOS install.
echo "==> copying source"
mkdir -p /app
cp -a /src/. /app/
cd /app
rm -rf node_modules src-tauri/target dist

echo "==> installing frontend deps"
CI=true pnpm install --frozen-lockfile

echo "==> building frontend"
pnpm build

echo "==> building Linux binary"
cd src-tauri
cargo build --release --bins --features tauri/custom-protocol
cd ..

BIN=src-tauri/target/release/pgscope
file "$BIN"

echo "==> starting Xvfb"
Xvfb :99 -screen 0 1500x950x24 &
XVFB_PID=$!
export DISPLAY=:99
sleep 2

echo "==> waiting for the database"
until pg_isready -h "${PGHOST:-host.docker.internal}" -p "${PGPORT:-54330}" -U pgscope >/dev/null 2>&1; do
  sleep 1
done

echo "==> launching pgscope"
"$BIN" > "$OUT/app.log" 2>&1 &
APP_PID=$!

# Give the webview time to boot, connect, and render.
sleep 25

if ! kill -0 "$APP_PID" 2>/dev/null; then
  echo "!! pgscope exited early" >&2
  cat "$OUT/app.log" >&2
  exit 1
fi

echo "==> window geometry"
xwininfo -root -tree | tee "$OUT/windows.txt"

echo "==> capturing screenshot"
import -window root "$OUT/linux-titlebar.png"

# Exercise the custom window controls. On non-macOS the titlebar draws its own
# traffic lights and wires them to the window_* commands; rendering them is not
# proof that they work. The lights are 12px circles at 8px gaps starting at
# x=14, vertically centred in the 42px bar.
echo "==> testing custom traffic lights"
apt-get update -qq && apt-get install -y -qq xdotool >/dev/null 2>&1

CLOSE_X=20; BAR_Y=21

# Only *close* is asserted here. Xvfb runs without a window manager, and
# minimize/maximize are WM operations — they are no-ops under a bare X server
# regardless of whether the wiring is correct, so testing them here would prove
# nothing. Close goes straight to the toolkit and is a real signal.
echo "-- clicking close (red); the app should exit"
xdotool mousemove $CLOSE_X $BAR_Y click 1
sleep 4

if kill -0 "$APP_PID" 2>/dev/null; then
  echo "!! close button did NOT terminate the app" | tee "$OUT/controls.txt"
  CONTROLS=fail
else
  echo "close button terminated the app as expected" | tee "$OUT/controls.txt"
  CONTROLS=ok
fi
echo "controls=$CONTROLS"

echo "==> app log"
cat "$OUT/app.log" || true

if [ "${CONTROLS:-fail}" != "ok" ]; then
  echo "!! window control smoke test failed" >&2
  exit 1
fi

kill "$APP_PID" 2>/dev/null || true
kill "$XVFB_PID" 2>/dev/null || true
echo "==> done; artifacts in $OUT"
