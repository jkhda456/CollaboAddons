#!/bin/sh
# Runs in the guest (tests/run.sh boots it): every step's output in $OUT/NAME.{out,err,code}.
T=/t
OUT=$T/out
rm -rf $OUT; mkdir -p $OUT
python3 $T/server.py 8000 8443 >/dev/null 2>&1 &
sleep 2
step() { name=$1; shift; ( "$@" ) > $OUT/$name.out 2> $OUT/$name.err; echo $? > $OUT/$name.code; }
export WEB_START_TIMEOUT=600
step open        web open http://127.0.0.1:8000/index.html 180
step info        web info
step read        web read
step eval        web eval 'document.title + " " + (1 + 2)'
step korean      web eval 'document.querySelector("h1").textContent'
step type        web type 'Claude 한글' '#name'
step select      web select '#color' green
step submit      web click Greet
step result      web eval 'document.getElementById("out").textContent'
step count1      web click Count
step count2      web click Count
step counted     web eval 'document.getElementById("count").textContent'
step keys        web key Tab shift+Tab ctrl+a
step keyseen     web eval 'window.keys.join(",")'
step links       web links
step shot        web screenshot $OUT/page.png
step guilist     gui list
step follow      web click 'Second page'
step second      web eval 'location.pathname + " " + document.title'
step back        web back
step backed      web eval 'location.pathname'
step scroll      web scroll down 2
# --screenshot: a picture of what an action did, where it went marked (a click's as it goes in)
step shotclick   web click Count --screenshot $OUT/shot-click.png
step shotxy      web click 30 40 --screenshot $OUT/shot-xy.png
step shothover   web hover 'Second page' --screenshot=$OUT/shot-hover.png
step shottype    web type x '#name' --screenshot $OUT/shot-type.png
step shotkey     web key Tab --screenshot $OUT/shot-key.png
step shotselect  web select '#color' blue --screenshot $OUT/shot-select.png
step shotscroll  sh -c 'web eval "document.body.style.minHeight = \"3000px\"; 1" >/dev/null && web scroll down 2 --screenshot /t/out/shot-scroll.png'
step shottop     web scroll top --screenshot $OUT/shot-top.png
step shotfollow  web click 'Second page' --screenshot $OUT/shot-follow.png
step shotback    web back --screenshot $OUT/shot-back.png
step shotnodir   web click Count --screenshot /nonexistent/shot.png
step shotbadcmd  web eval 1 --screenshot $OUT/shot-bad.png
step shotnofile  web click Count --screenshot
# a browser with no display (gui not on its PATH, no socket): WebKit's picture all the same; and
# one the display's settings forbid (capture: false): the action still goes, the picture does not
mkdir -p /tmp/nogui && ln -sf /usr/bin/wpe-browser /tmp/nogui/wpe-browser
step shotbare    env COLLABO_GUI_CONFIG=/tmp/nogui/gui.json COLLABO_GUI_SOCKET=/tmp/nogui/none.sock PATH=/tmp/nogui \
                 /usr/bin/web -n bare open http://127.0.0.1:8000/second.html 180 --screenshot $OUT/shot-bare.png
echo '{"capture": false}' > /tmp/nogui/gui.json
step shotdenied  /usr/bin/web -n bare click 'Back home' --screenshot $OUT/shot-denied.png
/usr/bin/web -n bare close >/dev/null
step badhost     web open http://127.0.0.1:9/ 30
step after       web info
# how the browser presents itself: headers, user agent, languages, cookies, injected scripts
step header      web header set X-Collabo-Test yes
step headerother web header set X-Only-Other other --host example.com
step lang        web lang ko-KR,en
step ua          web ua chrome
step uas         web ua list
step headers     web open http://127.0.0.1:8000/headers 60
step echoed      web text
step fetched     web run /t/js/fetch.js /headers
step navigator   web eval 'navigator.platform + " " + navigator.vendor + " " + navigator.userAgentData.brands.map(b => b.brand).join("/") + " " + navigator.language'
step unset       web header unset X-Collabo-Test
step cookieset   web cookie set 'flavor=choco; Max-Age=600' 'a=1; b=2'
step cookielist  web cookie list
step cookiesent  web run /t/js/fetch.js /headers
step cookiedel   web cookie delete a
step badheader   web header set Cookie x=1
step inject      web inject add /t/js/inject.js
step injected    web open http://127.0.0.1:8000/second.html 60
step injectedat  web eval 'window.injectedAt'
step settings    web settings
# JavaScript files
step runerr      web run /t/js/bad.js
step runstdin    sh -c 'echo "return args.length + 6 * 7" | web run - x y'
step runtimeout  sh -c 'echo "await new Promise(done => setTimeout(done, 20000))" | web run --timeout 2 -'
step uadefault   web ua webkit
step platform    sh -c 'web reload >/dev/null && web eval navigator.platform'
# who it says it is, answered by WebKit (patch 0012): a Mac of the recorded ones, its screen and
# window, Safari's languages, a time zone; the browser's own scripts in a world of their own
step uasafari    web ua default
step identity    web identity
step natives     sh -c 'web open http://127.0.0.1:8000/index.html 60 >/dev/null && web read >/dev/null && web run /t/js/natives.js'
step isolated    web eval --isolated 'typeof globalThis.__wpeRefs + " " + globalThis.__wpeRefs.length'
step isorun      sh -c 'echo "return typeof window.keys + \" \" + typeof globalThis.__wpeRefs" | web run --isolated -'
step safarilang  sh -c 'web open http://127.0.0.1:8000/headers 60 >/dev/null && web text'
step idnew       web identity new 12345
step idsame      web identity
step tz          web tz Asia/Seoul
step tzpage      sh -c 'web reload >/dev/null && web eval "Intl.DateTimeFormat().resolvedOptions().timeZone + \" \" + new Date(2026, 0, 1).getTimezoneOffset()"'
step tzbad       web tz Mars/Base
step tzback      web tz default
# input as a person gives it: the pointer's path, held buttons and keys, spaced keys, the wheel's
# notches; and all at once with humanize off
step humanpage   web open http://127.0.0.1:8000/events.html 60
step humanclick  web click Press
step humantype   web type 'Hi there' '#field'
step humanwheel  web scroll down 3
step human       web run /t/js/events.js
step humanoff    web humanize off
step rawpage     web reload
step rawclick    web click Press
step rawtype     web type 'Hi there' '#field'
step raw         web run /t/js/events.js
step humanon     web humanize on
# robot checks: recognized, and got past as a person does (a checkbox once it shows, a button held)
step checknone   web challenge
step checkopen   web open http://127.0.0.1:8000/checkbox.html 60
step checkbox    web challenge solve 60
step holdopen    web open http://127.0.0.1:8000/hold-check.html 60
step holdstatus  web challenge
step hold        web challenge solve 90
step enginebad   web search x --engine nope
# history
step history     web history
step historyq    web history second
# developer tools: the network as the Web Inspector records it, saved and read as HAR
step netoff      web network
step dton        web devtools on
step netpage     web open http://127.0.0.1:8000/net.html 60
step netdone     web wait text 'all done' 60
step netstate    web eval 'document.getElementById("state").textContent'
step netlist     web network --all
step netapi      web network show 'api/data?x=1'
step netpost     web network show api/echo
step netws       web network show /ws
step netbody     web network body 'api/data?x=1'
step netpng      web network body pixel.png $OUT/pixel.png
step netbin      web network body pixel.png
step netfetch    web network --type fetch
step net404      web network --status 404
step search      web network search 세계
step searchhead  web network search devtools=yes --in headers
step searchre    web network search 'echo ping \d' --regex
step searchcase  web network search WORKER --case
step searchtype  web network search worker --type script
step searchnone  web network search no-such-text-anywhere
step searchbad   web network search x --in nowhere
step har         web har $OUT/net.har
step harview     web har view $OUT/net.har
step harone      web har view $OUT/net.har 1
step harsearch   web har search $OUT/net.har hello --type fetch
# requests exported for other tools: printed, into files, sent again with them
step excurl      web network export api/echo
step exfile      web network export api/echo -o $OUT/echo.sh
step exrun       $OUT/echo.sh
step exall       web network export all --type fetch -o $OUT/fetches.sh
step exallrun    sh $OUT/fetches.sh
step expython    web network export api/echo --as python -o $OUT/echo.py
step expyrun     python3 $OUT/echo.py
step exraw       web network export api/echo --as raw -o $OUT/echo.raw
step exrawrun    python3 $T/send-raw.py 127.0.0.1 8000 $OUT/echo.raw
step exhttp      web network export api/echo --as http -o $OUT/echo.http
step exfetch     web network export 'api/data?x=1' --as fetch -o $OUT/data.js
step exfetchrun  sh -c '{ printf "return (await (await "; sed "\$ s/;\$//" /t/out/data.js; printf ").json()).query"; } | web run -'
step exnode      web network export api/echo --as node -o $OUT/echo.js
step exhar       web network export api/echo pixel.png --as har -o $OUT/two.har
step exharview   web har view $OUT/two.har
step exfromhar   web har export $OUT/net.har api/echo
step exnocookie  web network export api/echo --no-cookies
step exws        web network export /ws
step exwsraw     web network export /ws --as raw
step exbad       web network export api/echo --as nope
step exnone      web network export no-such-request
step swap        web open http://localhost:8000/second.html 60
step netswap     web network localhost
step watch       sh -c '(sleep 3; web eval "fetch(\"/api/data?watched=1\"); 1") >/dev/null & web network watch watched --timeout 8'
step dtinfo      web info
step dtoff       web devtools off
step notrec      sh -c 'web eval "fetch(\"/api/data?unrecorded=1\"); 1" >/dev/null; sleep 2; web network unrecorded'
step netclear    web network clear
step netcleared  web network
# intercept: the page's requests wait for the agent, each sent (changed or not), dropped or answered
# (site/hold.html makes them one after the other)
step icoff       web intercept
step ion         web intercept pause --type document,fetch
step iopen       web open http://127.0.0.1:8000/hold.html 60
step ilist       web intercept list
step ishow       web intercept show hold.html
step iinfo       web info
step isend       web intercept send hold.html
step iwait1      web intercept wait 30
step imove       web intercept send hold=1 --url '/api/data?changed=1'
step iwait2      web intercept wait 30
step ipost       web intercept show api/echo
step ibody       web intercept send api/echo --body '{"hello":"agent 한"}' --header 'X-Agent: yes'
step iwait3      web intercept wait 30
step idrop       web intercept stop drop=1
step iwait4      web intercept wait 30
step irespond    web intercept respond mock=1 --status 201 --body '{"mocked":true}'
step idone       web wait text 'hold done' 30
step istate      web eval 'document.getElementById("state").textContent'
step inet        web network api/
step inetshow    web network show api/echo
step inotpaused  web intercept send hold.html
step ibad        web intercept drop all --url x
step iresp       web intercept on api/data --stage response
step irespwait   sh -c 'web eval "fetch(\"/api/data?resp=1\").then(r => r.text()).then(t => document.title = \"got \" + t); 1" >/dev/null; web intercept wait 30'
step irespshow   web intercept show resp=1
step irespdrop   web intercept drop resp=1
step irespedit   web intercept send resp=1 --status 299 --body '{"edited":1}'
step iresptitle  sh -c 'sleep 1; web eval document.title'
step iclick      sh -c 'web intercept on clicked=1 >/dev/null; web click Ask'
step iclicksend  web intercept send clicked=1
step iclicked    web wait text clicked=1 10
step iworker     sh -c 'web intercept on worker=1 >/dev/null; web open http://127.0.0.1:8000/net.html 60 >/dev/null; web intercept wait 30'
step iworkersend web intercept send worker=1
step iworkerdone web wait text 'all done' 30
step iworkernet  web network worker=1
step irun        sh -c 'web intercept on run=1 >/dev/null; echo "return (await (await fetch(\"/api/data?run=1\")).json()).query" | web run -'
step irunsend    web intercept send run=1
step ifileon     web intercept on --type document
step ifileopen   web open http://127.0.0.1:8000/nowhere.html
step ifile       sh -c 'printf "<!doctype html><meta charset=utf-8><title>Served</title><p>served here</p>" > /tmp/served.html; web intercept respond nowhere --body-file /tmp/served.html --mime text/html'
step ifileload   web wait load 30
step ioffopen    web open http://127.0.0.1:8000/second.html
step ioff        web intercept off
step ioffload    web wait load 30
step ihar        web har $OUT/intercept.har
# intercept in the network process (WebKit patch 0013): what the page's inspector cannot hold — a
# synchronous XMLHttpRequest, beacons, a redirect's next hop, a response with its body, a WebSocket's
# handshake and messages, a service worker's own requests. The server says what reached it (/api/seen).
SEEN=http://127.0.0.1:8000/api/seen
step ncaps       web --capabilities
step nclear      curl -s "$SEEN?clear=1"
step nopen       web open http://127.0.0.1:8000/more.html 60
step non         web intercept on api/sync api/beacon api/any ws2
step nstatus     web intercept status
step nsync       web eval 'syncXhr("one")'
step nsynclist   web intercept list
step nsyncjson   web intercept show api/sync --json
step nsyncbody   web intercept body api/sync
step nsyncseen   curl -s "$SEEN"
step nsyncnet    web network api/sync
step nsyncblock  web read
step nsyncsend   web intercept send api/sync --header 'X-Agent: sync-agent' --body 'changed sync body'
step nsyncdone   web wait text 'sync 200' 20
step nsyncseen2  curl -s "$SEEN?clear=1"
step nbeacon     sh -c 'web eval "beacon(\"drop\")" >/dev/null; web intercept wait 20'
step nbeaconshow web intercept show beacon
step nbeacondrop web intercept drop beacon
step nbeacon2    sh -c 'web eval "beacon(\"two\")" >/dev/null; web intercept wait 20'
step nbeaconsend web intercept send beacon --url '/api/beacon?tag=changed' --header 'X-Agent: beacon-agent' --body 'changed beacon'
step nbeaconseen sh -c 'sleep 2; curl -s "$0?clear=1"' "$SEEN"
step nping       sh -c 'web click Ping >/dev/null; web intercept wait 20'
step npingshow   web intercept show ping=1 --json
step npingsend   web intercept send ping=1 --header 'X-Agent: ping-agent'
step npingseen   sh -c 'sleep 1; curl -s "$0?clear=1"' "$SEEN"
step nredir      sh -c 'web eval "redirect(\"r1\"); 1" >/dev/null; web intercept wait 20'
step nredirseen  curl -s "$SEEN"
step nredirshow  web intercept show any?hop
step nredirsend  web intercept send any?hop --url '/api/any?hop=changed'
step nredirdone  web wait text 'redirect {' 20
step nredirnet   web network show hop=changed
step nredirseen2 curl -s "$SEEN?clear=1"
step nws         sh -c 'web eval "socket(); 1" >/dev/null; web intercept wait 20'
step nwsshow     web intercept show ws2
step nwsrespond  web intercept respond ws2 --body x
step nwssend     web intercept send ws2 --header 'X-Agent: ws-agent' --url '/ws2?changed=1'
step nwsopen     web wait text 'hello /ws2?changed=1 x-agent=ws-agent' 20
step nmsgon      web intercept on ws2 --stage messages
step nmsg        sh -c 'web eval "ws.send(\"hello there\"); 1" >/dev/null; web intercept wait 20'
step nmsglist    web intercept list
step nmsgshow    web intercept show ws2
step nmsgrespond web intercept respond ws2
step nmsgsend    web intercept send ws2 --body 'changed text'
step nmsgrecv    web intercept wait 20
step nmsgrshow   web intercept show ws2 --json
step nmsgrsend   web intercept send ws2 --body 'echo replaced'
step nmsgseen    web wait text 'echo replaced' 20
step nmsgbin     sh -c 'web eval "ws.send(new Uint8Array([1, 2, 3])); 1" >/dev/null; web intercept wait 20'
step nmsgbinbody web intercept body ws2 $OUT/msg.bin
step nmsgbinsend web intercept send ws2
step nmsgbinrecv web intercept wait 20
step nmsgbinshow web intercept show ws2
step nmsgbindrop web intercept drop ws2
step nmsgafter   sh -c 'web eval "ws.send(\"after\"); 1" >/dev/null; web intercept wait 20 >/dev/null; web intercept send ws2 >/dev/null; web intercept wait 20 >/dev/null; web intercept send ws2'
step nmsgpage    sh -c 'sleep 1; web eval "said.join(\" | \")"'
step nmsgseen2   curl -s "$SEEN?clear=1"
step nmsgnet     web network show ws2
step nbodyon     web intercept on api/any --stage body
step nbody       sh -c 'web eval "fetch(\"/api/any?body=1\").then(r => r.text()).then(t => say(\"body \" + t)); 1" >/dev/null; web intercept wait 20'
step nbodyshow   web intercept show body=1
step nbodyread   web intercept body body=1
step nbodysend   web intercept send body=1 --header 'X-Extra: 1' --body '{"replaced":true}'
step nbodydone   web wait text 'body {"replaced":true}' 20
step nbody2      sh -c 'web eval "fetch(\"/api/any?body=2\").then(r => r.text()).then(t => say(\"second \" + t), e => say(\"second failed \" + e.name)); 1" >/dev/null; web intercept wait 20'
step nbodydrop   web intercept drop body=2
step nbodydropped web wait text 'second failed' 20
step nbody3      sh -c 'web eval "fetch(\"/api/any?body=3\").then(r => r.text()).then(t => say(\"third \" + t)); 1" >/dev/null; web intercept wait 20'
step nbodykeep   web intercept send body=3 --header 'X-Kept: 1'
step nbodykept   web wait text 'third {' 20
step nother      sh -c 'web -n other open http://127.0.0.1:8000/more.html 60 >/dev/null; web -n other eval "syncXhr(\"other\")"'
step notherlist  web intercept list --json
step notherclose web -n other close
step nnavon      web intercept on api/sync
step nnav        sh -c 'web eval "setTimeout(() => syncXhr(\"nav\"), 10); 1" >/dev/null; web intercept wait 20'
step nnavopen    web open http://127.0.0.1:8000/second.html 30
step nnavdrop    web intercept drop api/sync
step nnavopen2   web open http://127.0.0.1:8000/second.html 30
step nswon       web intercept on --type service-worker
step nswopen     sh -c 'web open http://127.0.0.1:8000/sw/page.html 60 >/dev/null; web wait text controlled 60'
step nswnet      sh -c 'web eval "ask(\"/sw/net\"); 1" >/dev/null; web intercept wait 30'
step nswlist     web intercept list
step nswsend     web intercept send sw=1 --header 'X-Agent: sw-agent'
step nswanswers  sh -c 'web wait text "/sw/net 200" 30 >/dev/null; web eval "ask(\"/sw/cached\").then(() => ask(\"/sw/made\")); 1" >/dev/null; web wait text "/sw/made 200" 30; web eval "said.join(\" | \")"'
step nswnetlist  web network /sw/
step nswrecord   web network show sw=1
step nswseen     curl -s "$SEEN?clear=1"
step nswoff      sh -c 'web intercept off; web eval "navigator.serviceWorker.getRegistration(\"/sw/\").then(r => r && r.unregister()); 1"'
step nhar        web har $OUT/net-intercept.har
step ncancel     sh -c 'web open http://127.0.0.1:8000/more.html 30 >/dev/null; web intercept on api/any >/dev/null; web eval "redirect(\"gone\"); 1" >/dev/null; web intercept wait 20'
step ncancelopen web open http://127.0.0.1:8000/second.html 30
step ncancellist sh -c 'sleep 1; web intercept list'
step ncancelnet  web network hop=gone
step ncancelseen curl -s "$SEEN?clear=1"
step idtoffopen  sh -c 'web intercept on >/dev/null; web reload'
step idtoff      web devtools off
step idtoffload  web wait load 30
# web proxy: a browser (-n px) through the tests' proxy (proxy.py: HTTP, CONNECT, SOCKS; it logs
# what it forwarded); the server says what reached it ($SEEN); the other browsers go straight
python3 $T/proxy.py 8100 $OUT/proxy.log >/dev/null 2>&1 &
python3 $T/proxy.py 8101 $OUT/proxy-auth.log --auth 'agent:pa:ss' >/dev/null 2>&1 &
sleep 1
curl -s "$SEEN?clear=1" >/dev/null
step pxnone      web -n px proxy
step pxbad       web -n px proxy ftp://127.0.0.1:21
step pxnoport    web -n px proxy http://127.0.0.1
step pxset       web -n px proxy http://127.0.0.1:8100 --bypass none
step pxopen      web -n px open 'http://127.0.0.1:8000/api/any?px=http' 60
step pxws        sh -c 'web -n px open http://127.0.0.1:8000/more.html 60 >/dev/null; web -n px eval "socket(); 1" >/dev/null; web -n px wait text "hello /ws2" 20'
step pxtlsfail   web -n px open 'https://127.0.0.1:8443/api/any?px=tls-untrusted' 60
step pxca        web -n px proxy ca $T/certs/ca.der
step pxtls       web -n px open 'https://127.0.0.1:8443/api/any?px=tls' 60
step pxwss       sh -c 'web -n px open https://127.0.0.1:8443/more.html 60 >/dev/null; web -n px eval "const w = new WebSocket(\"wss://127.0.0.1:8443/ws2?wss=1\"); w.onmessage = e => say(e.data); 1" >/dev/null; web -n px wait text "hello /ws2?wss=1" 20'
step pxother     web open 'http://127.0.0.1:8000/api/any?px=other-browser' 60
step pxdefault   sh -c 'web -n px proxy bypass default >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=bypassed" 60'
step pxsocks     sh -c 'web -n px proxy socks5h://127.0.0.1:8100 --bypass none >/dev/null; web -n px open "http://localhost:8000/api/any?px=socks5" 60'
step pxsocks4    sh -c 'web -n px proxy socks4://127.0.0.1:8100 >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=socks4" 60'
step pxauthnone  sh -c 'web -n px proxy "http://127.0.0.1:8101" >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=auth-none" 60'
step pxauthbad   sh -c 'web -n px proxy "http://agent:wrong@127.0.0.1:8101" >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=auth-bad" 60'
step pxauthbadtls web -n px open 'https://127.0.0.1:8443/api/any?px=auth-bad-tls' 60
step pxauth      sh -c 'web -n px proxy "http://agent:pa%3Ass@127.0.0.1:8101" >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=auth" 60'
step pxauthtls   web -n px open 'https://127.0.0.1:8443/api/any?px=auth-tls' 60
step pxauthws    sh -c 'web -n px open http://127.0.0.1:8000/more.html 60 >/dev/null; web -n px eval "socket(); 1" >/dev/null; web -n px wait text "hello /ws2" 20'
step pxauthagain sh -c 'web -n px proxy "http://agent:wrong2@127.0.0.1:8101" >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=auth-again" 60'
step pxsocksauth sh -c 'web -n px proxy "socks5://agent:pa%3Ass@127.0.0.1:8101" >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=socks-auth" 60'
step pxstatus    web -n px proxy
step pxsite401   web -n px open 'http://127.0.0.1:8000/api/auth' 30
step pxsettings  web -n px settings
step pxintercept sh -c 'web -n px proxy http://127.0.0.1:8100 >/dev/null; web -n px intercept on px=held >/dev/null; web -n px eval "fetch(\"/api/any?px=held\"); 1" >/dev/null; web -n px intercept wait 20 >/dev/null; web -n px intercept send px=held --header "X-Agent: through"; sleep 1; web -n px intercept off'
step pxdown      sh -c 'web -n px proxy http://127.0.0.1:8199 >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=down" 30'
step pxoff       sh -c 'web -n px proxy off >/dev/null; web -n px open "http://127.0.0.1:8000/api/any?px=off" 60'
step pxcaclear   web -n px proxy ca clear
step pxclose     web -n px close
step pxenv       env WEB_PROXY=http://127.0.0.1:8100 WEB_PROXY_BYPASS=none WEB_PROXY_CA=$T/certs/ca.pem web -n pxenv open 'https://127.0.0.1:8443/api/any?px=env' 180
step pxopt       web -n pxenv --proxy socks5://127.0.0.1:8100 open 'http://127.0.0.1:8000/api/any?px=option' 60
step pxenvset    web -n pxenv proxy
step pxenvclose  web -n pxenv close
step pxlog       cat $OUT/proxy.log
step pxauthlog   cat $OUT/proxy-auth.log
step pxseen      curl -s "$SEEN?clear=1"
step pxcaps      web --capabilities
step close       web close
sleep 2
step gone        web list
# the guide for agents (web --agent-help): all of it, topics, a part by its first letters; no browser starts
step agenthelp   web --agent-help
step agenttopics web --agent-help topics
step agentparts  web --agent-help net inter
step agentcmd    web -n helponly agent-help act
step agentproxy  web --agent-help proxy
step agentbad    web --agent-help nope
step agentnone   web list
step helpline    web --help
# history kept across a restart; browser options at a start, and for one running already
step history2    web --user-agent firefox history 3
step uastart     web info
step optrunning  web --header 'X-Late: 1' --lang en settings
step close2      web close
step envstart    env WEB_HEADERS='X-Env: 1' WEB_USER_AGENT=iphone web -n envtest settings
step close3      web -n envtest close
# the developer tools from a browser's start (its first page in a process paused for them)
step dtstart     env WEB_DEVTOOLS=1 web -n dt open http://127.0.0.1:8000/second.html 120
step dtfirst     web -n dt network
step dtset       web -n dt settings
step dtcrash     sh -c 'for p in /proc/[0-9]*; do [ "$(cat $p/comm 2>/dev/null)" = WPEWebProcess ] && kill -9 ${p#/proc/}; done; sleep 2; web -n dt reload 60 >/dev/null && web -n dt network second.html'
step close4      web -n dt close
cp /tmp/.collabo-gui/logs/web.log $OUT/browser.log 2>/dev/null
echo done > $OUT/finished
