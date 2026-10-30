"""c-sh end to end: the daemon, the command line and every backend, as one scripted session.

    python3 tests/test_csh.py native          c-sh built for this machine (build/host/debug/c-sh)
    python3 tests/test_csh.py guest RUNTIME   in the guest (out/c-sh.cpio; RUNTIME:
                                              dist/runtime/collabo-core-<platform>)

The SSH server is this machine's OpenSSH (tests/sshd.sh), which the guest reaches as 192.0.2.1;
the telnet, rlogin and raw servers (tests/servers.py) run where c-sh runs. The steps are one shell
script (one boot in the guest); each step's output and exit status land in files, and the checks
read them afterwards."""
import os, shutil, subprocess, sys, tempfile

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
mode = sys.argv[1] if len(sys.argv) > 1 else "native"
work = tempfile.mkdtemp(prefix="c-sh-test-")

# (name, shell command). $W: the shared folder; $SSH: user@host:port of the test sshd; $KEY: its
# user key; $TEL, $RAW, $RL: the local servers' ports.
STEPS = [
    ("version", "c-sh version"),
    ("list_none", "c-sh list"),
    ("help", "c-sh help"),
    ("bad_cmd", "c-sh frobnicate"),
    # A local shell: run (output, status, cwd kept), enter/wait/read, screen, keys, resize.
    ("open_sh", "c-sh open sh run --size 60x15"),
    ("open_dup", "c-sh open sh run"),
    ("wait_sh", "c-sh wait sh idle 300 --timeout 10"),
    ("run_echo", "c-sh run sh 'echo hello; echo 한글 출력'"),
    ("run_status", "c-sh run sh 'exit_code() { return 42; }; exit_code'"),
    ("run_cwd", "[ \"$(c-sh run sh pwd)\" = \"$(pwd)\" ] && echo cwd-matches"),
    ("run_cd", "c-sh run sh 'cd /tmp && pwd'"),
    ("run_pwd", "c-sh run sh pwd"),
    ("run_long", "c-sh run sh 'echo this-is-a-long-command-line-that-has-to-wrap-in-a-sixty-column-terminal-for-sure; echo after'"),
    ("run_bg", "c-sh run sh 'sleep 1 &'"),
    ("run_timeout", "c-sh run sh 'sleep 5' --timeout 1"),
    # SIGPIPE is the session's default again (c-sh ignores it): yes ends when head has had enough,
    # where an ignored one leaves busybox's yes writing for ever.
    ("run_sigpipe", "c-sh run sh 'yes 123456789 | head -3000 | wc -l; echo pipeline-ended' --timeout 30"),
    ("interrupt", "c-sh key sh ctrl+c && c-sh run sh 'echo back'"),
    ("enter", "c-sh enter sh 'echo typed-line' && c-sh wait sh text typed-line --timeout 5"),
    ("read_new", "sleep 0.3; c-sh read sh"),
    ("read_again", "c-sh read sh"),
    ("send_escape", "c-sh send sh 'echo esc\\n' && c-sh wait sh regex '(?m)^esc' --timeout 5"),
    ("read_all", "c-sh read sh --all"),
    ("resize", "c-sh resize sh 100x20 && c-sh run sh 'stty size'"),
    ("screen", "c-sh run sh clear && c-sh enter sh 'echo on-the-screen' && c-sh wait sh idle 300 && c-sh screen sh"),
    ("info", "c-sh info sh"),
    ("wait_timeout", "c-sh wait sh text never-appears --timeout 1"),
    ("no_session", "c-sh read nosuch"),
    # A full-screen program: its screen, then a key to leave it.
    ("vi_open", "c-sh enter sh 'vi $W/vi.txt' && c-sh wait sh idle 500 --timeout 10 && c-sh key sh i && c-sh send sh 'written by c-sh' && c-sh key sh Escape && c-sh wait sh idle 300 && c-sh screen sh"),
    ("vi_save", "c-sh enter sh ':wq' && c-sh wait sh idle 500 --timeout 10 && cat $W/vi.txt"),
    # A program that ends on its own.
    ("open_prog", "c-sh open once run -- sh -c 'echo one-shot; exit 9'"),
    ("wait_prog", "c-sh wait once closed --timeout 10; c-sh list"),
    ("logs_prog", "c-sh logs once"),
    ("close_prog", "c-sh close once && c-sh list"),
    # Raw TCP, telnet (login, window size, terminal type), rlogin.
    ("servers", "for k in raw telnet rlogin; do python3 $T/servers.py $k 0 > $W/srv-$k.txt & done; "
                "for i in $(seq 300); do [ -s $W/srv-raw.txt ] && [ -s $W/srv-telnet.txt ] && [ -s $W/srv-rlogin.txt ] && break; sleep 0.1; done; "
                "cat $W/srv-*.txt"),
    ("raw", "c-sh open r raw://127.0.0.1:$(cut -d' ' -f2 $W/srv-raw.txt) && c-sh wait r text READY && c-sh enter r 'hello raw' && c-sh wait r text 'echo: hello' --timeout 5 && c-sh read r"),
    ("telnet", "c-sh open t telnet://alice@127.0.0.1:$(cut -d' ' -f2 $W/srv-telnet.txt) --password secret --size 100x30 && c-sh run t size --prompt '\\$ ?$' && c-sh run t term --prompt '\\$ ?$'"),
    ("telnet_resize", "c-sh resize t 90x20 && c-sh run t size --prompt '\\$ ?$'"),
    ("telnet_pwenv", "PW=secret c-sh open t3 telnet://dave@127.0.0.1:$(cut -d' ' -f2 $W/srv-telnet.txt) --password-env PW && c-sh run t3 user --prompt '\\$ ?$' && c-sh close t3"),
    ("pw_missing", "c-sh open t4 telnet://x@127.0.0.1:1 --password-env NOT_SET_ANYWHERE"),
    ("telnet_badpw", "c-sh open t2 telnet://bob@127.0.0.1:$(cut -d' ' -f2 $W/srv-telnet.txt) --password wrong"),
    ("rlogin", "c-sh open rl rlogin://carol@127.0.0.1:$(cut -d' ' -f2 $W/srv-rlogin.txt) && c-sh wait rl idle 300 && c-sh run rl user --prompt '\\$ ?$'"),
    ("telnet_exit", "c-sh run t exit --prompt 'never' --timeout 3; c-sh wait t closed --timeout 5; c-sh list"),
    # SSH: a new host key is remembered, a shell, run, a one-shot command, a refused key.
    ("ssh_open", "c-sh open h ssh://$SSH --identity $KEY --no-agent --known-hosts $W/kh"),
    ("ssh_kh", "cut -d' ' -f2 $W/kh"),
    ("ssh_run", "c-sh wait h idle 500 --timeout 15; c-sh run h 'echo from-ssh $(id -un); echo $TERM'"),
    ("ssh_status", "c-sh run h 'sh -c \"exit 6\"'"),
    ("ssh_resize", "c-sh resize h 132x40 && c-sh run h 'stty size'"),
    ("ssh_strict", "c-sh open h2 ssh://$SSH --identity $KEY --no-agent --known-hosts $W/kh --strict && c-sh info h2"),
    ("ssh_exec", "c-sh open job ssh://$SSH --identity $KEY --no-agent --known-hosts $W/kh --command 'echo exec-out; exit 3' --no-pty && c-sh wait job closed --timeout 10; c-sh logs job; c-sh list"),
    ("ssh_unknown_strict", "c-sh open h3 ssh://$SSH --identity $KEY --no-agent --known-hosts $W/empty-kh --strict"),
    ("ssh_changed", "sed 's/AAAA/AAAB/' $W/kh > $W/bad-kh && c-sh open h4 ssh://$SSH --identity $KEY --no-agent --known-hosts $W/bad-kh"),
    ("ssh_badkey", "c-sh open h5 ssh://$SSH --identity $W/otherkey --no-agent --known-hosts $W/kh"),
    # The ssh-agent (here this machine's; in the sandbox the host's, lent through vsock): keys the
    # server does not know come first, the right one third.
    ("ssh_agent", "if command -v ssh-agent >/dev/null 2>&1; then eval \"$(ssh-agent -s)\" >/dev/null && "
                  "ssh-add -q $W/otherkey $W/key 2>/dev/null && c-sh open ag ssh://$SSH --known-hosts $W/kh && "
                  "c-sh run ag 'echo agent-ok'; r=$?; ssh-agent -k >/dev/null; exit $r; else echo SKIP; fi"),
    ("ssh_exit", "c-sh run h 'exit 4'"),
    # A serial line: a pty answering like a modem (tests/fakeserial.py) on this machine. Natively
    # c-sh opens it itself; the guest reaches it through the engine's bridge (--serial).
    ("ser_open", "c-sh open m serial:$SER --baud 9600"),
    ("ser_at", "c-sh enter m AT && c-sh wait m text OK --timeout 5 && c-sh run m ATI --idle 300"),
    # (a pty takes the speed, stop bits and flow control, but refuses parity and 5-7 data bits)
    ("ser_line", "c-sh line m baud=19200 stop=2 flow=xonxoff"),
    ("ser_modem", "c-sh line m dtr=off"),
    ("ser_break", "c-sh break m 50"),
    ("ser_info", "c-sh info m"),
    ("ser_close", "c-sh close m"),
    ("ser_reopen", "c-sh open m2 serial:$SER && c-sh close m2"),
    ("ser_ports", "if [ $MODE = guest ]; then c-sh ports; else echo SKIP; fi"),
    ("ser_notshared", "if [ $MODE = guest ]; then c-sh open x serial:host:/dev/ttyS3; else echo SKIP; fi"),
    ("ser_notty", "if [ $MODE = guest ]; then c-sh open y serial:host:$NOTTY; else echo SKIP; fi"),
    ("ser_telnet_break", "c-sh break t2 2>&1; c-sh break sh"),
    ("forget", "c-sh forget \"$(cut -d' ' -f1 $W/kh)\" --known-hosts $W/kh && wc -l < $W/kh"),
    ("shutdown", "c-sh shutdown; sleep 0.5; c-sh list"),
]

# name: (exit status, texts the output must contain, texts it must not)
CHECKS = {
    "version": (0, ["c-sh "], []),
    "list_none": (0, ["no sessions"], []),
    "help": (0, ["open NAME TARGET", "run NAME COMMAND", "ssh://"], []),
    "bad_cmd": (2, ["unknown command"], []),
    "open_sh": (0, ["opened sh"], []),
    "open_dup": (1, ["already open"], []),
    "run_echo": (0, ["hello\n", "한글 출력"], ["CSH", "printf"]),
    "run_status": (42, [], ["CSH"]),
    "run_cwd": (0, ["cwd-matches"], []),
    "run_cd": (0, ["/tmp"], []),
    "run_pwd": (0, ["/tmp"], []),
    "run_long": (0, ["this-is-a-long-command-line", "after"], ["CSH", "printf", '"$?"']),
    "run_bg": (0, [], ["CSH"]),
    "run_timeout": (3, ["timed out", "still running"], []),
    "run_sigpipe": (0, ["3000", "pipeline-ended"], ["timed out"]),
    "interrupt": (0, ["back"], []),
    "enter": (0, [], []),
    "read_new": (0, ["typed-line"], []),
    "read_again": (0, [], ["typed-line"]),
    "send_escape": (0, [], []),
    "read_all": (0, ["hello", "typed-line"], []),
    "resize": (0, ["20 100"], []),
    "screen": (0, ["on-the-screen"], ["CSHBEGIN", "CSHDONE"]),
    "info": (0, ["name:    sh", "size:    100x20", "state:   open"], []),
    "wait_timeout": (3, ["timed out"], []),
    "no_session": (4, ["no session"], []),
    "vi_open": (0, ["written by c-sh"], []),
    "vi_save": (0, ["written by c-sh"], []),
    "open_prog": (0, ["opened once"], []),
    "wait_prog": (0, ["closed (exit 9)"], []),
    "logs_prog": (0, ["one-shot"], []),
    "close_prog": (0, ["closed once"], ["once\t"]),
    "servers": (0, ["ready"], []),
    "raw": (0, ["RAW READY", "echo: hello raw"], []),
    "telnet": (0, ["logged in", "100x30", "xterm-256color"], []),
    "telnet_resize": (0, ["90x20"], []),
    "telnet_pwenv": (0, ["logged in", "dave"], []),
    "pw_missing": (2, ["is not set"], []),
    "telnet_badpw": (1, ["login failed", "Login incorrect"], []),
    "rlogin": (0, ["carol"], []),
    "telnet_exit": (None, ["t\ttelnet", "closed"], []),
    "ssh_open": (0, ["opened h", "new (trusted on first sight"], []),
    "ssh_kh": (0, ["ssh-ed25519"], []),
    "ssh_run": (0, ["from-ssh ", "xterm-256color"], ["CSH"]),
    "ssh_status": (6, [], []),
    "ssh_resize": (0, ["40 132"], []),
    "ssh_strict": (0, ["— known"], []),
    "ssh_exec": (0, ["exec-out", "job\tssh", "closed (exit 3)"], []),
    "ssh_unknown_strict": (1, ["is not in"], []),
    "ssh_changed": (1, ["CHANGED"], []),
    "ssh_badkey": (1, ["no public key was accepted", "the server accepts: publickey"], []),
    "ssh_agent": (0, ["from the ssh-agent", "agent-ok"], []),
    "ssh_exit": (4, ["session ended (exit 4)"], []),
    "ser_open": (0, ["opened m", "9600 8N1"], []),
    "ser_at": (0, ["c-sh test modem"], []),
    "ser_line": (0, ["19200 8N2 xonxoff"], []),
    "ser_modem": (1, ["now 19200 8N2", "modem lines were not set"], []),
    "ser_break": (0, ["sent a break"], []),
    "ser_info": (0, ["reaches:", "19200 8N2"], []),
    "ser_close": (0, ["closed m"], []),
    "ser_reopen": (0, ["opened m2", "closed m2"], []),
    "ser_ports": (0, ["the host shares serial ports: allow"], []),
    "ser_notshared": (1, ["not among the serial ports the app shares"], []),
    "ser_notty": (1, ["is not a serial device"], []),
    "ser_telnet_break": (1, ["cannot send a break"], []),
    "forget": (0, ["removed 1 entry", "0"], []),
    "shutdown": (0, ["no sessions"], []),
}


def script(env):
    lines = ["#!/bin/sh", "export " + " ".join("%s='%s'" % kv for kv in env.items()), 'cd "$W"']
    for name, cmd in STEPS:
        lines.append("( %s ) > \"$W/%s.out\" 2>&1; echo $? > \"$W/%s.rc\"" % (cmd, name, name))
    lines.append("c-sh shutdown >/dev/null 2>&1; true")
    return "\n".join(lines) + "\n"


def main():
    sshd = subprocess.run(["bash", os.path.join(T, "sshd.sh")], capture_output=True, text=True)
    if sshd.returncode != 0:
        sys.exit("cannot start sshd: " + sshd.stderr)
    port, _hostpub, userkey, user, sshdir = sshd.stdout.split()
    fake = subprocess.Popen([sys.executable, os.path.join(T, "fakeserial.py")], stdout=subprocess.PIPE, text=True)
    serial_dev = fake.stdout.readline().strip()
    notatty = os.path.join(work, "not-a-tty")
    open(notatty, "w").write("plain file\n")
    try:
        shutil.copy(userkey, os.path.join(work, "key"))
        os.chmod(os.path.join(work, "key"), 0o600)
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", os.path.join(work, "otherkey")], check=True)
        open(os.path.join(work, "empty-kh"), "w").close()
        if mode == "native":
            exe = os.path.join(A, "build", "host", "debug")
            env = {"W": work, "T": T, "SSH": "%s@127.0.0.1:%s" % (user, port), "KEY": work + "/key",
                   "COLLABO_CSH_DIR": work + "/csh", "SHELL": "/bin/sh", "PS1": "$ ", "MODE": "native", "SER": serial_dev,
                   "PATH": exe + ":" + os.environ["PATH"], "HOME": work}
            with open(os.path.join(work, "steps.sh"), "w") as f:
                f.write(script(env))
            subprocess.run(["sh", os.path.join(work, "steps.sh")], timeout=600, env=dict(os.environ, **env))
        else:
            rt = sys.argv[2]
            shutil.copytree(T, os.path.join(work, "t"))
            env = {"W": "/w", "T": "/w/t", "SSH": "%s@192.0.2.1:%s" % (user, port), "KEY": "/w/key", "HOME": "/w",
                   "MODE": "guest", "SER": "host:" + serial_dev, "NOTTY": notatty}
            with open(os.path.join(work, "steps.sh"), "w") as f:
                f.write(script(env))
            # The guest reads the shared folder as another user would: the keys must be readable.
            os.chmod(os.path.join(work, "key"), 0o644)
            os.chmod(os.path.join(work, "otherkey"), 0o644)
            img = os.path.join(rt, "app", "images")
            cmd = [os.path.join(rt, "bin", "collabo-core-engine"), "exec", "--kernel", img + "/vmlinux.wasm",
                   "--initramfs", img + "/initramfs.cpio", "--python-image", img + "/python.cpio",
                   "--tools-image", img + "/tools.cpio", "--addon-dir", os.path.join(A, "out"), "--addon", "c-sh",
                   "--allow-loopback", "--serial", serial_dev, "--serial", notatty,
                   "--mount", work + ":/w", "--", "/bin/sh", "/w/steps.sh"]
            r = subprocess.run(cmd, capture_output=True, timeout=1200)
            open(os.path.join(work, "engine.log"), "wb").write(r.stdout + r.stderr)
    finally:
        fake.kill()
        subprocess.run(["kill", open(os.path.join(sshdir, "sshd.pid")).read().strip()])
        shutil.rmtree(sshdir, ignore_errors=True)
        subprocess.run("pkill -f 'tests/servers.py' 2>/dev/null; pkill -f '%s/csh' 2>/dev/null" % work, shell=True)

    fails = []
    for name, _ in STEPS:
        try:
            rc = int(open(os.path.join(work, name + ".rc")).read().strip())
            out = open(os.path.join(work, name + ".out"), encoding="utf-8", errors="replace").read()
        except OSError:
            fails.append("%s: did not run" % name)
            continue
        want_rc, must, must_not = CHECKS.get(name, (0, [], []))
        if out.strip().endswith("SKIP"):
            continue
        problems = []
        if want_rc is not None and rc != want_rc:
            problems.append("exit %d, wanted %d" % (rc, want_rc))
        problems += ["missing %r" % m for m in must if m not in out]
        problems += ["unexpected %r" % m for m in must_not if m in out]
        if problems:
            fails.append("%s: %s\n    output: %s" % (name, "; ".join(problems), out.strip().replace("\n", "\n            ")[:1500]))
    print("%s: %d steps, %d failed (%s)" % (mode, len(STEPS), len(fails), work))
    for f in fails:
        print("FAIL " + f)
    if not fails:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if fails else 0)


main()
