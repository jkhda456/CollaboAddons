"""A program in a pseudo-terminal for the doom add-on's tests: Session(argv, cols, rows); .send(bytes);
.pump(seconds) reads what it writes (as fast as a terminal would); .expect(regex, timeout) on the text
read since the last expect; .screen is a vt.Screen fed with all of it, when asked (.feed_screen()).

Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import termios
import time

import vt


class Session:
    def __init__(self, argv, cols=100, rows=30, env=None, cwd=None):
        self.cols, self.rows = cols, rows
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            if cwd:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, {**os.environ, **(env or {})})
        self.resize(cols, rows)
        self.pending = bytearray()   # read, not yet given to the screen
        self.text = bytearray()      # read since the last expect (the last MB of it)
        self.total = 0
        self.screen = vt.Screen(cols, rows)
        self.chunks = []             # (time, bytes) of each read, when record is on
        self.record = False

    def resize(self, cols, rows):
        self.cols, self.rows = cols, rows
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        if hasattr(self, "screen"):
            self.feed_screen()
            old = self.screen
            self.screen = vt.Screen(cols, rows)
            self.screen.modes, self.screen.alt = old.modes, old.alt
            self.screen.rest = getattr(old, "rest", b"")
            self.screen.wide_blocks = old.wide_blocks
            if old.alt:  # on the alternate screen: the main one (blank here) comes back after it
                self.screen.grid = self.screen.blank()
                self.screen.saved = getattr(old, "saved", (0, 0))

    def pump(self, seconds=0.1, rate=None):
        """reads for `seconds`; `rate`: no more than that many bytes a second (a slow terminal)"""
        end = time.time() + seconds
        allowance, last = 0.0, time.time()
        while True:
            left = end - time.time()
            if rate:
                now = time.time()
                allowance = min(allowance + (now - last) * rate, rate * 0.05)
                last = now
                if allowance < 1024:
                    time.sleep(min(0.005, max(0.0, left)))
                    if left <= 0:
                        return True
                    continue
            r, _, _ = select.select([self.fd], [], [], max(0.0, left))
            if r:
                try:
                    data = os.read(self.fd, int(allowance) if rate else 1 << 20)
                    allowance -= len(data)
                except OSError:
                    data = b""
                if not data:
                    return False
                self.pending += data
                self.text += data
                if len(self.text) > (1 << 20):
                    del self.text[:len(self.text) - (1 << 20)]
                self.total += len(data)
                if self.record:
                    self.chunks.append((time.time(), len(data)))
            if left <= 0:
                return True

    def send(self, data):
        if isinstance(data, str):
            data = data.encode()
        os.write(self.fd, data)

    def expect(self, pattern, timeout=30):
        rx = re.compile(pattern.encode() if isinstance(pattern, str) else pattern, re.S)
        end = time.time() + timeout
        while time.time() < end:
            m = rx.search(self.text)
            if m:
                out = bytes(self.text[:m.end()])
                del self.text[:m.end()]
                return out.decode(errors="replace")
            self.pump(0.1)
        raise TimeoutError(f"no {pattern!r} in:\n{bytes(self.text[-3000:]).decode(errors='replace')}")

    def feed_screen(self):
        if self.pending:
            self.screen.feed(bytes(self.pending))
            self.pending = bytearray()
        return self.screen

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except Exception:
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass
