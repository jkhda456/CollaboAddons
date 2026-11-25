#!/usr/bin/env python3
"""A proxy for the tests of `web proxy`: it forwards what the browser sends through it and logs it.

    proxy.py PORT LOG [--auth USER:PASSWORD]

One port speaks HTTP (requests with a whole URL, WebSocket upgrades among them, and CONNECT) and
SOCKS 4, 4a and 5 (told apart by the first byte). --auth: HTTP requests need Proxy-Authorization
(Basic) and SOCKS 5 the username/password method (SOCKS 4 is refused). Each line of LOG:

    http GET http://127.0.0.1:8000/headers     a request it forwarded
    connect 127.0.0.1:8443                      a tunnel (https, wss)
    socks5 127.0.0.1:8000                       a SOCKS connection (socks4, socks4a: the same)
    407 GET http://...                          refused: no credentials, or wrong ones
"""
import base64
import select
import socket
import socketserver
import sys
import threading
import urllib.parse

PORT, LOG = int(sys.argv[1]), sys.argv[2]
AUTH = sys.argv[sys.argv.index("--auth") + 1] if "--auth" in sys.argv else None
LOCK = threading.Lock()


def log(line):
    with LOCK, open(LOG, "a") as f:
        f.write(line + "\n")


def relay(a, b, first=b""):
    """bytes each way until one side ends"""
    if first:
        b.sendall(first)
    sockets = [a, b]
    try:
        while True:
            readable, _, _ = select.select(sockets, [], [], 120)
            if not readable:
                return
            for s in readable:
                data = s.recv(65536)
                if not data:
                    return
                (b if s is a else a).sendall(data)
    except OSError:
        return


def read_head(s, buffer=b""):
    while b"\r\n\r\n" not in buffer:
        data = s.recv(65536)
        if not data:
            return None, buffer
        buffer += data
    head, _, rest = buffer.partition(b"\r\n\r\n")
    return head.decode("latin-1"), rest


def recv_exactly(s, n):
    data = b""
    while len(data) < n:
        more = s.recv(n - len(data))
        if not more:
            raise OSError("closed")
        data += more
    return data


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        first = s.recv(1, socket.MSG_PEEK)
        if first == b"\x05":
            self.socks5(s)
        elif first == b"\x04":
            self.socks4(s)
        elif first:
            self.http(s)

    def http(self, s):
        buffer = b""
        while True:
            head, buffer = read_head(s, buffer)
            if head is None:
                return
            lines = head.split("\r\n")
            method, target, version = lines[0].split(" ", 2)
            headers = [tuple(part.strip() for part in line.split(":", 1)) for line in lines[1:] if ":" in line]
            given = next((v for k, v in headers if k.lower() == "proxy-authorization"), "")
            if AUTH and given != "Basic " + base64.b64encode(AUTH.encode()).decode():
                log("407 %s %s" % (method, target))
                body = b"proxy credentials wanted"
                s.sendall(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"test proxy\"\r\n"
                          b"Content-Type: text/plain\r\nContent-Length: %d\r\n\r\n%s" % (len(body), body))
                length = int(next((v for k, v in headers if k.lower() == "content-length"), "0") or 0)
                buffer = buffer[length:]
                continue  # the same connection, as proxies keep it
            break
        if method == "CONNECT":
            host, _, port = target.rpartition(":")
            log("connect %s" % target)
            upstream = socket.create_connection((host.strip("[]"), int(port)))
            s.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
            relay(s, upstream, buffer)
            upstream.close()
            return
        url = urllib.parse.urlsplit(target)
        log("http %s %s" % (method, target))
        upgrade = any(k.lower() == "upgrade" for k, _ in headers)
        out = ["%s %s %s" % (method, (url.path or "/") + ("?" + url.query if url.query else ""), version)]
        for k, v in headers:
            if k.lower() in ("proxy-authorization", "proxy-connection") or (k.lower() == "connection" and not upgrade):
                continue
            out.append("%s: %s" % (k, v))
        if not upgrade:
            out.append("Connection: close")
        upstream = socket.create_connection((url.hostname, url.port or 80))
        upstream.sendall(("\r\n".join(out) + "\r\n\r\n").encode("latin-1") + buffer)
        if upgrade:
            relay(s, upstream)
        else:
            # the answer with Connection: close (the server closes after it, so does the proxy)
            reply, rest = read_head(upstream)
            if reply is not None:
                kept = [line for line in reply.split("\r\n") if not line.lower().startswith(("connection:", "proxy-connection:"))]
                s.sendall(("\r\n".join(kept) + "\r\nConnection: close\r\n\r\n").encode("latin-1") + rest)
                relay(s, upstream)
        upstream.close()

    def socks5(self, s):
        _, count = recv_exactly(s, 2)
        methods = recv_exactly(s, count)
        method = 2 if AUTH else 0
        if method not in methods:
            log("socks5 refused: methods %s" % list(methods))
            s.sendall(b"\x05\xff")
            return
        s.sendall(bytes([5, method]))
        if AUTH:
            _, length = recv_exactly(s, 2)
            user = recv_exactly(s, length)
            password = recv_exactly(s, recv_exactly(s, 1)[0])
            if (user + b":" + password).decode() != AUTH:
                log("socks5 refused: user %s" % user.decode())
                s.sendall(b"\x01\x01")
                return
            s.sendall(b"\x01\x00")
        _, command, _, kind = recv_exactly(s, 4)
        if kind == 1:
            host = socket.inet_ntoa(recv_exactly(s, 4))
        elif kind == 3:
            host = recv_exactly(s, recv_exactly(s, 1)[0]).decode()
        else:
            host = socket.inet_ntop(socket.AF_INET6, recv_exactly(s, 16))
        port = int.from_bytes(recv_exactly(s, 2), "big")
        log("socks5 %s:%d%s" % (host, port, " (a name: looked up here)" if kind == 3 else ""))
        upstream = socket.create_connection((host, port))
        s.sendall(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
        relay(s, upstream)
        upstream.close()

    def socks4(self, s):
        _, command = recv_exactly(s, 2)
        port = int.from_bytes(recv_exactly(s, 2), "big")
        address = recv_exactly(s, 4)

        def text():
            data = b""
            while not data.endswith(b"\x00"):
                data += recv_exactly(s, 1)
            return data[:-1].decode()
        text()  # the user id
        if address[:3] == b"\x00\x00\x00" and address[3]:
            host, kind = text(), "socks4a"
        else:
            host, kind = socket.inet_ntoa(address), "socks4"
        if AUTH:
            log("%s refused (credentials wanted)" % kind)
            s.sendall(b"\x00\x5b" + b"\x00" * 6)
            return
        log("%s %s:%d" % (kind, host, port))
        upstream = socket.create_connection((host, port))
        s.sendall(b"\x00\x5a" + b"\x00" * 6)
        relay(s, upstream)
        upstream.close()


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


open(LOG, "a").close()
Server(("127.0.0.1", PORT), Handler).serve_forever()
