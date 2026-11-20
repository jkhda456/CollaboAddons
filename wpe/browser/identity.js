// What of the browser's identity WebKit has no property for, as the page's own scripts would see it
// in that browser: Firefox's navigator.oscpu, Chromium's navigator.userAgentData. Run at the start of
// every document, in every frame, before the page's scripts (browser.c: identity_script), and only
// for a user agent that needs it: Safari's identity (the default) needs none. The rest — platform,
// vendor, hardwareConcurrency, maxTouchPoints, screen, window, time zone — WebKit answers itself
// (WebKit patch 0012, set by extension.c), in workers too and with native getters.
//
//   id: { oscpu?, brands?: [[brand, major]...], fullVersion?, mobile?, chPlatform?, chPlatformVersion? }
(id) => {
    const proto = Navigator.prototype;
    const define = (name, value) => {
        try {
            Object.defineProperty(proto, name, { get() { return value; }, configurable: true, enumerable: true });
        } catch (e) {}
    };
    if (id.oscpu)
        define('oscpu', id.oscpu);
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
