// web run fetch.js PATH: what the server saw of a request the page made (fetch)
const response = await fetch(args[0], { cache: 'no-store' });
return await response.json();
