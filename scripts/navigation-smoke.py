#!/usr/bin/env python3
"""Verify lowercase j/k navigation, unbound J/K and fullscreen focus. No model calls."""
import fcntl
import json
import os
import pathlib
import pty
import select
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time

root = pathlib.Path(__file__).resolve().parents[1]
binary = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/duet-workbench"

with tempfile.TemporaryDirectory(prefix="duet-navigation-") as directory:
    project = pathlib.Path(directory)
    for name in ["a.rs", "b.rs"]:
        (project / name).write_text("".join(f"let value_{i} = {i};\n" for i in range(100)))
    for args in [["init", "-q"], ["add", "."],
                 ["-c", "user.name=Probe", "-c", "user.email=probe@localhost", "commit", "-qm", "fixture"]]:
        subprocess.run(["git", "-C", directory, *args], check=True, stdout=subprocess.DEVNULL)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
    process = subprocess.Popen([str(binary),'--review-editor','native',"--helix", str(project/"missing-hx"), "open", directory], stdin=slave, stdout=slave,
                               stderr=slave, env={**os.environ, "TERM": "xterm-256color"})

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            readable, _, _ = select.select([master], [], [], 0.01)
            if readable:
                os.read(master, 65536)

    def send(keys):
        os.write(master, keys)
        drain(0.6)  # Includes the bounded, debounced persistence interval.

    def state():
        with sqlite3.connect(project / ".duet/workspace.db") as db:
            ui = json.loads(db.execute("SELECT value FROM ui_state WHERE key='workspace'").fetchone()[0])
            scroll = db.execute("SELECT scroll FROM reading_progress WHERE path='a.rs'").fetchone()[0]
            return ui, scroll

    try:
        drain(0.6)
        send(b'\x1bOR')  # Baseline file navigation belongs to the project view.
        send(b"jJK")
        assert state()[0]["project_tree_selected"] == "b.rs" and state()[1] == 0
        send(b"jk")
        assert state()[0]["project_tree_selected"] == "a.rs"
        send(b"k")
        assert state()[0]["project_tree_selected"] == "a.rs"
        send(b"\tjJK")
        assert state()[0]["panel"] == "Reader" and state()[1] == 1
        send(b"k")
        assert state()[1] == 0
        send(b"fj")
        assert state()[0]["panel"] == "Reader" and state()[1] == 1
        send(b"\t")
        assert state()[0]["panel"] == "Queue" # fixed evidence pane is hidden
        send(b"\tk")
        assert state()[0]["panel"] == "Reader" and state()[1] == 0
        send(b":note jJkK\r")
        assert state()[0]["notes"][-1].endswith("jJkK")
        assert state()[1] == 0
        send(b"q")
        assert process.wait(timeout=3) == 0
        print("PTY navigation passed: j/k, unbound J/K, queue, reader, fullscreen, tabs, input")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
