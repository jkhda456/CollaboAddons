#!/usr/bin/env python3
"""Sends an HTTP message as it is (web network export --as raw) and prints the response: its status
line, headers and body (by its Content-Length: the request may keep the connection open).

    send-raw.py HOST PORT FILE
"""
import socket
import sys

host, port, path = sys.argv[1], int(sys.argv[2]), sys.argv[3]
with socket.create_connection((host, port), timeout=10) as s:
    s.sendall(open(path, "rb").read())
    data = b""
    while b"\r\n\r\n" not in data:
        data += s.recv(65536)
    head, _, body = data.partition(b"\r\n\r\n")
    length = next((int(line.split(b":", 1)[1]) for line in head.split(b"\r\n") if line.lower().startswith(b"content-length:")), 0)
    while len(body) < length:
        body += s.recv(65536)
print(head.decode("latin-1"))
print()
print(body.decode("utf-8", "replace"))
