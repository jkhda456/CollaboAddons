// web run events.js: how the input of the actions reached events.html (counts and timing, ms)
const by = t => log.filter(e => e[0] === t);
const down = by('mousedown'), up = by('mouseup'), keys = by('keydown');
const holds = down.map((d, i) => up[i] ? Math.round(up[i][1] - d[1]) : -1);
const gaps = keys.slice(1).map((k, i) => Math.round(k[1] - keys[i][1]));
return ['moves=' + by('mousemove').length, 'holds=' + holds.join(','), 'minhold=' + Math.min(...holds),
  'keys=' + keys.length, 'mingap=' + (gaps.length ? Math.min(...gaps) : -1), 'maxgap=' + (gaps.length ? Math.max(...gaps) : -1),
  'wheels=' + by('wheel').length, 'trusted=' + log.every(e => e[4]), 'value=' + document.getElementById('field').value].join(' ');
