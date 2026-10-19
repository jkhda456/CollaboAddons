"""A pretend serial device for c-sh's tests: a pty whose far end answers like a small modem.

    python3 fakeserial.py        prints the device's path (the pty's slave), then serves:
        AT    -> OK          ATI -> c-sh test modem
        anything else        -> echoed back in capitals
    It keeps running until killed."""
import os, pty, sys, termios, tty

master, slave = pty.openpty()
path = os.ttyname(slave)
tty.setraw(master)
print(path, flush=True)
buf = b""
while True:
    try:
        data = os.read(master, 1024)
    except OSError:
        break
    if not data:
        break
    buf += data
    while b"\r" in buf or b"\n" in buf:
        cut = min(i for i in (buf.find(b"\r"), buf.find(b"\n")) if i >= 0)
        line, buf = buf[:cut].strip(), buf[cut + 1:]
        if not line:
            continue
        if line == b"AT":
            reply = b"OK"
        elif line == b"ATI":
            reply = b"c-sh test modem"
        else:
            reply = line.upper()
        os.write(master, reply + b"\r\n")
