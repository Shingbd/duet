#!/usr/bin/env python3
"""Local PTY performance probe; temporary Git fixture, no model/network calls.
Usage: python3 scripts/performance-probe.py /path/to/binary [...]
Not an SSH end-to-end benchmark; deliberately keeps debug/release comparable.
"""
import fcntl
import json
import os
import pathlib
import pty
import select
import statistics
import struct
import subprocess
import sys
import tempfile
import termios
import time


def probe(binary):
    with tempfile.TemporaryDirectory(prefix="duet-perf-") as directory:
        project = pathlib.Path(directory)
        (project / "large.rs").write_text("".join(
            f"let value_{i:05} = process_value({i}); // code reader benchmark\n"
            for i in range(6000)))
        for args in [["init", "-q"], ["add", "large.rs"],
                     ["-c", "user.name=Probe", "-c", "user.email=probe@localhost",
                      "commit", "-qm", "fixture"]]:
            subprocess.run(["git", "-C", directory, *args], check=True,
                           stdout=subprocess.DEVNULL)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        process = subprocess.Popen([str(binary), "--review-editor", "native", "open", directory], stdin=slave,
                                   stdout=slave, stderr=slave,
                                   env={**os.environ, "TERM": "xterm-256color"})

        def drain(seconds, marker=None):
            deadline = time.perf_counter() + seconds
            while time.perf_counter() < deadline:
                ready, _, _ = select.select([master], [], [], min(0.01, max(0, deadline-time.perf_counter())))
                if ready:
                    data = os.read(master, 65536)
                    if marker and marker in data:
                        return time.perf_counter()
            if marker:
                raise RuntimeError("input response timed out")

        def cpu():
            fields = pathlib.Path(f"/proc/{process.pid}/stat").read_text().split(") ", 1)[1].split()
            return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")

        try:
            drain(1)
            # Fresh project opens directly on the 6000-line Code page.
            drain(1)
            start_cpu, start = cpu(), time.perf_counter()
            drain(3)
            idle_cpu = 100 * (cpu()-start_cpu) / (time.perf_counter()-start)
            os.write(master, b":")
            drain(0.5)
            samples = []
            for _ in range(12):
                drain(0.05)
                started = time.perf_counter()
                os.write(master, b"Z")
                samples.append(1000 * (drain(3, b"Z") - started))
            os.write(master, b"\x1b")
            drain(0.2)
            os.write(master, b"q")
            drain(0.5)
            assert process.wait(timeout=5) == 0
            return {"binary": str(binary), "fixture": "6000-line file, 120x32 PTY",
                    "idle_cpu_percent": round(idle_cpu, 2),
                    "local_input_ms_p50": round(statistics.median(samples), 2),
                    "local_input_ms_max": round(max(samples), 2), "samples": len(samples)}
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    root = pathlib.Path(__file__).resolve().parents[1]
    for arg in sys.argv[1:] or [root / "target/debug/duet-workbench"]:
        print(json.dumps(probe(pathlib.Path(arg).resolve()), ensure_ascii=False), flush=True)
