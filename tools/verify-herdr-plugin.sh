#!/bin/sh
set -eu

source_plugin_id=hebrew-tty.terminal-proxy
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
HERDR_BIN_PATH=${HERDR_BIN_PATH:-$(command -v herdr || true)}
panes_file=$(mktemp)
baseline_logs_file=$(mktemp)
started_actions_file=$(mktemp)
run_dir=$(mktemp -d)
linked_plugin_id=
cleaned=0
config_dir=
verify_token=$(python3 -c 'import secrets; print(secrets.token_hex(16))')
plugin_id=$source_plugin_id.verify-$verify_token
plugin_root=$run_dir/plugin

close_pane() {
  pane_id=$1
  [ -n "$pane_id" ] || return
  timeout 5 "$HERDR_BIN_PATH" plugin pane close "$pane_id" >/dev/null 2>&1 || true
}

pane_from_log() {
  log_id=$1
  logs=$2
  python3 -c '
import json, sys
payload = json.loads(sys.stdin.read())
log_id = sys.argv[1]
for item in payload["result"]["logs"]:
    if item["log_id"] != log_id or item["status"] != "succeeded":
        continue
    opened = json.loads(item["stdout"])
    print(opened["result"]["plugin_pane"]["pane"]["pane_id"])
    break
' "$log_id" <<EOF
$logs
EOF
}

cleanup_pending_panes() {
  python3 - "$HERDR_BIN_PATH" "$plugin_id" "$baseline_logs_file" "$started_actions_file" "$verify_token" "$run_dir" <<'PY'
import json
import subprocess
import sys
import time

herdr, plugin_id, baseline_path, started_path, token, run_dir = sys.argv[1:]
try:
    baseline = set(open(baseline_path).read().splitlines())
    expected = len(open(started_path).read().splitlines())
except OSError:
    raise SystemExit(0)
known = set()
for path in __import__("glob").glob(f"{run_dir}/*.json"):
    try:
        known.add(json.load(open(path))["result"]["log"]["log_id"])
    except (OSError, KeyError, TypeError, json.JSONDecodeError):
        pass
if expected == 0:
    raise SystemExit(0)
deadline = time.monotonic() + 30
closed = set()
while time.monotonic() < deadline:
    try:
        result = subprocess.run(
            [herdr, "plugin", "log", "list", "--plugin", plugin_id],
            text=True,
            capture_output=True,
            timeout=max(0.1, deadline - time.monotonic()),
        )
    except subprocess.TimeoutExpired:
        break
    if result.returncode != 0:
        break
    logs = json.loads(result.stdout)["result"]["logs"]
    current = [item for item in logs if item["log_id"] not in baseline]
    terminal = 0
    for item in current:
        if item["status"] not in {"succeeded", "failed"}:
            continue
        owned = item["log_id"] in known
        pane_id = None
        if item["status"] == "succeeded" and item.get("stdout"):
            try:
                pane_id = json.loads(item["stdout"])["result"]["plugin_pane"]["pane"]["pane_id"]
            except (KeyError, TypeError, json.JSONDecodeError):
                pass
        if not owned and pane_id and token:
            try:
                process = subprocess.run(
                    [herdr, "pane", "process-info", "--pane", pane_id],
                    text=True,
                    capture_output=True,
                    timeout=max(0.1, deadline - time.monotonic()),
                )
            except subprocess.TimeoutExpired:
                break
            if process.returncode == 0:
                try:
                    processes = json.loads(process.stdout)["result"]["process_info"]["foreground_processes"]
                    marker = f"hebrew-tty-verify:{token}"
                    owned = any(marker in process["argv"] for process in processes)
                except (KeyError, TypeError, json.JSONDecodeError):
                    pass
        if not owned:
            continue
        terminal += 1
        if not pane_id or pane_id in closed:
            continue
        try:
            subprocess.run(
                [herdr, "plugin", "pane", "close", pane_id],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=max(0.1, deadline - time.monotonic()),
            )
        except subprocess.TimeoutExpired:
            break
        closed.add(pane_id)
    if terminal >= expected:
        break
    time.sleep(0.05)
PY
}

cleanup() {
  [ "$cleaned" -eq 0 ] || return
  cleaned=1
  set +e
  if [ -n "$HERDR_BIN_PATH" ]; then
    while IFS= read -r pane_id; do
      close_pane "$pane_id"
    done < "$panes_file"
    cleanup_pending_panes
  fi
  if [ -n "$config_dir" ]; then
    rm -f "$config_dir/verify-command"
  fi
  if [ -n "$linked_plugin_id" ] && [ -n "$HERDR_BIN_PATH" ]; then
    timeout 5 "$HERDR_BIN_PATH" plugin unlink "$linked_plugin_id" >/dev/null 2>&1 || true
  fi
  rm -f "$panes_file" "$baseline_logs_file" "$started_actions_file"
  rm -rf "$run_dir"
}

terminate() {
  status=$1
  trap - EXIT
  trap '' HUP INT TERM
  cleanup
  exit "$status"
}

trap cleanup EXIT
trap 'terminate 129' HUP
trap 'terminate 130' INT
trap 'terminate 143' TERM

: "${HERDR_ENV:?run this verification inside a Herdr pane}"
: "${HERDR_BIN_PATH:?herdr is not installed}"
export HERDR_BIN_PATH

existing=$("$HERDR_BIN_PATH" plugin action list --plugin "$source_plugin_id")
if python3 -c 'import json, sys; raise SystemExit(0 if json.load(sys.stdin)["result"]["actions"] else 1)' <<EOF
$existing
EOF
then
  echo "verify-herdr-plugin: $source_plugin_id is already registered" >&2
  exit 1
fi

linked_plugin_id=$source_plugin_id
link_output=$("$HERDR_BIN_PATH" plugin link "$root" --enabled 2>&1)
case "$link_output" in
  *warning*|*WARNING*|*Warning*)
    printf '%s\n' "$link_output" >&2
    exit 1
    ;;
esac

actions=$("$HERDR_BIN_PATH" plugin action list --plugin "$source_plugin_id")
python3 -c '
import json, sys
items = json.load(sys.stdin)["result"]["actions"]
plugin_id = sys.argv[1]
assert {item["action_id"] for item in items} == {"claude", "pi", "codex"}
assert all(item["plugin_id"] == plugin_id for item in items)
' "$source_plugin_id" <<EOF
$actions
EOF
"$HERDR_BIN_PATH" plugin unlink "$source_plugin_id" >/dev/null
linked_plugin_id=

mkdir -p "$plugin_root"
python3 -c '
import pathlib, sys
source, destination, plugin_id, original_id = sys.argv[1:]
manifest = pathlib.Path(source).read_text()
manifest = manifest.replace(original_id, "id = " + chr(34) + plugin_id + chr(34), 1)
pathlib.Path(destination).write_text(manifest)
' "$root/herdr-plugin.toml" "$plugin_root/herdr-plugin.toml" "$plugin_id" 'id = "hebrew-tty.terminal-proxy"'
ln -s "$root/bin" "$plugin_root/bin"
ln -s "$root/plugins" "$plugin_root/plugins"

linked_plugin_id=$plugin_id
link_output=$("$HERDR_BIN_PATH" plugin link "$plugin_root" --enabled 2>&1)
case "$link_output" in
  *warning*|*WARNING*|*Warning*)
    printf '%s\n' "$link_output" >&2
    exit 1
    ;;
esac

"$HERDR_BIN_PATH" plugin log list --plugin "$plugin_id" | python3 -c '
import json, sys
for item in json.load(sys.stdin)["result"]["logs"]:
    print(item["log_id"])
' > "$baseline_logs_file"

config_dir=$("$HERDR_BIN_PATH" plugin config-dir "$plugin_id")
printf '%s\n' "$verify_token" > "$config_dir/verify-command"

for agent in claude pi codex; do
  printf '%s\n' "$agent" >> "$started_actions_file"
  invocation_file=$run_dir/$agent.json
  "$HERDR_BIN_PATH" plugin action invoke "$agent" --plugin "$plugin_id" > "$invocation_file"
  log_id=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["result"]["log"]["log_id"])' "$invocation_file")
  expected_cwd=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["result"]["context"]["focused_pane_cwd"])' "$invocation_file")

  pane_id=
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    logs=$("$HERDR_BIN_PATH" plugin log list --plugin "$plugin_id")
    failure=$(python3 -c '
import json, sys
payload = json.load(sys.stdin)
log_id = sys.argv[1]
for item in payload["result"]["logs"]:
    if item["log_id"] == log_id and item["status"] == "failed":
        print(item.get("stderr") or item.get("error") or "plugin action failed")
        break
' "$log_id" <<EOF
$logs
EOF
)
    if [ -n "$failure" ]; then
      printf '%s\n' "$failure" >&2
      exit 1
    fi
    pane_id=$(pane_from_log "$log_id" "$logs")
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

  close_pane "$pane_id"
done

rm -f "$config_dir/verify-command"
"$HERDR_BIN_PATH" plugin unlink "$plugin_id" >/dev/null
linked_plugin_id=
printf 'verified Herdr actions: claude, pi, codex\n'
