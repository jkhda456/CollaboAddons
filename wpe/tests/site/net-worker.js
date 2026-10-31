// a dedicated worker's own request (net.js)
fetch('/api/data?worker=1').then(r => r.json()).then(d => postMessage('worker ' + d.ok));
