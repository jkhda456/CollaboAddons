// devtools.js: the browser's developer tools (`web devtools`, `web network`, `web har`): the network
// as WebKit's Web Inspector records it — every request the page makes (its frames, its workers),
// with the headers that went over the wire, its body, the response, its timings and its content —
// kept as HTTP Archive (HAR 1.2) entries. It runs in a JavaScript context of the browser's own
// (browser.c), not in the page: browser.c hands it the inspector protocol's messages (an inspector
// frontend in the program, WebKit patch 0011) and asks it for listings and the archive; what it
// sends to the inspector it leaves in an outbox that browser.c sends.
// With `web intercept` on, the requests it is asked to (and their responses, if so) wait before they
// go, as the Web Inspector's interception has them wait (Network.setInterceptionEnabled): each until
// the agent sends it (changed or not), drops it or answers it itself.
(() => {
    const MAX_BODY = 8 * 1000 * 1000;      // a response body kept: up to 8 MB
    const MAX_BODIES = 64 * 1000 * 1000;   // and 64 MB in all (the oldest go first)
    const MAX_FRAME = 64 * 1000;           // of a WebSocket message's data (characters)
    const MAX_FRAMES = 2000;               // messages kept per WebSocket
    const LISTED = 100;                    // `web network` lists the last 100
    // the short names of types `--type` takes (of `web network`, `web intercept on`)
    const TYPE_NAMES = { css: 'stylesheet', js: 'script', doc: 'document', img: 'image', ws: 'websocket', xhr: 'xhr|fetch', api: 'xhr|fetch' };

    let bodies = 'text';                   // which response bodies are kept: text, all, none
    let nextCommand = 1;
    const outbox = [];                     // messages for the inspector, sent by browser.c
    const replies = new Map();             // "channel id" (or "id"): what to do with the reply
    const channels = new Map();            // a page's target, or a worker in it: {key, target, worker}
    let records = [];                      // {id, har, key, requestId, ...}, in the order they began
    const open = new Map();                // "channel requestId": the records still loading
    let pages = [];
    let nextId = 1, finished = 0, kept = 0, fetching = 0;
    let intercept = null;                  // web intercept on: {patterns, regex, caseSensitive, types, methods, stages}
    const paused = new Map();              // record id: {r, channel, stage, request | response, since}
    let pauseSerial = 0;                   // counts the requests that paused (browser.c wakes its waits)
    const decisions = new Map();           // a decision's id: {lines, waiting} until WebKit has answered it
    let nextDecision = 1;
    let version = '';                      // WebKit's, for the archives (browser.c says)

    // ── talking to the inspector ─────────────────────────────────────────────────────────────

    function command(method, params, then) {
        const id = nextCommand++;
        if (then)
            replies.set(String(id), then);
        outbox.push(JSON.stringify({ id, method, params: params || {} }));
    }

    // a command to a target's backend, or a worker's in it
    function send(channel, method, params, then) {
        const id = nextCommand++;
        if (then)
            replies.set(channel.key + ' ' + id, then);
        let message = JSON.stringify({ id, method, params: params || {} });
        if (channel.worker)
            message = JSON.stringify({ id: nextCommand++, method: 'Worker.sendMessageToWorker', params: { workerId: channel.worker, message } });
        command('Target.sendMessageToTarget', { targetId: channel.target, message });
    }

    function reply(key, message) {
        const then = replies.get(key);
        if (!then)
            return;
        replies.delete(key);
        then(message.result, message.error);
    }

    function receive(text) {
        const message = JSON.parse(text);
        if ('id' in message)
            return reply(String(message.id), message);
        const p = message.params || {};
        switch (message.method) {
        case 'Target.targetCreated':
            return targetCreated(p.targetInfo);
        case 'Target.targetDestroyed':
            return gone(p.targetId, 'its page process went away');
        case 'Target.dispatchMessageFromTarget':
            return fromChannel(channels.get(p.targetId), JSON.parse(p.message));
        }
    }

    function fromChannel(channel, message) {
        if (!channel)
            return;
        if ('id' in message)
            return reply(channel.key + ' ' + message.id, message);
        const handler = events[message.method];
        if (handler)
            handler(channel, message.params || {});
    }

    // A page's process (a page target; a new one for each site the page goes to, paused before it
    // loads anything so that nothing it loads is missed) and the frame and worker targets WebKit
    // may have: the Network domain on, and the Page domain for the page's navigations and load times.
    function targetCreated(info) {
        if (!info || !['page', 'frame', 'worker'].includes(info.type))
            return;
        const channel = { key: info.targetId, target: info.targetId, type: info.type };
        channels.set(channel.key, channel);
        if (info.type !== 'worker') {
            send(channel, 'Page.enable');
            if (!info.isProvisional)
                send(channel, 'Page.getResourceTree', {}, result => {
                    if (result && result.frameTree)
                        channel.mainFrame = result.frameTree.frame.id;
                });
        }
        send(channel, 'Network.enable');
        if (info.type !== 'worker') {
            // the bodies stay readable after the page goes on (browser.c reads them as they finish)
            send(channel, 'Network.setClearResourceDataOnNavigate', { clearResourceDataOnNavigate: false });
            send(channel, 'Worker.enable');
            // web intercept: before it loads anything (its workers' requests are its own loads)
            interceptIn(channel);
        }
        if (info.isPaused)
            command('Target.resume', { targetId: info.targetId });
    }

    // a target or worker that is gone: what it was still loading ends there
    function gone(key, why) {
        for (const k of [...channels.keys()]) {
            if (k === key || k.startsWith(key + '/'))
                channels.delete(k);
        }
        for (const [k, r] of [...open]) {
            if (r.key === key || r.key.startsWith(key + '/')) {
                if (!r.socket)
                    r.har.response._error = r.har.response._error || why;
                unhold(r, why);
                end(r, undefined);
                open.delete(k);
            }
        }
        for (const held of [...paused.values()]) {
            if (held.channel.key === key)
                unhold(held.r, why);
        }
    }

    // ── what the inspector says ──────────────────────────────────────────────────────────────

    // A time the inspector gives (seconds since its frontend connected, per target) as seconds
    // since 1970: the events that carry both say how far apart they are.
    function wall(channel, timestamp) {
        return channel.wall !== undefined && timestamp !== undefined ? channel.wall + timestamp : Date.now() / 1000;
    }

    function learnWall(channel, timestamp, walltime) {
        if (timestamp !== undefined && walltime)
            channel.wall = walltime - timestamp;
    }

    const events = {
        'Network.requestWillBeSent'(channel, p) {
            learnWall(channel, p.timestamp, p.walltime);
            const previous = open.get(channel.key + ' ' + p.requestId);
            if (previous && p.redirectResponse) {
                // a redirect: the hop before is an entry of its own, which says where it went
                respond(previous, p.redirectResponse);
                previous.har.response.redirectURL = p.request.url;
                previous.responded = p.timestamp;
                end(previous, p.timestamp);
            }
            if (p.type === 'Document' && !previous && (!channel.mainFrame || p.frameId === channel.mainFrame) && !channel.worker)
                newPage(channel, p);
            const r = begin(channel, p.requestId, p.request.url, p.request.method, p.request.headers, p.walltime || wall(channel, p.timestamp));
            r.start = p.timestamp;
            r.type = p.type || 'Other';
            r.frame = p.frameId;
            r.har._resourceType = resourceType(r.type);
            if (p.initiator)
                r.har._initiator = initiatorOf(p.initiator);
            if (p.request.postData !== undefined) {
                const mimeType = header(p.request.headers, 'content-type') || '';
                const postData = { mimeType, text: p.request.postData };
                if (/x-www-form-urlencoded/i.test(mimeType))
                    postData.params = parameters(p.request.postData);
                r.har.request.postData = postData;
                r.har.request.bodySize = utf8Length(p.request.postData);
            } else
                r.har.request.bodySize = 0;
        },
        'Network.responseReceived'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            respond(r, p.response);
            if (p.type)
                r.type = p.type;
            r.har._resourceType = resourceType(r.type);
            r.responded = p.timestamp;
        },
        'Network.dataReceived'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            r.decoded = (r.decoded || 0) + (p.dataLength || 0);
            r.encoded = (r.encoded || 0) + (p.encodedDataLength || 0);
        },
        'Network.loadingFinished'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            if (p.metrics)
                measured(r, p.metrics);
            else {
                r.har.response.content.size = r.decoded || 0;
                if (r.encoded)
                    r.har.response.bodySize = r.encoded;
            }
            unhold(r);
            end(r, p.timestamp);
            if (wantsBody(r))
                fetchBody(r);
        },
        'Network.loadingFailed'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            r.har.response._error = r.dropped ? 'dropped: web intercept drop' : p.errorText || (p.canceled ? 'canceled' : 'failed');
            if (p.canceled && !r.dropped)
                r.canceled = true;
            // the page gave it up while it waited (a navigation, an abort): WebKit lets go of it too
            unhold(r, p.canceled ? 'canceled by the page' : 'failed', true);
            end(r, p.timestamp);
        },
        // web intercept: a request about to go, or a response about to reach the page, waits for the agent
        'Network.requestIntercepted'(channel, p) {
            let r = recordOf(channel, p.requestId);
            if (!r)
                r = begin(channel, p.requestId, p.request.url, p.request.method, p.request.headers, Date.now() / 1000);
            if (!interceptable(r, p.request.url, p.request.method))
                return send(channel, 'Network.interceptContinue', { requestId: p.requestId, stage: 'request' });
            hold(r, channel, 'request', { request: p.request });
        },
        'Network.responseIntercepted'(channel, p) {
            const r = recordOf(channel, p.requestId);
            if (!r || !interceptable(r, r.har.request.url, r.har.request.method))
                return send(channel, 'Network.interceptContinue', { requestId: p.requestId, stage: 'response' });
            hold(r, channel, 'response', { response: p.response });
        },
        'Network.requestServedFromMemoryCache'(channel, p) {
            const resource = p.resource || {};
            const r = begin(channel, p.requestId, resource.url, 'GET', {}, wall(channel, p.timestamp));
            r.start = p.timestamp;
            r.type = resource.type || 'Other';
            r.frame = p.frameId;
            r.har._resourceType = resourceType(r.type);
            r.har._fromCache = 'memory';
            if (p.initiator)
                r.har._initiator = initiatorOf(p.initiator);
            if (resource.response)
                respond(r, resource.response);
            r.har.response.content.size = resource.bodySize || 0;
            r.har.response.bodySize = 0;
            r.har.response.headersSize = 0;
            end(r, p.timestamp);
            if (wantsBody(r))
                fetchBody(r);
        },
        'Network.webSocketCreated'(channel, p) {
            const r = begin(channel, p.requestId, p.url, 'GET', {}, Date.now() / 1000);
            r.type = 'WebSocket';
            r.socket = true;
            r.har._resourceType = 'websocket';
            r.har._webSocketMessages = [];
        },
        'Network.webSocketWillSendHandshakeRequest'(channel, p) {
            learnWall(channel, p.timestamp, p.walltime);
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            r.start = p.timestamp;
            r.har.startedDateTime = iso(p.walltime || wall(channel, p.timestamp));
            r.har.request.headers = headerList(p.request.headers);
            r.har.request.cookies = requestCookies(p.request.headers);
            r.har.request.httpVersion = 'HTTP/1.1';
            r.har.request.bodySize = 0;
        },
        'Network.webSocketHandshakeResponseReceived'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (!r)
                return;
            const response = r.har.response;
            response.status = p.response.status;
            response.statusText = p.response.statusText || '';
            response.headers = headerList(p.response.headers);
            response.cookies = responseCookies(p.response.headers);
            response.httpVersion = 'HTTP/1.1';
            r.responded = p.timestamp;
            timed(r);
        },
        'Network.webSocketFrameSent'(channel, p) {
            frame(channel, p, 'send');
        },
        'Network.webSocketFrameReceived'(channel, p) {
            frame(channel, p, 'receive');
        },
        'Network.webSocketFrameError'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (r)
                r.har.response._error = p.errorMessage;
        },
        'Network.webSocketClosed'(channel, p) {
            const r = open.get(channel.key + ' ' + p.requestId);
            if (r)
                end(r, p.timestamp);
        },
        'Page.frameNavigated'(channel, p) {
            if (p.frame && !p.frame.parentId)
                channel.mainFrame = p.frame.id;
        },
        'Page.domContentEventFired'(channel, p) {
            const page = pageOf(channel);
            if (page && page.start !== undefined && p.timestamp >= page.start)
                page.pageTimings.onContentLoad = round((p.timestamp - page.start) * 1000);
        },
        'Page.loadEventFired'(channel, p) {
            const page = pageOf(channel);
            if (page && page.start !== undefined && p.timestamp >= page.start)
                page.pageTimings.onLoad = round((p.timestamp - page.start) * 1000);
        },
        'Worker.workerCreated'(channel, p) {
            const worker = { key: channel.key + '/' + p.workerId, target: channel.target, worker: p.workerId, type: 'worker' };
            channels.set(worker.key, worker);
            send(worker, 'Network.enable');
            send(channel, 'Worker.initialized', { workerId: p.workerId });
        },
        'Worker.workerTerminated'(channel, p) {
            gone(channel.key + '/' + p.workerId, 'its worker ended');
        },
        'Worker.dispatchMessageFromWorker'(channel, p) {
            fromChannel(channels.get(channel.key + '/' + p.workerId), JSON.parse(p.message));
        },
    };

    // ── the entries ──────────────────────────────────────────────────────────────────────────

    function newPage(channel, p) {
        pages.push({
            id: 'page_' + (pages.length + 1),
            startedDateTime: iso(p.walltime || wall(channel, p.timestamp)),
            title: p.request.url,
            pageTimings: {},
            key: channel.key,
            start: p.timestamp,
        });
    }

    function pageOf(channel) {
        for (let i = pages.length - 1; i >= 0; i--) {
            if (pages[i].key === channel.key.split('/')[0])
                return pages[i];
        }
        return null;
    }

    function begin(channel, requestId, url, method, headers, when) {
        const page = pages[pages.length - 1];
        const har = {
            pageref: page ? page.id : undefined,
            startedDateTime: iso(when),
            time: 0,
            request: {
                method: method || 'GET', url: url || '', httpVersion: '', cookies: requestCookies(headers),
                headers: headerList(headers), queryString: queryOf(url), headersSize: -1, bodySize: -1,
            },
            response: {
                status: 0, statusText: '', httpVersion: '', cookies: [], headers: [],
                content: { size: 0, mimeType: 'x-unknown' }, redirectURL: '', headersSize: -1, bodySize: -1,
            },
            cache: {},
            timings: { blocked: -1, dns: -1, connect: -1, ssl: -1, send: 0, wait: 0, receive: 0 },
        };
        const r = { id: nextId++, har, key: channel.key, requestId, done: false, seq: 0, kept: 0 };
        records.push(r);
        open.set(channel.key + ' ' + requestId, r);
        return r;
    }

    function respond(r, response) {
        const har = r.har.response;
        har.status = response.status || 0;
        har.statusText = response.statusText || '';
        har.headers = headerList(response.headers);
        har.cookies = responseCookies(response.headers);
        har.content.mimeType = response.mimeType || header(response.headers, 'content-type') || 'x-unknown';
        if (response.requestHeaders) {
            // the request's headers as they went over the wire (with the cookies the network added)
            r.har.request.headers = headerList(response.requestHeaders);
            r.har.request.cookies = requestCookies(response.requestHeaders);
        }
        if (response.timing)
            r.timing = response.timing;
        if (response.source === 'memory-cache' || response.source === 'disk-cache')
            r.har._fromCache = response.source === 'memory-cache' ? 'memory' : 'disk';
        else if (response.source === 'service-worker')
            r.har._fromServiceWorker = true;
        const tls = response.security && response.security.connection;
        if (tls && tls.protocol && tls.protocol !== 'Unknown')
            r.har._tls = tls;
    }

    function measured(r, m) {
        const request = r.har.request, response = r.har.response;
        const version = httpVersion(m.protocol);
        if (version)
            request.httpVersion = response.httpVersion = version;
        if (m.requestHeaders) {
            request.headers = headerList(m.requestHeaders);
            request.cookies = requestCookies(m.requestHeaders);
        }
        if (m.requestHeaderBytesSent !== undefined)
            request.headersSize = m.requestHeaderBytesSent;
        if (m.requestBodyBytesSent !== undefined)
            request.bodySize = m.requestBodyBytesSent;
        if (m.responseHeaderBytesReceived !== undefined)
            response.headersSize = m.responseHeaderBytesReceived;
        if (m.responseBodyBytesReceived !== undefined)
            response.bodySize = m.responseBodyBytesReceived;
        response.content.size = m.responseBodyDecodedSize !== undefined ? m.responseBodyDecodedSize : (r.decoded || 0);
        if (response.bodySize >= 0 && response.content.size > response.bodySize)
            response.content.compression = response.content.size - response.bodySize;
        if (response.headersSize >= 0 && response.bodySize >= 0)
            response._transferSize = response.headersSize + response.bodySize;
        if (m.remoteAddress) {
            const at = m.remoteAddress.match(/^\[?(.*?)\]?:(\d+)$/);
            r.har.serverIPAddress = at ? at[1] : m.remoteAddress;
            if (at)
                r.har._serverPort = +at[2];
        }
        if (m.connectionIdentifier)
            r.har.connection = String(m.connectionIdentifier);
        if (m.priority)
            r.har._priority = m.priority;
        if (m.securityConnection && m.securityConnection.protocol && m.securityConnection.protocol !== 'Unknown')
            r.har._tls = m.securityConnection;
        if (m.isProxyConnection)
            r.har._proxied = true;
    }

    function end(r, timestamp) {
        if (r.done)
            return;
        r.done = true;
        r.end = timestamp;
        r.seq = ++finished;
        open.delete(r.key + ' ' + r.requestId);
        timed(r);
    }

    // HAR's timings from the resource's timing (seconds, and milliseconds after fetchStart), as the
    // Web Inspector's HAR export makes them
    function timed(r) {
        const timings = r.har.timings, t = r.timing;
        const startTime = (t && t.startTime) || r.start;
        if (startTime === undefined)
            return;
        const fetchStart = (t && t.fetchStart >= startTime) ? t.fetchStart : startTime;
        const at = offset => offset > 0 ? fetchStart + offset / 1000 : NaN;
        let dnsStart = at(t && t.domainLookupStart), dnsEnd = at(t && t.domainLookupEnd);
        let connectStart = at(t && t.connectStart), connectEnd = at(t && t.connectEnd);
        if (!(dnsStart < dnsEnd))
            dnsStart = dnsEnd = NaN;
        if (!(connectStart < connectEnd))
            connectStart = connectEnd = NaN;
        const secureStart = at(t && t.secureConnectionStart);
        const requestStart = at(t && t.requestStart) || startTime;
        // no response (a failure): the time went in waiting for one
        const responseEnd0 = r.end !== undefined ? r.end : r.responded;
        const responseStart = at(t && t.responseStart) || r.responded || (r.har.response.status ? requestStart : responseEnd0) || requestStart;
        const responseEnd = responseEnd0 !== undefined ? Math.max(responseEnd0, responseStart) : responseStart;
        const ms = (from, to) => Math.max(0, round((to - from) * 1000));
        timings.blocked = ms(startTime, dnsStart || connectStart || requestStart);
        timings.dns = dnsStart ? ms(dnsStart, dnsEnd || connectStart || requestStart) : -1;
        timings.connect = connectStart ? ms(connectStart, connectEnd || requestStart) : -1;
        timings.ssl = secureStart ? ms(secureStart, connectEnd || requestStart) : -1;
        timings.send = (dnsEnd || connectEnd) ? ms(connectEnd || dnsEnd, requestStart) : 0;
        timings.wait = ms(requestStart, responseStart);
        timings.receive = r.socket ? 0 : ms(responseStart, responseEnd);
        // the sum of what is known: ssl is within connect
        r.har.time = round(['blocked', 'dns', 'connect', 'send', 'wait', 'receive'].reduce((sum, k) => sum + Math.max(0, timings[k]), 0));
    }

    function frame(channel, p, type) {
        const r = open.get(channel.key + ' ' + p.requestId);
        if (!r)
            return;
        const messages = r.har._webSocketMessages;
        r.frames = (r.frames || 0) + 1;
        r.frameBytes = (r.frameBytes || 0) + (p.response.payloadLength || 0);
        if (messages.length >= MAX_FRAMES)
            return;
        let data = p.response.payloadData || '';
        if (data.length > MAX_FRAME)
            data = data.slice(0, MAX_FRAME);
        messages.push({ type, time: wall(channel, p.timestamp), opcode: p.response.opcode, data });
    }

    // ── bodies ───────────────────────────────────────────────────────────────────────────────

    function textual(mimeType, type) {
        const m = (mimeType || '').toLowerCase().split(';')[0].trim();
        if (/^text\/|[/+](json|xml|javascript|ecmascript)$|^application\/(x-www-form-urlencoded|graphql|ld\+json|manifest\+json)$|^image\/svg\+xml$/.test(m))
            return true;
        return (!m || m === 'x-unknown') && ['Document', 'StyleSheet', 'Script', 'XHR', 'Fetch', 'EventSource'].includes(type);
    }

    function wantsBody(r) {
        const response = r.har.response;
        if (bodies === 'none' || r.socket || response._error || response.content.size > MAX_BODY)
            return false;
        if (response.status === 204 || response.status === 304 || (response.status >= 300 && response.status < 400))
            return false;
        return bodies === 'all' || textual(response.content.mimeType, r.type);
    }

    function fetchBody(r) {
        const channel = channels.get(r.key);
        if (r.fetching || r.kept || r.bodyError)
            return;
        if (!channel) {
            r.bodyError = 'its page process is gone';
            return;
        }
        r.fetching = true;
        fetching++;
        send(channel, 'Network.getResponseBody', { requestId: r.requestId }, (result, error) => {
            r.fetching = false;
            fetching--;
            if (result && typeof result.body === 'string')
                keep(r, result.body, result.base64Encoded);
            else
                r.bodyError = (error && error.message) || 'no body';
        });
    }

    function keep(r, body, base64) {
        const content = r.har.response.content;
        content.text = body;
        if (base64)
            content.encoding = 'base64';
        r.kept = Math.max(1, body.length);
        kept += r.kept;
        for (const old of records) {
            if (kept <= MAX_BODIES)
                break;
            if (old !== r && old.kept) {
                delete old.har.response.content.text;
                delete old.har.response.content.encoding;
                kept -= old.kept;
                old.kept = 0;
                old.bodyError = 'left out to keep the bodies within ' + size(MAX_BODIES);
            }
        }
    }

    // ── web intercept: requests that wait for the agent ──────────────────────────────────────
    // A page's process holds its requests (and their responses) while its interception is on and
    // one of its intercepts matches: one for any address, per stage asked for. Which of them wait
    // for the agent is decided here (their address, type, method); the others go on at once.

    function interceptIn(channel) {
        if (channel.worker || channel.type === 'worker')
            return;
        const want = intercept ? intercept.stages : {};
        const has = channel.intercepts || (channel.intercepts = {});
        const on = !!(want.request || want.response);
        if (on && !channel.intercepting)
            send(channel, 'Network.setInterceptionEnabled', { enabled: true });
        for (const stage of ['request', 'response']) {
            if (!!want[stage] !== !!has[stage])
                send(channel, want[stage] ? 'Network.addInterception' : 'Network.removeInterception', { url: '', stage });
            has[stage] = !!want[stage];
        }
        // off: WebKit sends what it still holds as it was
        if (!on && channel.intercepting)
            send(channel, 'Network.setInterceptionEnabled', { enabled: false });
        channel.intercepting = on;
    }

    // the record of a request of the target: a worker's requests are loads of its page's process,
    // intercepted there but recorded from the worker
    function recordOf(channel, requestId) {
        const own = open.get(channel.key + ' ' + requestId);
        if (own)
            return own;
        const target = channel.key.split('/')[0];
        for (const r of open.values()) {
            if (r.requestId === requestId && r.key.split('/')[0] === target)
                return r;
        }
        return null;
    }

    function interceptable(r, url, method) {
        const o = intercept;
        if (!o || !o.stages)
            return false;
        if (o.types && !o.types.test(typeOf(r)))
            return false;
        if (o.methods && !o.methods.includes(String(method || 'GET').toUpperCase()))
            return false;
        return !o.patterns.length || o.patterns.some(p => p.test(url || ''));
    }

    function hold(r, channel, stage, what) {
        paused.set(r.id, Object.assign({ r, channel, stage, since: Date.now() }, what));
        r.paused = stage;
        pauseSerial++;
    }

    function note(r, stage, action, extra) {
        const n = Object.assign({ stage, action }, extra || {});
        (r.har._intercepted = r.har._intercepted || []).push(n);
        return n;
    }

    // r waits no more (decided, given up by the page, gone with its process); release: WebKit lets
    // go of it too (it sends a request as it was; a canceled one stays canceled)
    function unhold(r, why, release) {
        const held = paused.get(r.id);
        if (!held)
            return;
        paused.delete(r.id);
        r.paused = null;
        if (release)
            send(held.channel, 'Network.interceptContinue', { requestId: r.requestId, stage: held.stage });
        if (why)
            note(r, held.stage, why);
    }

    function pausedOf(which) {
        const list = [...paused.values()].sort((a, b) => a.r.id - b.r.id);
        if (/^#?\d+$/.test(which))
            return list.filter(h => h.r.id === +which.replace('#', ''));
        return which === 'all' ? list : list.filter(h => urlOf(h).includes(which));
    }

    // why `which` (a number, a part of the address) is no paused request
    function notPaused(which) {
        const number = /^#?\d+$/.test(which) ? +which.replace('#', '') : null;
        let r = null;
        for (let i = records.length - 1; i >= 0 && !r; i--) {
            if (number !== null ? records[i].id === number : (records[i].har.request.url || '').includes(which))
                r = records[i];
        }
        const last = r && r.har._intercepted ? r.har._intercepted[r.har._intercepted.length - 1] : null;
        const was = r ? '#' + r.id + ' is not paused' + (last ? ' (' + last.stage + ' ' + last.action + ')' : '') + ': web network show ' + r.id : '';
        return number !== null ? (r ? was : 'no request #' + number)
            : 'no paused request has ' + JSON.stringify(which) + ' in its address' + (r ? '; ' + was : '');
    }

    function urlOf(held) {
        return (held.request && held.request.url) || held.r.har.request.url || '';
    }

    function methodOf(held) {
        return (held.request && held.request.method) || held.r.har.request.method || 'GET';
    }

    function waited(held) {
        return ((Date.now() - held.since) / 1000).toFixed(1) + ' s';
    }

    // [TEXT...] [--regex] [--case] [--type T[,T]] [--method M[,M]] [--stage request|response|both]
    function interceptOptions(words) {
        const o = { texts: [], regex: false, caseSensitive: false, typeNames: null, types: null, methods: null,
                    stages: { request: true, response: false } };
        for (let i = 0; i < words.length; i++) {
            const w = words[i], more = i + 1 < words.length;
            if (w === '--regex')
                o.regex = true;
            else if (w === '--case')
                o.caseSensitive = true;
            else if (w === '--type' && more)
                o.typeNames = words[++i].toLowerCase().split(/[,\s]+/).filter(Boolean);
            else if (w === '--method' && more)
                o.methods = words[++i].toUpperCase().split(/[,\s]+/).filter(Boolean);
            else if (w === '--stage' && more) {
                const stage = words[++i].toLowerCase();
                if (!['request', 'response', 'both'].includes(stage))
                    return 'E' + '--stage is request (the default: before it goes), response (before the page gets it) or both, not ' + stage;
                o.stages = { request: stage !== 'response', response: stage !== 'request' };
            } else if (w.startsWith('--'))
                return 'E' + 'unknown option ' + w + ' (intercept on [TEXT...] [--regex] [--case] [--type T[,T]] [--method M[,M]] [--stage request|response|both])';
            else
                o.texts.push(w);
        }
        if (o.typeNames && o.typeNames.length)
            o.types = new RegExp('^(' + o.typeNames.map(t => TYPE_NAMES[t] || t.replace(/[^a-z]/g, '')).join('|') + ')$');
        else
            o.typeNames = null;
        try {
            o.patterns = o.texts.map(t => new RegExp(o.regex ? t : t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), o.caseSensitive ? '' : 'i'));
        } catch (e) {
            return 'E' + 'not a regular expression: ' + e.message;
        }
        return o;
    }

    function interceptRules(o) {
        const one = !o.typeNames;
        let what = (one ? 'every request' : o.typeNames.join(', ') + ' requests') + ' the page makes';
        if (o.methods)
            what += ' by ' + o.methods.join(', ');
        if (o.texts.length)
            what += ' whose address ' + (o.regex ? 'matches ' : 'has ') + o.texts.map(t => JSON.stringify(t)).join(' or ')
                + (o.caseSensitive ? '' : ' (any case)');
        const when = o.stages.request && o.stages.response
            ? (one ? 'pauses before it goes, and its response before the page gets it' : 'pause before they go, and their responses before the page gets them')
            : o.stages.response ? (one ? 'pauses with its response, before the page gets it' : 'pause with their responses, before the page gets them')
            : (one ? 'pauses before it goes' : 'pause before they go');
        return what + ' ' + when;
    }

    function interceptStatus() {
        if (!intercept)
            return 'intercept: off (web intercept on: the page\'s requests pause until sent, dropped or answered)\n';
        return 'intercept: on: ' + interceptRules(intercept) + '; ' + paused.size + ' paused now'
            + (paused.size ? ' (web intercept lists them)' : '') + '\n';
    }

    const PAUSED_HEAD = '   #  stage         method  type          waited  url';
    const DECIDE_HINT = '(web intercept show N; send N [--url U] [--header \'NAME: VALUE\'] [--body TEXT] | drop N | respond N [--status S] --body TEXT;'
        + ' N: a number, all, or a part of the address)';

    function pausedRow(held) {
        const stage = held.stage === 'request' ? 'request' : 'response ' + ((held.response && held.response.status) || '');
        return pad(held.r.id, 4, true) + '  ' + pad(stage, 13) + ' ' + pad(methodOf(held), 7) + ' ' + pad(typeOf(held.r), 11) + ' '
            + pad(waited(held), 8, true) + '  ' + shortened(urlOf(held), 100);
    }

    // the requests waiting for the agent; lead: what comes before the count ("paused: ")
    function pausedListing(lead) {
        const list = pausedOf('all');
        if (!list.length)
            return (lead || '') + 'no request is paused' + (intercept ? '' : ' (intercept: off)') + '\n';
        return (lead || '') + list.length + ' request' + (list.length === 1 ? '' : 's') + ' paused, waiting for the agent:\n'
            + PAUSED_HEAD + '\n' + list.map(pausedRow).join('\n') + '\n' + DECIDE_HINT + '\n';
    }

    function pausedDetail(held) {
        const r = held.r, id = r.id;
        const lines = ['#' + id + ' ' + methodOf(held) + ' ' + urlOf(held)];
        if (held.stage === 'request') {
            const request = held.request || {};
            lines.push('paused before it goes (waiting ' + waited(held) + ')   type: ' + typeOf(r) + (r.har.pageref ? '   page: ' + r.har.pageref : ''));
            if (r.har._initiator)
                lines.push('initiator: ' + r.har._initiator.type + (r.har._initiator.url ? ' ' + shortened(r.har._initiator.url, 150) : ''));
            lines.push('', 'request headers (as the page made them; the network adds Cookie, Host, Accept-Encoding, Connection):');
            const headers = headerList(request.headers);
            lines.push(...(headers.length ? headerLines(headers) : ['  (none)']));
            const query = queryOf(urlOf(held));
            if (query.length)
                lines.push('', 'query:', ...query.map(q => '  ' + q.name + ' = ' + q.value));
            if (request.postData !== undefined)
                lines.push('', 'request body (' + (header(request.headers, 'content-type') || 'no type') + ', ' + size(utf8Length(request.postData)) + '):',
                    preview(request.postData, 4000));
            lines.push('', 'web intercept send ' + id + '   as it is; changed: [--url U] [--method M] [--header \'NAME: VALUE\']... [--unset NAME]... [--body TEXT | --body-file FILE]',
                'web intercept drop ' + id + '   the page gets a network error [--error general|access|cancel|timeout]',
                'web intercept respond ' + id + '   answered here, not sent: [--status S] [--header \'NAME: VALUE\']... [--mime TYPE] [--body TEXT | --body-file FILE]');
        } else {
            const response = held.response || {};
            lines.push('paused with its response, before the page gets it (waiting ' + waited(held) + ')   type: ' + typeOf(r));
            lines.push('status: ' + response.status + (response.statusText ? ' ' + response.statusText : '') + '   ' + (response.mimeType || ''));
            lines.push('', 'response headers:');
            const headers = headerList(response.headers);
            lines.push(...(headers.length ? headerLines(headers) : ['  (none)']));
            lines.push('', 'its body has not come yet.',
                'web intercept send ' + id + '   the page gets it as it is; changed: [--status S] [--header \'NAME: VALUE\']... [--unset NAME]... [--mime TYPE] '
                    + '--body TEXT | --body-file FILE (a changed response is given whole: no --body, no body)',
                'web intercept respond ' + id + '   another response in its place: [--status S] [--header \'NAME: VALUE\']... [--mime TYPE] [--body TEXT | --body-file FILE]');
        }
        return lines.join('\n') + '\n';
    }

    // base64 of the UTF-8 of text
    function toBase64(text) {
        const digits = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
        const bytes = unescape(encodeURIComponent(text));
        let out = '';
        for (let i = 0; i < bytes.length; i += 3) {
            const n = (bytes.charCodeAt(i) << 16) | ((bytes.charCodeAt(i + 1) || 0) << 8) | (bytes.charCodeAt(i + 2) || 0);
            out += digits[(n >> 18) & 63] + digits[(n >> 12) & 63] + (i + 1 < bytes.length ? digits[(n >> 6) & 63] : '=')
                + (i + 2 < bytes.length ? digits[n & 63] : '=');
        }
        return out;
    }

    function base64Size(text) {
        const clean = text.replace(/[^A-Za-z0-9+/]/g, '');
        return Math.floor(clean.length * 3 / 4);
    }

    // an address given relative to the request's own
    function resolve(base, ref) {
        if (/^[a-z][a-z0-9+.-]*:/i.test(ref))
            return ref;
        const origin = (base.match(/^[a-z][a-z0-9+.-]*:\/\/[^/?#]*/i) || [''])[0];
        if (ref.startsWith('//'))
            return base.slice(0, base.indexOf(':') + 1) + ref;
        if (ref.startsWith('/'))
            return origin + ref;
        const plain = base.split('#')[0];
        if (ref.startsWith('#'))
            return plain + ref;
        if (ref.startsWith('?'))
            return plain.split('?')[0] + ref;
        const path = plain.split('?')[0];
        return path.slice(0, path.lastIndexOf('/') + 1) + ref;
    }

    // headers (WebKit's object) with --unset NAME and --header 'NAME: VALUE' (an empty VALUE: off)
    function changedHeaders(original, sets, unsets) {
        const headers = Object.assign({}, original || {});
        const off = name => {
            for (const key of Object.keys(headers)) {
                if (key.toLowerCase() === name.toLowerCase())
                    delete headers[key];
            }
        };
        for (const name of unsets)
            off(name);
        for (const line of sets) {
            const colon = line.indexOf(':');
            const name = colon > 0 ? line.slice(0, colon).trim() : '';
            if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name))
                return 'a header is \'NAME: VALUE\', not ' + JSON.stringify(line);
            const value = line.slice(colon + 1).trim();
            off(name);
            if (value)
                headers[name] = value;
        }
        return headers;
    }

    const REASONS = { 200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 301: 'Moved Permanently', 302: 'Found',
                      303: 'See Other', 304: 'Not Modified', 307: 'Temporary Redirect', 308: 'Permanent Redirect', 400: 'Bad Request',
                      401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 405: 'Method Not Allowed', 409: 'Conflict',
                      410: 'Gone', 418: 'I\'m a teapot', 422: 'Unprocessable Content', 429: 'Too Many Requests',
                      500: 'Internal Server Error', 502: 'Bad Gateway', 503: 'Service Unavailable', 504: 'Gateway Timeout' };
    const ERRORS = { general: 'General', access: 'AccessControl', accesscontrol: 'AccessControl', cors: 'AccessControl',
                     cancel: 'Cancellation', canceled: 'Cancellation', cancellation: 'Cancellation', timeout: 'Timeout' };

    // the type a body answered here has, when nothing says: what the request was for
    function naturalType(r) {
        return { document: 'text/html', stylesheet: 'text/css', script: 'text/javascript', fetch: 'application/json', xhr: 'application/json' }[typeOf(r)] || 'text/plain';
    }

    // what the agent decided for one paused request: {method, params, line, note} or {error}
    function decision1(held, action, o) {
        const r = held.r, id = '#' + r.id, requestId = r.requestId;
        const what = id + ' ' + methodOf(held) + ' ' + shortened(urlOf(held), 150);
        const hasBody = o.body !== undefined || o.body64 !== undefined;
        const content = () => o.body64 !== undefined ? { content: o.body64, base64Encoded: true } : { content: o.body || '', base64Encoded: false };
        const bodySize = () => o.body64 !== undefined ? base64Size(o.body64) : utf8Length(o.body || '');
        const status = o.status !== undefined ? +o.status : undefined;
        if (held.stage === 'request') {
            if (action === 'drop') {
                const errorType = ERRORS[(o.error || 'general').toLowerCase()];
                return { method: 'Network.interceptRequestWithError', params: { requestId, errorType },
                         line: 'dropped ' + what + (errorType === 'Cancellation' ? ' (canceled)' : ' (the page gets a network error' + (errorType !== 'General' ? ': ' + errorType : '') + ')'),
                         note: ['dropped', { error: errorType }] };
            }
            if (action === 'respond') {
                const headers = changedHeaders({}, o.headers, []);
                if (typeof headers === 'string')
                    return { error: id + ': ' + headers };
                const mimeType = o.mime || (header(headers, 'content-type') || '').split(';')[0].trim() || naturalType(r);
                const code = status || 200;
                const params = Object.assign({ requestId, mimeType, status: code, statusText: o.statusText || REASONS[code] || '', headers }, content());
                return { method: 'Network.interceptRequestWithResponse', params,
                         line: 'answered ' + what + ' here: ' + code + (params.statusText ? ' ' + params.statusText : '') + ', ' + size(bodySize()) + ' of ' + mimeType + ' (not sent)',
                         note: ['answered here', { status: code }] };
            }
            if (status !== undefined || o.mime !== undefined || o.statusText !== undefined)
                return { error: id + ': --status and --mime are a response\'s (respond ' + r.id + ' answers it here)' };
            const params = { requestId }, changed = [];
            const request = held.request || {};
            if (o.url !== undefined) {
                params.url = resolve(urlOf(held), o.url);
                changed.push('url');
            }
            if (o.method !== undefined) {
                params.method = o.method.toUpperCase();
                changed.push('method');
            }
            if (o.headers.length || o.unset.length) {
                const headers = changedHeaders(request.headers, o.headers, o.unset);
                if (typeof headers === 'string')
                    return { error: id + ': ' + headers };
                params.headers = headers;
                changed.push('headers');
            }
            if (hasBody) {
                params.postData = o.body64 !== undefined ? o.body64 : toBase64(o.body);
                changed.push('body');
            }
            if (!changed.length)
                return { method: 'Network.interceptContinue', params: { requestId, stage: 'request' }, line: 'sent ' + what, note: ['sent'] };
            // the record says what went
            const har = r.har.request;
            const from = { url: har.url, method: har.method };
            if (params.url) {
                har.url = params.url;
                har.queryString = queryOf(params.url);
            }
            if (params.method)
                har.method = params.method;
            if (params.headers) {
                har.headers = headerList(params.headers);
                har.cookies = requestCookies(params.headers);
            }
            if (hasBody) {
                const text = o.body64 !== undefined ? fromBase64(o.body64) : o.body;
                har.postData = { mimeType: header(params.headers || request.headers, 'content-type') || '', text: text !== null ? text : o.body64 };
                if (text === null)
                    har.postData.encoding = 'base64';
                har.bodySize = bodySize();
            }
            return { method: 'Network.interceptWithRequest', params,
                     line: 'sent ' + id + ' ' + (params.method || methodOf(held)) + ' ' + shortened(params.url || urlOf(held), 150) + ' (changed: ' + changed.join(', ') + ')',
                     note: ['sent changed', { changed, from }] };
        }
        // the response stage
        if (action === 'drop')
            return { error: id + ' waits with its response, and WebKit cannot fail a response: respond ' + r.id + ' --status 403 --body \'\' gives the page another' };
        if (o.url !== undefined || o.method !== undefined)
            return { error: id + ': its request has gone already (it waits with its response): --url and --method do not apply' };
        const response = held.response || {};
        if (action === 'send' && !o.headers.length && !o.unset.length && !hasBody && status === undefined && o.mime === undefined && o.statusText === undefined)
            return { method: 'Network.interceptContinue', params: { requestId, stage: 'response' },
                     line: 'sent ' + what + ' on: the page gets its response (' + response.status + ')', note: ['sent'] };
        const headers = changedHeaders(action === 'send' ? response.headers : {}, o.headers, o.unset);
        if (typeof headers === 'string')
            return { error: id + ': ' + headers };
        const code = status || (action === 'send' ? response.status : 200);
        const mimeType = o.mime || (action === 'send' && response.mimeType) || (header(headers, 'content-type') || '').split(';')[0].trim() || naturalType(r);
        const params = Object.assign({ requestId, mimeType, status: code, statusText: o.statusText || (code === response.status ? response.statusText : REASONS[code]) || '', headers }, content());
        return { method: 'Network.interceptWithResponse', params,
                 line: (action === 'send' ? 'sent ' + what + ' on with its response changed: ' : 'answered ' + what + ' in its response\'s place: ') + code
                     + (params.statusText ? ' ' + params.statusText : '') + ', ' + size(bodySize()) + ' of ' + mimeType + (hasBody ? '' : ' (no body: --body gives one)'),
                 note: [action === 'send' ? 'sent changed' : 'answered here', { status: code }] };
    }

    // send|drop|respond WHICH... [OPTION...]: "P" and the decision's id (decision() answers once
    // WebKit has), or "E" (usage), "N" (not paused) and why
    function decide(action, words) {
        const o = { which: [], headers: [], unset: [] };
        const takes = { '--url': 'url', '--method': 'method', '--body': 'body', '--body64': 'body64', '--status': 'status',
                        '--status-text': 'statusText', '--mime': 'mime', '--error': 'error' };
        const allowed = { send: ['url', 'method', 'body', 'body64', 'status', 'statusText', 'mime', 'headers', 'unset'],
                          respond: ['body', 'body64', 'status', 'statusText', 'mime', 'headers'], drop: ['error'] }[action];
        const given = new Set();
        for (let i = 0; i < words.length; i++) {
            const w = words[i], more = i + 1 < words.length;
            if ((w === '--header' || w === '-H') && more)
                o.headers.push(words[++i]), given.add('headers');
            else if (w === '--unset' && more)
                o.unset.push(words[++i]), given.add('unset');
            else if (takes[w] && more)
                o[takes[w]] = words[++i], given.add(takes[w]);
            else if (w.startsWith('-'))
                return 'E' + 'unknown option ' + w + ' for ' + action + ' (web intercept show N says what each takes)';
            else
                o.which.push(w);
        }
        for (const name of given) {
            if (!allowed.includes(name))
                return 'E' + action + ' takes no --' + (name === 'body64' ? 'body-file' : name.replace(/[A-Z]/g, c => '-' + c.toLowerCase())) + ' (' + action + ' '
                    + { send: '[--url U] [--method M] [--header \'NAME: VALUE\'] [--unset NAME] [--body TEXT]',
                        respond: '[--status S] [--header \'NAME: VALUE\'] [--mime TYPE] [--body TEXT]', drop: '[--error general|access|cancel|timeout]' }[action] + ')';
        }
        if (o.body !== undefined && o.body64 !== undefined)
            return 'E' + 'one body: --body or --body-file';
        if (o.status !== undefined && !(/^\d{3}$/.test(o.status) && +o.status >= 100 && +o.status <= 599))
            return 'E' + '--status is an HTTP status (100 to 599), not ' + o.status;
        if (o.error !== undefined && !ERRORS[o.error.toLowerCase()])
            return 'E' + '--error is general (the default), access, cancel or timeout, not ' + o.error;
        if (!o.which.length)
            return 'E' + action + ' which? N (a number web intercept lists), all, or a part of the address';
        const chosen = new Set();
        for (const which of o.which) {
            const found = pausedOf(which);
            if (!found.length) {
                if (which === 'all')
                    return 'N' + 'no request is paused' + (intercept ? '' : ' (intercept: off)');
                return 'N' + notPaused(which);
            }
            found.forEach(held => chosen.add(held));
        }
        const id = nextDecision++;
        const d = { lines: [], waiting: 0, failed: false };
        for (const held of chosen) {
            const step = decision1(held, action, o);
            if (step.error) {
                d.lines.push(step.error);
                d.failed = true;
                continue;
            }
            const at = d.lines.push(step.line) - 1;
            unhold(held.r);
            const n = note(held.r, held.stage, step.note[0], step.note[1]);
            if (action === 'drop')
                held.r.dropped = true;
            d.waiting++;
            send(held.channel, step.method, step.params, (result, error) => {
                d.waiting--;
                if (error) {
                    d.failed = true;
                    d.lines[at] += ': WebKit says ' + (error.message || 'no');
                    n.failed = error.message || 'no';
                }
            });
        }
        decisions.set(id, d);
        return 'P' + id;
    }

    // ── helpers ──────────────────────────────────────────────────────────────────────────────

    function round(n) {
        return Math.round(n * 1000) / 1000;
    }

    function iso(seconds) {
        return new Date(seconds * 1000).toISOString();
    }

    function utf8Length(text) {
        let n = 0;
        for (let i = 0; i < text.length; i++) {
            const c = text.charCodeAt(i);
            n += c < 0x80 ? 1 : c < 0x800 ? 2 : (c >= 0xd800 && c < 0xdc00) ? (i++, 4) : 3;
        }
        return n;
    }

    // WebKit's headers: an object; a header given more than once has its values on lines
    function headerList(headers) {
        const list = [];
        for (const name in headers || {}) {
            for (const value of String(headers[name]).split('\n'))
                list.push({ name, value });
        }
        return list;
    }

    function header(headers, name) {
        for (const key in headers || {}) {
            if (key.toLowerCase() === name)
                return String(headers[key]);
        }
        return undefined;
    }

    function harHeader(list, name) {
        const found = (list || []).find(h => h.name.toLowerCase() === name);
        return found ? found.value : undefined;
    }

    function decode(text) {
        try {
            return decodeURIComponent(text.replace(/\+/g, ' '));
        } catch (e) {
            return text;
        }
    }

    function parameters(text) {
        return text.split('&').filter(Boolean).map(pair => {
            const eq = pair.indexOf('=');
            return eq < 0 ? { name: decode(pair), value: '' } : { name: decode(pair.slice(0, eq)), value: decode(pair.slice(eq + 1)) };
        });
    }

    function queryOf(url) {
        const q = (url || '').indexOf('?');
        return q < 0 ? [] : parameters(url.slice(q + 1).split('#')[0]);
    }

    function requestCookies(headers) {
        const cookie = header(headers, 'cookie');
        if (!cookie)
            return [];
        return cookie.split(/;\s*/).filter(Boolean).map(pair => {
            const eq = pair.indexOf('=');
            return eq < 0 ? { name: pair, value: '' } : { name: pair.slice(0, eq), value: pair.slice(eq + 1) };
        });
    }

    function responseCookies(headers) {
        const lines = header(headers, 'set-cookie');
        if (!lines)
            return [];
        return lines.split('\n').filter(Boolean).map(line => {
            const [pair, ...attributes] = line.split(/;\s*/);
            const eq = pair.indexOf('=');
            const cookie = { name: eq < 0 ? pair : pair.slice(0, eq), value: eq < 0 ? '' : pair.slice(eq + 1) };
            for (const attribute of attributes) {
                const [key, ...rest] = attribute.split('=');
                const value = rest.join('=');
                switch (key.toLowerCase()) {
                case 'path': cookie.path = value; break;
                case 'domain': cookie.domain = value; break;
                case 'expires': {
                    const date = new Date(value);
                    if (!isNaN(date))
                        cookie.expires = date.toISOString();
                    break;
                }
                case 'max-age': cookie.expires = cookie.expires || new Date(Date.now() + 1000 * +value).toISOString(); break;
                case 'httponly': cookie.httpOnly = true; break;
                case 'secure': cookie.secure = true; break;
                case 'samesite': cookie.sameSite = value; break;
                }
            }
            return cookie;
        });
    }

    function httpVersion(protocol) {
        const p = (protocol || '').toLowerCase();
        return p === 'h2' || p === 'http/2' ? 'HTTP/2' : p === 'h3' || p.startsWith('h3-') ? 'HTTP/3'
            : p.startsWith('http/') ? p.toUpperCase() : protocol || '';
    }

    function resourceType(type) {
        const names = { StyleSheet: 'stylesheet', XHR: 'xhr', EventSource: 'eventsource', WebSocket: 'websocket' };
        return names[type] || String(type || 'other').toLowerCase();
    }

    function initiatorOf(initiator) {
        const result = { type: initiator.type };
        if (initiator.url)
            result.url = initiator.url;
        if (initiator.lineNumber !== undefined)
            result.lineNumber = initiator.lineNumber;
        const frames = initiator.stackTrace && (initiator.stackTrace.callFrames || initiator.stackTrace);
        if (Array.isArray(frames) && frames.length)
            result.stack = { callFrames: frames.slice(0, 20).map(f => ({ functionName: f.functionName || '', url: f.url || '', lineNumber: f.lineNumber, columnNumber: f.columnNumber })) };
        return result;
    }

    function size(n) {
        if (!(n >= 0))
            return '';
        return n < 1000 ? n + ' B' : n < 1000 * 1000 ? (n / 1000).toFixed(1) + ' kB' : (n / 1000 / 1000).toFixed(1) + ' MB';
    }

    function duration(ms) {
        if (!(ms >= 0))
            return '';
        return ms < 10 ? ms.toFixed(1) + ' ms' : ms < 10000 ? Math.round(ms) + ' ms' : (ms / 1000).toFixed(1) + ' s';
    }

    function pad(text, width, right) {
        text = String(text);
        return right ? text.padStart(width) : text.padEnd(width);
    }

    // ── listings: of the recording, and of a HAR file (`web har view`) ───────────────────────

    // a HAR file's entries as records
    function recordsOf(har) {
        const entries = (har && har.log && har.log.entries) || [];
        return entries.map((e, i) => ({ id: i + 1, har: e, done: true, seq: i + 1, type: e._resourceType || '', viewed: true }));
    }

    function typeOf(r) {
        const har = r.har;
        if (har._resourceType)
            return har._resourceType;
        const m = (har.response && har.response.content && har.response.content.mimeType) || '';
        return /html/.test(m) ? 'document' : /css/.test(m) ? 'stylesheet' : /javascript|ecmascript/.test(m) ? 'script'
            : /^image\//.test(m) ? 'image' : /font/.test(m) ? 'font' : /json|xml/.test(m) ? 'fetch' : 'other';
    }

    function transferred(r) {
        const response = r.har.response || {};
        if (response._transferSize >= 0)
            return response._transferSize;
        if (r.har._webSocketMessages)
            return r.frameBytes !== undefined ? r.frameBytes : r.har._webSocketMessages.reduce((n, m) => n + (m.data || '').length, 0);
        if (response.headersSize >= 0 && response.bodySize >= 0)
            return response.headersSize + response.bodySize;
        return response.bodySize >= 0 ? response.bodySize : (response.content && response.content.size) || 0;
    }

    function statusOf(r) {
        const response = r.har.response || {};
        if (r.paused)
            return 'paused';
        if ((!r.done || r.har._pending) && !r.har._webSocketMessages)
            return '...';
        if (response._error && !response.status)
            return r.canceled ? 'cancel' : 'failed';
        return response.status ? String(response.status) : response._error ? 'failed' : '-';
    }

    // a data: URL is the data itself: its start says enough
    function shortened(url, max) {
        return url.length > max && (url.startsWith('data:') || max < 200) ? url.slice(0, max) + '...' : url;
    }

    function row(r) {
        const har = r.har, response = har.response || {};
        const url = shortened((har.request && har.request.url) || '', 100);
        let note = '';
        if (response.redirectURL)
            note = '  -> ' + response.redirectURL;
        if (response._error)
            note += '  (' + response._error + ')';
        if (har._webSocketMessages)
            note += '  (' + (r.frames !== undefined ? r.frames : har._webSocketMessages.length) + ' messages' + (r.done ? '' : ', open') + ')';
        // what the agent made of it (web intercept): a dropped one says so as its error
        const changed = (har._intercepted || []).filter(n => !n.failed && (n.action === 'sent changed' || n.action === 'answered here'));
        if (changed.length)
            note += '  (' + changed.map(n => n.stage + ' ' + (n.action === 'answered here' ? 'answered here' : 'changed')).join(', ') + ')';
        const cache = har._fromCache || response._fromCache;
        const time = r.done || (har._webSocketMessages && har.response.status) ? duration(har.time) : '';
        return pad(r.id, 4, true) + '  ' + pad(statusOf(r), 6) + ' ' + pad((har.request && har.request.method) || '', 7) + ' '
            + pad(typeOf(r), 11) + ' ' + pad(cache && !transferred(r) ? '(' + cache + ')' : size(transferred(r)), 9, true) + ' '
            + pad(time, 8, true) + '  ' + url + note;
    }

    const HEAD = '   #  status method  type             size     time  url';

    // [TEXT] [--type T] [--status S] [--failed] [--last N | --all]: TEXT a part of the address
    function filtered(list, words) {
        let text = null, type = null, status = null, failed = false, last = LISTED, all = false;
        for (let i = 0; i < words.length; i++) {
            const w = words[i];
            if (w === '--type' && i + 1 < words.length)
                type = words[++i].toLowerCase();
            else if (w === '--status' && i + 1 < words.length)
                status = words[++i].toLowerCase();
            else if (w === '--failed')
                failed = true;
            else if (w === '--last' && i + 1 < words.length)
                last = Math.max(1, parseInt(words[++i], 10) || LISTED);
            else if (w === '--all')
                all = true;
            else if (text === null)
                text = w;
            else
                text += ' ' + w;
        }
        const typeTest = type ? new RegExp('^(' + (TYPE_NAMES[type] || type) + ')$') : null;
        const matches = list.filter(r => {
            const har = r.har, response = har.response || {};
            if (text && !((har.request && har.request.url) || '').includes(text))
                return false;
            if (typeTest && !typeTest.test(typeOf(r)))
                return false;
            if (failed && !response._error && !(response.status >= 400))
                return false;
            if (status) {
                const s = statusOf(r);
                if (/^\dxx$/.test(status) ? s[0] !== status[0] || s.length !== 3 : s !== status)
                    return false;
            }
            return true;
        });
        return { matches, from: all || matches.length <= last ? 0 : matches.length - last, text, words };
    }

    function listing(list, words, title) {
        const { matches, from } = filtered(list, words);
        if (!list.length)
            return title + 'nothing recorded yet\n';
        if (!matches.length)
            return title + 'no request matches (of ' + list.length + ')\n';
        const lines = [title + HEAD];
        if (from)
            lines.push('   (' + from + ' earlier: --all)');
        for (const r of matches.slice(from))
            lines.push(row(r));
        let bytes = 0, resources = 0, failures = 0;
        for (const r of matches) {
            bytes += transferred(r);
            resources += (r.har.response && r.har.response.content && r.har.response.content.size) || 0;
            if (statusOf(r) === 'failed' || r.har.response.status >= 400)
                failures++;
        }
        lines.push(matches.length + ' request' + (matches.length === 1 ? '' : 's') + ', ' + size(bytes) + ' transferred, '
            + size(resources) + ' of content' + (failures ? ', ' + failures + ' failed' : '')
            + (title ? ' (web har view FILE N: one in detail)' : ' (web network show N: one in detail; web network export N: as curl...)'));
        return lines.join('\n') + '\n';
    }

    // ── search: every request's address, headers, bodies and messages (as the Network panel's) ──

    // base64 of UTF-8 text (a text body some browsers keep so) as text; null if it is not
    function fromBase64(text) {
        const digits = {};
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'.split('').forEach((c, i) => { digits[c] = i; });
        let bits = 0, n = 0;
        const bytes = [];
        for (let i = 0; i < text.length; i++) {
            const v = digits[text[i]];
            if (v === undefined)
                continue;
            bits = ((bits << 6) | v) & 0xffffff;
            n += 6;
            if (n >= 8) {
                n -= 8;
                bytes.push(String.fromCharCode((bits >> n) & 255));
            }
        }
        try {
            return decodeURIComponent(escape(bytes.join('')));
        } catch (e) {
            return null;
        }
    }

    // what can be searched in a request: [where, text, by lines]
    function searchable(r, parts) {
        const har = r.har, request = har.request || {}, response = har.response || {}, content = response.content || {};
        const fields = [];
        if (parts.url)
            fields.push(['url', request.url || '', false]);
        if (parts.headers) {
            for (const h of request.headers || [])
                fields.push(['request header', h.name + ': ' + h.value, false]);
            for (const h of response.headers || [])
                fields.push(['response header', h.name + ': ' + h.value, false]);
        }
        if (parts.body) {
            if (request.postData && request.postData.text)
                fields.push(['request body', request.postData.text, true]);
            if (content.text !== undefined) {
                const text = content.encoding !== 'base64' ? content.text : textual(content.mimeType, r.type) ? fromBase64(content.text) : null;
                if (text !== null)
                    fields.push(['response body', text, true]);
            }
        }
        if (parts.messages && har._webSocketMessages) {
            har._webSocketMessages.forEach((m, i) => {
                if (m.opcode === 1)
                    fields.push([(m.type === 'send' ? 'sent' : 'received') + ' message ' + (i + 1), m.data || '', false]);
            });
        }
        return fields;
    }

    // a text body that would have been searched, had it been kept
    function unsearched(r) {
        const response = r.har.response || {}, content = response.content || {};
        return r.done && !r.har._webSocketMessages && !response._error && content.text === undefined && content.size > 0
            && !(response.status >= 300 && response.status < 400) && textual(content.mimeType, r.type);
    }

    // the line around a match, a long one (minified code) cut to the part around it
    function excerpt(line, index, length) {
        const from = Math.max(0, index - 60), to = Math.min(line.length, index + length + 60);
        return (from ? '...' : '') + line.slice(from, to).replace(/\s+/g, ' ').trim() + (to < line.length ? '...' : '');
    }

    const PER_REQUEST = 5;

    // PATTERN [--case] [--regex] [--in url,headers,body,messages] [--url TEXT] [--type T] [--status S]
    // [--failed] [--max N]: "T" and the matches by request, "N" and why none, "E" and what is wrong;
    // viewing: in a HAR file
    function searchIn(list, pattern, words, viewing) {
        let flags = 'i', regex = false, max = 200, where = 'url,headers,body,messages';
        const filter = ['--all'];
        for (let i = 0; i < words.length; i++) {
            const w = words[i], value = i + 1 < words.length ? words[i + 1] : undefined;
            if (w === '--case')
                flags = '';
            else if (w === '--regex')
                regex = true;
            else if (w === '--max' && value !== undefined)
                max = Math.max(1, parseInt(words[++i], 10) || max);
            else if (w === '--in' && value !== undefined)
                where = words[++i];
            else if (w === '--url' && value !== undefined)
                filter.push(words[++i]);
            else if ((w === '--type' || w === '--status') && value !== undefined)
                filter.push(w, words[++i]);
            else if (w === '--failed')
                filter.push(w);
            else
                return 'E' + 'unknown option ' + w + ' (--case, --regex, --in url,headers,body,messages, --url TEXT, --type T, --status S, --failed, --max N)';
        }
        if (!pattern)
            return 'E' + 'nothing to search for';
        let re;
        try {
            re = new RegExp(regex ? pattern : pattern.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g' + flags);
        } catch (e) {
            return 'E' + 'not a regular expression: ' + e.message;
        }
        const names = { url: 'url', urls: 'url', header: 'headers', headers: 'headers', body: 'body', bodies: 'body',
                        message: 'messages', messages: 'messages', ws: 'messages' };
        const parts = {};
        for (const part of where.split(/[,\s]+/).filter(Boolean)) {
            if (!names[part.toLowerCase()])
                return 'E' + '--in takes url, headers, body, messages (not ' + part + ')';
            parts[names[part.toLowerCase()]] = true;
        }
        const { matches: requests } = filtered(list, filter);
        const lines = [];
        let found = 0, inRequests = 0, shown = 0, notSearched = 0;
        for (const r of requests) {
            if (parts.body && unsearched(r))
                notSearched++;
            const hits = [];
            let count = 0;
            for (const [label, text, byLines] of searchable(r, parts)) {
                re.lastIndex = 0;
                let line = 1, counted = 0, m;
                while ((m = re.exec(text)) && count < 10000) {
                    if (!m[0].length) {
                        re.lastIndex++;
                        continue;
                    }
                    count++;
                    if (hits.length >= PER_REQUEST)
                        continue;
                    for (let nl = text.indexOf('\n', counted); nl >= 0 && nl < m.index; nl = text.indexOf('\n', nl + 1)) {
                        line++;
                        counted = nl + 1;
                    }
                    const start = text.lastIndexOf('\n', m.index - 1) + 1;
                    const stop = text.indexOf('\n', m.index) < 0 ? text.length : text.indexOf('\n', m.index);
                    const at = byLines ? ' ' + line + ':' + (m.index - start + 1) : '';
                    hits.push('       ' + label + at + '  ' + excerpt(text.slice(start, stop), m.index - start, m[0].length));
                }
            }
            if (!count)
                continue;
            found += count;
            inRequests++;
            if (shown >= max)
                continue;
            lines.push(row(r), ...hits.slice(0, max - shown));
            shown += Math.min(hits.length, max - shown);
            if (count > hits.length)
                lines.push('       (and ' + (count - hits.length) + ' more in it)');
        }
        const kept = notSearched ? '; ' + notSearched + ' text bod' + (notSearched === 1 ? 'y' : 'ies') + (viewing ? ' not in the file' : ' not kept')
            + ', not searched' : '';
        if (!found)
            return 'N' + 'no match for ' + JSON.stringify(pattern) + ' in ' + requests.length + ' request' + (requests.length === 1 ? '' : 's') + kept + '\n';
        if (shown >= max)
            lines.push('   (more not shown: --max N)');
        lines.push(found + ' match' + (found === 1 ? '' : 'es') + ' in ' + inRequests + ' request' + (inRequests === 1 ? '' : 's')
            + ' (of ' + requests.length + ')' + kept + (viewing ? ' (web har view FILE N)' : ' (web network show N, web network body N)'));
        return 'T' + lines.join('\n') + '\n';
    }

    // a record: by its number, else the last whose address has the text
    function pick(list, which) {
        if (/^\d+$/.test(which))
            return list.find(r => r.id === +which) || null;
        for (let i = list.length - 1; i >= 0; i--) {
            if (((list[i].har.request && list[i].har.request.url) || '').includes(which))
                return list[i];
        }
        return null;
    }

    function interceptedText(n) {
        let text = n.stage + ' ' + n.action;
        if (n.changed)
            text += ' (' + n.changed.join(', ') + (n.from && n.changed.includes('url') ? '; it was ' + n.from.method + ' ' + shortened(n.from.url, 150) : '') + ')';
        else if (n.status)
            text += ' (' + n.status + ')';
        else if (n.error && n.error !== 'General')
            text += ' (' + n.error + ')';
        return text + (n.failed ? ' (not done: ' + n.failed + ')' : '');
    }

    function headerLines(list) {
        return (list || []).map(h => '  ' + h.name + ': ' + h.value);
    }

    function preview(text, max) {
        return text.length > max ? text.slice(0, max) + '\n  ... (' + size(text.length) + ' in all: web network body N)' : text;
    }

    function detail(r, viewing) {
        const har = r.har, request = har.request || {}, response = har.response || {}, timings = har.timings || {};
        const lines = ['#' + r.id + ' ' + request.method + ' ' + request.url];
        const status = response.status ? response.status + (response.statusText ? ' ' + response.statusText : '') : statusOf(r);
        lines.push('status: ' + status + (response.httpVersion ? '   ' + response.httpVersion : '') + '   type: ' + typeOf(r)
            + (har._fromCache ? '   from the ' + har._fromCache + ' cache' : '') + (har.pageref ? '   page: ' + har.pageref : ''));
        if (response._error)
            lines.push('error: ' + response._error);
        if (r.paused)
            lines.push('paused ' + (r.paused === 'request' ? 'before it goes' : 'with its response') + ', waiting for the agent (web intercept show ' + r.id + ')');
        if (har._intercepted)
            lines.push('intercepted: ' + har._intercepted.map(interceptedText).join('; '));
        if (response.redirectURL)
            lines.push('redirected to: ' + response.redirectURL);
        if (har.serverIPAddress)
            lines.push('remote address: ' + har.serverIPAddress + (har._serverPort ? ':' + har._serverPort : '')
                + (har.connection ? '   connection: ' + har.connection : '') + (har._priority ? '   priority: ' + har._priority : '')
                + (har._tls && har._tls.protocol ? '   ' + har._tls.protocol + (har._tls.cipher ? ' ' + har._tls.cipher : '') : ''));
        const parts = ['blocked', 'dns', 'connect', 'ssl', 'send', 'wait', 'receive'].filter(k => timings[k] >= 0 && (timings[k] > 0 || k === 'wait'))
            .map(k => k + ' ' + duration(timings[k]));
        lines.push('started: ' + (har.startedDateTime || '?') + (r.done && !har._pending ? '   time: ' + duration(har.time) + (parts.length ? ' (' + parts.join(', ') + ')' : '') : '   (still loading)'));
        if (har._initiator) {
            // the script's frame the request came from (not fetch's own, [native code])
            const i = har._initiator, top = i.stack && i.stack.callFrames && (i.stack.callFrames.find(f => f.url && f.url !== '[native code]') || i.stack.callFrames[0]);
            const where = i.url ? shortened(i.url, 150) + (i.lineNumber !== undefined ? ':' + i.lineNumber : '')
                : top ? (top.functionName || '(anonymous)') + ' ' + shortened(top.url, 150) + ':' + top.lineNumber : '';
            lines.push('initiator: ' + i.type + (where ? ' ' + where : ''));
        }
        const content = response.content || {};
        lines.push('size: ' + size(transferred(r)) + ' transferred' + (response.headersSize >= 0 ? ' (headers ' + size(response.headersSize) + ', body ' + size(Math.max(0, response.bodySize)) + ')' : '')
            + ', ' + size(content.size || 0) + ' of content (' + content.mimeType + ')');
        lines.push('', 'request headers' + (request.headersSize >= 0 ? ' (as sent, ' + size(request.headersSize) + ')' : '') + ':');
        lines.push(...(request.headers && request.headers.length ? headerLines(request.headers) : ['  (none known)']));
        if (request.queryString && request.queryString.length)
            lines.push('', 'query:', ...request.queryString.map(q => '  ' + q.name + ' = ' + q.value));
        if (request.postData)
            lines.push('', 'request body (' + (request.postData.mimeType || 'no type') + ', ' + size(utf8Length(request.postData.text || '')) + '):',
                preview(request.postData.text || '', 4000));
        lines.push('', 'response headers:');
        lines.push(...(response.headers && response.headers.length ? headerLines(response.headers) : ['  (none)']));
        if (har._webSocketMessages) {
            const messages = har._webSocketMessages;
            lines.push('', 'messages (' + (r.frames !== undefined ? r.frames : messages.length) + (messages.length >= MAX_FRAMES ? ', the first ' + MAX_FRAMES + ' kept' : '') + '):');
            const control = { 8: '(close) ', 9: '(ping) ', 10: '(pong) ', 2: '(binary, base64) ' };
            for (const m of messages.slice(-30))
                lines.push('  ' + (m.type === 'send' ? '>' : '<') + ' ' + new Date(m.time * 1000).toISOString().slice(11, 23) + '  '
                    + (control[m.opcode] || '') + (m.data.length > 200 ? m.data.slice(0, 200) + '...' : m.data));
            if (messages.length > 30)
                lines.push('  (the last 30 of them)');
        } else if (content.text !== undefined) {
            lines.push('', 'response body' + (content.encoding === 'base64' ? ' (base64)' : '') + ':');
            lines.push(content.encoding === 'base64' ? '  (' + size(content.size) + ' of ' + content.mimeType + ': web network body N FILE)' : preview(content.text, 4000));
        } else if (!viewing)
            lines.push('', 'response body: ' + (r.bodyError || (r.done ? 'not kept (web network body N reads it' + (bodies !== 'all' ? '; devtools on --bodies all keeps them all' : '') + ')' : 'still loading')));
        return lines.join('\n') + '\n';
    }

    // a HAR file's text: the archive, or "E" and why it is not one
    function parseHar(text) {
        let har;
        try {
            har = JSON.parse(text);
        } catch (e) {
            return 'E' + 'not JSON: ' + e.message;
        }
        if (!har || !har.log || !Array.isArray(har.log.entries))
            return 'E' + 'not a HAR file (no log.entries)';
        return har;
    }

    // a HAR of records: of the recording (its pages), or of a HAR file (log: its own pages and creator)
    function archive(list, withBodies, pageList, log) {
        const entries = list.map(r => {
            let entry = r.har;
            if (!withBodies && entry.response && entry.response.content && entry.response.content.text !== undefined) {
                const content = Object.assign({}, entry.response.content);
                delete content.text;
                delete content.encoding;
                entry = Object.assign({}, entry, { response: Object.assign({}, entry.response, { content }) });
            }
            // still loading when the archive was made (a WebSocket: still open)
            return r.done ? entry : Object.assign({}, entry, { _pending: true });
        });
        const referenced = new Set(entries.map(e => e.pageref));
        return JSON.stringify({
            log: {
                version: '1.2',
                creator: (log && log.creator) || { name: 'collabo wpe-browser (WebKit)', version },
                browser: (log && log.browser) || { name: 'WebKit', version },
                pages: (pageList || []).filter(p => referenced.has(p.id))
                    .map(p => ({ startedDateTime: p.startedDateTime, id: p.id, title: p.title, pageTimings: p.pageTimings })),
                entries,
            },
        }, null, 1);
    }

    // ── export: requests as commands and files of other tools (web network export) ───────────
    // Each request as it went (the headers the network process sent, cookies too; its body when
    // WebKit gave it as text), for curl, a browser's console (fetch), Node.js, Python, the .http
    // files of REST clients (VS Code's REST Client, JetBrains' HTTP Client), the HTTP message
    // itself (Burp's Repeater, nc), or a HAR of just those.

    const FORMATS = {
        curl: { script: '#!/bin/sh', comment: '#' },
        fetch: { comment: '//' },
        node: { script: '#!/usr/bin/env node', comment: '//' },
        python: { script: '#!/usr/bin/env python3', comment: '#' },
        http: { comment: '###' },
        raw: {},
        har: {},
    };
    const FORMAT_NAMES = { sh: 'curl', bash: 'curl', js: 'fetch', javascript: 'fetch', nodejs: 'node', py: 'python', python3: 'python',
                           rest: 'http', jetbrains: 'http', vscode: 'http', burp: 'raw', message: 'raw' };
    // hop-by-hop headers and what the tools set themselves
    const OWN_HEADERS = /^(host|content-length|connection|keep-alive|proxy-connection|transfer-encoding|upgrade|te|trailer|expect)$/i;
    // what a page's fetch may not set (the browser does: the Fetch standard's forbidden headers)
    const FORBIDDEN = /^(accept-charset|accept-encoding|access-control-request-headers|access-control-request-method|connection|content-length|cookie2?|date|dnt|expect|host|keep-alive|origin|referer|te|trailer|transfer-encoding|upgrade|via|user-agent)$|^(proxy-|sec-)/i;

    // the request of a record as it went: {method, url, headers [{name, value}], body (text; undefined: none, null: not known)}
    function requestOf(r, noCookies) {
        const request = r.har.request || {};
        const headers = (request.headers || []).filter(h => h.name && !h.name.startsWith(':') && !(noCookies && /^cookie2?$/i.test(h.name)));
        const post = request.postData;
        let body = post && post.text !== undefined && post.encoding !== 'base64' ? post.text : undefined;
        if (body === undefined && (request.bodySize > 0 || (post && post.text !== undefined)))
            body = null;
        return { method: (request.method || 'GET').toUpperCase(), url: request.url || '', headers, body, version: request.httpVersion || '' };
    }

    function label(r) {
        return '#' + r.id + ' ' + ((r.har.request && r.har.request.method) || 'GET') + ' ' + ((r.har.request && r.har.request.url) || '')
            + ' (' + statusOf(r) + ', ' + typeOf(r) + ')';
    }

    function shQuote(text) {
        return "'" + String(text).replace(/'/g, "'\\''") + "'";
    }

    // the values of a header given more than once, as one (cookies with "; ")
    function joined(headers, skip) {
        const out = {};
        const names = {};
        for (const h of headers) {
            if (skip && skip.test(h.name))
                continue;
            const key = h.name.toLowerCase();
            if (names[key] === undefined) {
                names[key] = h.name;
                out[h.name] = h.value;
            } else
                out[names[key]] += (key === 'cookie' ? '; ' : ', ') + h.value;
        }
        return out;
    }

    function asCurl(q) {
        const parts = ['curl ' + shQuote(q.url)];
        if (/[[\]{}]/.test(q.url))
            parts.push('--globoff'); // not curl's ranges
        const hasBody = typeof q.body === 'string';
        if (q.method === 'HEAD' && !hasBody)
            parts.push('--head');
        else if (hasBody ? q.method !== 'POST' : q.method !== 'GET')
            parts.push('-X ' + shQuote(q.method));
        let compressed = false;
        for (const h of q.headers) {
            if (/^accept-encoding$/i.test(h.name))
                compressed = /gzip|deflate|br|zstd/i.test(h.value); // curl's own, and it decodes what comes
            else if (!OWN_HEADERS.test(h.name))
                parts.push('-H ' + shQuote(h.value.trim() ? h.name + ': ' + h.value : h.name + ';')); // "NAME;": sent empty
        }
        if (hasBody)
            parts.push('--data-raw ' + shQuote(q.body));
        if (compressed)
            parts.push('--compressed');
        return parts.join(' \\\n  ') + '\n';
    }

    // as the Network panel's "Copy as fetch": what a page's script can set, the cookies as credentials
    function asFetch(q) {
        const init = { headers: {} };
        let cookies = false, referrer, mode = 'cors';
        for (const h of q.headers) {
            if (/^cookie$/i.test(h.name))
                cookies = true;
            else if (/^referer$/i.test(h.name))
                referrer = h.value;
            else if (/^sec-fetch-mode$/i.test(h.name) && ['cors', 'no-cors', 'same-origin'].includes(h.value))
                mode = h.value;
            else if (!FORBIDDEN.test(h.name))
                init.headers[h.name.toLowerCase()] = init.headers[h.name.toLowerCase()] ? init.headers[h.name.toLowerCase()] + ', ' + h.value : h.value;
        }
        if (referrer)
            init.referrer = referrer;
        if (typeof q.body === 'string')
            init.body = q.body;
        init.method = q.method;
        init.mode = mode;
        init.credentials = cookies ? 'include' : 'omit';
        return 'fetch(' + JSON.stringify(q.url) + ', ' + JSON.stringify(init, null, 2) + ');\n';
    }

    // Node.js's fetch: every header the request had (cookies too) but those the client sets itself
    function asNode(q) {
        const init = { method: q.method, headers: joined(q.headers, /^(accept-encoding|host|content-length|connection|keep-alive|proxy-connection|transfer-encoding|upgrade|te|trailer|expect)$/i) };
        if (typeof q.body === 'string')
            init.body = q.body;
        return '  response = await fetch(' + JSON.stringify(q.url) + ', ' + JSON.stringify(init, null, 2).replace(/\n/g, '\n  ') + ');\n'
            + '  console.log(response.status, response.statusText);\n'
            + '  console.log(await response.text());\n';
    }

    // Python's standard library (urllib: no pip), which does not decode compressed bodies: no Accept-Encoding
    function asPython(q) {
        const headers = joined(q.headers, /^(accept-encoding|host|content-length|connection|keep-alive|proxy-connection|transfer-encoding|upgrade|te|trailer|expect)$/i);
        const lines = ['send(urllib.request.Request(', '    ' + JSON.stringify(q.url) + ',', '    method=' + JSON.stringify(q.method) + ','];
        const names = Object.keys(headers);
        if (names.length)
            lines.push('    headers={', ...names.map(n => '        ' + JSON.stringify(n) + ': ' + JSON.stringify(headers[n]) + ','), '    },');
        if (typeof q.body === 'string')
            lines.push('    data=' + JSON.stringify(q.body) + '.encode(),');
        lines.push('))');
        return lines.join('\n') + '\n';
    }

    const PYTHON_HEAD = ['import urllib.error', 'import urllib.request', '', '',
        'def send(request):',
        '    try:',
        '        response = urllib.request.urlopen(request)',
        '    except urllib.error.HTTPError as error:',
        '        response = error  # a 4xx or 5xx: its answer all the same',
        '    with response:',
        '        print(response.status, response.reason)',
        '        print(response.read().decode("utf-8", "replace"))', '', '', ''].join('\n');

    // a REST client's .http request: the method, the address, the headers, a blank line, the body
    function asHttp(q) {
        const version = /^HTTP\/(1\.[01]|2)$/.test(q.version) ? ' ' + q.version : '';
        const lines = [q.method + ' ' + q.url + version];
        for (const h of q.headers) {
            if (!OWN_HEADERS.test(h.name))
                lines.push(h.name + ': ' + h.value);
        }
        return lines.join('\n') + '\n' + (typeof q.body === 'string' ? '\n' + q.body + '\n' : '');
    }

    // the HTTP/1.1 message as it went over the wire (CRLF): its request line, every header, its body
    function asRaw(q) {
        const at = q.url.match(/^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)([^#]*)/i);
        const target = at ? (at[2] || '/') : q.url;
        const version = /^HTTP\/1\.[01]$/.test(q.version) ? q.version : 'HTTP/1.1';
        const lines = [q.method + ' ' + target + ' ' + version];
        if (at && !q.headers.some(h => /^host$/i.test(h.name)))
            lines.push('Host: ' + at[1]);
        for (const h of q.headers)
            lines.push(h.name + ': ' + h.value);
        const hasBody = typeof q.body === 'string';
        if (hasBody && !q.headers.some(h => /^content-length$/i.test(h.name)))
            lines.push('Content-Length: ' + utf8Length(q.body));
        return lines.join('\r\n') + '\r\n\r\n' + (hasBody ? q.body : '');
    }

    // WHICH... [--as FORMAT] [--no-cookies] [--type T] [--status S] [--failed]: "T" and "COUNT FORMAT
    // NOTES\n", the #! line a file of it starts with (or nothing) and "\n", NOTES lines of what is
    // not in it, the text; "E" (usage), "N" (none) and why. viewing: the records are a HAR file's (its log)
    function exported(list, words, viewing) {
        const which = [], filter = ['--all'];
        let format = 'curl', noCookies = false;
        for (let i = 0; i < words.length; i++) {
            const w = words[i], more = i + 1 < words.length;
            if (w === '--as' && more)
                format = words[++i].toLowerCase();
            else if (w.startsWith('--as='))
                format = w.slice(5).toLowerCase();
            else if (w === '--no-cookies')
                noCookies = true;
            else if ((w === '--type' || w === '--status') && more)
                filter.push(w, words[++i]);
            else if (w === '--failed')
                filter.push(w);
            else if (w.startsWith('--'))
                return 'E' + 'unknown option ' + w + ' (export WHICH... [--as FORMAT] [-o FILE] [--no-cookies] [--type T] [--status S] [--failed])';
            else
                which.push(w);
        }
        format = FORMAT_NAMES[format] || format;
        if (!FORMATS[format])
            return 'E' + 'export --as ' + format + '? curl (the default), fetch, node, python, http (REST clients\' .http), raw (the HTTP message), har';
        if (!which.length)
            return 'E' + 'export which? N (a number web network lists), N-M, all, or a part of the address (the last request that has it)';
        const chosen = new Set();
        for (const w of which) {
            const range = w.match(/^#?(\d+)-(\d+)$/);
            const found = w === 'all' ? list : range ? list.filter(r => r.id >= +range[1] && r.id <= +range[2])
                        : [pick(list, w.replace(/^#(?=\d+$)/, ''))].filter(Boolean);
            if (!found.length)
                return 'N' + (w === 'all' ? 'nothing recorded yet' : 'no request ' + w + (viewing ? ' in the file' : ' (web network lists them)'));
            found.forEach(r => chosen.add(r));
        }
        const { matches } = filtered([...chosen].sort((a, b) => a.id - b.id), filter);
        if (!matches.length)
            return 'N' + 'no request of those matches ' + filter.slice(1).join(' ');
        const notes = [];
        const usable = matches.filter(r => {
            if (!r.har._webSocketMessages || ['raw', 'http', 'har'].includes(format))
                return true;
            notes.push('#' + r.id + ' is a WebSocket: left out (--as raw, http or har have its handshake)');
            return false;
        });
        if (!usable.length)
            return 'N' + notes.join('; ');
        if (format === 'har') {
            const text = viewing ? archive(usable, true, viewing.pages || [], viewing) : archive(usable, true, pages);
            return 'T' + usable.length + ' har ' + notes.length + '\n\n' + notes.map(n => n + '\n').join('') + text;
        }
        const f = FORMATS[format];
        const blocks = [];
        for (const r of usable) {
            const q = requestOf(r, noCookies);
            const lead = (format === 'node' ? '  ' : '') + (f.comment === '###' ? '#' : f.comment) + ' ';
            let said = '';
            if (q.body === null) {
                const why = 'its body is not known (' + (viewing ? 'not in the file' : 'WebKit gives the developer tools text bodies only') + '): left out';
                notes.push('#' + r.id + ': ' + why);
                said = lead + why + '\n';
                q.body = undefined;
            }
            const text = format === 'curl' ? asCurl(q) : format === 'fetch' ? asFetch(q) : format === 'node' ? asNode(q)
                       : format === 'python' ? asPython(q) : format === 'http' ? asHttp(q) : asRaw(q);
            blocks.push(f.comment ? (format === 'node' ? '  ' : '') + f.comment + ' ' + label(r) + '\n' + said + text : text);
        }
        let text = format === 'node' ? '(async () => {\n  let response;\n' + blocks.join('\n') + '})();\n'
                 : format === 'python' ? PYTHON_HEAD + blocks.join('\n')
                 : format === 'raw' ? blocks.join('') : blocks.join('\n');
        return 'T' + usable.length + ' ' + format + ' ' + notes.length + '\n' + (f.script || '') + '\n' + notes.map(n => n + '\n').join('') + text;
    }

    // ── what browser.c calls ─────────────────────────────────────────────────────────────────

    return {
        // devtools on: the inspector's frontend is connected (the targets come as events)
        start(options) {
            const o = JSON.parse(options || '{}');
            if (o.bodies)
                bodies = o.bodies;
            if (o.version)
                version = o.version;
            // on in the middle of a page: the entries from now on are that page's
            if (!pages.length && o.uri && !o.uri.startsWith('about:'))
                pages.push({ id: 'page_1', startedDateTime: iso(Date.now() / 1000), title: o.title || o.uri, pageTimings: {}, key: null });
            command('Target.setPauseOnStart', { pauseOnStart: true });
        },
        // devtools off: the frontend is gone; what was loading ends unfinished
        stop() {
            for (const held of [...paused.values()])
                unhold(held.r, 'sent as it was (the developer tools went off)');
            intercept = null;
            for (const r of [...open.values()]) {
                if (!r.socket)
                    r.har.response._error = r.har.response._error || 'the recording stopped first';
                end(r, undefined);
            }
            open.clear();
            channels.clear();
            replies.clear();
            fetching = 0;
        },
        setBodies(mode) {
            bodies = mode;
        },
        receive,
        // web intercept on [TEXT...] [OPTION...]: "T" and the state, "E" and what is wrong
        interceptOn(words) {
            const o = interceptOptions(JSON.parse(words));
            if (typeof o === 'string')
                return o;
            intercept = o;
            for (const channel of channels.values())
                interceptIn(channel);
            return 'T' + interceptStatus();
        },
        // web intercept off: what waits goes as it was
        interceptOff() {
            const was = !!intercept, released = paused.size;
            intercept = null;
            for (const held of [...paused.values()])
                unhold(held.r, 'sent as it was (web intercept off)');
            for (const channel of channels.values())
                interceptIn(channel);
            return 'intercept: off' + (was ? '' : ' (it was)') + (released === 1 ? '; the paused request went on as it was'
                : released ? '; the ' + released + ' paused requests went on as they were' : '') + '\n';
        },
        interceptStatus,
        // "SERIAL COUNT ON": how many requests paused so far, how many wait now, whether it is on
        pauseState() {
            return pauseSerial + ' ' + paused.size + ' ' + (intercept ? 1 : 0);
        },
        pausedList(lead) {
            return pausedListing(lead);
        },
        // a paused request in detail: "T" and it, "N" and why not
        pausedShow(which) {
            const found = pausedOf(which);
            if (found.length)
                return 'T' + pausedDetail(found[found.length - 1]);
            return 'N' + notPaused(which) + '; ' + (paused.size ? paused.size + ' paused (web intercept lists them)' : 'none is paused');
        },
        decide(action, words) {
            return decide(action, JSON.parse(words));
        },
        // a decision's answer once WebKit has answered it all ("P" until then; late: answer anyway)
        decision(id, late) {
            const d = decisions.get(+id);
            if (!d)
                return 'Eno decision ' + id;
            if (d.waiting > 0 && !late)
                return 'P';
            decisions.delete(+id);
            const left = paused.size;
            return (d.failed ? 'F' : 'T') + d.lines.join('\n') + '\n' + (d.waiting > 0 ? '(WebKit has not answered for ' + d.waiting + ' of them yet)\n' : '')
                + (left ? left + ' still paused (web intercept lists them)\n' : intercept ? 'nothing else is paused\n' : '');
        },
        takeOutbox() {
            return outbox.splice(0).join('\n');
        },
        busy() {
            return fetching;
        },
        title(title) {
            const page = pages[pages.length - 1];
            if (page && title)
                page.title = title;
        },
        status() {
            const loading = records.filter(r => !r.done).length;
            const shown = new Set(records.map(r => r.har.pageref)).size - (records.some(r => !r.har.pageref) ? 1 : 0);
            return records.length + ' request' + (records.length === 1 ? '' : 's') + (loading ? ' (' + loading + ' still loading)' : '')
                + ' of ' + shown + ' page' + (shown === 1 ? '' : 's') + '; bodies: ' + bodies + ', ' + size(kept) + ' kept';
        },
        clear() {
            const loading = [...open.values()];
            records = loading;
            pages = pages.slice(-1);
            kept = records.reduce((n, r) => n + r.kept, 0);
        },
        list(words) {
            return listing(records, JSON.parse(words), '');
        },
        // what finished after `since` (the watch's cursor): "CURSOR\n" and the rows
        poll(since, words) {
            since = +since;
            const done = records.filter(r => r.done && r.seq > since).sort((a, b) => a.seq - b.seq);
            const { matches } = filtered(done, JSON.parse(words));
            return (since < 0 ? finished : done.reduce((m, r) => Math.max(m, r.seq), since)) + '\n'
                + (since < 0 ? HEAD + '\n' : matches.map(row).join('\n') + (matches.length ? '\n' : ''));
        },
        show(which) {
            const r = pick(records, which);
            return r ? detail(r, false) : null;
        },
        // the body of a response: "T" and its text, "B" and base64, "P" while it is read, "E" and why not
        body(which) {
            const r = pick(records, which);
            if (!r)
                return 'Eno request ' + which + ' (web network lists them)';
            if (r.socket)
                return 'Ea WebSocket has messages, not a body (web network show ' + r.id + ')';
            const content = r.har.response.content;
            if (content.text !== undefined)
                return (content.encoding === 'base64' ? 'B' : 'T') + content.text;
            if (!r.done)
                return 'P';
            if (r.har.response._error)
                return 'Eit failed: ' + r.har.response._error;
            if (!r.fetching && !r.bodyError)
                fetchBody(r);
            return r.fetching ? 'P' : 'E' + (r.bodyError || 'no body');
        },
        // search PATTERN [OPTION...]: in the recording (searchIn)
        search(pattern, words) {
            return searchIn(records, pattern, JSON.parse(words));
        },
        // web har search FILE PATTERN [OPTION...]: in a HAR file of any browser
        searchHar(text, words) {
            const har = parseHar(text);
            if (typeof har === 'string')
                return har;
            const [pattern, ...options] = JSON.parse(words);
            return searchIn(recordsOf(har), pattern, options, true);
        },
        // the HAR: {"bodies": false} leaves the response bodies out
        har(options) {
            const o = JSON.parse(options || '{}');
            version = o.version || version;
            return archive(records, o.bodies !== false, pages);
        },
        // web network export WHICH... [OPTION...]: exported()
        exportRequests(words) {
            return exported(records, JSON.parse(words));
        },
        // web har export FILE WHICH... [OPTION...]: entries of a HAR file of any browser
        exportHar(text, words) {
            const har = parseHar(text);
            return typeof har === 'string' ? har : exported(recordsOf(har), JSON.parse(words), har.log);
        },
        // web har view FILE [N | TEXT | listing words]: a HAR file of any browser, listed as the recording is
        view(text, words) {
            const har = parseHar(text);
            if (typeof har === 'string')
                return har;
            const list = recordsOf(har);
            const w = JSON.parse(words);
            const creator = har.log.creator ? har.log.creator.name + (har.log.creator.version ? ' ' + har.log.creator.version : '') : '?';
            const title = 'HAR ' + (har.log.version || '?') + ' by ' + creator + ': ' + list.length + ' entr' + (list.length === 1 ? 'y' : 'ies')
                + ', ' + (har.log.pages || []).length + ' page' + ((har.log.pages || []).length === 1 ? '' : 's') + '\n';
            if (w.length === 1 && /^\d+$/.test(w[0])) {
                const r = pick(list, w[0]);
                return r ? 'T' + detail(r, true) + (r.har.response && r.har.response.content && r.har.response.content.text === undefined ? '\nresponse body: not in the file\n' : '')
                    : 'Eno entry ' + w[0] + ' (of ' + list.length + ')';
            }
            return 'T' + listing(list, w, title);
        },
    };
})()
