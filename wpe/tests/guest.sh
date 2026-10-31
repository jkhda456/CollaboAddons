#!/bin/sh
# Runs in the guest (tests/run.sh boots it): every step's output in $OUT/NAME.{out,err,code}.
T=/t
OUT=$T/out
rm -rf $OUT; mkdir -p $OUT
python3 $T/server.py 8000 >/dev/null 2>&1 &
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
step uadefault   web ua default
step platform    sh -c 'web reload >/dev/null && web eval navigator.platform'
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
step idtoffopen  sh -c 'web intercept on >/dev/null; web reload'
step idtoff      web devtools off
step idtoffload  web wait load 30
step close       web close
sleep 2
step gone        web list
# the guide for agents (web --agent-help): all of it, topics, a part by its first letters; no browser starts
step agenthelp   web --agent-help
step agenttopics web --agent-help topics
step agentparts  web --agent-help net inter
step agentcmd    web -n helponly agent-help act
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
