// The requests a page makes, for the developer tools' checks (tests/guest.sh): fetch (one that sets
// a cookie, a POST with a JSON body, a 404, a redirect), XMLHttpRequest, a worker's fetch, a WebSocket.
(async () => {
    const state = document.getElementById('state');
    const data = await (await fetch('/api/data?x=1&y=two')).json();
    const echoed = await (await fetch('/api/echo', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ hello: '세계' }),
    })).json();
    const xhr = await new Promise(done => {
        const x = new XMLHttpRequest();
        x.open('GET', '/api/data?xhr=1');
        x.onload = () => done(JSON.parse(x.responseText).ok);
        x.send();
    });
    await fetch('/missing.js');
    await (await fetch('/redirect')).text();
    const worker = await new Promise(done => {
        const w = new Worker('net-worker.js');
        w.onmessage = e => done(e.data);
    });
    const echo = await new Promise(done => {
        const ws = new WebSocket(`ws://${location.host}/ws`);
        ws.onopen = () => ws.send('ping 1');
        ws.onmessage = e => { ws.close(); done(e.data); };
        ws.onerror = () => done('websocket failed');
    });
    state.textContent = `all done: ${data.ok} ${echoed.body.hello} ${xhr} ${worker} ${echo}`;
})();
