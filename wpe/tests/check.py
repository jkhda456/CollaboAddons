#!/usr/bin/env python3
"""Checks what tests/guest.sh left in OUT (NAME.out / .err / .code per step)."""
import json
import os
import re
import sys
import zlib

OUT = sys.argv[1]
failed = 0


def get(name, ext):
    try:
        return open(os.path.join(OUT, f"{name}.{ext}"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return None


def check(name, ok_code=0, *, contains=(), absent=()):
    global failed
    code, out, err = get(name, "code"), get(name, "out") or "", get(name, "err") or ""
    problems = []
    if code is None:
        problems.append("did not run")
    elif int(code) != ok_code:
        problems.append(f"exit {code.strip()} (want {ok_code})")
    for text in contains:
        if text not in out + err:
            problems.append(f"no {text!r}")
    for text in absent:
        if text in out + err:
            problems.append(f"has {text!r}")
    if problems:
        failed += 1
        print(f"FAIL {name}: {'; '.join(problems)}\n     out: {out.strip()[:400]!r}\n     err: {err.strip()[:400]!r}")
    else:
        print(f"ok   {name}")


def png_ok(path):
    try:
        data = open(path, "rb").read()
    except FileNotFoundError:
        return "no file"
    if not data.startswith(b"\x89PNG"):
        return "not a PNG"
    # the page is not one color: look at the decompressed rows
    chunks, pos, idat = {}, 8, b""
    while pos < len(data):
        length = int.from_bytes(data[pos:pos + 4], "big")
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IDAT":
            idat += body
        elif kind == b"IHDR":
            chunks["w"] = int.from_bytes(body[0:4], "big")
        pos += 12 + length
    raw = zlib.decompress(idat)
    # sampled over the whole image (the top rows of a page are often all background)
    return None if len(set(raw[::97])) > 8 else "one color"


check("open", contains=("127.0.0.1:8000/index.html", "WPE 시험 페이지"))
check("info", contains=("url: http://127.0.0.1:8000/index.html", "loading: no"))
check("read", contains=("# 안녕하세요, WPE", "[1]", "Greet", "Second page", "input text name"))
check("eval", contains=("WPE 시험 페이지 (200000) 3",))
check("korean", contains=("안녕하세요, WPE",))
check("type", contains=("typed 9 characters",))
check("select", contains=("selected green",))
check("submit", contains=("clicked",))
check("result", contains=("hello Claude 한글 / g",))
check("counted", contains=("2",))
check("keyseen", contains=("Tab", "Shift", "Control", "a"))
check("links", contains=("Second page  http://127.0.0.1:8000/second.html",))
check("shot", contains=("page.png",))
problem = png_ok(os.path.join(OUT, "page.png"))
print(f"{'ok  ' if not problem else 'FAIL'} screenshot png{': ' + problem if problem else ''}")
failed += bool(problem)
check("guilist", contains=("web",))
check("follow", contains=("clicked", "second.html"))
check("second", contains=("/second.html Second",))
check("back", contains=("index.html",))
check("backed", contains=("/index.html",))
check("scroll", contains=("scrolled down 2",))
check("shotclick", contains=("clicked", "screenshot: /t/out/shot-click.png (1024x768, before the click; marked: the point and the element)"))
check("shotxy", contains=("clicked (30, 40)", "shot-xy.png (1024x768, before the click; marked: the point and the element)"))
check("shothover", contains=("hovering", "shot-hover.png (1024x768, after the hover; marked: the point and the element)"))
check("shottype", contains=("typed 1 characters", "shot-type.png (1024x768, after typing; marked: the point and the element)"))
check("shotkey", contains=("pressed 1 key", "shot-key.png (1024x768, after the keys; marked: the element)"))
check("shotselect", contains=("selected blue", "shot-select.png (1024x768, after selecting; marked: the point and the element)"))
check("shotscroll", contains=("scrolled down 2 (the page's y from 0 to ", "shot-scroll.png (1024x768, after the scroll; marked: the point)"),
      absent=("from 0 to 0)",))
check("shottop", contains=("at 0", "shot-top.png (1024x768, after the scroll)"))
check("shotfollow", contains=("second.html", "shot-follow.png (1024x768, before the click; marked: the point and the element)"))
check("shotback", contains=("index.html", "shot-back.png (1024x768, after the load)"))
check("shotnodir", contains=("clicked", "screenshot not taken: /nonexistent/shot.png: No such file or directory"))
check("shotbadcmd", 2, contains=("--screenshot is for click",))
check("shotnofile", 2, contains=("--screenshot needs a file",))
check("shotbare", contains=("second.html", "shot-bare.png (1024x768, after the load)"))
check("shotdenied", contains=("clicked", "index.html", "screenshot not taken: the display's settings do not allow screenshots"))
check("badhost", 1, contains=("failed to load",))
check("after", contains=("url:",))
check("header", contains=("X-Collabo-Test: yes",))
check("headerother", contains=("X-Only-Other: other", "example.com"))
check("lang", contains=("ko-KR, en",))
check("ua", contains=("user agent: chrome", "Chrome/151", "Win32", "Sec-CH-UA"))
check("uas", contains=("safari", "firefox", "iphone", "android"))
check("headers", contains=("127.0.0.1:8000/headers",))
check("echoed", contains=('"x-collabo-test": "yes"', "Chrome/151", "ko-KR"), absent=("x-only-other",))
check("fetched", contains=('"x-collabo-test": "yes"', "Chrome/151"), absent=("x-only-other",))
check("navigator", contains=("Win32 Google Inc. Chromium/Google Chrome/Not.A/Brand ko-KR",))
check("unset", absent=("X-Collabo-Test",))
check("cookieset", contains=("set 3 cookies", "flavor=choco"))
check("cookielist", contains=("flavor=choco", "a=1", "b=2"))
check("cookiesent", contains=("flavor=choco", "a=1", "b=2"), absent=("x-collabo-test",))
check("cookiedel", contains=("deleted 1 cookie",))
check("badheader", 2, contains=("web cookie set",))
check("inject", contains=("inject.js",))
check("injectedat", contains=("loading 0",))
check("settings", contains=("user agent: chrome", "languages: ko-KR, en", "X-Only-Other", "inject.js"))
check("runerr", 1, contains=("TypeError", "bad.js:2"), absent=("bad.js:4",))
check("runstdin", contains=("44",))
check("runtimeout", 3, contains=("not finished within 2 s",))
check("uadefault", contains=("user agent: webkit",))
check("platform", contains=("Linux x86_64",), absent=("wasm",))
check("uasafari", contains=("user agent: safari", "Version/26.0.1 Safari/605.1.15", "MacIntel", "device: screen"))
check("identity", contains=("user agent: safari", "device: screen", "cores", "humanize: on", "time zone: the guest's"))
check("natives", contains=("MacIntel Apple Computer, Inc.", "native=true", "worker=MacIntel", "screen-holds-window=true",
                           "outer>inner=true", "pointer-fine=true", "languages=ko-KR ", "marks=0", "globals=0"))
check("isolated", contains=("object ",), absent=("undefined",))
check("isorun", contains=("undefined object",))
check("safarilang", contains=('"accept-language": "ko-KR,ko;q=0.9"', "Version/26.0.1"))
check("idnew", contains=("(seed 12345)",))
check("idsame", contains=("(seed 12345)",))
check("tz", contains=("time zone: Asia/Seoul",))
check("tzpage", contains=("Asia/Seoul -540",))
check("tzbad", 2, contains=('"Mars/Base" is not a time zone',))
check("tzback", contains=("time zone: the guest's",))
check("humanclick", contains=("clicked", '"tag":"button"'))
check("humantype", contains=("typed 8 characters",))
check("humanwheel", contains=("scrolled down 3",))
check("rawtype", contains=("typed 8 characters",))
check("humanoff", contains=("humanize: off",))
check("humanon", contains=("humanize: on",))
check("checknone", contains=("no robot check on this page",))
check("checkopen", contains=("checkbox.html", "challenge: cloudflare:"))
check("checkbox", contains=("passed (cloudflare)", "second.html?verified=1"))
check("holdstatus", contains=("press-and-hold:", "web challenge solve presses and holds it"))
check("hold", contains=("passed (press-and-hold)", "second.html?held="))
check("enginebad", 2, contains=('no search engine "nope"',))


def numbers(name):
    return {k: v for k, v in re.findall(r"(\w+)=([^ \n]*)", get(name, "out") or "")}


def input_timing_ok():
    """With humanize the pointer travels and buttons and keys are held and spaced as a person's;
    without it, all at once"""
    problems = []
    h, r = numbers("human"), numbers("raw")
    try:
        if int(h["moves"]) < 10:
            problems.append(f"human: {h['moves']} moves before the click")
        if int(h["minhold"]) < 40:
            problems.append(f"human: a button held {h['minhold']} ms")
        if int(h["mingap"]) < 20 or int(h["maxgap"]) > 1500:
            problems.append(f"human: keys {h['mingap']}-{h['maxgap']} ms apart")
        if int(h["wheels"]) != 3 or h["trusted"] != "true" or h["value"] != "Hi":
            problems.append(f"human: wheels {h['wheels']}, trusted {h['trusted']}, value {h['value']!r}")
        if int(r["moves"]) > 3 or int(r["minhold"]) > 15 or int(r["maxgap"]) > 30 or r["value"] != "Hi":
            problems.append(f"raw: {r}")
    except (KeyError, ValueError) as e:
        problems.append(f"no numbers: {e}: {h} {r}")
    return problems
check("history", contains=("index.html", "second.html", "/headers", "Second"))
check("historyq", contains=("second.html",), absent=("/headers",))
check("close", contains=("closing",))
check("gone", contains=("no browser is running",))
check("agenthelp", contains=("The rules that matter most", "## start:", "## recipes:", "language: 한국어 too",
                              "web intercept respond", "only after a `web read`"))
check("agenttopics", contains=("start       how the browser works", "intercept ", "recipes "), absent=("## ",))
check("agentparts", contains=("## network:", "## intercept:", "web network export"),
      absent=("## start:", "## recipes:", "The rules that matter most"))
check("agentcmd", contains=("## act:", "KimLee", "ctrl+a"), absent=("## wait:",))
check("agentproxy", contains=("## proxy:", "web proxy ca FILE", "socks5://agent:****@"), absent=("## errors:",))
check("agentbad", 2, contains=('no topic "nope"', "web --agent-help topics"))
check("agentnone", contains=("no browser is running",))
check("helpline", contains=("web --agent-help [TOPIC...]",))
check("history2", contains=("second.html",))
check("uastart", contains=("user agent: firefox",))
check("optrunning", contains=("X-Late: 1", "languages: en"))
check("envstart", contains=("user agent: iphone", "X-Env: 1"))
check("netoff", 1, contains=("developer tools are off",))
check("dton", contains=("developer tools: on", "web network lists"))
check("netpage", contains=("net.html",))
check("netdone", contains=("found",))
check("netstate", contains=("all done: true 세계 true worker true echo ping 1",))
check("netlist", contains=("net.html", "net.css", "net.js", "pixel.png", "api/data?x=1", "api/echo", "api/data?xhr=1",
                           "api/data?worker=1", "missing.js", "/redirect  -> ", "second.html", "/ws", "websocket", "POST"))
check("netapi", contains=("status: 200 OK", "HTTP/1.1", "remote address: 127.0.0.1:8000", "query:", "y = two",
                          "Set-Cookie: devtools=yes", '"ok": true', "time:"))
check("netpost", contains=("POST", "request body (application/json", '{"hello":"세계"}', "b=2; devtools=yes", "type: fetch"))
check("netws", contains=("101", "messages (4)", "> ", "ping 1", "< ", "echo ping 1", "(close)"))
check("netbody", contains=('"query": "x=1&y=two"',))
check("netpng", contains=("pixel.png: 69 bytes",))
check("netbin", 1, contains=("binary",))
check("netfetch", contains=("api/echo",), absent=("pixel.png",))
check("net404", contains=("missing.js",), absent=("api/echo",))
check("search", contains=("api/echo", "request body 1:", "response body 1:", "세계", "match"), absent=("pixel.png",))
check("searchhead", contains=("response header  Set-Cookie: devtools=yes", "request header  Cookie: ", "devtools=yes"),
      absent=("response body",))
check("searchre", contains=("ws://127.0.0.1:8000/ws", "received message", "echo ping 1"))
check("searchcase", 4, contains=("no match",))
check("searchtype", contains=("net.js", "net-worker.js", "response body"), absent=("GET     fetch",))
check("searchnone", 4, contains=('no match for "no-such-text-anywhere"',))
check("searchbad", 2, contains=("--in takes",))
check("har", contains=("net.har", "HAR 1.2"))
check("harview", contains=("HAR 1.2 by collabo wpe-browser", "api/echo"))
check("harone", contains=("#1 GET http://127.0.0.1:8000/", "request headers"))
check("harsearch", contains=("api/echo", "web har view FILE N"), absent=("net.js",))
check("excurl", contains=("curl 'http://127.0.0.1:8000/api/echo'", "-H 'Cookie: ", "devtools=yes", "--data-raw '{\"hello\":\"세계\"}'",
                          "--compressed", "# #"), absent=("#!/bin/sh", "-H 'Host:", "Content-Length"))
check("exfile", contains=("echo.sh: 1 request as curl", "executable"))
check("exrun", contains=('"hello": "세계"', '"cookie": "', 'devtools=yes"', '"referer": "http://127.0.0.1:8000/net.html"'))
check("exall", contains=("requests as curl",))
check("exallrun", contains=('"query": "x=1&y=two"', '"hello": "세계"', '"query": "worker=1"'))
check("expython", contains=("echo.py: 1 request as python",))
check("expyrun", contains=("200 OK", '"hello": "세계"', '"cookie": "', 'devtools=yes"'))
check("exraw", contains=("echo.raw: 1 request as raw",))
check("exrawrun", contains=("HTTP/1.1 200 OK", '"hello": "세계"', '"connection": "Keep-Alive"'))
check("exhttp", contains=("echo.http: 1 request as http",))
check("exfetchrun", contains=("x=1&y=two",))
check("exnode", contains=("echo.js: 1 request as node", "executable"))
check("exhar", contains=("two.har: 2 requests as har",))
check("exharview", contains=("2 entries", "api/echo", "pixel.png"))
check("exfromhar", contains=("curl 'http://127.0.0.1:8000/api/echo'", "--data-raw"))
check("exnocookie", contains=("curl 'http://127.0.0.1:8000/api/echo'",), absent=("Cookie",))
check("exws", 4, contains=("is a WebSocket",))
check("exwsraw", contains=("GET /ws HTTP/1.1", "Sec-WebSocket-Key"))
check("exbad", 2, contains=("--as nope?",))
check("exnone", 4, contains=("no request no-such-request",))
check("swap", contains=("localhost:8000/second.html",))
check("netswap", contains=("localhost:8000/second.html", "document"))
check("watch", contains=("watched=1",))
check("dtinfo", contains=("devtools: on",))
check("dtoff", contains=("developer tools: off", "kept"))
check("notrec", contains=("no request matches",), absent=("unrecorded=1",))
check("netclear", contains=("cleared",))
check("netcleared", contains=("nothing recorded yet",))
check("icoff", contains=("intercept: off",))
check("ion", contains=("developer tools: on", "document, fetch requests the page makes pause before they go"))
check("iopen", 6, contains=("paused: 1 request paused", "hold.html", "document", "web wait load"))
check("ilist", contains=("request", "hold.html"))
check("ishow", contains=("GET http://127.0.0.1:8000/hold.html", "paused before it goes", "User-Agent:", "web intercept drop"))
check("iinfo", contains=("intercept: on, 1 request paused",))
check("isend", contains=("sent #", "hold.html"))
check("iwait1", contains=("api/data?hold=1", "fetch"))
check("imove", contains=("api/data?changed=1", "changed: url"))
check("iwait2", contains=("POST", "api/echo"))
check("ipost", contains=('{"hello":"page"}', "Content-Type: application/json"))
check("ibody", contains=("changed: headers, body",))
check("iwait3", contains=("drop=1",))
check("idrop", contains=("dropped #", "network error"))
check("iwait4", contains=("mock=1",))
check("irespond", contains=("201 Created", "not sent"))
check("idone", contains=("found",))
check("istate", contains=('query changed=1 | echo agent 한 yes | dropped TypeError | mock 201 {"mocked":true} | hold done',))
check("inet", contains=("(request changed)", "dropped: web intercept drop", "(request answered here)", "201"))
check("inetshow", contains=("intercepted: request sent changed (headers, body)", "X-Agent: yes", "agent 한"))
check("inotpaused", 4, contains=("is not paused",))
check("ibad", 2, contains=("drop takes no --url",))
check("iresp", contains=("pauses with its response",))
check("irespwait", contains=("response 200", "resp=1"))
check("irespshow", contains=("paused with its response", "status: 200 OK", "its body has not come yet"))
check("irespdrop", 7, contains=("cannot fail a response", "not supported"))
check("irespedit", contains=("299", "its response changed"))
check("iresptitle", contains=('got {"edited":1}',))
check("iclick", 6, contains=("clicked", "Ask", "api/data?clicked=1"))
check("iclicksend", contains=("sent #",))
check("iclicked", contains=("found",))
check("iworker", contains=("worker=1", "fetch"))
check("iworkersend", contains=("sent #",))
check("iworkerdone", contains=("found",))
check("iworkernet", contains=("1 request,",))  # recorded once: by the worker, held by its page
check("irun", 6, contains=("run=1", "what it returns is lost"))
check("irunsend", contains=("sent #",))
check("ifileon", contains=("document requests",))
check("ifileopen", 6, contains=("nowhere.html",))
check("ifile", contains=("answered #", "text/html", "not sent"))
check("ifileload", contains=("nowhere.html", "Served"))
check("ioffopen", 6, contains=("second.html",))
check("ioff", contains=("intercept: off", "went on as it was"))
check("ioffload", contains=("second.html", "Second"))
check("ihar", contains=("intercept.har",))


# intercept in the network process (WebKit patch 0013)
def caps_ok():
    try:
        caps = json.loads(get("ncaps", "out") or "")
    except ValueError as e:
        return [f"web --capabilities is not JSON: {e}"]
    problems = []
    i = caps.get("intercept", {})
    if caps.get("protocol") != 2 or set(i.get("stages", {})) != {"request", "response", "body", "send", "receive"}:
        problems.append(f"protocol/stages: {caps.get('protocol')} {list(i.get('stages', {}))}")
    if "respond" not in i.get("http", {}).get("request", {}) or i.get("websocket", {}).get("messages", {}).get("send") != ["data", "opcode"]:
        problems.append("http/websocket actions missing")
    if caps.get("exitStatus", {}).get("7", "").find("not supported") < 0:
        problems.append("exit 7 not described")
    return problems


problems = caps_ok()
print(f"{'ok  ' if not problems else 'FAIL'} capabilities json{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)
check("non", contains=("pauses before it goes",))
check("nstatus", contains=("intercept: on", "network process: connected", "holding requests", "WebSocket handshakes"))
check("nsync", 6, contains=("api/sync?tag=one", "synchronous", "what it returns is lost"))
check("nsynclist", contains=("1 request paused", "POST", "xhr", "(synchronous)"))
check("nsyncjson", contains=('"kind": "sync-xhr"', '"via": "network-process"', '"bodyEncoding": "base64"', '"page":', '"frame":'))
check("nsyncbody", contains=("sync body one",))
check("nsyncseen", absent=("tag=one",))
check("nsyncnet", contains=("paused", "api/sync?tag=one"))
check("nsyncblock", 6, contains=("synchronous request that is paused",))
check("nsyncsend", contains=("sent #", "changed: headers, body"))
check("nsyncdone", contains=("found",))
check("nsyncseen2", contains=('"/api/sync?tag=one"', '"sync-agent"', '"changed sync body"'), absent=('"sync body one"',))
check("nbeacon", contains=("api/beacon?tag=drop",))
check("nbeaconshow", contains=("beacon body drop",))
check("nbeacondrop", contains=("dropped #",))
check("nbeacon2", contains=("api/beacon?tag=two",))
check("nbeaconsend", contains=("sent #", "changed: url, headers, body"))
check("nbeaconseen", contains=('"/api/beacon?tag=changed"', '"beacon-agent"', '"changed beacon"'), absent=("tag=drop", "tag=two"))
check("nping", contains=("ping=1",))
check("npingshow", contains=('"type": "ping"', '"kind": "ping"', '"Ping-To"'))
check("npingsend", contains=("sent #", "changed: headers"))
check("npingseen", contains=('"/api/any?ping=1"', '"ping-agent"'))
check("nredir", contains=("api/any?hop=r1", "redirect hop 1"))
check("nredirseen", contains=('"/redirect2?n=r1"',), absent=("hop=r1",))
check("nredirshow", contains=("redirect: hop 1, from http://127.0.0.1:8000/redirect2?n=r1 (302)", "redirect chain: #"))
check("nredirsend", contains=("changed: url",))
check("nredirdone", contains=("found",))
check("nredirnet", contains=("redirect chain: #", "(this)", "intercepted: request sent changed (url"))
check("nredirseen2", contains=('"/api/any?hop=changed"',), absent=("hop=r1",))
check("nws", contains=("handshake", "ws2"))
check("nwsshow", contains=("its opening handshake", "web intercept drop"))
check("nwsrespond", 7, contains=("not supported", "handshake cannot be answered"))
check("nwssend", contains=("changed: url, headers",))
check("nwsopen", contains=("found",))
check("nmsgon", contains=("each WebSocket message the page sends and receives",))
check("nmsg", contains=("send 1", "ws2"))
check("nmsglist", contains=("send 1", "text", "on WebSocket #"))
check("nmsgshow", contains=("message the page sends", "hello there", "--opcode text|binary"))
check("nmsgrespond", 7, contains=("cannot be answered here",))
check("nmsgsend", contains=("sent #", "changed:"))
check("nmsgrecv", contains=("receive",))
check("nmsgrshow", contains=('"kind": "websocket-message"', '"direction": "receive"', '"data": "echo changed text"'))
check("nmsgrsend", contains=("sent #",))
check("nmsgseen", contains=("found",))
check("nmsgbin", contains=("binary",))
check("nmsgbinbody", contains=("msg.bin: 3 bytes",))
check("nmsgbinsend", contains=("sent #",))
check("nmsgbinrecv", contains=("receive",))
check("nmsgbinshow", contains=("binary", "AwIB"))  # 3,2,1 as base64
check("nmsgbindrop", contains=("dropped #", "the page does not get it"))
check("nmsgafter", contains=("sent #",))
check("nmsgpage", contains=("echo replaced", "echo after"), absent=("bytes 3,2,1", "echo hello there"))
check("nmsgseen2", contains=('"changed text"', '"010203"', '"after"'), absent=('"hello there"',))
check("nmsgnet", contains=("send message 1 sent changed", "receive message 2 sent changed", "receive message 3 dropped"))
check("nbodyon", contains=("(with its body)",))
check("nbody", contains=("body 200", "body=1"))
check("nbodyshow", contains=("and its whole body", '"query": "body=1"'))
check("nbodyread", contains=('"query": "body=1"',))
check("nbodysend", contains=("its response changed", "200"))
check("nbodydone", contains=("found",))
check("nbody2", contains=("body=2",))
check("nbodydrop", contains=("dropped #", "network error instead"))
check("nbodydropped", contains=("found",))
check("nbodykeep", contains=("its own body",))
check("nbodykept", contains=("found",))
check("nother", contains=('200 {"got": "sync body other"',))
check("notherlist", absent=("tag=other",))
check("nnav", contains=("api/sync?tag=nav",))
check("nnavopen", 6, contains=("synchronous request that is paused",))
check("nnavdrop", contains=("dropped #",))
check("nnavopen2", contains=("second.html",))
check("nswopen", contains=("found",))
check("nswnet", contains=("api/any?sw=1",))
check("nswlist", contains=("a service worker's",))
check("nswsend", contains=("changed: headers",))
check("nswanswers", contains=('/sw/net 200 {"query": "sw=1", "x-agent": "sw-agent"}', "/sw/cached 200 from the cache storage", "/sw/made 200 made by the worker"))
check("nswnetlist", contains=("service worker: network", "service worker: cache-storage", "service worker: synthesized"))
check("nswrecord", contains=("loaded by: a service worker", "200", "X-Agent: sw-agent"))
check("nswseen", contains=('"/api/any?sw=1"', '"sw-agent"'))
check("nhar", contains=("net-intercept.har",))
check("ncancel", contains=("api/any?hop=gone", "redirect hop 1"))
check("ncancelopen", contains=("second.html",))
check("ncancellist", contains=("no request is paused",))
check("ncancelnet", contains=("hop=gone",), absent=("paused",))
check("ncancelseen", absent=("hop=gone",))
check("idtoffopen", 6, contains=("second.html",))
check("idtoff", contains=("intercept: off too", "went on as they were"))
check("idtoffload", contains=("second.html",))
check("dtstart", contains=("second.html",))
check("dtfirst", contains=("second.html", "document", "200"))
check("dtset", contains=("developer tools: on",))
check("dtcrash", contains=("2 requests",))  # the page again, in the process after the crash


def har_ok(path):
    """the HAR file: its entries as an HTTP Archive has them, from the Web Inspector's data"""
    try:
        har = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError) as e:
        return [f"not read: {e}"]
    log = har.get("log", {})
    problems = [] if log.get("version") == "1.2" else ["no log.version 1.2"]
    entries = log.get("entries", [])
    def entry(part, method="GET"):
        return next((e for e in entries if part in e["request"]["url"] and e["request"]["method"] == method), None)
    page = entry("/net.html")
    if not page:
        return problems + ["no net.html entry"]
    for key in ("startedDateTime", "time", "request", "response", "cache", "timings"):
        if key not in page:
            problems.append(f"net.html: no {key}")
    if page["response"]["status"] != 200 or not page["response"]["httpVersion"].startswith("HTTP/1."):
        problems.append(f"net.html: {page['response']['status']} {page['response']['httpVersion']!r}")
    if page.get("serverIPAddress") != "127.0.0.1":
        problems.append(f"net.html: server {page.get('serverIPAddress')!r}")
    if "all done" not in page["response"]["content"].get("text", "") and "net.js" not in page["response"]["content"].get("text", ""):
        problems.append("net.html: no body")
    if not all(page["timings"].get(k, -2) >= -1 for k in ("blocked", "dns", "connect", "send", "wait", "receive")):
        problems.append(f"net.html: timings {page['timings']}")
    if page["request"]["headersSize"] <= 0 or page["response"]["bodySize"] <= 0:
        problems.append("net.html: no sizes")
    post = entry("/api/echo", "POST")
    if not post or post["request"].get("postData", {}).get("text") != '{"hello":"세계"}' \
            or "application/json" not in post["request"]["postData"].get("mimeType", ""):
        problems.append(f"api/echo: postData {post and post['request'].get('postData')}")
    elif not any(c["name"] == "devtools" for c in post["request"]["cookies"]):
        problems.append("api/echo: the cookie sent is not there")
    elif '"hello": "세계"' not in post["response"]["content"].get("text", ""):
        problems.append("api/echo: no response body")
    data = entry("/api/data?x=1")
    if not data or not any(c["name"] == "devtools" for c in data["response"]["cookies"]):
        problems.append("api/data: no response cookie")
    elif {"name": "y", "value": "two"} not in data["request"]["queryString"]:
        problems.append("api/data: no query string")
    redirect = entry("/redirect")
    if not redirect or redirect["response"]["status"] != 302 or not redirect["response"]["redirectURL"].endswith("/second.html"):
        problems.append(f"redirect: {redirect and redirect['response']}")
    if not entry("/second.html"):
        problems.append("no second.html after the redirect")
    if not entry("worker=1"):
        problems.append("no worker's fetch")
    missing = entry("/missing.js")
    if not missing or missing["response"]["status"] != 404:
        problems.append("missing.js: not 404")
    pixel = entry("/pixel.png")
    # not kept as it came (bodies: text), read since (web network body): base64
    if not pixel or pixel.get("_resourceType") != "image" or pixel["response"]["content"].get("encoding") != "base64":
        problems.append(f"pixel.png: {pixel and (pixel.get('_resourceType'), pixel['response']['content'])}")
    ws = entry("/ws")
    messages = (ws or {}).get("_webSocketMessages", [])
    data_messages = [m for m in messages if m["opcode"] == 1]
    if not ws or ws["response"]["status"] != 101 or [m["type"] for m in data_messages] != ["send", "receive"] \
            or data_messages[1]["data"] != "echo ping 1":
        problems.append(f"ws: {ws and (ws['response']['status'], messages)}")
    pages = log.get("pages", [])
    if not pages or not any(p["title"] == "Network" and "onLoad" in p["pageTimings"] for p in pages):
        problems.append(f"pages: {pages}")
    elif page.get("pageref") not in [p["id"] for p in pages]:
        problems.append("net.html: no pageref")
    return problems


problems = har_ok(os.path.join(OUT, "net.har"))
print(f"{'ok  ' if not problems else 'FAIL'} har file{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)


def intercept_har_ok(path):
    """what the agent decided, in the HAR file: _intercepted, and the requests as they went"""
    try:
        entries = json.load(open(path, encoding="utf-8"))["log"]["entries"]
    except (OSError, ValueError, KeyError) as e:
        return [f"not read: {e}"]
    def entry(part):
        return next((e for e in entries if part in e["request"]["url"]), None)
    def actions(e):
        return [n["action"] for n in (e or {}).get("_intercepted", [])]
    problems = []
    moved = entry("api/data?changed=1")
    if not moved or actions(moved) != ["sent changed"] or moved["_intercepted"][0]["from"]["url"] != "http://127.0.0.1:8000/api/data?hold=1":
        problems.append(f"changed=1: {moved and moved.get('_intercepted')}")
    echo = entry("/api/echo")
    if not echo or actions(echo) != ["sent changed"] or echo["request"].get("postData", {}).get("text") != '{"hello":"agent 한"}':
        problems.append(f"api/echo: {echo and (echo.get('_intercepted'), echo['request'].get('postData'))}")
    dropped = entry("drop=1")
    if not dropped or actions(dropped) != ["dropped"] or "web intercept drop" not in dropped["response"].get("_error", ""):
        problems.append(f"drop=1: {dropped and (dropped.get('_intercepted'), dropped['response'].get('_error'))}")
    mock = entry("mock=1")
    if not mock or actions(mock) != ["answered here"] or mock["response"]["status"] != 201:
        problems.append(f"mock=1: {mock and (mock.get('_intercepted'), mock['response']['status'])}")
    resp = entry("resp=1")
    if not resp or resp["_intercepted"][0]["stage"] != "response" or resp["response"]["status"] != 299:
        problems.append(f"resp=1: {resp and (resp.get('_intercepted'), resp['response']['status'])}")
    return problems


def exported_ok():
    """what web network export wrote, used here: the HAR of two, the Node.js script and the .http file
    sent to the test server (on this computer) as their tools would"""
    import http.client
    import shutil
    import socket
    import subprocess
    problems = []
    try:
        two = json.load(open(os.path.join(OUT, "two.har"), encoding="utf-8"))["log"]
        urls = [e["request"]["url"] for e in two["entries"]]
        if len(urls) != 2 or not any("/api/echo" in u for u in urls) or not any("/pixel.png" in u for u in urls) or not two["pages"]:
            problems.append(f"two.har: {urls}, {len(two['pages'])} pages")
    except (OSError, ValueError, KeyError) as e:
        problems.append(f"two.har: {e}")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    server = subprocess.Popen([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "server.py"), str(port)])
    try:
        for _ in range(50):
            try:
                socket.create_connection(("127.0.0.1", port), timeout=1).close()
                break
            except OSError:
                import time
                time.sleep(0.1)
        here = lambda text: text.replace("127.0.0.1:8000", f"127.0.0.1:{port}")
        node = shutil.which("node") or os.path.join(os.path.dirname(OUT), "../../../.tools/node/bin/node")
        try:
            script = here(open(os.path.join(OUT, "echo.js"), encoding="utf-8").read())
            if os.path.exists(node):
                out = subprocess.run([node, "-"], input=script, capture_output=True, text=True, timeout=30)
                if "200 OK" not in out.stdout or '"hello": "세계"' not in out.stdout or 'devtools=yes"' not in out.stdout:
                    problems.append(f"echo.js under node: {out.stdout[:200]!r} {out.stderr[:300]!r}")
            else:
                print("     (no node here: echo.js not run)")
        except (OSError, subprocess.SubprocessError) as e:
            problems.append(f"echo.js: {e}")
        try:
            text = here(open(os.path.join(OUT, "echo.http"), encoding="utf-8").read())
            lines = [line for line in text.split("\n") if not line.startswith("#")]
            method, url = lines[0].split()[:2]
            blank = lines.index("")
            headers = dict(line.split(": ", 1) for line in lines[1:blank])
            body = "\n".join(lines[blank + 1:]).strip("\n")
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
            connection.request(method, url.split(f"127.0.0.1:{port}", 1)[1], body=body.encode(), headers=headers)
            answer = connection.getresponse().read().decode()
            if '"hello": "세계"' not in answer or "devtools=yes" not in answer:
                problems.append(f"echo.http: {answer[:200]!r}")
        except (OSError, ValueError, http.client.HTTPException) as e:
            problems.append(f"echo.http: {e}")
    finally:
        server.terminate()
    return problems


def png_rgb(path):
    """A PNG as libpng writes it (8-bit RGB or RGBA): width, height, rows of RGB bytes"""
    data = open(path, "rb").read()
    pos, idat = 8, b""
    while pos < len(data):
        length = int.from_bytes(data[pos:pos + 4], "big")
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            w, h, depth, kind_of_color = int.from_bytes(body[0:4], "big"), int.from_bytes(body[4:8], "big"), body[8], body[9]
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    if depth != 8 or kind_of_color not in (2, 6):
        raise ValueError(f"PNG of depth {depth}, color type {kind_of_color}")
    bpp = 3 if kind_of_color == 2 else 4
    raw, stride, rows, prev, i = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp), 0
    for _ in range(h):
        kind, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride) if kind else ():
            a, b, c = line[x - bpp] if x >= bpp else 0, prev[x], prev[x - bpp] if x >= bpp else 0
            if kind == 1:
                line[x] = (line[x] + a) & 255
            elif kind == 2:
                line[x] = (line[x] + b) & 255
            elif kind == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line) if bpp == 3 else bytes(v for k, v in enumerate(line) if k % 4 != 3))
        prev = line
    return w, h, rows


def shots_ok():
    """The marks are where the actions went: the ring around the point (radius 12, the mark's
    red), the arrow of a scroll, the caption's dark band in a free corner"""
    problems = []

    def pixel(rows, x, y):
        return tuple(rows[y][3 * x:3 * x + 3])

    def red(rgb):
        return rgb[0] > 200 and rgb[1] < 100 and rgb[2] < 140

    def point_of(name):
        m = re.search(r'"x":(\d+),"y":(\d+)', get(name, "out") or "") or re.search(r"\((\d+), (\d+)\)", get(name, "out") or "")
        return (int(m.group(1)), int(m.group(2))) if m else None

    for name, png, ring in (("shotclick", "shot-click.png", True), ("shotxy", "shot-xy.png", True),
                            ("shothover", "shot-hover.png", True), ("shotscroll", "shot-scroll.png", False),
                            ("shotfollow", "shot-follow.png", True), ("shotbare", "shot-bare.png", False)):
        try:
            w, h, rows = png_rgb(os.path.join(OUT, png))
        except (OSError, ValueError, zlib.error) as e:
            problems.append(f"{png}: {e}")
            continue
        if (w, h) != (1024, 768):
            problems.append(f"{png}: {w}x{h}")
            continue
        if ring:
            point = point_of(name)
            if not point:
                problems.append(f"{png}: no point in the answer")
            elif not red(pixel(rows, point[0] + 12, point[1])) or not red(pixel(rows, point[0] - 13, point[1])):
                problems.append(f"{png}: no ring at {point}: {pixel(rows, point[0] + 12, point[1])}")
            if not all(v < 100 for v in pixel(rows, 12, h - 12)) and not all(v < 100 for v in pixel(rows, 12, 12)):
                problems.append(f"{png}: no caption in a corner")
        if name == "shotscroll" and not red(pixel(rows, 512, 384 + 32)):
            problems.append(f"{png}: no arrow down from the middle: {pixel(rows, 512, 384 + 32)}")
        if name == "shotbare" and png_ok(os.path.join(OUT, png)):
            problems.append(f"{png}: {png_ok(os.path.join(OUT, png))}")
    if os.path.exists(os.path.join(OUT, "shot-denied.png")):
        problems.append("shot-denied.png written though capture is off")
    return problems


# web proxy: through tests/proxy.py (what it forwarded: proxy.log, proxy-auth.log); the server's
# /api/seen says what reached it
check("pxnone", contains=("proxy: none",))
check("pxbad", 2, contains=("not ftp://",))
check("pxnoport", 2, contains=("give the proxy's port",))
check("pxset", contains=("proxy: http://127.0.0.1:8100", "none, every host through the proxy", "loads under way were stopped"))
check("pxopen", contains=("px=http",))
check("pxws", contains=("found",))
check("pxtlsfail", 1, contains=("Unacceptable TLS certificate", "web proxy ca FILE"))
check("pxca", contains=("trusted from the next connection on", "CN=Collabo Test Proxy CA", "/t/certs/ca.der"))
check("pxtls", contains=("https://127.0.0.1:8443/api/any?px=tls",))
check("pxwss", contains=("found",))
check("pxother", contains=("px=other-browser",))
check("pxdefault", contains=("px=bypassed",))
check("pxsocks", contains=("px=socks5",))
check("pxsocks4", contains=("px=socks4",))
check("pxauthnone", 1, contains=("the proxy http://127.0.0.1:8101 wants a user and password", "407"))
check("pxauthbad", 1, contains=("refused the user and password given", "agent:****@"), absent=("wrong",))
check("pxauthbadtls", 1, contains=("refused the user and password given",))
check("pxauth", contains=("px=auth",))
check("pxauthtls", contains=("px=auth-tls",))
check("pxauthws", contains=("found",))
check("pxauthagain", 1, contains=("refused the user and password given",))  # the right ones before are not reused
check("pxsocksauth", contains=("px=socks-auth",))
check("pxstatus", contains=("socks5://agent:****@127.0.0.1:8101", "ca.der: O=collaboCore wpe tests,CN=Collabo Test Proxy CA"),
      absent=("pa:ss", "pa%3Ass"))
check("pxsite401", contains=("/api/auth",))
check("pxsettings", contains=("proxy: socks5://agent:****@127.0.0.1:8101",))
check("pxintercept", contains=("sent #", "px=held", "changed: headers"))
check("pxdown", 1, contains=("Could not connect to proxy", "through the proxy http://127.0.0.1:8199"))
check("pxoff", contains=("px=off",))
check("pxcaclear", contains=("none from the next connection on",))
check("pxenv", contains=("px=env",))
check("pxopt", contains=("px=option",))
check("pxenvset", contains=("proxy: socks5://127.0.0.1:8100", "/t/certs/ca.pem"))


def proxy_ok():
    problems = []
    log, auth = get("pxlog", "out") or "", get("pxauthlog", "out") or ""
    for line in ("http GET http://127.0.0.1:8000/api/any?px=http", "connect 127.0.0.1:8000", "connect 127.0.0.1:8443",
                 "socks5 localhost:8000 (a name", "socks4 127.0.0.1:8000", "http GET http://127.0.0.1:8000/api/any?px=held",
                 "socks5 127.0.0.1:8000"):
        if line not in log:
            problems.append(f"proxy.log has no {line!r}")
    for text in ("px=other-browser", "px=bypassed", "px=off", "GET ws://"):
        if text in log:
            problems.append(f"proxy.log has {text!r} (went straight, or a WebSocket not in a tunnel)")
    for line in ("407 GET http://127.0.0.1:8000/api/any?px=auth-none", "407 GET http://127.0.0.1:8000/api/any?px=auth-bad",
                 "407 CONNECT 127.0.0.1:8443", "http GET http://127.0.0.1:8000/api/any?px=auth", "connect 127.0.0.1:8443",
                 "connect 127.0.0.1:8000", "socks5 127.0.0.1:8000"):
        if line not in auth:
            problems.append(f"proxy-auth.log has no {line!r}")
    if auth.count("px=auth-bad\n") > 2:
        problems.append(f"wrong credentials tried {auth.count('px=auth-bad')} times")
    try:
        seen = {r["path"]: r for r in json.loads(get("pxseen", "out") or "[]")}
    except ValueError as e:
        return problems + [f"pxseen: {e}"]
    for q in ("px=http", "px=tls", "px=other-browser", "px=bypassed", "px=socks5", "px=socks4", "px=auth", "px=auth-tls",
              "px=socks-auth", "px=held", "px=off", "px=env", "px=option"):
        if "/api/any?" + q not in seen:
            problems.append(f"the server did not get {q}")
    if seen.get("/api/any?px=held", {}).get("x-agent") != "through":
        problems.append("the intercepted request did not arrive changed")
    if "/ws2?wss=1" not in seen:
        problems.append("no wss through the proxy")
    for q in ("px=auth-none", "px=auth-bad", "px=auth-bad-tls", "px=auth-again", "px=tls-untrusted", "px=down"):
        if "/api/any?" + q in seen:
            problems.append(f"the server got {q}, which the proxy or TLS should have stopped")
    try:
        caps = json.loads(get("pxcaps", "out") or "")
        if "socks5" not in caps.get("proxy", {}).get("schemes", []):
            problems.append("web --capabilities says nothing of the proxy")
    except ValueError as e:
        problems.append(f"pxcaps: {e}")
    return problems


problems = proxy_ok()
print(f"{'ok  ' if not problems else 'FAIL'} proxy logs{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)

problems = input_timing_ok()
print(f"{'ok  ' if not problems else 'FAIL'} input timing{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)

problems = shots_ok()
print(f"{'ok  ' if not problems else 'FAIL'} screenshot marks{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)

problems = exported_ok()
print(f"{'ok  ' if not problems else 'FAIL'} exported files{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)

problems = intercept_har_ok(os.path.join(OUT, "intercept.har"))
print(f"{'ok  ' if not problems else 'FAIL'} intercept har{': ' + '; '.join(problems) if problems else ''}")
failed += bool(problems)
print("all passed" if not failed else f"{failed} failed")
sys.exit(1 if failed else 0)
