// The browser's identity as the page's own scripts see it, made to match the user agent it sends
// (browser.c: the user agent profiles). Run at the start of every document, in every frame, before
// the page's scripts. WebKit says navigator.platform is what uname says ("Linux wasm32": a platform
// no site knows, and one that contradicts any user agent); a vendor, Firefox's oscpu, Chromium's
// userAgentData and a touch screen go with the profile.
//
//   id: { platform, vendor, oscpu?, touch, brands?: [[brand, major]...], fullVersion?, mobile?,
//         chPlatform?, chPlatformVersion? }
(id) => {
    const proto = Navigator.prototype;
    const define = (name, value) => {
        try {
            Object.defineProperty(proto, name, { get() { return value; }, configurable: true, enumerable: true });
        } catch (e) {}
    };
    define('platform', id.platform);
    define('vendor', id.vendor);
    if (id.oscpu)
        define('oscpu', id.oscpu);
    if (id.touch)
        define('maxTouchPoints', 5);
    if (!id.brands)
        return;
    const brands = () => id.brands.map(([brand, version]) => ({ brand, version }));
    const fullVersionList = () => id.brands.map(([brand, version]) =>
        ({ brand, version: /^\d+$/.test(version) && id.fullVersion.startsWith(version + '.') ? id.fullVersion : version + '.0.0.0' }));
    const high = {
        architecture: id.mobile ? '' : 'x86', bitness: id.mobile ? '' : '64', formFactors: [id.mobile ? 'Mobile' : 'Desktop'],
        model: '', platformVersion: id.chPlatformVersion, uaFullVersion: id.fullVersion, wow64: false,
    };
    const data = {
        get brands() { return brands(); },
        get mobile() { return !!id.mobile; },
        get platform() { return id.chPlatform; },
        getHighEntropyValues(hints) {
            const values = { brands: brands(), mobile: !!id.mobile, platform: id.chPlatform };
            for (const hint of Array.from(hints || [])) {
                if (hint === 'fullVersionList')
                    values.fullVersionList = fullVersionList();
                else if (hint in high)
                    values[hint] = high[hint];
            }
            return Promise.resolve(values);
        },
        toJSON() { return { brands: brands(), mobile: !!id.mobile, platform: id.chPlatform }; },
    };
    define('userAgentData', data);
}
