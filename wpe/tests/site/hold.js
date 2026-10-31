// The requests `web intercept` decides, one after the other (tests/guest.sh): one the agent sends
// elsewhere, a POST whose body and headers it changes, one it drops, one it answers itself.
(async () => {
    const state = document.getElementById('state');
    const seen = [];
    const step = text => { seen.push(text); state.textContent = seen.join(' | '); };
    const moved = await (await fetch('/api/data?hold=1')).json();
    step('query ' + moved.query);
    const echoed = await (await fetch('/api/echo', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ hello: 'page' }),
    })).json();
    step('echo ' + echoed.body.hello + ' ' + (echoed.headers['x-agent'] || '-'));
    try {
        await fetch('/api/data?drop=1');
        step('not dropped');
    } catch (e) {
        step('dropped ' + e.name);
    }
    const answered = await fetch('/api/data?mock=1');
    step('mock ' + answered.status + ' ' + (await answered.text()));
    step('hold done');
})();
