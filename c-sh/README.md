# c-sh add-on

**c-sh** (collabo sh): terminal sessions that stay connected between commands. A small daemon
holds each connection — SSH, telnet, rlogin, a raw TCP socket, a serial line, or a program in a
terminal here — and the `c-sh` command drives it one step at a time: run a command and get its
output and exit status, type, press keys, look at the screen, wait for something to appear.
Where a shell tool connects and disconnects for every command, an agent opens a c-sh session
once and keeps working in it: the remote shell keeps its directory, its environment, its `sudo`,
the program it is in.

```
$ c-sh open web ssh://deploy@10.0.0.5
opened web: ssh deploy@10.0.0.5:22
host key SHA256:rACR…Ki9g — new (trusted on first sight and remembered); logged in with publickey ssh-ed25519 from the ssh-agent (me@laptop)

$ c-sh run web 'cd /srv/app && git log --oneline -3'
4f1c2e0 Fix the cache key
…
$ c-sh run web 'test -f .env'; echo $?          # the remote command's exit status
1
$ c-sh run web pwd                                # still there
/srv/app
$ c-sh enter web 'top'; c-sh screen web           # a full-screen program: its screen as text
$ c-sh key web q
$ c-sh close web
```

| In the guest | |
|---|---|
| `/usr/bin/c-sh` | the command and its daemon, one program (Rust, `rust/`, 2.4 MB) |

## Turning it on

```json
"addons": {"c-sh": {}}
```

or `--addon c-sh` on the engine's command line (it is bundled with the runtime). SSH through the
host's ssh-agent needs the app's `sshAgent` (then `SSH_AUTH_SOCK` is set in the guest); the
network policy applies to every connection, as to everything in the guest.

From the host, the Dart package wraps `c-sh` as LLM tools: `TermTools` (`term_open`,
`term_run` — output and exit code —, `term_input` — text and keys —, `term_read`,
`term_screen`, `term_wait`, `term_list`, `term_close`), next to `SandboxTools`, `GuiTools` and
`WebTools` (`addons/mod` 0028).

## Commands

`c-sh help` lists them; the ones an agent uses most:

| Command | |
|---|---|
| `open NAME TARGET [options]` | connect and keep the session; prints the host key decision and how it logged in |
| `run NAME COMMAND [--timeout S]` | (a POSIX shell: sh, bash, zsh, ash) runs COMMAND and prints its output; c-sh's exit status is the command's (3: still running after the timeout; the session's status when the command ended the shell) |
| `run NAME COMMAND --prompt RE` | anything that is not a POSIX shell — a router's CLI, a REPL, a BBS: until the last line matches RE (`--idle MS`: until the output is quiet) |
| `enter NAME TEXT`, `send NAME TEXT`, `key NAME KEY...` | a line of input, text as typed (`\n` `\r` `\t` `\e` escapes), keys (`Enter`, `Tab`, `Esc`, arrows, `PageUp`, `F1`–`F12`, `ctrl+c`, `alt+x`, `shift+Tab`, `ctrl+Up`, …) |
| `screen NAME` | the screen as a person would see it now (curses programs too) |
| `read NAME [--all \| --since N] [--timeout MS] [--raw]` | output since the last `read` (or `run`), as plain text: escape sequences out, carriage returns applied (a progress bar reads as its last state) |
| `wait NAME [text STR \| regex RE \| idle MS \| closed] [--timeout S]` | until the output shows it |
| `list`, `info NAME`, `resize NAME WxH`, `logs NAME`, `close NAME` | the sessions; one in detail (size, cursor, idle time, host key, how it logged in); its terminal's size; its transcript (the last MiB); end it |
| `ports`, `break NAME [MS]`, `line NAME baud=N dtr=off ...` | the host's serial ports and whether this sandbox may open each; a break (serial, telnet BRK, SSH break); a serial line's settings and modem lines |
| `forget HOST[:PORT]`, `shutdown` | drop a host's keys from known_hosts; stop the daemon, closing every session |

Exit status: 0 done (`run`: the command's), 1 failed, 2 usage, 3 timed out, 4 no such session,
5 the daemon could not be reached.

### Targets

| | |
|---|---|
| `ssh://[user@]host[:port]`, `user@host`, `host` | SSH (the default scheme) |
| `telnet://[user@]host[:port]` | telnet (NAWS window size, terminal type, ECHO/SGA) |
| `rlogin://[user@]host[:port]` | rlogin (RFC 1282) |
| `raw://host:port`, `tcp://host:port` | a TCP socket, bytes straight through |
| `serial:/dev/ttyUSB0?baud=9600&parity=even`, `serial:COM3,115200,8,n,1,N` | a serial line: this system's device if it has one, else **the host's**, through the engine (`serial:host:PATH` always the host's) |
| `serial:usb:VID:PID[:SERIAL]` | the host's USB serial adapter by identity, wherever it is plugged in (`c-sh ports` shows the identities) |
| `run`, `run:COMMAND`, `run -- PROGRAM ARG...` | the login shell, a shell command line, or a program, in a terminal here (in the folder `open` ran in) |

Options of `open`: `--size WxH` (80x24), `--term NAME` (xterm-256color), `--env KEY=VALUE`,
`--connect-timeout S`; SSH: `--user`, `--identity FILE` (repeatable), `--passphrase`,
`--no-agent` / `--agent SOCKET`, `--command CMD` (one command instead of a shell; the session
ends with it), `--no-pty`, `--known-hosts FILE`, `--accept-new` / `--strict` / `--insecure`;
serial: `--baud N`, `--data-bits 5-8`, `--parity none|even|odd|mark|space`, `--stop-bits 1|2`,
`--flow none|rtscts|xonxoff`, or `--line 115200,8,n,1,N` (PuTTY's form); the target's query
(`?baud=…&parity=…`) works too, the options win. telnet, rlogin, serial and raw: `--user` and `--password` answer the `login:` and `Password:`
prompts. `--password-env VAR` and `--password-file FILE` (and `--passphrase-env` / `-file`) keep a
secret out of every command line: the command reads it and hands it to the daemon over its socket.

### Host keys

known_hosts (`~/.ssh/known_hosts`, OpenSSH's format, hashed entries too) decides: a key seen for
the first time is trusted and remembered (`--accept-new`, the default — as OpenSSH's
`StrictHostKeyChecking=accept-new`), a key that differs from the one recorded is **refused**,
with the fingerprint and the line, and `c-sh forget HOST` once the change is known to be
legitimate. `--strict` refuses unknown keys too; `--insecure` takes any key (for throwaway
hosts; `info` says so). `open` and `info` show the fingerprint and what was decided.

### Serial ports and USB serial adapters

The sandbox has no serial devices of its own. The computer's — USB serial adapters
(`/dev/ttyUSB*`, `/dev/ttyACM*`, `/dev/cu.usbserial-*`, `COM3`), on-board UARTs — reach it through
the engine (vsock port 1083, `engine/src/serial.rs`), as the app's `serial` setting allows:
`off` (the default: none), `allow` with a list (paths, globs, or USB identities
`usb:VID:PID[:SERIAL]`) or an empty one (every serial port the system lists), `ask` (the listed
ones, and the app is asked about each other one as it is opened). On the engine's command line:
`--serial /dev/ttyUSB0`, `--serial usb:0403:6001`, `--serial '*'` (launcher.conf `serial =`).

```
$ c-sh ports
the host shares serial ports: allow
/dev/ttyACM0    allowed  usb:2341:0043 Arduino (www.arduino.cc) Arduino Uno [cdc_acm]
/dev/ttyUSB0    allowed  usb:0403:6001:A50285BI FTDI FT232R USB UART [ftdi_sio]
                         /dev/serial/by-id/usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0
$ c-sh open board serial:usb:0403:6001 --baud 115200
$ c-sh enter board ''; c-sh wait board regex 'login: $'      # a board's console
$ c-sh run board 'uname -a'                                  # once logged in to its shell
$ c-sh break board                                           # e.g. into a bootloader
$ c-sh line board baud=9600 dtr=off rts=on                   # shows CTS/DSR/DCD/RI too
```

Only terminal devices are ever opened, whatever the list says; "every serial port" never includes a
terminal or a pty (the user's own), which are shared only when named. The engine opens a port
exclusively (TIOCEXCL) and releases it when the session closes; `c-sh close` waits until the host
has closed it, so it can be opened again at once. The app hears of every listing, open, refusal
and close (`serial` events); Dart: `CollaboConfig(serial: SerialPolicy.allow(['usb:0403:6001']))`,
`SerialPolicy.ask()`, `sandbox.serialEvents`, `updatePolicy(serial: …)`.

USB identities come from the system (Linux: sysfs, with the `/dev/serial/by-id` names); macOS and
Windows list their ports by name only, so there a device is named by its path (`COM3`).

### Authentication

publickey with the ssh-agent's keys first — in the sandbox, the host's agent, so the keys never
enter the guest — then `--identity` files (OpenSSH format, bcrypt-encrypted ones with a
passphrase, and PEM: PKCS#1, SEC1, PKCS#8), else `~/.ssh/id_{ed25519,ecdsa,rsa}`; then
keyboard-interactive and password, with `--password`. RSA keys sign with rsa-sha2-512/256 as the
server's `server-sig-algs` allows.

## How it works

- **One daemon, a thread per session.** The first `c-sh open` starts `c-sh daemon` (its files in
  `/tmp/.collabo-csh`, `COLLABO_CSH_DIR`: the socket, `daemon.log`); each command is a request on
  its Unix socket. A session's reader thread waits on the connection and feeds what arrives into
  the session's terminal; commands send input and read the terminal. One lock keeps the two from
  touching the connection at once.
- **The terminal.** A vt100 screen model (the `vt100` crate) gives `screen`; a transcript (the
  last MiB, with absolute offsets) gives `read`, `wait` and `logs`. The terminal answers the
  queries programs make (cursor position, device attributes), as a terminal would.
- **`run`.** c-sh types the command between two markers, `printf`ed by the shell itself: one
  before the command runs, one after it with `$?`. The output is what lies between them, as plain
  text, and the exit status is the second marker's number. The markers are typed split by quotes
  (`"__CSHBEG""IN_…"`), so the echo of the typed line — however the terminal wraps it — never
  contains them, and each erases itself from the screen once printed (CR, erase-line), so
  `screen` stays clean. A leading space keeps the line out of bash's history
  (`HISTCONTROL=ignorespace`). A shell that ends (`exit`) ends the wait at once, with its status.
- **SSH is c-sh's own** (`rust/src/ssh`): the transport (version exchange, the binary packet
  protocol, key exchange, rekeying, strict kex), user authentication and a session channel with a
  pty, written for c-sh. Key exchange: curve25519-sha256, ECDH on P-256/384/521,
  diffie-hellman-group14/16/18 (SHA-256/512, group14-sha1 last); host keys ssh-ed25519,
  ecdsa-sha2-nistp256/384/521, rsa-sha2-512/256, ssh-rsa; ciphers chacha20-poly1305@openssh.com,
  aes256/128-gcm@openssh.com, aes256/192/128-ctr, aes256/128-cbc; MACs hmac-sha2-256/512 (and
  their -etm@openssh.com forms), hmac-sha1. The cryptographic primitives are the guest's OpenSSL
  (`python/deps/lib/libcrypto.a`, the library python, curl and git link), called directly
  (`rust/src/crypto.rs`, no binding crate); Blowfish for bcrypt-encrypted keys is in Rust (bcrypt's
  key schedule is not OpenSSL's), with its initial tables computed from π
  (`rust/src/ssh/privatekey/bcrypt/bcrypt_init.rs`).
- **Serial through the engine.** `c-sh` connects to the engine's bridge (vsock 1083) and asks it
  to open the device with the line's settings; then the connection carries frames — data either
  way, and JSON control messages: break, the modem lines (DTR, RTS; CTS, DSR, DCD, RI), new line
  settings, close. The engine's side is termios and the modem ioctls on Linux and macOS, the COM
  port functions on Windows, each port read by a thread of its own.
- **telnet, rlogin, raw, serial, local** (`rust/src/backend`): telnet's option negotiation (ECHO,
  SGA, NAWS — the window size, again on `resize` — TTYPE), rlogin's handshake, a raw socket, a
  tty in raw mode, a program on a pty (`posix_spawn` with `POSIX_SPAWN_SETSID`, so the pty is its
  controlling terminal: Ctrl-C reaches it). The guest has no `fork`, and its std cannot duplicate
  a descriptor: c-sh does both through `posix_spawn` and `fcntl(F_DUPFD_CLOEXEC)`.

## Limits

- Not in this version: SFTP/scp, port forwarding, X11 and agent forwarding, the
  diffie-hellman-group-exchange key exchanges and post-quantum hybrids (servers that offer only
  those cannot be reached), SSH certificates, compression, rlogin's out-of-band window-size
  requests (the engine's TCP carries no urgent data; the size goes with the handshake).
- `run` needs a POSIX shell at the far end (sh, bash, zsh, ash, dash, ksh); `run --prompt` covers
  the rest without an exit status. A command that waits for input (a pager, `sudo`'s password)
  holds `run` until its timeout; `screen`, `enter` and `key` take over from there.
- Password and keyboard-interactive authentication are checked against the protocol, not against
  a real login database (the tests' sshd runs unprivileged, without PAM).
- Serial: the standard rates on Linux (50 to 4000000 baud; macOS and Windows take any the driver
  does). USB identities only on Linux. The Windows code type-checks but has not run on Windows
  yet. Other USB devices (not serial) are not reachable yet.

## Building and tests

```sh
./build.sh userspace python          # musl, compiler-rt; OpenSSL for the guest
addons/c-sh/build.sh                 # -> out/c-sh.cpio (or: python3 addons/build.py build c-sh)
addons/c-sh/tests/run.sh             # unit tests, the session natively, the session in the guest
```

`tests/run.sh` needs this machine's OpenSSH (`sshd`, `ssh-keygen`; `tests/sshd.sh` runs a
throwaway server, which the guest reaches as 192.0.2.1) and `libcrypto.so.3` (native builds
link it). The unit tests check the cryptography against published vectors (SHA, HMAC RFC 4231,
Poly1305 and ChaCha20 RFC 8439, bcrypt_pbkdf's OpenBSD vector), read every kind of key
`ssh-keygen` writes, and connect to OpenSSH with each cipher, MAC and key exchange c-sh offers.
`tests/test_csh.py` is one scripted session (67 steps: local shells and programs, `vi`, telnet
with a login, rlogin, raw, SSH with new, known, changed and refused host keys, an exec session,
refused keys, the ssh-agent, the exits and timeouts, a serial line — a pty answering like a
modem, `tests/fakeserial.py`, opened directly natively and through the engine's bridge in the
guest, with a refused device and a refused file that is not a terminal) run the same way natively
and in one guest boot; the telnet, rlogin and raw servers are `tests/servers.py`, where c-sh runs.

## Licenses

c-sh: MIT (crates: `out/licenses/crates.txt`). OpenSSL (linked in): Apache-2.0, its
license with python.cpio's.
