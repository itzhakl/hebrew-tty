#!/bin/sh
set -eu

plugin_id=hebrew-tty.terminal-proxy
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
HERDR_BIN_PATH=${HERDR_BIN_PATH:-$(command -v herdr || true)}
panes_file=$(mktemp)
linked=0
config_dir=

cleanup() {
  while IFS= read -r pane_id; do
    [ -n "$pane_id" ] || continue
    "$HERDR_BIN_PATH" plugin pane close "$pane_id" >/dev/null 2>&1 || true
  done < "$panes_file"
  rm -f "$panes_file"
  if [ -n "$config_dir" ]; then
    rm -f "$config_dir/verify-command"
  fi
  if [ "$linked" -eq 1 ]; then
    "$HERDR_BIN_PATH" plugin unlink "$plugin_id" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT HUP INT TERM

: "${HERDR_ENV:?run this verification inside a Herdr pane}"
: "${HERDR_BIN_PATH:?herdr is not installed}"
export HERDR_BIN_PATH

existing=$("$HERDR_BIN_PATH" plugin action list --plugin "$plugin_id")
if python3 -c 'import json, sys; raise SystemExit(0 if json.load(sys.stdin)["result"]["actions"] else 1)' <<EOF
$existing
EOF
then
  echo "verify-herdr-plugin: $plugin_id is already registered" >&2
  exit 1
fi

link_output=$("$HERDR_BIN_PATH" plugin link "$root" --enabled 2>&1)
linked=1
case "$link_output" in
  *warning*|*WARNING*|*Warning*)
    printf '%s\n' "$link_output" >&2
    exit 1
    ;;
esac

actions=$("$HERDR_BIN_PATH" plugin action list --plugin "$plugin_id")
python3 -c '
import json, sys
items = json.load(sys.stdin)["result"]["actions"]
assert {item["action_id"] for item in items} == {"claude", "pi", "codex"}
assert all(item["plugin_id"] == "hebrew-tty.terminal-proxy" for item in items)
' <<EOF
$actions
EOF

config_dir=$("$HERDR_BIN_PATH" plugin config-dir "$plugin_id")
: > "$config_dir/verify-command"

for agent in claude pi codex; do
  invocation=$("$HERDR_BIN_PATH" plugin action invoke "$agent" --plugin "$plugin_id")
  log_id=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["result"]["log"]["log_id"])' <<EOF
$invocation
EOF
)
  expected_cwd=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["result"]["context"]["focused_pane_cwd"])' <<EOF
$invocation
EOF
)

  pane_id=
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    logs=$("$HERDR_BIN_PATH" plugin log list --plugin "$plugin_id")
    pane_id=$(python3 -c '
import json, sys
payload = json.load(sys.stdin)
log_id = sys.argv[1]
for item in payload["result"]["logs"]:
    if item["log_id"] != log_id:
        continue
    if item["status"] == "failed":
        raise SystemExit(item.get("stderr") or item.get("error") or "plugin action failed")
    if item["status"] == "succeeded":
        opened = json.loads(item["stdout"])
        print(opened["result"]["plugin_pane"]["pane"]["pane_id"])
    break
' "$log_id" <<EOF
$logs
EOF
)
    [ -z "$pane_id" ] || break
    attempts=$((attempts + 1))
    sleep 0.05
  done
  if [ -z "$pane_id" ]; then
    echo "verify-herdr-plugin: $agent action did not open a pane" >&2
    exit 1
  fi
  printf '%s\n' "$pane_id" >> "$panes_file"

  process_info=$("$HERDR_BIN_PATH" pane process-info --pane "$pane_id")
  pane=$("$HERDR_BIN_PATH" pane get "$pane_id")
  python3 -c '
import json, os, sys
process_info = json.loads(sys.argv[1])["result"]["process_info"]
pane = json.loads(sys.argv[2])["result"]["pane"]
root, agent, expected_cwd = sys.argv[3:]
processes = process_info["foreground_processes"]
assert len(processes) == 1
argv = processes[0]["argv"]
binary = os.path.realpath(argv[0])
expected = {
    os.path.realpath(os.path.join(root, "target/debug/hebrew-tty")),
    os.path.realpath(os.path.join(root, "target/release/hebrew-tty")),
}
assert binary in expected
assert argv[1:3] == ["--as", agent]
assert pane["cwd"] == expected_cwd
' "$process_info" "$pane" "$root" "$agent" "$expected_cwd"

  "$HERDR_BIN_PATH" plugin pane close "$pane_id" >/dev/null
done

rm -f "$config_dir/verify-command"
"$HERDR_BIN_PATH" plugin unlink "$plugin_id" >/dev/null
linked=0
printf 'verified Herdr actions: claude, pi, codex\n'
