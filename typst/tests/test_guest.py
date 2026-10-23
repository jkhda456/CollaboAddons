#!/usr/bin/env python3
"""The typst add-on in the guest: what it installs, its output against the official binary's
(PDF, PNG, SVG, HTML byte for byte), the CLI (diagnostics, eval, query, init, --input, stdin),
`typst watch` (polling: the guest kernel has no inotify), packages through the host's request
API (a fake one on the host's loopback, and the real one under a deny rule), the report template,
and speed. Run by tests/run.sh; see common.py for the environment variables.
"""
import io
import json
import os
import re
import shutil
import socketserver
import sys
import tarfile
import threading
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (A, B, Timer, check, guest, png_difference, present, ref, reference,  # noqa: E402
                    same_files, skip, summary, workdir)

DEV_ASSETS = os.path.join(B, "vendor", "typst-dev-assets-0.15.1", "files")

HELLO = r"""#set document(title: [시험 문서])
#set text(lang: "ko")
#set heading(numbering: "1.")
#outline()

= 수식과 코드 <intro>
안녕하세요, *Typst*! _기울임_ 그리고 `raw`.#footnote[각주입니다.] 링크: #link("https://typst.app")[Typst].
참조: @intro, @tbl.

$ sum_(i=1)^n i = (n(n+1))/2 $ 그리고 인라인 $alpha + beta^2 <= sqrt(x)$.

```rust
fn main() { println!("안녕 {}", 1 + 2); }
```

#figure(table(columns: 3, [가], [나], [다], [1], [2], [3]), caption: [표 시험]) <tbl>

= English
#set text(lang: "en")
#set par(justify: true)
#lorem(120)

/ Term: description
+ one
+ two
"""

ASSETS_DOC = r"""#set page(width: 16cm, height: auto)
#grid(columns: 3, gutter: 4pt,
  image("assets/images/rhino.png"), image("assets/images/tiger.jpg"), image("assets/images/small.gif"),
  image("assets/images/small.webp"), image("assets/images/tetrahedron.svg"), image("assets/images/glacier.jpg"),
)
#let zoo = csv("assets/data/zoo.csv")
#table(columns: zoo.first().len(), ..zoo.flatten())
#json("assets/data/zoo.json").map(a => a.name).join(", ")
#yaml("assets/data/yaml-types.yaml").keys().join(", ")
#toml("assets/data/toml-types.toml").keys().join(", ")
#xml("assets/data/hello.xml").first().tag
#let p = plugin("assets/plugins/hello.wasm")
#str(p.double_it(bytes("plugin!")))
#cite(<netwok>) and #cite(<arrgh>, form: "prose").
#bibliography("assets/bib/works.bib", style: "ieee")
"""

BROKEN = """= Price
It costs $5 and *more.
"""

# A PDF as an image: its PDF output is the same, but rasterizing it (hayro on vello_cpu) runs
# SIMD code (NEON natively, scalar in the guest) whose rounding differs: anti-aliased edges can
# differ by a level or two.
PDF_IMAGE_DOC = r"""#set page(width: 12cm, height: auto)
#image("assets/images/star.pdf", width: 50%) #image("assets/images/diagrams.pdf")
"""

DATA_DOC = r"""#let who = sys.inputs.at("who", default: "nobody")
#let n = int(sys.inputs.at("n", default: "0"))
#datetime.today().display("[year]-[month]-[day]") / #who / #(n * 2)
#calc.pow(2, 62) #regex("[a-z]+") #("a,b,c".split(","))
"""

# The Typst Universe packages the image carries, without network.
UNIVERSE_DOC = r"""#import "@preview/cmarker:0.1.10"
#import "@preview/mitex:0.2.7": mitex
#import "@preview/cetz:0.5.2"
#import "@preview/cetz-plot:0.1.4": plot
#import "@preview/lilaq:0.6.0" as lq
#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge
#set text(lang: "ko")
#set page(width: 14cm, height: auto)

#cmarker.render("# Markdown 제목\n\n*강조*와 **굵게**, `코드`, [링크](https://typst.app).\n\n- 항목\n\n| a | b |\n|---|---|\n| 1 | 2 |\n")
#mitex(`\frac{a}{b} + \sqrt{x^2 + y^2} = \sum_{i=1}^{n} i`)
#lq.diagram(width: 6cm, height: 4cm, lq.plot((1, 2, 3, 4), (1, 4, 9, 16)))
#cetz.canvas({ plot.plot(size: (4, 3), { plot.add(((0, 0), (1, 1), (2, 4))) }) })
#diagram(node((0, 0), [A]), edge("->"), node((1, 0), [B]))
"""

DEMO_LIB = '#let hello(name) = [Hello, #name, from demo!]\n'
DEMO_TOML = """[package]
name = "demo"
version = "0.1.0"
entrypoint = "lib.typ"

[template]
path = "template"
entrypoint = "main.typ"
"""
DEMO_MAIN = '#import "@preview/demo:0.1.0": hello\n#hello("template")\n'


def demo_archive():
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name, text in (("typst.toml", DEMO_TOML), ("lib.typ", DEMO_LIB), ("template/main.typ", DEMO_MAIN)):
            data = text.encode()
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


class FakeRequestApi(socketserver.ThreadingTCPServer):
    """The host's request API protocol (engine/src/http.rs) on loopback, serving a fake Typst
    Universe: demo 0.1.0, a package whose download breaks, one the policy refuses, 404s."""
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self):
        self.archive = demo_archive()
        self.requests = []
        super().__init__(("127.0.0.1", 0), self.Handler)
        threading.Thread(target=self.serve_forever, daemon=True).start()

    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            head = b""
            while not head.endswith(b"\n\n"):
                byte = self.request.recv(1)
                if not byte:
                    return
                head += byte
            line = head.decode().split("\n")[0]
            self.server.requests.append(head.decode())
            url = line.split(" ", 1)[1]
            send = self.request.sendall
            if url.endswith("/preview/demo-0.1.0.tar.gz"):
                data = self.server.archive
                send(b"200 OK\ncontent-type: application/gzip\n\n")
                for i in range(0, len(data), 100):   # many small chunks
                    chunk = data[i:i + 100]
                    send(b"%x\n" % len(chunk) + chunk)
                send(b"0\nOK\n")
            elif url.endswith("/preview/index.json"):
                body = json.dumps([{"name": "demo", "version": "0.1.0"}]).encode()
                send(b"200 OK\n\n%x\n" % len(body) + body + b"0\nOK\n")
            elif url.endswith("/preview/broken-0.1.0.tar.gz"):
                send(b"200 OK\n\n5\n\x1f\x8b\x08\x00\x00" + b"0\nERROR network: response body interrupted: reset\n")
            elif url.endswith("/preview/blocked-0.1.0.tar.gz"):
                send(b"ERROR denied: packages.typst.org is not in the allow list\n")
            else:
                send(b"404 Not Found\n\n0\nOK\n")


def outputs(folder):
    return sorted(os.listdir(folder)) if os.path.isdir(folder) else []


def main():
    if not present():
        return 2
    reference()
    work = workdir("typst-test-")
    try:
        return checks(work)
    finally:
        shutil.rmtree(work, ignore_errors=True)


def checks(work):

    # ── what the add-on installs ───────────────────────────────────────────────────────────
    rc, out = guest(work, "typst --version; echo ==; typst fonts; echo ==; typst info; echo ==; "
                          "ls /usr/share/doc/typst /usr/share/doc/typst/examples "
                          "/usr/share/typst/packages/collabo/report/0.1.0 "
                          "/etc/codex/skills/typst-report /root/.agent/skills/typst-report /root/.claude/skills/typst-report")
    check("typst --version: 0.15.1 at v0.15.1's commit, says it is the WASM port",
          "typst 0.15.1 (9dfd3a08) [WASM native port for collaboCore]" in out, out)
    fonts = ("NanumGothic", "NanumGothicCoding", "NanumMyeongjo", "Libertinus Serif", "New Computer Modern",
             "New Computer Modern Math", "DejaVu Sans Mono")
    check("typst fonts: the Nanum fonts and the embedded ones", all(f"\n{f}\n" in out for f in fonts), out)
    check("typst info: the shared package path", "Shared package path /usr/share/typst/packages" in out
          and "linux on wasm32, WASM native port for collaboCore" in out, out)
    check("the guide, examples, the template package and the skills are installed",
          all(f in out for f in ("AGENTS.md", "report-en.typ", "lib.typ", "typst.toml", "SKILL.md"))
          and out.count("SKILL.md") == 3, out)

    # ── output identical to the official binary's ──────────────────────────────────────────
    shutil.copytree(DEV_ASSETS, os.path.join(work, "assets"), ignore=shutil.ignore_patterns("fonts", "screenshots"))
    for name, text in (("hello.typ", HELLO), ("assets-doc.typ", ASSETS_DOC), ("data.typ", DATA_DOC),
                       ("pdf-image.typ", PDF_IMAGE_DOC)):
        with open(os.path.join(work, name), "w") as f:
            f.write(text)
    shutil.copy(os.path.join(A, "share/doc/examples/report-en.typ"), work)
    jobs = [
        ("hello.typ", "hello.pdf", []), ("hello.typ", "hello-{p}.png", ["--ppi", "100"]),
        ("hello.typ", "hello-{p}.svg", []), ("hello.typ", "hello.html", ["--features", "html", "--format", "html"]),
        ("hello.typ", "hello-a2b.pdf", ["--pdf-standard", "a-2b"]),
        ("assets-doc.typ", "assets.pdf", []), ("assets-doc.typ", "assets-{p}.png", []),
        ("data.typ", "data.pdf", ["--input", "who=에이전트", "--input", "n=21"]),
        ("report-en.typ", "report-en.pdf", []), ("report-en.typ", "report-en-{p}.png", ["--ppi", "60"]),
        ("pdf-image.typ", "pdf-image.pdf", []), ("pdf-image.typ", "pdf-image-{p}.png", []),
    ]
    os.makedirs(os.path.join(work, "ref"))
    os.makedirs(os.path.join(work, "guest"))
    os.chmod(os.path.join(work, "guest"), 0o777)
    ref_errors = []
    for src, dst, args in jobs:
        rc, _, err = ref(["compile", *args, src, f"ref/{dst}"], cwd=work)
        if rc != 0:
            ref_errors.append(f"{src} -> {dst}: {err}")
    check("the reference compiles the documents", not ref_errors, "\n".join(ref_errors))
    script = "; ".join(f"typst compile {' '.join(args)} {src} 'guest/{dst}' || echo FAILED {dst}" for src, dst, args in jobs)
    with Timer() as t:
        rc, out = guest(work, script)
    check("the guest compiles the documents", "FAILED" not in out and rc == 0, out)
    names = [n for n in outputs(os.path.join(work, "ref")) if not n.startswith("pdf-image-")]
    bad = same_files(os.path.join(work, "ref"), os.path.join(work, "guest"), names)
    check(f"PDF, PNG, SVG, HTML and PDF/A identical to the official binary's ({len(names)} files)",
          not bad and len(names) >= 12, f"differ: {bad}; reference made {names}")
    rendered = [n for n in outputs(os.path.join(work, "ref")) if n.startswith("pdf-image-")]
    diffs = [(n, *png_difference(os.path.join(work, "ref", n), os.path.join(work, "guest", n)))
             for n in rendered if os.path.exists(os.path.join(work, "guest", n))]
    print("     PDF images rasterized: " + ", ".join(f"{n}: {d} of {t} pixels differ, by up to {m}" for n, d, t, m in diffs))
    check("PDF images: the same PDF; their PNG within two levels on under 1% of the pixels",
          rendered and len(diffs) == len(rendered) and all(m <= 2 and d * 100 < t for _, d, t, m in diffs)
          and not same_files(os.path.join(work, "ref"), os.path.join(work, "guest"), ["pdf-image.pdf"]), diffs)

    # Threads: one job or the default (2 CPUs) give the same bytes.
    rc, out = guest(work, "typst compile -j 1 hello.typ guest/hello-j1.pdf && typst compile -j 4 hello.typ guest/hello-j4.pdf",
                    cpus=4)
    pdfs = [open(os.path.join(work, p), "rb").read() if os.path.exists(os.path.join(work, p)) else p
            for p in ("guest/hello-j1.pdf", "guest/hello-j4.pdf", "ref/hello.pdf")]
    check("-j 1 and -j 4 (4 CPUs) give the reference's PDF", rc == 0 and pdfs[0] == pdfs[1] == pdfs[2], out)

    # ── the CLI ────────────────────────────────────────────────────────────────────────────
    with open(os.path.join(work, "broken.typ"), "w") as f:
        f.write(BROKEN)
    rc, out = guest(work, "typst compile --diagnostic-format short broken.typ; echo \"status $?\"; "
                          "typst eval --in hello.typ 'query(heading).map(h => h.body.text)'; "
                          "typst eval '(1 + 2) * 7'; "
                          "typst query hello.typ '<tbl>' --field kind 2>/dev/null; "
                          "printf '= From stdin\\n' | typst compile - guest/stdin.pdf && echo stdin-ok; "
                          "typst compile nonexistent.typ; echo \"status $?\"")
    _, _, ref_err = ref(["compile", "--diagnostic-format", "short", "broken.typ"], cwd=work)
    check("a broken document: exit status 1, the same diagnostics as the reference",
          "status 1" in out and ref_err.strip() and ref_err.strip() in out, f"guest: {out}\nreference: {ref_err}")
    check("typst eval --in: the headings", '["차 례","수식과 코드","English"]' in out, out)
    check("typst eval: an expression", "\n21\n" in f"\n{out}", out)
    check("typst query (deprecated, still there)", '["table"]' in out, out)
    check("typst compile - (from stdin)", "stdin-ok" in out, out)
    check("a missing file: an error, exit status 1", "input file not found (searched at nonexistent.typ)" in out
          and out.count("status 1") == 2, out)

    # ── the report template ────────────────────────────────────────────────────────────────
    rc, out = guest(work, "typst init @collabo/report r1 && typst init @collabo/report:0.1.0 r2 && "
                          "cd r1 && typst compile main.typ ../guest/template.pdf && typst compile main.typ '../guest/template-{p}.png' --ppi 50 "
                          "&& typst eval --in main.typ 'query(figure).len()' && ls")
    check("typst init @collabo/report (with and without a version), offline", rc == 0
          and all(f in out for f in ("data.json", "incidents.csv", "main.typ")), out)
    os.makedirs(os.path.join(work, "reft"))
    rc, _, err = ref(["init", "@collabo/report:0.1.0", "reft/r"], cwd=work)
    rc2, _, err2 = ref(["compile", "main.typ", "../../ref/template.pdf"], cwd=os.path.join(work, "reft/r"))
    ref(["compile", "main.typ", "../../ref/template-{p}.png", "--ppi", "50"], cwd=os.path.join(work, "reft/r"))
    pages = [n for n in outputs(os.path.join(work, "ref")) if n.startswith("template")]
    bad = same_files(os.path.join(work, "ref"), os.path.join(work, "guest"), pages)
    check(f"the template's report identical to the reference's ({len(pages)} files)", rc == rc2 == 0
          and len(pages) >= 5 and not bad, f"{err}{err2} differ: {bad}")
    check("the template's captioned charts and table are figures", "\n3\n" in f"\n{out}", out)

    # ── typst watch: polls (no inotify in the guest kernel), sees guest and host changes ──
    with open(os.path.join(work, "w.typ"), "w") as f:
        f.write("= One\n")
    os.chmod(os.path.join(work, "w.typ"), 0o666)

    def host_edit():
        time.sleep(7)
        with open(os.path.join(work, "w.typ"), "a") as f:
            f.write("= Three (from the host)\n")
    threading.Thread(target=host_edit, daemon=True).start()
    rc, out = guest(work, "typst watch w.typ guest/w.pdf > /tmp/watch.log 2>&1 & P=$!; sleep 2; "
                          "echo '= Two' >> w.typ; sleep 9; kill $P; tr '\\r' '\\n' < /tmp/watch.log | grep -c 'compiled successfully'; "
                          "typst eval --in w.typ 'query(heading).len()'")
    counts = re.findall(r"^(\d+)$", out, re.M)
    check("typst watch: recompiles on a change in the guest and one by the host",
          len(counts) >= 2 and int(counts[0]) >= 3 and counts[-1] == "3", out)

    # ── the Typst Universe packages in the image, without network ─────────────────────────
    with open(os.path.join(work, "universe.typ"), "w") as f:
        f.write(UNIVERSE_DOC)
    rc_ref, _, err = ref(["compile", "universe.typ", "ref/universe.pdf"], cwd=work)
    ref(["compile", "universe.typ", "ref/universe.png"], cwd=work)
    rc, out = guest(work, "typst compile universe.typ guest/universe.pdf && typst compile universe.typ guest/universe.png "
                          "&& echo universe-ok; ls /usr/share/typst/packages/preview", engine_args=("--no-network",))
    check("cmarker, mitex, cetz, cetz-plot, lilaq, fletcher work without network, as the reference renders them",
          rc_ref == 0 and "universe-ok" in out and not same_files(os.path.join(work, "ref"), os.path.join(work, "guest"),
                                                                 ["universe.pdf", "universe.png"]), f"{err}\n{out}")

    # ── packages through the host's request API ────────────────────────────────────────────
    api = FakeRequestApi()
    port = api.server_address[1]
    docs = {"pkg.typ": '#import "@preview/demo:0.1.0": hello\n#hello("guest")\n',
            "missing.typ": '#import "@preview/nothere:0.1.0": x\n',
            "broken-pkg.typ": '#import "@preview/broken:0.1.0": x\n',
            "blocked.typ": '#import "@preview/blocked:0.1.0": x\n'}
    for name, text in docs.items():
        with open(os.path.join(work, name), "w") as f:
            f.write(text)
    bridge = f"export COLLABO_BRIDGE=tcp:192.0.2.1:{port}; "
    rc, out = guest(work, bridge + "typst compile pkg.typ guest/pkg.pdf && echo pkg-ok; "
                    "ls $HOME/.cache/typst/packages/preview/demo/0.1.0; typst compile missing.typ; typst compile broken-pkg.typ; "
                    "typst compile blocked.typ; typst init @preview/demo d && cat d/main.typ",
                    engine_args=("--allow-loopback",))
    check("a package downloaded in many chunks, unpacked into the cache, used", "pkg-ok" in out
          and "lib.typ" in out and "typst.toml" in out, out)
    check("the request: GET with the package's URL, no other headers than accept",
          any(r == "GET https://packages.typst.org/preview/demo-0.1.0.tar.gz\naccept: */*\n\n" for r in api.requests),
          api.requests)
    check("a 404: package not found", "package not found" in out and "@preview/nothere:0.1.0" in out, out)
    check("a download broken in the middle: an error naming the host's message", "response body interrupted" in out, out)
    check("a refusal by the policy: says so", "network policy does not allow" in out, out)
    check("typst init @preview/demo (the version from the index)", "#hello(\"template\")" in out, out)

    rc, out = guest(work, "typst compile pkg.typ guest/pkg2.pdf", engine_args=("--deny", "packages.typst.org"))
    check("the real request API (vsock) under a deny rule: the policy's refusal", rc != 0
          and "network policy does not allow" in out and "packages.typst.org" in out, out)
    try:
        urllib.request.urlopen("https://packages.typst.org/preview/index.json", timeout=10).read(100)
        online = True
    except Exception as e:  # noqa: BLE001
        online, why = False, e
    if online:
        with open(os.path.join(work, "cetz.typ"), "w") as f:
            f.write('#import "@preview/cetz:0.4.2": canvas, draw\n#canvas({ draw.circle((0, 0)); draw.content((0, 0))[가] })\n')
        rc, out = guest(work, "typst compile cetz.typ guest/cetz.png && echo cetz-ok")
        check("a real Typst Universe package (cetz) through the host", "cetz-ok" in out, out)
    else:
        skip("a real Typst Universe package through the host", f"this machine is offline ({why})")

    # ── speed ──────────────────────────────────────────────────────────────────────────────
    big = ['#import "@collabo/report:0.1.0": *', '#show: report.with(title: [성능], author: "test")']
    for i in range(1, 41):
        big += [f"= 장 {i}", "#lorem(300)", "", "한국어 문단입니다. 이 문장은 성능 측정을 위해 반복됩니다.",
                "$ integral_0^oo e^(-x^2) dif x = sqrt(pi) / 2 $",
                "#data-table(range(12).map(k => (항목: \"행 \" + str(k), 값: k * 1234.5)))",
                f"#bar-chart(range(6).map(k => (\"항목\" + str(k), calc.rem(k * 37 + {i}, 50))))"]
    with open(os.path.join(work, "big.typ"), "w") as f:
        f.write("\n".join(big) + "\n")
    rc, out = guest(work, "true")
    with Timer() as boot:
        guest(work, "true")
    with Timer() as t:
        rc, out = guest(work, "typst compile big.typ guest/big.pdf && typst compile big.typ guest/big.pdf && echo big-ok")
    with Timer() as rt:
        ref(["compile", "big.typ", "ref/big.pdf"], cwd=work)
    each = (t.seconds - boot.seconds) / 2
    print(f"     a 40-chapter report: {each:.2f} s in the guest, {rt.seconds:.2f} s natively (boot {boot.seconds:.2f} s)")
    check("a 40-chapter report compiles in under 10 s, the same PDF as the reference", "big-ok" in out and each < 10
          and not same_files(os.path.join(work, "ref"), os.path.join(work, "guest"), ["big.pdf"]), out)

    return summary()


if __name__ == "__main__":
    sys.exit(main())
