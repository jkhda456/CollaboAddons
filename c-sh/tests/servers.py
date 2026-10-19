"""Small servers for c-sh's tests (standard library only, so they run here and in the guest):

    python3 servers.py raw PORT      a banner, then each line echoed back as "echo: LINE"
    python3 servers.py telnet PORT   option negotiation (ECHO, SGA, NAWS, TTYPE), a login and
                                     password prompt, then a tiny shell: size, term, echo, exit
    python3 servers.py rlogin PORT   the RFC 1282 handshake, then the same tiny shell

Prints "ready PORT" once listening (PORT 0 picks a free one)."""
import socket
import socketserver
import sys
import threading

IAC, DONT, DO, WONT, WILL, SB, SE = 255, 254, 253, 252, 251, 250, 240
ECHO, SGA, TTYPE, NAWS = 1, 3, 24, 31


class Lines:
    """Reads input a line at a time from a socket, through a byte filter (telnet's)."""

    def __init__(self, sock, filt=None, echo=False):
        self.sock, self.filt, self.echo, self.buf = sock, filt, echo, b""

    def line(self):
        while True:
            for sep in (b"\r\n", b"\r\0", b"\n", b"\r"):
                i = self.buf.find(sep)
                if i >= 0:
                    line, self.buf = self.buf[:i], self.buf[i + len(sep):]
                    if self.echo:
                        self.sock.sendall(b"\r\n")
                    return line.decode("utf-8", "replace")
            data = self.sock.recv(4096)
            if not data:
                return None
            if self.filt:
                data = self.filt(data)
            if self.echo:
                self.sock.sendall(data.replace(b"\r", b"").replace(b"\n", b"").replace(b"\0", b""))
            self.buf += data


def tiny_shell(sock, lines, info):
    sock.sendall(b"$ ")
    while True:
        line = lines.line()
        if line is None:
            return
        cmd = line.strip()
        if cmd == "exit":
            sock.sendall(b"bye\r\n")
            return
        if cmd == "size":
            out = "%sx%s" % (info.get("cols"), info.get("rows"))
        elif cmd == "term":
            out = info.get("term", "?")
        elif cmd == "user":
            out = info.get("user", "?")
        elif cmd.startswith("echo "):
            out = cmd[5:]
        elif cmd == "":
            out = None
        else:
            out = "unknown: " + cmd
        if out is not None:
            sock.sendall(out.encode() + b"\r\n")
        sock.sendall(b"$ ")


class Raw(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        s.sendall(b"RAW READY\r\n")
        lines = Lines(s)
        while True:
            line = lines.line()
            if line is None:
                return
            s.sendall(("echo: %s\r\n" % line).encode())


class Telnet(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        info = {}
        state = {"mode": "data", "sub": b""}

        def filt(data):
            out = b""
            for b in data:
                m = state["mode"]
                if m == "data":
                    if b == IAC:
                        state["mode"] = "iac"
                    else:
                        out += bytes([b])
                elif m == "iac":
                    if b == IAC:
                        out += bytes([IAC])
                        state["mode"] = "data"
                    elif b in (DO, DONT, WILL, WONT):
                        state["mode"] = ("opt", b)
                    elif b == SB:
                        state["mode"], state["sub"] = "sub", b""
                    else:
                        state["mode"] = "data"
                elif isinstance(m, tuple):
                    cmd = m[1]
                    if cmd == WILL and b == TTYPE:
                        s.sendall(bytes([IAC, SB, TTYPE, 1, IAC, SE]))  # SEND
                    state["mode"] = "data"
                elif m == "sub":
                    if b == IAC:
                        state["mode"] = "subiac"
                    else:
                        state["sub"] += bytes([b])
                elif m == "subiac":
                    if b == SE:
                        sub = state["sub"]
                        if sub[:1] == bytes([NAWS]) and len(sub) >= 5:
                            info["cols"] = sub[1] * 256 + sub[2]
                            info["rows"] = sub[3] * 256 + sub[4]
                        elif sub[:2] == bytes([TTYPE, 0]):
                            info["term"] = sub[2:].decode()
                        state["mode"] = "data"
                    else:
                        state["sub"] += bytes([b])
                        state["mode"] = "sub"
            return out

        s.sendall(bytes([IAC, WILL, ECHO, IAC, WILL, SGA, IAC, DO, NAWS, IAC, DO, TTYPE]))
        lines = Lines(s, filt, echo=True)
        s.sendall(b"Welcome to the test telnet server\r\nlogin: ")
        user = lines.line()
        if user is None:
            return
        info["user"] = user
        lines.echo = False
        s.sendall(b"Password: ")
        password = lines.line()
        lines.echo = True
        if password != "secret":
            s.sendall(b"\r\nLogin incorrect\r\n")
            return
        s.sendall(b"\r\n")
        tiny_shell(s, lines, info)


class Rlogin(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        data = b""
        while data.count(b"\0") < 4:
            more = s.recv(1024)
            if not more:
                return
            data += more
        parts = data.split(b"\0")
        # parts: ['', local, remote, term/speed, rest...]
        info = {"user": parts[2].decode(), "term": parts[3].decode().split("/")[0]}
        s.sendall(b"\0")
        s.sendall(("rlogin as %s from %s\r\n" % (parts[2].decode(), parts[1].decode())).encode())
        lines = Lines(s, echo=True)
        lines.buf = b"\0".join(parts[4:])
        tiny_shell(s, lines, info)


def main():
    kind, port = sys.argv[1], int(sys.argv[2])
    handler = {"raw": Raw, "telnet": Telnet, "rlogin": Rlogin}[kind]
    socketserver.ThreadingTCPServer.allow_reuse_address = True
    server = socketserver.ThreadingTCPServer(("127.0.0.1", port), handler)
    server.daemon_threads = True
    print("ready", server.server_address[1], flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
