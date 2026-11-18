#!/usr/bin/env python3
"""A small terminal emulator for the doom add-on's tests: what a terminal would show after the bytes
doom sends — cursor moves, colours (SGR: 16, 256 and 24-bit), erase, the alternate screen, private
modes — as a grid of cells (character, foreground, background). And pictures of it (PNG): block
characters drawn as blocks, other characters as their colour by how much of the cell they ink.

    vt.py FILE.ans [COLSxROWS] [OUT.png]      the screen as text (and a picture)

Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
"""
import re
import struct
import sys
import zlib

ANSI16 = [0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
          0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff]
CUBE = [0, 95, 135, 175, 215, 255]
DEFAULT_FG, DEFAULT_BG = 0xc0c0c0, 0x000000


def xterm256(i):
    if i < 16:
        return ANSI16[i]
    if i < 232:
        i -= 16
        return CUBE[i // 36] << 16 | CUBE[i // 6 % 6] << 8 | CUBE[i % 6]
    g = 8 + (i - 232) * 10
    return g << 16 | g << 8 | g


# quadrant characters: which quarters (TL=1 TR=2 BL=4 BR=8) are foreground
QUAD = {' ': 0, '▘': 1, '▝': 2, '▀': 3, '▖': 4, '▌': 5, '▞': 6, '▛': 7,
        '▗': 8, '▚': 9, '▐': 10, '▜': 11, '▄': 12, '▙': 13, '▟': 14, '█': 15}


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.alt = False
        self.main = self.blank()
        self.grid = self.main
        self.x = self.y = 0
        self.fg, self.bg, self.reverse = None, None, False
        self.modes = {}
        self.title = ""
        self.writes = 0
        self.wide_blocks = False  # draw block elements two columns wide (East Asian ambiguous width)

    def blank(self):
        return [[(' ', None, None) for _ in range(self.cols)] for _ in range(self.rows)]

    def sgr(self, params):
        ps = [int(p) if p else 0 for p in params.split(';')] if params else [0]
        i = 0
        while i < len(ps):
            p = ps[i]
            if p == 0:
                self.fg = self.bg = None
                self.reverse = False
            elif p == 7:
                self.reverse = True
            elif p == 27:
                self.reverse = False
            elif 30 <= p <= 37:
                self.fg = ANSI16[p - 30]
            elif 90 <= p <= 97:
                self.fg = ANSI16[p - 90 + 8]
            elif 40 <= p <= 47:
                self.bg = ANSI16[p - 40]
            elif 100 <= p <= 107:
                self.bg = ANSI16[p - 100 + 8]
            elif p == 39:
                self.fg = None
            elif p == 49:
                self.bg = None
            elif p in (38, 48) and i + 1 < len(ps):
                if ps[i + 1] == 2 and i + 4 < len(ps):
                    c = ps[i + 2] << 16 | ps[i + 3] << 8 | ps[i + 4]
                    i += 4
                elif ps[i + 1] == 5 and i + 2 < len(ps):
                    c = xterm256(ps[i + 2])
                    i += 2
                else:
                    c = None
                    i += 1
                if p == 38:
                    self.fg = c
                else:
                    self.bg = c
            i += 1

    def put(self, ch):
        if self.x >= self.cols:
            if self.modes.get(7, True):
                self.x = 0
                self.y += 1
                if self.y >= self.rows:
                    self.grid.pop(0)
                    self.grid.append([(' ', None, None)] * self.cols)
                    self.y = self.rows - 1
            else:
                self.x = self.cols - 1
        fg, bg = (self.bg, self.fg) if self.reverse else (self.fg, self.bg)
        self.grid[self.y][self.x] = (ch, fg, bg)
        self.x += 1
        if self.wide_blocks and '\u2580' <= ch <= '\u259f' and self.x < self.cols:
            self.grid[self.y][self.x] = ('', fg, bg)  # the glyph's second column
            self.x += 1

    TOKEN = re.compile(r"\x1b\[([<=>?]?)([0-9;:]*)[ -/]*([@-~])|\x1b\]([^\x07\x1b]*)(?:\x07|\x1b\\)|\x1b.?"
                       r"|([^\x1b\r\n\b]+)|([\r\n\b])", re.S)

    INCOMPLETE = re.compile(rb"\x1b(?:\[[0-?]*[ -/]*|\][^\x07\x1b]*(?:\x1b)?)?\Z")

    def feed(self, data):
        """bytes as a terminal gets them, in any pieces: an escape sequence or a UTF-8 character
        cut at the end waits for the rest"""
        if isinstance(data, str):
            data = data.encode()
        data = getattr(self, "rest", b"") + data
        self.rest = b""
        m = self.INCOMPLETE.search(data, max(0, len(data) - 4096))
        if m:
            data, self.rest = data[:m.start()], data[m.start():]
        for k in range(1, 4):  # a UTF-8 character not yet whole
            if len(data) >= k and (data[-k] & 0xc0) == 0xc0:
                need = 2 if data[-k] < 0xe0 else 3 if data[-k] < 0xf0 else 4
                if need > k:
                    data, self.rest = data[:-k], data[-k:] + self.rest
                break
        data = data.decode('utf-8', 'replace')
        self.writes += 1
        for m in self.TOKEN.finditer(data):
            if m.group(3):
                self.csi(m.group(1), m.group(2), m.group(3))
            elif m.group(5):
                for ch in m.group(5):
                    if ch >= ' ':
                        self.put(ch)
            elif m.group(6):
                c = m.group(6)
                if c == '\r':
                    self.x = 0
                elif c == '\n':
                    if self.y + 1 >= self.rows:
                        self.grid.pop(0)
                        self.grid.append([(' ', None, None)] * self.cols)
                    else:
                        self.y += 1
                else:
                    self.x = max(self.x - 1, 0)
            elif m.group(4) is not None:
                if m.group(4).startswith(('0;', '2;')):
                    self.title = m.group(4)[2:]

    def csi(self, prefix, params, final):
        ps = [int(p) if p else 0 for p in params.split(';')] if params else []
        a = ps[0] if ps else 0
        if prefix == '?':
            if final in 'hl':
                for p in ps:
                    on = final == 'h'
                    self.modes[p] = on
                    if p == 1049 and on != self.alt:
                        self.alt = on
                        if on:
                            self.saved = (self.x, self.y)
                        else:
                            x, y = getattr(self, "saved", (0, 0))
                            self.x, self.y = min(x, self.cols - 1), min(y, self.rows - 1)
                        self.grid = self.blank() if on else self.main
            return
        if prefix:
            return
        if final == 'H' or final == 'f':
            self.y = min(max((ps[0] if ps and ps[0] else 1) - 1, 0), self.rows - 1)
            self.x = min(max((ps[1] if len(ps) > 1 and ps[1] else 1) - 1, 0), self.cols - 1)
        elif final == 'C':
            self.x = min(self.x + (a or 1), self.cols - 1)
        elif final == 'D':
            self.x = max(self.x - (a or 1), 0)
        elif final == 'A':
            self.y = max(self.y - (a or 1), 0)
        elif final == 'B':
            self.y = min(self.y + (a or 1), self.rows - 1)
        elif final == 'G':
            self.x = min(max((a or 1) - 1, 0), self.cols - 1)
        elif final == 'm':
            self.sgr(params)
        elif final == 'J':
            if a == 2 or a == 3:
                self.grid[:] = self.blank()
            elif a == 0:
                for x in range(self.x, self.cols):
                    self.grid[self.y][x] = (' ', None, self.bg)
                for y in range(self.y + 1, self.rows):
                    self.grid[y] = [(' ', None, self.bg)] * self.cols
        elif final == 'K':
            r = range(self.x, self.cols) if a == 0 else range(0, self.x + 1) if a == 1 else range(self.cols)
            for x in r:
                self.grid[self.y][x] = (' ', None, self.bg)

    def text(self):
        return "\n".join("".join(c[0] for c in row).rstrip() for row in self.grid)

    def png(self, path, cw=4, ch=8):
        """the screen as a picture: each cell cw x ch pixels"""
        W, H = self.cols * cw, self.rows * ch
        rows = []
        for y in range(self.rows):
            lines = [bytearray(W * 3) for _ in range(ch)]
            for x in range(self.cols):
                c, fg, bg = self.grid[y][x]
                fg = DEFAULT_FG if fg is None else fg
                bg = DEFAULT_BG if bg is None else bg
                q = QUAD.get(c)
                for py in range(ch):
                    for px in range(cw):
                        if q is not None:
                            bit = (1 if px < cw // 2 else 2) << (0 if py < ch // 2 else 2)
                            col = fg if q & bit else bg
                        elif c == ' ':
                            col = bg
                        else:  # a character: its colour, as much as it inks
                            col = mix(bg, fg, ink(c))
                        o = (x * cw + px) * 3
                        lines[py][o:o + 3] = bytes((col >> 16 & 255, col >> 8 & 255, col & 255))
            rows.extend(lines)
        write_png(path, W, H, rows)


RAMP = " .'`^\",:;Il!i><~+_-?][}{1)(|/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$"


def ink(c):
    i = RAMP.find(c)
    return (i / (len(RAMP) - 1)) ** 2 if i >= 0 else 0.6


def mix(a, b, t):
    return (int((a >> 16 & 255) * (1 - t) + (b >> 16 & 255) * t) << 16 | int((a >> 8 & 255) * (1 - t) + (b >> 8 & 255) * t) << 8
            | int((a & 255) * (1 - t) + (b & 255) * t))


def write_png(path, w, h, rows):
    raw = b"".join(b"\0" + bytes(r) for r in rows)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def read_ppm(path):
    d = open(path, "rb").read()
    m = re.match(rb"P6\s+(\d+)\s+(\d+)\s+(\d+)\s", d)
    w, h = int(m.group(1)), int(m.group(2))
    px = d[m.end():]
    return w, h, px


def ppm_to_png(path, out, scale=2):
    w, h, px = read_ppm(path)
    rows = []
    for y in range(h):
        line = bytearray()
        for x in range(w):
            line += px[(y * w + x) * 3:(y * w + x) * 3 + 3] * scale
        rows.extend([line] * scale)
    write_png(out, w * scale, h * scale, rows)


if __name__ == "__main__":
    src = sys.argv[1]
    if src.endswith(".ppm"):
        ppm_to_png(src, sys.argv[2])
        sys.exit(0)
    cols, rows = 80, 25
    if len(sys.argv) > 2:
        cols, rows = map(int, sys.argv[2].lower().split("x"))
    s = Screen(cols, rows)
    s.feed(open(src, "rb").read())
    if len(sys.argv) > 3:
        s.png(sys.argv[3])
    else:
        print(s.text())
