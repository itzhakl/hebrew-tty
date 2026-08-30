#!/usr/bin/env python3
"""Record and verify no-submit terminal measurements for supported agents."""

import argparse
import datetime as dt
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time
import unicodedata
import uuid

SCHEMA_VERSION = 2
ROWS = 30
WIDTHS = (48, 80)
AGENTS = ("claude", "pi", "codex")
HOSTS = ("direct", "herdr")
PROBE = "HTP_A אבגדה HTP_B 0123456789 abcDEF HTP_C וזחטי HTP_D"
HEBREW_RUNS = ("אבגדה", "וזחטי")
KNOWN_DIALOGS = (
    ("Do you trust the files in this folder", b"\r"),
    ("Yes, I trust this folder", b"\r"),
    ("Trust this folder", b"\r"),
)
BLOCKED_DIALOG_MARKERS = (
    "login", "sign in", "api key", "select a model", "choose a provider",
    "purchase", "subscription", "permission to", "allow this command",
)


def fail(message):
    raise SystemExit(message)


def version(agent):
    result = subprocess.run(
        [agent, "--version"], capture_output=True, text=True, timeout=10, check=False
    )
    value = (result.stdout or result.stderr).strip().splitlines()
    if result.returncode or not value:
        fail(f"cannot identify {agent} version")
    return value[0]


def resize(fd, rows, columns):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))


def cell_width(char):
    if unicodedata.combining(char):
        return 0
    return 2 if unicodedata.east_asian_width(char) in {"W", "F"} else 1


class Screen:
    def __init__(self, rows, columns):
        self.rows = rows
        self.columns = columns
        self.cells = [[" " for _ in range(columns)] for _ in range(rows)]
        self.row = 0
        self.column = 0
        self.saved = (0, 0)
        self.state = "text"
        self.control = ""

    def scroll(self):
        if self.row >= self.rows:
            self.cells.pop(0)
            self.cells.append([" " for _ in range(self.columns)])
            self.row = self.rows - 1

    def put(self, char):
        width = cell_width(char)
        if width == 0:
            if self.column:
                self.cells[self.row][self.column - 1] += char
            return
        if self.column >= self.columns:
            self.column = 0
            self.row += 1
            self.scroll()
        self.cells[self.row][self.column] = char
        if width == 2 and self.column + 1 < self.columns:
            self.cells[self.row][self.column + 1] = ""
        self.column += width

    def csi(self, value):
        final = value[-1]
        raw = value[:-1].lstrip("?>!")
        parts = raw.split(";") if raw else []
        params = [int(item) if item.isdigit() else 0 for item in parts]
        first = params[0] if params else 0
        if final in "Hf":
            self.row = max(0, min(self.rows - 1, (params[0] if params else 1) - 1))
            self.column = max(0, min(self.columns - 1, (params[1] if len(params) > 1 else 1) - 1))
        elif final == "A":
            self.row = max(0, self.row - (first or 1))
        elif final in "Be":
            self.row = min(self.rows - 1, self.row + (first or 1))
        elif final in "Ca":
            self.column = min(self.columns - 1, self.column + (first or 1))
        elif final == "D":
            self.column = max(0, self.column - (first or 1))
        elif final in "G`":
            self.column = max(0, min(self.columns - 1, (first or 1) - 1))
        elif final == "d":
            self.row = max(0, min(self.rows - 1, (first or 1) - 1))
        elif final == "J" and first in {0, 2, 3}:
            if first in {2, 3}:
                self.cells = [[" " for _ in range(self.columns)] for _ in range(self.rows)]
            else:
                self.cells[self.row][self.column:] = [" "] * (self.columns - self.column)
                for row in range(self.row + 1, self.rows):
                    self.cells[row] = [" "] * self.columns
        elif final == "K":
            if first == 0:
                self.cells[self.row][self.column:] = [" "] * (self.columns - self.column)
            elif first == 1:
                self.cells[self.row][:self.column + 1] = [" "] * (self.column + 1)
            elif first == 2:
                self.cells[self.row] = [" "] * self.columns
        elif final == "s":
            self.saved = (self.row, self.column)
        elif final == "u":
            self.row, self.column = self.saved

    def feed(self, text):
        for char in text:
            if self.state == "osc":
                if char == "\x07":
                    self.state = "text"
                elif char == "\x1b":
                    self.state = "osc_escape"
                continue
            if self.state == "osc_escape":
                self.state = "text" if char == "\\" else "osc"
                continue
            if self.state == "escape":
                if char == "[":
                    self.state = "csi"
                    self.control = ""
                elif char == "]":
                    self.state = "osc"
                elif char == "7":
                    self.saved = (self.row, self.column)
                    self.state = "text"
                elif char == "8":
                    self.row, self.column = self.saved
                    self.state = "text"
                else:
                    self.state = "text"
                continue
            if self.state == "csi":
                self.control += char
                if "@" <= char <= "~":
                    self.csi(self.control)
                    self.state = "text"
                continue
            if char == "\x1b":
                self.state = "escape"
            elif char == "\r":
                self.column = 0
            elif char in "\n\v\f":
                self.row += 1
                self.scroll()
            elif char == "\b":
                self.column = max(0, self.column - 1)
            elif char == "\t":
                self.column = min(self.columns - 1, (self.column // 8 + 1) * 8)
            elif char >= " ":
                self.put(char)

    def lines(self):
        return ["".join(row).rstrip() for row in self.cells]


def herdr_json(session, environment, *arguments):
    result = subprocess.run(
        ["herdr", "--session", session, *arguments], capture_output=True,
        text=True, timeout=10, check=False, env=environment,
    )
    if result.returncode:
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return None


def herdr_session_exists(session, environment, running=None):
    result = subprocess.run(
        ["herdr", "session", "list", "--json"], capture_output=True,
        text=True, timeout=10, check=False, env=environment,
    )
    if result.returncode:
        return False
    try:
        sessions = json.loads(result.stdout).get("sessions", [])
    except (AttributeError, json.JSONDecodeError):
        return False
    return any(
        item.get("name") == session
        and (running is None or item.get("running") is running)
        for item in sessions
    )


def contains_agent_process(value, agent):
    if isinstance(value, dict):
        return any(contains_agent_process(item, agent) for item in value.values())
    if isinstance(value, list):
        return any(contains_agent_process(item, agent) for item in value)
    if not isinstance(value, str):
        return False
    return any(Path(token).name == agent for token in re.findall(r"[^\s\"']+", value))


def nested_provenance(session, environment, agent):
    if not herdr_session_exists(session, environment, running=True):
        return None
    panes = herdr_json(session, environment, "pane", "list")
    if panes is None:
        return None
    pane_ids = set(re.findall(r'w\d+:p\d+', json.dumps(panes)))
    for pane_id in pane_ids:
        info = herdr_json(session, environment, "pane", "process-info", "--pane", pane_id)
        if info is not None and contains_agent_process(info, agent):
            return {"pane_id": pane_id, "process_info": info}
    return None


def stop_test_session(session, environment):
    subprocess.run(["herdr", "session", "stop", session], capture_output=True, timeout=10, check=False, env=environment)
    subprocess.run(["herdr", "session", "delete", session], capture_output=True, timeout=10, check=False, env=environment)


def capture(agent, host, columns, startup_seconds, settle_seconds):
    if shutil.which(agent) is None:
        fail(f"blocked: {agent} is not installed")
    if host == "herdr" and os.environ.get("HERDR_ENV") != "1":
        fail("blocked: Herdr recording requires HERDR_ENV=1")

    env = dict(os.environ)
    env.update({"TERM": "xterm-256color", "COLUMNS": str(columns), "LINES": str(ROWS)})
    if host in {"direct", "herdr"}:
        for key in tuple(env):
            if key == "HERDR_ENV" or key.startswith("HERDR_"):
                env.pop(key, None)

    session = f"htp-probe-{os.getpid()}-{uuid.uuid4().hex[:8]}" if host == "herdr" else None
    herdr_root = Path(tempfile.mkdtemp(prefix="htp-herdr-")) if host == "herdr" else None
    if herdr_root is not None:
        env["XDG_CONFIG_HOME"] = str(herdr_root)
        if herdr_session_exists(session, env):
            fail("blocked: uniquely named Herdr session already exists")
    pid, master = pty.fork()
    if pid == 0:
        os.environ.clear()
        os.environ.update(env)
        command = [agent] if host == "direct" else ["herdr", "--session", session]
        os.execvp(command[0], command)
    resize(master, ROWS, columns)
    screen = Screen(ROWS, columns)
    started = time.monotonic()
    agent_started_at = started if host == "direct" else None
    typed_at = None
    deadline = started + startup_seconds + settle_seconds + (5 if host == "herdr" else 0)
    events = [{"kind": "resize", "elapsed_ms": 0, "rows": ROWS, "columns": columns}]
    dismissed = set()
    pane_verified = None
    try:
        while time.monotonic() < deadline:
            now = time.monotonic()
            if host == "herdr" and agent_started_at is None and now - started >= 2:
                os.write(master, f"exec {agent}\r".encode())
                agent_started_at = now
                events.append({"kind": "agent_launch", "elapsed_ms": round((now - started) * 1000)})
            visible = "\n".join(screen.lines())
            lowered = visible.lower()
            blocked = [marker for marker in BLOCKED_DIALOG_MARKERS if marker in lowered]
            known = next(((marker, key) for marker, key in KNOWN_DIALOGS if marker.lower() in lowered), None)
            if blocked and known is None:
                fail(f"blocked: unsafe startup dialog detected ({blocked[0]})")
            if known and known[0] not in dismissed:
                os.write(master, known[1])
                dismissed.add(known[0])
                events.append({"kind": "known_dialog_dismissed", "elapsed_ms": round((now - started) * 1000), "dialog": known[0]})
            if host == "herdr" and agent_started_at is not None and pane_verified is None:
                pane_verified = nested_provenance(session, env, agent)
            if typed_at is None and agent_started_at is not None and now - agent_started_at >= startup_seconds:
                os.write(master, PROBE.encode())
                typed_at = now
                events.append({
                    "kind": "input", "elapsed_ms": round((now - started) * 1000),
                    "text": PROBE, "submitted": False,
                })
            ready, _, _ = select.select([master], [], [], 0.1)
            if ready:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                screen.feed(chunk.decode("utf-8", "replace"))
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
        try:
            os.close(master)
        finally:
            if session is not None:
                stop_test_session(session, env)
            if herdr_root is not None:
                shutil.rmtree(herdr_root)

    if typed_at is None:
        fail("blocked: agent exited before the fixed probe could be typed")
    if host == "herdr" and pane_verified is None:
        fail("blocked: isolated Herdr pane did not verify the agent process")
    provenance = {
        "path": "direct_pty" if host == "direct" else "herdr_outer_screen",
        "controlling_tty": True,
        "isolated_session": host == "herdr",
    }
    if host == "herdr":
        provenance.update({
            "herdr_version": version("herdr"),
            "session_verified": True,
            "pane_verified": True,
            "agent_process_verified": True,
        })
    return screen.lines(), events, provenance


def probe_fragments(rows):
    allowed = set(PROBE)
    candidates = []
    for run in rows:
        filtered = "".join(ch for ch in run if ch in allowed)
        if "HTP_" in filtered or any(word in filtered or word[::-1] in filtered for word in HEBREW_RUNS):
            candidates.append(filtered.strip())
    unique = []
    for item in candidates:
        if item and item not in unique:
            unique.append(item)
    return unique


def classify(fragments, columns):
    row_orders = []
    observed_runs = {"logical": set(), "visual": set()}
    for row_number, fragment in enumerate(fragments, start=1):
        logical_runs = {word for word in HEBREW_RUNS if word in fragment}
        visual_runs = {word for word in HEBREW_RUNS if word[::-1] in fragment}
        if logical_runs and visual_runs:
            fail(f"blocked: reconstructed row {row_number} has mixed Hebrew order")
        if logical_runs:
            row_orders.append((row_number, "logical"))
            observed_runs["logical"].update(logical_runs)
        elif visual_runs:
            row_orders.append((row_number, "visual"))
            observed_runs["visual"].update(visual_runs)

    logical = observed_runs["logical"] == set(HEBREW_RUNS) and not observed_runs["visual"]
    visual = observed_runs["visual"] == set(HEBREW_RUNS) and not observed_runs["logical"]
    if logical == visual:
        fail("blocked: recording does not unambiguously identify logical/visual order")

    markers = ("HTP_A", "HTP_B", "HTP_C", "HTP_D")
    marker_rows = []
    for marker in markers:
        matches = [row for row, fragment in enumerate(fragments) if fragment.count(marker) == 1]
        if len(matches) != 1 or sum(fragment.count(marker) for fragment in fragments) != 1:
            fail(f"blocked: wrapped marker order lacks exactly one {marker}")
        marker_rows.append(matches[0])

    if len(PROBE) <= columns:
        wrapping = "not_observed"
    else:
        normal = all(left <= right for left, right in zip(marker_rows, marker_rows[1:]))
        reversed_rows = all(left >= right for left, right in zip(marker_rows, marker_rows[1:]))
        spans_rows = len(set(marker_rows)) > 1
        if not spans_rows or normal == reversed_rows:
            fail("blocked: wrapped marker order is ambiguous or insufficient")
        wrapping = "post_bidi" if normal else "pre_bidi"
    order = "logical" if logical else "visual"
    evidence = [
        f"Hebrew order by reconstructed row: "
        + ", ".join(f"row {row}={row_order}" for row, row_order in row_orders),
        (
            f"wrapping was not exercised because the {len(PROBE)}-column probe fits at {columns} columns"
            if wrapping == "not_observed"
            else f"wrapped marker rows A/B/C/D={marker_rows} identify {wrapping} at {columns} columns"
        ),
    ]
    return {"order": order, "wrapping": wrapping, "evidence": evidence}


def record(args):
    rows, events, provenance = capture(args.agent, args.host, args.columns, args.startup_seconds, args.settle_seconds)
    fragments = probe_fragments(rows)
    if not fragments:
        fail("blocked: synthetic probe was not observed; an interactive startup gate may be open")
    classification = classify(fragments, args.columns)
    document = {
        "schema_version": SCHEMA_VERSION,
        "recorded_at": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "agent": args.agent,
        "agent_version": version(args.agent),
        "host": args.host,
        "terminal": {
            "rows": ROWS, "columns": args.columns, "term": "xterm-256color",
            "herdr_env": args.host == "herdr",
        },
        "provenance": provenance,
        "probe": PROBE,
        "events": events,
        "observed": {
            "probe_fragments": fragments,
            "source": "reconstructed_terminal_rows",
        },
        "classification": classification,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n")
    print(f"recorded {args.output}")


def validate_document(path, document):
    required = {
        "schema_version", "recorded_at", "agent", "agent_version", "host",
        "terminal", "provenance", "probe", "events", "observed", "classification",
    }
    errors = []
    if set(document) != required:
        errors.append("top-level schema fields differ")
    if document.get("schema_version") != SCHEMA_VERSION:
        errors.append("unsupported schema_version")
    if document.get("probe") != PROBE:
        errors.append("probe is not the fixed synthetic string")
    if document.get("agent") not in AGENTS or document.get("host") not in HOSTS:
        errors.append("unsupported agent or host")
    terminal = document.get("terminal", {})
    if terminal.get("columns") not in WIDTHS or terminal.get("rows") != ROWS:
        errors.append("unexpected terminal dimensions")
    provenance = document.get("provenance", {})
    if provenance.get("controlling_tty") is not True:
        errors.append("child did not have a verified controlling tty")
    if document.get("host") == "direct":
        if provenance.get("path") != "direct_pty" or provenance.get("isolated_session") is not False:
            errors.append("direct provenance is not verified")
    elif (
        provenance.get("path") != "herdr_outer_screen"
        or provenance.get("isolated_session") is not True
        or provenance.get("session_verified") is not True
        or provenance.get("pane_verified") is not True
        or provenance.get("agent_process_verified") is not True
    ):
        errors.append("Herdr session/pane/process provenance is not verified")
    events = document.get("events", [])
    inputs = [event for event in events if event.get("kind") == "input"]
    if len(inputs) != 1 or inputs[0].get("text") != PROBE or inputs[0].get("submitted") is not False:
        errors.append("input event must contain the unsubmitted fixed probe")
    observed = document.get("observed", {})
    fragments = observed.get("probe_fragments", [])
    if observed.get("source") != "reconstructed_terminal_rows":
        errors.append("observations were not reconstructed from terminal rows")
    if not fragments:
        errors.append("no recorded probe fragments")
    else:
        try:
            expected = classify(fragments, terminal.get("columns", 0))
            actual = document.get("classification", {})
            if any(actual.get(key) != expected[key] for key in ("order", "wrapping")):
                errors.append("classification does not match recorded fragments")
        except SystemExit as error:
            errors.append(str(error))
    return [f"{path}: {error}" for error in errors]


def verify(directory):
    errors = []
    seen = set()
    for path in sorted(directory.glob("*.json")):
        if path.name == "measurement.schema.json":
            continue
        try:
            document = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{path}: {error}")
            continue
        errors.extend(validate_document(path, document))
        terminal = document.get("terminal", {})
        seen.add((document.get("agent"), document.get("host"), terminal.get("columns")))
    expected = {(agent, host, width) for agent in AGENTS for host in HOSTS for width in WIDTHS}
    for missing in sorted(expected - seen):
        errors.append(f"missing measurement: agent={missing[0]} host={missing[1]} columns={missing[2]}")
    for agent in AGENTS:
        for host in HOSTS:
            path_documents = []
            for path in sorted(directory.glob(f"{agent}-{host}-*.json")):
                path_documents.append(json.loads(path.read_text()))
            if path_documents and not any(
                item.get("classification", {}).get("wrapping") in {"pre_bidi", "post_bidi"}
                for item in path_documents
            ):
                errors.append(f"no wrapping classification: agent={agent} host={host}")
    if errors:
        fail("\n".join(errors))
    print(f"verified {len(seen)} genuine-path measurement fixtures")


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    recording = commands.add_parser("record")
    recording.add_argument("--agent", choices=AGENTS, required=True)
    recording.add_argument("--host", choices=HOSTS, required=True)
    recording.add_argument("--columns", choices=WIDTHS, type=int, required=True)
    recording.add_argument("--output", type=Path, required=True)
    recording.add_argument("--startup-seconds", type=float, default=8)
    recording.add_argument("--settle-seconds", type=float, default=3)
    checking = commands.add_parser("verify")
    checking.add_argument("directory", type=Path)
    return result


def main():
    args = parser().parse_args()
    if args.command == "record":
        record(args)
    else:
        verify(args.directory)


if __name__ == "__main__":
    main()
