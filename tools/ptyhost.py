#!/usr/bin/env python3
"""Run a command on a pty and relay it over plain pipes.

Node has no way to open a pty without a native module, and node-pty does not
build here. This is the smallest thing that fills the gap: the child gets a
real controlling terminal, and the filter upstream gets two ordinary pipes it
can read and rewrite.

The window size cannot come from our own stdio - stdout is a pipe, so it has
no size at all. It arrives on fd 3 instead, as "rows cols\\n", whenever the
filter's terminal is resized.
"""

import os
import pty
import select
import signal
import struct
import sys
import termios
import fcntl

CONTROL = 3


def set_size(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def main(argv):
    """`--argv0 NAME` runs the command under a different process name.

    herdr finds an agent pane by the name of the process in it, and a build
    named after its version reads as an unknown process. The caller decides
    what the child is called; we only carry it through.
    """
    argv0 = None
    if len(argv) >= 2 and argv[0] == "--argv0":
        argv0 = argv[1]
        argv = argv[2:]
    if not argv:
        sys.exit("ptyhost: nothing to run")
    rows, cols = 24, 80
    control = None
    try:
        os.fstat(CONTROL)
        control = CONTROL
    except OSError:
        pass

    pid, master = pty.fork()
    if pid == 0:
        try:
            os.execvp(argv[0], [argv0 or argv[0]] + argv[1:])
        except OSError as err:
            sys.stderr.write(f"ptyhost: {argv[0]}: {err.strerror}\n")
            os._exit(127)

    set_size(master, rows, cols)

    def resized(*_):
        pass

    signal.signal(signal.SIGWINCH, resized)

    fds = [0, master] + ([control] if control is not None else [])
    pending = b""
    while True:
        try:
            ready, _, _ = select.select(fds, [], [])
        except InterruptedError:
            continue
        if control is not None and control in ready:
            data = os.read(control, 256)
            if data:
                pending += data
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    try:
                        r, c = (int(x) for x in line.split())
                    except ValueError:
                        continue
                    set_size(master, r, c)
                    os.kill(pid, signal.SIGWINCH)
        if 0 in ready:
            data = os.read(0, 65536)
            if not data:
                fds.remove(0)
            else:
                os.write(master, data)
        if master in ready:
            try:
                data = os.read(master, 65536)
            except OSError:
                data = b""
            if not data:
                break
            os.write(1, data)

    _, status = os.waitpid(pid, 0)
    os.close(master)
    if os.WIFSIGNALED(status):
        sys.exit(128 + os.WTERMSIG(status))
    sys.exit(os.WEXITSTATUS(status))


main(sys.argv[1:])
