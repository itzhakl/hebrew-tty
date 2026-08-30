#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ ! -t 0 ] || [ ! -t 1 ]; then
  echo "smoke-ptyxis: stdin and stdout must be the active terminal" >&2
  exit 1
fi
if [ -z "${VTE_VERSION:-}" ]; then
  echo "smoke-ptyxis: VTE_VERSION is not set; run this in Ptyxis/VTE" >&2
  exit 1
fi

cargo test --test caret_mapping --test screen_layout >/dev/null
cargo build --quiet

report=$(python3 - "$root/target/debug/hebrew-tty" <<'PY'
import os
import re
import subprocess
import sys
import termios
import tty

binary = sys.argv[1]
fd = os.open("/dev/tty", os.O_RDWR)
columns = os.get_terminal_size(fd).columns
original = termios.tcgetattr(fd)
try:
    tty.setraw(fd)
    os.write(fd, b"\r\x1b[2K")
    subprocess.run(
        [
            binary,
            "--mode",
            "logical",
            "python3",
            "-c",
            "import os; os.write(1, 'אבגד'.encode())",
        ],
        stdin=fd,
        stdout=fd,
        stderr=fd,
        check=True,
    )
    os.write(fd, b"\x1b[6n")
    response = bytearray()
    while not response.endswith(b"R"):
        chunk = os.read(fd, 1)
        if not chunk:
            raise SystemExit("terminal closed before cursor report")
        response.extend(chunk)
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, original)
    os.close(fd)

match = re.fullmatch(rb"\x1b\[(\d+);(\d+)R", bytes(response))
if not match:
    raise SystemExit(f"invalid cursor report: {bytes(response)!r}")
reported_column = int(match.group(2))
expected_column = columns - 3
if reported_column != expected_column:
    raise SystemExit(
        f"mapped caret column {reported_column}, expected {expected_column} at width {columns}"
    )
print(f"width={columns} caret_column={reported_column}")
PY
)

printf '\nPtyxis smoke: %s\n' "$report"
printf 'Confirm the right-aligned row above reads אבגד from right to left [y/N]: '
IFS= read -r answer
case "$answer" in
  y|Y|yes|YES)
    printf 'Ptyxis smoke confirmed\n'
    ;;
  *)
    echo "smoke-ptyxis: visual confirmation was not accepted" >&2
    exit 1
    ;;
esac
