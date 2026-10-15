#!/usr/bin/env python3
"""Exercise the actual Rust TUI in a PTY, without Pi/model/network calls."""
import fcntl
import os
import pathlib
import pty
import select
import signal
import struct
import subprocess
import tempfile
import sqlite3
import json
import termios
import time
import sys

root = pathlib.Path(__file__).resolve().parents[1]
binary = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/duet-workbench"
assert binary.exists(), "Run cargo build first"
master, slave = pty.openpty()
before = termios.tcgetattr(slave)
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
project = tempfile.TemporaryDirectory(prefix="duet-pty-")
process = subprocess.Popen(
    [str(binary), "--review-editor", "native", "open", project.name], stdin=slave, stdout=slave, stderr=slave,
    env={**os.environ, "TERM": "xterm-256color"}, cwd=root,
)
output = bytearray()


def drain(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 0.05)
        if readable:
            output.extend(os.read(master, 65536))


try:
    drain(0.3)
    for keys in [b"?", b"\x1b", b":note " + "事务边界".encode() + b"\r", b"\x1bOS", b"t", b"\x1bOQ", b"m", b"\x1b"]:
        os.write(master, keys)
        drain(0.15)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 80, 0, 0))
    process.send_signal(signal.SIGWINCH)
    drain(0.2)
    os.write(master, b"q")
    drain(0.3)
    assert process.wait(timeout=3) == 0
    after = termios.tcgetattr(slave)
    assert before == after, "TTY settings were not restored"
    assert b"\x1b[?1049h" in output and b"\x1b[?1049l" in output
    assert b"DUET" in output
    # Differential rendering may insert cursor moves between wide glyphs.
    assert "事".encode() in output and "务".encode() in output
    db = sqlite3.connect(pathlib.Path(project.name) / ".duet/workspace.db")
    saved = json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
    assert saved["notes"] == ["事务边界"]
    db.close()
    print("PTY smoke passed: input, persistence, help, theme, resize, quit, restoration")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
    os.close(slave)
    project.cleanup()
