// web run natives.js: what the page reads of the browser's identity, and whether it is WebKit's own
// (native getters, in a worker too) and free of the browser's own marks
const getter = name => String(Object.getOwnPropertyDescriptor(Navigator.prototype, name).get).includes('[native code]');
const worker = await new Promise(done => {
  const w = new Worker(URL.createObjectURL(new Blob(['postMessage(navigator.platform + " " + navigator.hardwareConcurrency)'])));
  w.onmessage = e => done(e.data);
  setTimeout(() => done('no answer'), 10000);
});
return [navigator.platform, navigator.vendor, 'cores=' + navigator.hardwareConcurrency, 'native=' + (getter('platform') && getter('hardwareConcurrency')),
  'worker=' + worker, 'screen-holds-window=' + (screen.width >= outerWidth && screen.availHeight >= outerHeight),
  'outer>inner=' + (outerHeight > innerHeight), 'pointer-fine=' + matchMedia('(pointer: fine)').matches,
  'languages=' + navigator.languages.join('/'), 'marks=' + document.querySelectorAll('[data-wpe-ref]').length,
  'globals=' + Object.keys(window).filter(k => /^__wpe|^__web/.test(k)).length].join(' ');
