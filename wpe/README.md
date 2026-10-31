# wpe add-on

A web browser for agents in the sandbox: **WPE WebKit 2.54.1** built for the guest (WebAssembly
Linux), shown on the guest's display (`gui`, [tools/gui](../../tools/gui)) and driven from the
command line by `web`. Pages run as in a desktop browser — HTML, CSS, JavaScript, web fonts,
cookies and storage, forms, redirects, HTTPS — and an agent reads them as text with every link,
button and field numbered, acts on them with real pointer and key events, and looks at them as
screenshots.

```
$ web open news.ycombinator.com
https://news.ycombinator.com/
Hacker News

$ web read | head
Hacker News
https://news.ycombinator.com/

 [1](link) <https://news.ycombinator.com/> [2]Hacker News <https://news.ycombinator.com/news> [3]new ...
...
$ web click 'new'                    # by text, [N] number or CSS selector; waits for the new page
$ web type 'webkit' 'input[name=q]'; web key Enter
$ web screenshot                     # /tmp/web.png (gui screenshot web)
$ web click 640 220 --screenshot /tmp/click.png   # and a picture of where it clicked: ring, frame, caption
$ web ua chrome; web reload          # a site that turns WebKit away: say it is Chrome on Windows
$ web header set Referer https://example.com/ --host example.com
$ web run extract.js 'h2'            # a JavaScript file in the page: return, await, args
$ web history 5                      # the last pages visited

$ web devtools on                    # developer tools: record the network from here on
$ web open example.com; web network  # every request: status, method, type, size, time, address
$ web network show api/items         # one in detail: headers as sent, body, timing, response
$ web network search sessionToken    # in every request's address, headers, bodies, messages
$ web har site.har                   # all of it as an HTTP Archive; web har view FILE reads one
$ web network export api/items       # one request as a curl command (cookies and all), to run again
$ web network export 12 --as python -o replay.py   # or fetch, node, http (.http), raw, har: into a file

$ web intercept on --type document,fetch   # those requests now pause before they go
$ web open example.com               # exit 6: it ends as the document's request pauses, listed
$ web intercept show 1               # what would go: method, address, headers, body
$ web intercept send 1 --header 'X-Debug: 1'   # let it go (changed); or drop 1, respond 1 --body ...
$ web wait load                      # until the page has loaded, or the next request pauses (6)
```

| In the guest | |
|---|---|
| `/usr/bin/web` | the command agents use (C, 130 KB; `web help`, `web --agent-help`) |
| `/usr/bin/wpe-browser` | the browser: WebKit's UI, web and network processes in one static program |
| `/usr/libexec/wpe-webkit-2.0/WPEWebProcess`, `WPENetworkProcess` | links to it, where WebKit starts its processes |
| `/etc/fonts`, `/usr/share/fonts` | fontconfig, DejaVu (Latin, Greek, Cyrillic), Nanum Gothic (Korean, Hanja, kana) |
| `/usr/share/mime/globs2` | file types by name for `file://` pages and downloads (`share/mime`, the web's common types) |

## Turning it on

```json
"addons": {"wpe": {}}
```

or `--addon-dir addons/wpe/out --addon wpe` on the engine's command line. It needs the tools image
(the display; on by default) to show pages and take screenshots; without it the browser still
starts (headless) and `web read`, `web eval` and the actions work, `--screenshot` too. The sandbox's network policy
applies to the browser as to everything in the guest.

From the host, the Dart package wraps `web` as LLM tools: `WebTools` (`web_open`, `web_read`,
`web_act`, `web_eval` (a script or a file), `web_screenshot`, `web_info`, `web_history`,
`web_configure` (user agent, headers, languages, cookies)), next to `SandboxTools` and `GuiTools`;
`WebTools(sandbox, userAgent: 'chrome', headers: {...}, languages: ['ko-KR'])` is what its browser
starts with.

## Commands

`web [-n NAME] [OPTION...] COMMAND`: each command talks to the browser named NAME (`web` by
default), and starts it first if it is not running. Several names are several browsers, each with
its own cookies, storage and history (`~/.local/share/collabo-web/NAME`).

`web --agent-help [TOPIC...]` (also `web agent-help`) prints a guide written for an LLM agent
([browser/agent-help.txt](browser/agent-help.txt), built into `web` by `gen-js.py`): the rules that
matter most, the loop every task follows, each command with its real output, the pitfalls found
by trying (`[N]` exists only after a `read`, `type` adds to a field, `eval` cannot await, a 404
page loads with exit 0, when `--screenshot` is taken), exit statuses and their messages, and
recipes (logging in, forms, scraping, infinite scroll, finding and replaying an API, mocking and
blocking requests, downloads). `web --agent-help topics` lists its parts — start, read, act, wait,
screenshot, js, identity, network, intercept, browsers, errors, recipes — and a part's first
letters print just it (`web --agent-help net inter`). It needs no browser.

| Command | |
|---|---|
| `open URL [TIMEOUT]` | load a page and wait for it (seconds, 60); a bare host gets `https://`, words become a search |
| `back`, `forward`, `reload` | history, waiting for the page |
| `info` | address, title, loading, history, size, user agent, the last error or dialog |
| `history [N \| --all] [TEXT]`, `history clear` | the pages visited, oldest first, the last N (30): time, title, address; TEXT: those whose address or title has it. Every committed load (links, forms, `back`, `reload`) and every move within a page (a single-page application's `pushState`); kept in the browser's data folder (`history.tsv`, the last 5000) across restarts, in memory only with `--ephemeral` |
| `wait [load \| text STR \| selector CSS \| gone CSS] [TIMEOUT]` | until the page is so |
| `read [MAX] [--all]` | the page as text in reading order: headings `#`, lists `-`, links with their address, fields with their value; every visible link, button and field numbered `[N]` |
| `text`, `html [CSS]`, `links` | the page's text, HTML, links |
| `eval JS` | JavaScript in the page; the result as JSON |
| `run [--timeout S] FILE\|- [ARG...]` | a JavaScript file (`-`: stdin) in the page, as the body of an async function: `return` gives the result (JSON), `await` waits (up to S seconds, 120), `args` holds the ARGs as strings; an error names the file's line. `web` reads the file, so a path is the caller's |
| `screenshot [FILE] [--max-width N]` | PNG of the page (`gui screenshot`) |
| `click TARGET \| X Y` (`dblclick`, `rightclick`) | a TARGET is an `[N]` from `read`, a CSS selector, or a link's or button's text; scrolled into view, clicked where it is on top; a click that loads a page waits for it |
| `type TEXT [TARGET]` | types into TARGET (clicked first) or what has the focus: key events per character (any language) |
| `key KEY...` | `Enter`, `Tab`, `Escape`, `ArrowDown`, `Backspace`, `ctrl+a`, `shift+Tab`, … |
| `select TARGET VALUE`, `hover TARGET`, `scroll [down\|up\|left\|right [N] \| top \| bottom]` | |
| `... --screenshot FILE` | with any of the actions above, `open`, `back`, `forward`, `reload` or `wait`: also a PNG of the page into FILE with where the action went marked (below) |
| `list`, `resize WxH`, `close` | the browsers; the page's size; end one |

`--screenshot FILE` (or `--screenshot=FILE`, anywhere after the command) is for checking that an
action went where it was meant to: the answer ends with `screenshot: FILE (1024x768, before the
click; marked: the point and the element)`, and the picture shows the view with a red ring (with
ticks; a dot for a press, two rings for `dblclick`) at the point the pointer went, a frame around
the element it acted on — the target found, or for `click X Y` what is at that point — and a
caption in a corner clear of them (`click (512, 300) · button "Search"`, `type 9 characters ·
input "your name"`, `scroll down 3 · y 0 → 249`). A click (`dblclick`, `rightclick`) is pictured
as it goes in, after the target was scrolled into view, so a click that loads another page still
shows what was clicked (`web screenshot` shows the new one); `hover`, `type`, `select` and `key`
(the focused element framed) once done and drawn, `scroll` once the page has stopped moving (an
arrow the way it went, the answer saying from where to where), `open`, `back`, `forward`,
`reload` once loaded, `wait` as it ends. The picture is WebKit's snapshot of the view, so it works
without the display too; the display's `capture: false` setting (gui status) refuses it as it
refuses `gui screenshot`. A picture that cannot be taken never fails the action: its answer and exit
status are the action's, with `screenshot not taken: WHY` at the end. It adds about 50 ms (a
scroll: until the page settles, ½ s or so). `web` writes FILE's path as the caller means it; its
folder must exist.

How the browser presents itself, for sites that turn it away or want a condition met. Each applies
from the next request on (`web reload` shows the current page with it); `web settings` shows them
all.

| Command | |
|---|---|
| `ua [PROFILE \| 'USER AGENT' \| list]` | the User-Agent: a profile — `default` (WebKit's own on desktop Linux), `safari` (Safari on a Mac: the same engine, the closest match), `chrome`, `edge`, `firefox` (on Windows), `iphone`, `android` (mobile sites; with `web resize 390x844`) — or any string. What the page's scripts see follows it (`identity.js`, at the start of every document): `navigator.platform` (WebKit's own says `Linux wasm32`), `vendor`, Firefox's `oscpu`, Chromium's `userAgentData`, a touch screen for the phones; a Chromium user agent also sends Chrome's client hints (`Sec-CH-UA`, `-Mobile`, `-Platform`) to https sites |
| `header [list]`, `header set NAME VALUE [--host HOST]`, `header unset NAME [--host HOST]`, `header clear` | headers put on every request pages make — the document, its images, scripts and styles, `fetch` and `XMLHttpRequest` — or, with `--host`, on those to HOST and its subdomains (do so for credentials); `set 'NAME: VALUE'` works too; an empty VALUE takes the header off the requests (curl's `-H 'NAME:'`). They replace the browser's own (`User-Agent`, `Referer`, `Accept-Language`...). Not `Cookie` (the cookie jar replaces it: `web cookie`) nor what the network stack sets (`Host`, `Content-Length`) |
| `lang [LANGUAGES \| default]` | preferred languages, `ko-KR,en` or an Accept-Language: the `Accept-Language` header, `navigator.languages`, the default locale of `Intl` |
| `cookie [list [URL] [--all]]`, `cookie set COOKIE... [--url URL]`, `cookie delete NAME [--url URL \| --all]`, `cookie clear` | the cookie jar. A COOKIE is a Set-Cookie (`sid=1; Domain=example.com; Path=/; Max-Age=3600; Secure; HttpOnly`) or a Cookie header (`a=1; b=2`: two cookies); it is for the page's site unless `--url` or `Domain=` says, for its whole site unless `Path=` says. A list prints them as Set-Cookies |
| `inject [list]`, `inject add FILE\|-`, `inject remove FILE`, `inject clear` | JavaScript run at the start of every page, in every frame, before the page's own scripts (Playwright's `addInitScript`): polyfills, stubs, hooks |
| `settings` | the user agent, languages, headers, injected scripts and developer tools |

The developer tools: the network as a browser's developer tools (WebKit's Web Inspector, Chrome's
DevTools) record it, for seeing what a page asks for and gets, and saving it as an HTTP Archive
(HAR 1.2). Off until turned on (`web devtools on`, the option `--devtools`, `WEB_DEVTOOLS=1`);
while on, every request the page makes is kept — the document, its subresources, `fetch` and
`XMLHttpRequest`, its frames' and dedicated workers' requests, WebSockets — across navigations,
also to other sites (another page process) and after a crash of the page's process.

| Command | |
|---|---|
| `devtools [status]`, `devtools on [--bodies text\|all\|none]`, `devtools off`, `devtools toggle` | the recording. `--bodies`: the response bodies kept as they come — `text` (the default: documents, scripts, styles, JSON, XML, SVG, form data), `all` (images and fonts too, as base64), `none`; at most 8 MB each and 64 MB in all (the oldest go first). Off keeps what was recorded |
| `network [TEXT] [--type T] [--status S] [--failed] [--last N \| --all]` | the requests recorded, oldest first, the last 100: number, status (`...` loading, `failed`), method, type (`document`, `stylesheet`, `script`, `image`, `font`, `xhr`, `fetch`, `websocket`, `other`...), bytes transferred (`(memory)`, `(disk)`: from a cache), time, address, where a redirect went; TEXT: those whose address has it; `--type xhr` takes fetch too, `--status 4xx`, `--failed` (an error or 4xx/5xx) |
| `network show N\|TEXT` | one (by its number, or the last whose address has TEXT): status, HTTP version, the server's address and port, connection, priority, TLS version and cipher, the timings (blocked, DNS, connect, TLS, send, wait, receive), the initiator (the parser's line, the script's frame), sizes (headers, body, decoded), the request's headers as they went over the wire (with the cookies the network process added), its query and body, the response's headers and the start of its body; a WebSocket's messages |
| `network body N\|TEXT [FILE]` | the response body (text to stdout, a binary one into FILE); one not kept is read from the page's process if it still has it |
| `network search TEXT [--case] [--regex] [--in url,headers,body,messages] [--url T] [--type T] [--status S] [--failed] [--max N]` | TEXT (any case, unless `--case`; a JavaScript regular expression with `--regex`) in every request's address, request and response headers, request body, response body (kept ones; base64 text decoded) and WebSocket messages, as the Network panel's search: by request, each match with where it is (`response body LINE:COLUMN`) and the text around it (a long line cut to it), 5 per request, `--max` (200) in all; `--in` where to look, the other options which requests. Waits for the bodies still being read; says how many text bodies were not kept. Exit 4: no match |
| `network watch [TEXT...] [--timeout S]` | each request as it finishes, from now on (turns the tools on), until S seconds or interrupted; the listing's options filter it |
| `network clear` | forget what was recorded |
| `har [FILE] [--no-bodies]` | the recording as a HAR file (default `/tmp/NAME.har`): pages (title, `onContentLoad`, `onLoad`), entries as the Web Inspector exports them (timings, sizes, `serverIPAddress`, `connection`, cookies, `postData`, `queryString`, `content.text`) and Chrome's fields (`_resourceType`, `_initiator`, `_priority`, `_transferSize`, `_fromCache`, `_webSocketMessages`, `_error`); still loading: `_pending` |
| `har view FILE [N \| TEXT... \| --type T ...]` | a HAR file, this browser's or another's (Chrome, Firefox, Safari): its requests listed as `network` lists them, or entry N in detail |
| `har search FILE TEXT [OPTION...]` | a HAR file searched as `network search` searches the recording |
| `network export N... [--as FORMAT] [-o FILE] [--no-cookies] [--type T] [--status S] [--failed]` | requests one by one, as commands and files of other tools: printed, or into FILE (a script starts with its `#!` line and is made executable). Each as it went over the wire — the headers the network process sent, the cookies among them (`--no-cookies` leaves them out), its body — less what the tool sets itself (`Host`, `Content-Length`, `Connection`; `Accept-Encoding` becomes curl's `--compressed`). FORMAT: `curl` (the default, as a browser's *Copy as cURL*: POSIX shell quoting, `--data-raw`, `--globoff` for an address with `[]{}`), `fetch` (as *Copy as fetch*, for a page's console or `web run`: what a script may set, the cookies as `credentials`), `node` (Node.js's `fetch`, every header, printing the response), `python` (the standard library's `urllib`, no pip; prints the response, a 4xx/5xx too), `http` (a `.http` file of VS Code's REST Client and JetBrains' HTTP Client), `raw` (the HTTP/1.1 message itself, CRLF: Burp's Repeater, `nc`), `har` (a HAR of just those, with their pages and bodies: Postman, Insomnia, a browser's import). N: a number, `N-M`, `all`, or a part of the address (the last request that has it); several can be given; the options narrow them. A WebSocket goes in `raw`, `http` and `har` only (its handshake); a body WebKit did not give as text (an uploaded file) is left out, and said so |
| `har export FILE N... [--as FORMAT] [-o OUT] [...]` | the same from a HAR file of any browser (Chrome's, Firefox's, Safari's, this one's): its entries by number, as `har view` lists them |

Intercept: the page's requests wait for approval, as in an intercepting proxy (Burp's
*Intercept*, the Web Inspector's request interception), but inside the browser — so HTTPS needs no
certificate of a proxy's, and the agent sees each request as the page made it. While it is on, each
request the rules take is **paused** before it goes (or, with `--stage response`, its response
before the page gets it) until the agent **sends** it (as it was or changed), **drops** it (the
page gets a network error) or **answers** it itself. The rest go as usual. It needs the developer
tools and turns them on; a paused request is in `web network` too (status `paused`), by the same
number, and what was decided for it shows there and in the HAR (`_intercepted`).

| Command | |
|---|---|
| `intercept [status]`, `intercept off` | the rules and how many are paused; off: those paused go as they were (`devtools off` does the same) |
| `intercept on [TEXT...] [--regex] [--case] [--type T[,T]] [--method M[,M]] [--stage request\|response\|both]` | from now on, the requests of the page, its frames and its workers that the rules take pause: TEXT, a part of the address (any of them; any case unless `--case`; a regular expression with `--regex`), the types as `network --type` names them (`document`, `script`, `stylesheet`, `image`, `font`, `fetch`, `xhr`, `api` = both...), the methods; all of them without rules. `--stage`: `request` (the default) pauses before it goes, `response` when its response has come and before the page gets it, `both` at each. Again with other rules: they replace these. Also `pause` |
| `intercept list` | the paused requests: number, stage (`request`, `response 200`), method, type, how long it waited, address |
| `intercept show N\|TEXT` | one in detail: at the request stage what would go (method, address, the headers as the page made them — the network adds `Cookie`, `Host`, `Accept-Encoding` after —, query, body); at the response stage the status and headers that came (not the body: it has not yet) |
| `intercept send N... [--url U] [--method M] [--header 'NAME: VALUE']... [--unset NAME]... [--body TEXT \| --body-file FILE]` | let it go, changed so: an address (relative to its own: `/api?x=1`, `?x=2`), a method, headers (`NAME:` with no value takes one off), a body. At the response stage: `[--status S] [--header ...] [--unset ...] [--mime TYPE] --body TEXT` gives the page the response changed — whole: its own body has not come, so the page gets `--body` (or none). Also `continue`, `forward` |
| `intercept drop N... [--error general\|access\|cancel\|timeout]` | stop it: never sent, the page gets a network error (`fetch` rejects, a document's load fails: `web info` says so); `cancel` cancels it as the page's own abort would. WebKit cannot fail a response: at the response stage, `respond` gives another. Also `stop`, `block`, `abort` |
| `intercept respond N... [--status S] [--header 'NAME: VALUE']... [--mime TYPE] [--body TEXT \| --body-file FILE]` | answer it here, nothing sent (a mock): status 200 unless said, the type from `--mime`, else `Content-Type`, else what the request is for (`text/html` for a document, `application/json` for `fetch`, ...); at the response stage, in its response's place. Also `fulfill` |
| `intercept wait [TIMEOUT]` | until a request pauses (or at once, if one is), seconds (30); exit 3 when none did |

N is a number from `intercept list` (or `web network`: the same), `all`, or a part of the address
(every paused one that has it); several can be given. `web` reads a `--body-file` (`-`: stdin), so
its path is the caller's, and sends its bytes as they are. The answer of `send`, `drop` and
`respond` comes once WebKit has taken the decision, and says what is still paused.

A command that waits on the page would wait for nothing while the page waits for the agent: `open`,
`back`, `forward`, `reload`, a `click` or `key` that loads a page, `wait` and `run` end as soon as
a request pauses (and `wait` at once while one is), with exit status **6** and the paused requests
listed; once they are decided, `web wait load` waits for the page again (it, too, ends at the next
pause). A load that finished while a request was paused (a `fetch` does not hold the page's load)
says so in a last line. A `run` that ended so leaves its script running in the page, its result
lost.

Browser options, before the command (`web --user-agent chrome open example.com`): a browser that
command starts gets them; one running already gets the settings among them (`--user-agent
PROFILE|UA`, `--header 'NAME: VALUE'`, `--header-for HOST 'NAME: VALUE'`, `--lang LANGUAGES`,
`--inject FILE`, `--devtools`; each but the user agent can be given more than once), and says the
others are for its start (`--ephemeral`: no cookies, storage or history kept;
`--ignore-tls-errors`; `--size WxH`). A browser's first settings can also come from the
environment of the `web` that starts it: `WEB_USER_AGENT` (a profile or a user agent),
`WEB_HEADERS` (lines `NAME: VALUE`), `WEB_LANG`, `WEB_DEVTOOLS` (`1`, or the bodies kept: `text`,
`all`, `none`); options say last. `WEB_BROWSER_ARGS` adds options (words) to a browser `web` starts.

Exit status: 0 done, 1 failed, 2 usage, 3 timed out, 4 not found, 5 no browser, 6 requests
are paused for the agent's approval (`web intercept`). Dialogs
(`alert`, `confirm`, `prompt`) are accepted and their text kept for `info`; permission requests
(location, notifications, camera) are denied; what the page would open in a new window opens in
the same one; downloads go to `~/Downloads`. By default the browser says it is WebKit on desktop
Linux (`X11; Linux x86_64`: the guest's own `wasm` is a platform sites do not know).

The browser is also an ordinary program on the display: `gui list`, `gui view web`,
`gui click web X Y`, `gui logs web` (its console messages) all work.

## How it works

- **One program, three kinds of process.** WebKit keeps its process model: the UI process (the
  browser, `browser/browser.c`), the network process, a web process per site. The guest cannot
  load libraries, so `wpe-browser` is all of them, statically linked (`browser/main.cpp` picks by
  the name it is run as; the injected bundle is linked in too, patch 0008), and WebKit starts its
  processes with `posix_spawn` (GLib's, which needs no `fork`). The browser asks for WebKit's
  `DocumentBrowser` cache model: no cache of processes for sites left behind and nothing started
  ahead (patch 0009), the two pages before kept for `back` — at most three web processes.
- **Calls through pointers of another type.** C code built for GLib calls functions through
  pointers of other types all the time (a `gboolean` function as a `GChildWatchFunc`, an
  interface init without its second parameter, a `GDestroyNotify` as a `GClosureNotify`); native
  code gets away with it, WebAssembly traps. Three layers: the casts that can be fixed where they
  are, found with `tools/check-fpcasts.py` and `tools/fpcast-scan.py` (clang's
  `-Wcast-function-type-strict`, kept where the *wasm* signatures differ), are fixed (patches to
  GLib, libsoup, glib-networking, WebKit, WebKit's IPC generator); the places where GLib calls
  what a program registered (signal handlers, closures, weak and toggle notifies, `GTask`
  callbacks, `GSource` callbacks) call it through `ffi_wasm_call` of the add-on's libffi
  (`ffi/`), which reads the function's real type from the program's own table
  (`/proc/self/exe`) and calls it as that, leaving out what it does not take; and every GLib
  signal goes through the generic (libffi) marshaller.
- **Shared memory without shared memory.** Each guest process is a WebAssembly memory of its own:
  nothing can be mapped into two of them. A `SharedMemory` is a file in memory (`memfd_create` is
  emulated with an unlinked `/dev/shm` or `/tmp` file) whose mapping is a copy, written back when
  it is unmapped and when a handle is made for another process (patch 0003). That is right for
  everything WebKit shares once (IPC messages larger than 4 KB, images, resources); the frames of a
  page, which the web process draws into the same buffers again and again, are written to the file
  after each frame and read again by the UI process when it gets the frame.
- **Rendering on the CPU.** No GPU and no EGL: the web process makes no EGL display and renders
  without compositing (`NonCompositedFrameRenderer`: Skia on the CPU into shared-memory buffers,
  patch 0004).
- **The display.** A WPE platform backend of our own, `collabo` (patch 0005,
  `Source/WebKit/WPEPlatform/wpe/collabo`): each toplevel is a canvas of the display server,
  frames go there as their damaged rectangles, and the server's input comes back as WPE events —
  pointer, wheel, keys (X11 keysyms), the input method's composition and commit (Hangul with
  `gui type --ime`), focus, hover, cursors, the clipboard. A frame is rendered when the server
  says it is on the canvas, which paces the web process to what the display takes.
- **JavaScript.** JavaScriptCore with its C interpreter (CLoop; nothing can be compiled at run
  time), JSVALUE32_64, the system allocator. Its garbage collector scans the stack conservatively,
  but WebAssembly keeps a function's values in locals it cannot see, so a collection never starts
  inside an allocation: it is asked for there and runs from a timer of the run loop, with nothing
  but the run loop below it (patch 0002); only an allocation far past its budget without the run
  loop getting a turn collects on the spot.
- **Headers on every request.** WebKit has no API to add headers to all of a page's requests
  from the UI process, but its web process extensions see each request as it leaves
  (`WebKitWebPage::send-request`) and can change its headers. The guest cannot load an extension,
  so the browser's (`browser/extension.c`) is linked into the program and WebKit calls it when a
  web process starts (patch 0010). The browser gives each new web process the header rules with
  its initialization data and sends the running ones a message when they change. Cookies are the
  network process's (its cookie jar replaces a Cookie header): `web cookie` goes to the cookie
  manager.
- **Agents' input** is not a separate path: `web click` finds the element (`browser/target.js`),
  scrolls it into view and sends pointer events to the view as the display would; `type` and
  `key` send key events. WebKit's UI process queues key events (one at a time, each after the page
  took the one before) apart from pointer events, so a command waits until the page has seen its
  input (a script counts it at the window) before it answers or the next one acts: a click cannot
  overtake the keys typed before it.
- **Developer tools.** WebKit's own records the network for its Web Inspector: the inspector's
  backend in each page process (the Network, Page and Worker domains) sees every request, and the
  network process gives it what went over the wire (the request's final headers, the timings,
  the connection, the sizes) while a frontend is connected. Its frontends are a web page (the Web
  Inspector, as large as an application) or one at the far end of the remote inspector, which this
  build leaves out; patch 0011 gives the program a frontend of its own instead, three C functions
  (`webkit_web_view_connect_inspector_frontend`, `webkit_inspector_frontend_send`,
  `webkit_inspector_frontend_disconnect`) that pass the protocol's messages as text. The browser
  hands them to `browser/devtools.js`, which runs in a JavaScript context of the UI process's own
  (not the page's), keeps each request as a HAR entry as the Web Inspector's HAR export makes it,
  asks for the response bodies as they finish, and writes the listings. A page that goes to
  another site gets another process, so another target: the frontend asks WebKit to pause each new
  target before it loads anything (`Target.setPauseOnStart`) and resumes it once the recording is
  on in it. A target takes messages only while the page has a process: before the first load and
  after a crash the browser connects once the next load is asked for (the frontend's messages then
  come before the load's own requests, which wait for a policy decision).
- **Intercept** is the Web Inspector's own request interception, which WebKit has in its page
  processes (no patch): with the Network domain's interception on and an intercept for any address
  (`Network.setInterceptionEnabled`, `addInterception`, per stage), a page's process holds each
  request it is about to hand the network process (`WebLoaderStrategy::scheduleLoad`) and each
  response before its resource sees it (`WebResourceLoader::didReceiveResponse`) and tells the
  frontend (`requestIntercepted`, `responseIntercepted`). devtools.js decides which wait for the
  agent (the rules: address, type, method) and lets the others go at once, and turns the agent's
  decision into the protocol's command: `interceptContinue`, `interceptWithRequest` (address,
  method, all the headers, the body as base64), `interceptRequestWithError`,
  `interceptRequestWithResponse`, `interceptWithResponse`. A target the page goes to is paused at
  its start already (as for the recording): the interception is on in it before it loads anything.
  While the interception is on WebKit takes nothing from its memory cache and starts no preloads
  (each use is a request of its own).
  The browser learns from devtools.js, after the inspector's messages, that a request paused, and
  ends the waits on the page from its run loop.
- **Compiled once per computer.** The browser is 170 MB of WebAssembly; compiling it takes the host
  about 20 s. The engine keeps large compiled programs on disk (`addons/mod` 0012:
  `~/.cache/collabo-core/programs`), so that is paid once per computer and version of the
  browser, and later sandboxes start it in about a second.

What it takes (an aarch64 host, 6 cores; `tests/run.sh` and a few public sites):

| | |
|---|---|
| the browser's first start on a computer | ~20 s (compiling it), then ~1 s in every sandbox |
| a simple page | well under a second (`tests/run.sh`: 184 checks, ~45 s with the boot) |
| the developer tools on | little: English Wikipedia articles 0.6–1.3 s off, 0.5–1.9 s on (with the bodies of text: ~1.7 MB a page kept); `web network`, `web har` (12 MB) well under a second |
| Hacker News, MDN, GitHub, X | 1–2 s; Korean Wikipedia, Naver, BBC, YouTube, Namuwiki 2–7 s; CNN ~25 s |
| the engine's memory | ~1 GB with the add-on booted, ~1.7 GB after a few sites (3 web processes) |

## Limits

- **Speed.** JavaScript is interpreted, in WebAssembly: script-heavy applications (editors, maps,
  large single-page apps) are slow, and some will time out.
- **No video or audio, no WebGL, no WebAssembly, no WebRTC**, no PDF viewer, no spell checking,
  no XSLT. Fonts: DejaVu and Nanum Gothic only (pages' web fonts load as usual). Sites behind a
  robot check that runs WebAssembly (AWS WAF: Amazon) stay on the check's empty page.
- **Pages that draw themselves later.** `open` answers when the document has loaded; a single-page
  application may still be filling itself in (Reddit: its splash for a few seconds more) —
  `web wait text …` or `web wait selector …` waits for what is wanted.
- **The clock.** Certificates are checked against the sandbox's clock (the host's). A host whose
  clock is days off fails on many HTTPS sites; the error says what time the sandbox thought it
  was. A stapled OCSP response outside its validity period is left out rather than failing the
  connection, as browsers do (glib-networking patch 0002).
- **Another browser's user agent** changes what sites are told, not what the engine can do: a
  site that tests for a feature WebKit lacks here (WebAssembly, WebGL) still finds it missing, and
  workers see WebKit's own `navigator` (the identity script runs in documents only). `safari` is the
  most consistent choice; client hints (`Sec-CH-UA`) go to https sites only.
- One page per browser (popups open in it); start more browsers with `-n NAME`.
- **The developer tools** see the page's requests, its frames' and its dedicated workers', not
  those of service workers (another process) nor the browser's own (favicons, downloads). A
  redirect's hop has no detailed timing (as in the Web Inspector). A request body is what WebKit
  gives as text (UTF-8, else Latin-1): a form's, JSON, text; not files an upload reads from disk. While they are on, each page process keeps the
  bodies it loaded for them (WPE's inspector keeps up to 50 MB), and the browser what it was asked
  to keep. Everything is in memory: `web har` saves it.
- **Intercept** holds what a page's process loads through the network process: documents (also
  of another site), frames, scripts, styles, images, fonts, `fetch`, `XMLHttpRequest`, a worker's
  requests (its page holds them). Not WebSockets, a synchronous `XMLHttpRequest`, `sendBeacon`
  and pings, `data:` addresses, service workers' requests, nor a redirect's next hop (the request
  pauses once, before its first). A response pauses before its body has come: its body can be
  replaced, not read or changed in place. A request's headers at the request stage are the page's
  (the network process adds cookies after: a `Cookie` header put on is the jar's). A document
  answered here has the type `--mime` says, without a charset: a page that is not ASCII says
  `<meta charset="utf-8">`. Nothing pauses on its own end: a paused request waits until decided,
  `intercept off`, `devtools off`, or the page goes (a navigation gives its requests up).

Debugging: `WPE_BROWSER_DEBUG=1` (in the environment of the first `web` command, which starts the
browser) logs loads and waits to `gui logs web`; a crash of any of its processes is reported by
the engine on its stderr with the program's frames by name (`a guest program trapped`; an `abort`
traps too, so that it says where). `WPE_WASM_KEEP_NAMES=0 tools/browser.sh` builds without the
names (45 MB smaller).

## Building

```sh
./build.sh userspace python         # musl, compiler-rt; zlib, OpenSSL, sqlite for the guest
addons/wpe/build.sh                 # or python3 addons/build.py build wpe
```

The WebKit source is the release tarball unpacked at `../org/wpewebkit-2.54.1` (`WK_SRC`); it is
exported into `build/src/webkit` and never modified. The host needs clang-19 and lld-19, python3,
perl; the build makes its own ruby, gperf, meson, unifdef and a host GLib (`build/host`). A first
build takes hours on 6 cores and about 6 GB of `build/`.

| Stage | |
|---|---|
| `host` | the build's tools (`tools/host-tools.sh`) |
| `deps` | libc++, the compat library, libffi (`ffi/`), GLib, libsoup, glib-networking (OpenSSL), ICU, HarfBuzz, FreeType, fontconfig, libxml2, libpng/jpeg/webp, nghttp2, libpsl, brotli, libgcrypt, libepoxy, libxkbcommon for wasm32, static (`tools/deps.sh`, sources and hashes in `sources.lock`, patches in `patches/<name>/`) |
| `webkit` | WebKit with `patches/webkit/*` (`tools/webkit.sh`): CLoop, Skia, no JIT, no media, the collabo display |
| `browser` | `wpe-browser.wasm`, `web.wasm` (`tools/browser.sh`) |
| `cpio` | `out/wpe.cpio`, the fonts, `out/licenses` |

`tests/run.sh` boots the runtime with the add-on and runs `tests/guest.sh` against pages served
in the guest (`tests/server.py`: `tests/site`, and `/headers`, which answers with the headers it
got): the guide for agents (all of it, its topics, parts by their first letters, an unknown
topic, no browser started); opening, reading, Korean text, typing, selecting, clicking, following a link, back, keys,
scrolling, a screenshot, a failed load; `--screenshot` on each kind of action (the ring at the
point and the caption read back from the PNG, a scroll's arrow, a missing folder, a browser
without the display, one whose display forbids pictures); the headers, user agent and languages a page and its
`fetch` send, the page's `navigator`, cookies, an injected script, JavaScript files (`tests/js`:
a result, an error's line, stdin, a timeout), the history (also after a restart), options at a
start and for a running browser, settings from the environment; the developer tools (`site/net.html`
and the server's `/api/data`, `/api/echo`, `/redirect`, `/pixel.png` and `/ws`, a WebSocket):
listing, filtering, a request in detail (headers as sent with cookies, query, POST body, timings),
bodies, searching (text, headers only, a regular expression in WebSocket messages, case, a type, no
match, a HAR file), a WebSocket's messages, a worker's request, a HAR file (checked entry by entry) and
viewing it, requests exported (a POST as curl, run with the guest's curl; all the fetches as one
script; as Python, run; as the raw message, sent over a socket; as fetch, run in the page by `web
run`; as Node.js and `.http`, sent here by Node and Python's `http.client` to the test server; two
as a HAR; from a HAR file; without cookies; a WebSocket; a wrong format; no such request), a navigation to another site (another process), watching, off, on from a browser's
start, after the page's process crashed; intercept (`site/hold.html` and its requests one after the
other): a document paused (`open` ends with 6), listed and shown, sent; a fetch sent to another
address, a POST sent with another body and a header (the server echoes them), one dropped (the
page's `fetch` rejects), one answered here (201); the page's results; their records and HAR entries;
the response stage (shown, not droppable, sent changed: the page gets the new body); a worker's
request; a click whose fetch pauses (`click` ends with 6); a `run` ended by its pause; a document answered from a file; off and `devtools off`
releasing a paused document; closing. The Dart
package's `web tools` test (`dart/collabo_core/test`) drives the same through `WebTools`, with a
runtime that has the add-on in `app/images/addons`.

## Patches

`patches/webkit/` (to the release; `tools/save-patches.sh` writes them from `build/src/webkit`):

| Patch | |
|---|---|
| `0001-build-for-wasm32` | WPE's libWPEWebKit as a static library on wasm32; double-conversion, SIMDe and Skia/skcms (no `musttail`) know wasm; `size_t` is `unsigned long` there (JSC's options, `RawHex`); xdgmime's globals that clash with GIO's copy renamed |
| `0002-jsc-collect-from-the-run-loop-on-wasm` | the heap asks for collections from allocations and runs them from a run-loop timer |
| `0003-shared-memory-between-processes-without-shared-memory` | `SharedMemory` as a memory file, written back when shared; `flushForSharing` / `reloadFromSharer` for the frames |
| `0004-cpu-rendering-without-egl` | no EGL library: no EGL display, no compositing, render on the CPU |
| `0005-wpe-platform-collabo-display` | the `collabo` WPE platform (display, toplevel, view, keymap, input method, clipboard) |
| `0006-callbacks-take-what-they-are-called-with` | interface init functions with the two parameters GLib passes; xdgmime's directory callback |
| `0007-ipc-complete-argument-types-before-taking-handler-addresses` | the IPC generator completes a message's argument types before the handler's address is taken (else clang gives the address no real signature and the call traps) |
| `0008-injected-bundle-linked-in` | the injected bundle's entry point when it is linked into the program |
| `0009-process-cache-and-prewarming-follow-the-cache-model` | the web process cache follows a cache model set after the start; no process started ahead below `PrimaryWebBrowser` |
| `0010-web-process-extension-linked-in` | a web process extension linked into the program: its entry point (`webkit_web_process_extension_initialize_with_user_data`) is called with the initialization data whether or not a directory of extensions is set |
| `0011-inspector-frontend-in-the-program` | an inspector frontend of the program's own on a web view's inspector controller (as the automation's passthrough channel): the protocol's messages as text to a C callback and from the program (the developer tools) |

The libraries': `glib/` 0001 (no libatomic on wasm32), 0002 (function pointer casts), 0003
(registered callbacks called as what they are: closures, notifies, `GTask`, signals through
libffi), 0004 (`GSource` callbacks likewise); `libsoup/` 0001 and `glib-networking/` 0001
(function pointer casts); `glib-networking/` 0002 (stale stapled OCSP responses left out);
`libgpg-error/` 0001 (the lock type of wasm32's musl: a lock of no size aborted every process
that used libgcrypt).

Outside the add-on, the browser relies on the engine and kernel changes of `addons/mod`: 0008
(futexes keyed by process: WebKit's processes are the same program), 0009 and 0010 (a trap in a
guest program — a failed `RELEASE_ASSERT`, a stack overflow, a call through a pointer of another
type — ends that program, not the machine), 0011 (`WebTools` in the Dart package) and 0012
(large programs compiled once per computer, and the compiler's memory given back).

## Licenses

WebKit: LGPL-2.1 and BSD (`out/licenses`); Skia: BSD; GLib, libsoup, glib-networking: LGPL-2.1;
the other libraries their own (copied to `out/licenses`); DejaVu fonts: Bitstream Vera license;
Nanum Gothic: SIL OFL 1.1. The add-on's own code (`browser/`, `compat/`, `ffi/`, `tools/`): as
collaboCore (GPL-3.0); the collabo WPE backend in patch 0005: BSD-2-Clause, as the WebKit files
around it, and the display's client library it compiles in (`tools/gui/client`): MIT.
