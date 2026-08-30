#!/bin/sh
set -eu

operation=${1:-}
agent=${2:-}
case "$agent" in
  claude|pi|codex) ;;
  *)
    echo "hebrew-tty Herdr plugin: expected claude, pi, or codex" >&2
    exit 2
    ;;
esac

case "$operation" in
  open)
    : "${HERDR_BIN_PATH:?HERDR_BIN_PATH is required}"
    : "${HERDR_PLUGIN_ID:?HERDR_PLUGIN_ID is required}"
    : "${HERDR_PANE_ID:?HERDR_PANE_ID is required}"
    cwd=$(python3 -c 'import json, os; print(json.loads(os.environ["HERDR_PLUGIN_CONTEXT_JSON"]).get("focused_pane_cwd") or os.path.expanduser("~"))')
    exec "$HERDR_BIN_PATH" plugin pane open \
      --plugin "$HERDR_PLUGIN_ID" \
      --entrypoint "$agent" \
      --placement split \
      --target-pane "$HERDR_PANE_ID" \
      --direction right \
      --cwd "$cwd" \
      --no-focus
    ;;
  run)
    : "${HERDR_PLUGIN_ROOT:?HERDR_PLUGIN_ROOT is required}"
    : "${HERDR_PLUGIN_CONFIG_DIR:?HERDR_PLUGIN_CONFIG_DIR is required}"
    export HEBREW_TTY_HOST=herdr
    if [ -f "$HERDR_PLUGIN_CONFIG_DIR/verify-command" ]; then
      exec "$HERDR_PLUGIN_ROOT/bin/hebrew-tty" --as "$agent" sh -c 'printf "hebrew-tty-plugin-ready\n"; sleep 300'
    fi
    exec "$HERDR_PLUGIN_ROOT/bin/hebrew-tty" --as "$agent" "$agent"
    ;;
  *)
    echo "hebrew-tty Herdr plugin: expected open or run" >&2
    exit 2
    ;;
esac
