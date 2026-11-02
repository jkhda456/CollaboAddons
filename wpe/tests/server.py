#!/usr/bin/env python3
"""The test site (tests/site) over HTTP, and what checks what the browser sends and records:

    /headers      the request's headers as JSON (text/plain, which the browser shows as text)
    /api/data     JSON, with a cookie (Set-Cookie: devtools=yes)
    /api/echo     a POST's JSON body back, with the request's headers
    /redirect     302 to /second.html
    /pixel.png    a 1x1 PNG
    /ws           a WebSocket that answers each text message with "echo " and it

    server.py PORT
"""
import base64
import hashlib
import http.server
import json
import os
import sys
import zlib

SITE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "site")


def png_pixel():
    def chunk(kind, body):
        return len(body).to_bytes(4, "big") + kind + body + zlib.crc32(kind + body).to_bytes(4, "big")
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", (1).to_bytes(4, "big") * 2 + b"\x08\x02\x00\x00\x00")
            + chunk(b"IDAT", zlib.compress(b"\x00\x33\x66\xff")) + chunk(b"IEND", b""))


class Handler(http.server.SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=SITE, **kwargs)

    def answer(self, status, body, kind="application/json", headers=()):
        if isinstance(body, (dict, list)):
            body = json.dumps(body, sort_keys=True, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/headers":
            body = json.dumps({name.lower(): value for name, value in self.headers.items()}, sort_keys=True).encode()
            return self.answer(200, body, "text/plain; charset=utf-8")
        if path == "/api/data":
            return self.answer(200, {"ok": True, "query": self.path.partition("?")[2]},
                               headers=[("Set-Cookie", "devtools=yes; Path=/")])
        if path == "/redirect":
            return self.answer(302, b"", "text/plain", headers=[("Location", "/second.html")])
        if path == "/pixel.png":
            return self.answer(200, png_pixel(), "image/png")
        if path == "/ws" and self.headers.get("Upgrade", "").lower() == "websocket":
            return self.websocket()
        return super().do_GET()

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length)
        if self.path.split("?")[0] != "/api/echo":
            return self.answer(404, {"error": "no such thing"})
        self.answer(200, {"body": json.loads(body or b"null"), "headers": {k.lower(): v for k, v in self.headers.items()}})

    def websocket(self):
        key = self.headers["Sec-WebSocket-Key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", base64.b64encode(hashlib.sha1(key.encode()).digest()).decode())
        self.end_headers()
        self.wfile.flush()
        while True:
            head = self.rfile.read(2)
            if len(head) < 2:
                break
            opcode, n = head[0] & 15, head[1] & 127
            if n == 126:
                n = int.from_bytes(self.rfile.read(2), "big")
            elif n == 127:
                n = int.from_bytes(self.rfile.read(8), "big")
            mask = self.rfile.read(4) if head[1] & 128 else b"\0\0\0\0"
            data = bytes(b ^ mask[i % 4] for i, b in enumerate(self.rfile.read(n)))
            if opcode == 8:
                self.wfile.write(b"\x88\x00")
                break
            if opcode == 1:
                reply = b"echo " + data
                self.wfile.write(bytes([0x81, len(reply)]) + reply)
                self.wfile.flush()
        self.close_connection = True

    def log_message(self, *args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
