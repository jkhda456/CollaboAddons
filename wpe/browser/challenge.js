// Recognizes a robot check on the page (`web challenge`, and the last line of `web open`'s answer):
// Cloudflare's interstitial and Turnstile, reCAPTCHA, hCaptcha, DataDome, HUMAN's press and hold,
// AWS WAF, a search engine's "unusual traffic" page, an outright block. Runs in the browser's own
// world (AGENT_WORLD): it reads the DOM and boxes, never the frames' insides (another origin).
// Returns null, or one line: KIND \t ACTION \t X \t Y \t HOLD_MS \t WHAT — ACTION is wait (it
// passes by itself), click (the checkbox at X, Y), hold (press X, Y for HOLD_MS), agent (a puzzle
// for whoever looks: a screenshot and clicks), blocked (nothing on the page lets it through).
(function () {
  "use strict";
  const title = document.title || "";
  const body = document.body;
  const text = body ? (body.innerText || "").slice(0, 6000) : "";
  const visible = el => {
    if (!el || !el.isConnected) return false;
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 4 && r.height > 4 && s.visibility !== "hidden" && s.display !== "none" && parseFloat(s.opacity) > 0.05
      && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  };
  const frames = Array.from(document.querySelectorAll("iframe"));
  const frame = re => frames.find(f => re.test(f.src || f.getAttribute("src") || "") && visible(f));
  const line = (kind, action, x, y, hold, what) =>
    [kind, action, Math.round(x || 0), Math.round(y || 0), Math.round(hold || 0), what.replace(/\s+/g, " ")].join("\t");
  // a checkbox widget's box: its box sits at the left, vertically centered
  const checkbox = (el, dx) => { const r = el.getBoundingClientRect(); return [r.left + Math.min(dx, r.width / 2), r.top + r.height / 2]; };
  const scripts = Array.from(document.scripts).map(s => s.src || "").join(" ");
  const has = re => re.test(text);

  // Cloudflare: Turnstile in a page, or the interstitial ("Just a moment...") with its widget,
  // which may sit in a closed shadow root: then the box of the element that holds it
  const cfPage = /^(just a moment|잠시만 기다|un instant|einen moment|un momento|attention required)/i.test(title)
    || !!document.querySelector("#challenge-stage, #challenge-form, #cf-challenge-running, .cf-browser-verification")
    || /\/cdn-cgi\/challenge-platform\//.test(scripts);
  if (/attention required/i.test(title) || has(/sorry, you have been blocked|you are unable to access|error 10(0[0-9]|20)\b/i))
    return line("cloudflare-block", "blocked", 0, 0, 0, `Cloudflare blocked this request (${title})`);
  // the widget: its frame, or — in a closed shadow root, as on the interstitial — the element
  // beside its hidden response field, the widget at its top left (300 x 65, the box 30 px in)
  let turnstile = frame(/challenges\.cloudflare\.com/), point = null;
  if (turnstile)
    point = checkbox(turnstile, 30);
  else {
    const response = document.querySelector('input[name="cf-turnstile-response"], input[id^="cf-chl-widget-"][id$="_response"]');
    const host = response && response.parentElement;
    if (host && visible(host)) {
      const r = host.getBoundingClientRect();
      point = [r.left + 30, r.top + Math.min(r.height, 65) / 2];
      turnstile = host;
    }
  }
  if (turnstile) {
    const [x, y] = point;
    return line(cfPage ? "cloudflare" : "turnstile", "click", x, y, 0,
      cfPage ? `Cloudflare's check (${title}), its Turnstile widget: the box is clicked once it asks` : "a Cloudflare Turnstile widget");
  }
  if (cfPage)
    return line("cloudflare", "wait", 0, 0, 0, `Cloudflare's check (${title}): it runs, then the page loads`);

  // reCAPTCHA: the checkbox (anchor) or the picture puzzle (bframe); Google's "unusual traffic"
  const recaptchaImages = frame(/recaptcha\/(api2|enterprise)\/bframe/);
  if (recaptchaImages && recaptchaImages.getBoundingClientRect().height > 200)
    return line("recaptcha-images", "agent", 0, 0, 0, "reCAPTCHA's picture puzzle: look (web screenshot) and click the tiles it asks for, then Verify");
  const recaptcha = frame(/recaptcha\/(api2|enterprise)\/anchor/);
  const sorry = location.hostname.endsWith("google.com") && location.pathname.startsWith("/sorry");
  if (recaptcha) {
    const [x, y] = checkbox(recaptcha, 28);
    return line(sorry ? "google-sorry" : "recaptcha", "click", x, y, 0, sorry ? "Google's \"unusual traffic\" check (reCAPTCHA)" : "a reCAPTCHA checkbox");
  }
  if (sorry)
    return line("google-sorry", "blocked", 0, 0, 0, "Google's \"unusual traffic\" page: this address is limited for now (another search engine: duckduckgo, bing)");

  // hCaptcha
  const hcaptchaImages = frame(/hcaptcha\.com.*frame=challenge/);
  if (hcaptchaImages && hcaptchaImages.getBoundingClientRect().height > 200)
    return line("hcaptcha-images", "agent", 0, 0, 0, "hCaptcha's picture puzzle: look (web screenshot) and click what it asks for");
  const hcaptcha = frame(/hcaptcha\.com.*frame=checkbox/) || frame(/newassets\.hcaptcha\.com/);
  if (hcaptcha) {
    const [x, y] = checkbox(hcaptcha, 30);
    return line("hcaptcha", "click", x, y, 0, "an hCaptcha checkbox");
  }

  // DataDome: its puzzle (a slider) in a frame; HUMAN (PerimeterX): press and hold
  if (frame(/captcha-delivery\.com/))
    return line("datadome", "agent", 0, 0, 0, "DataDome's slider puzzle: look (web screenshot), then drag the piece into place");
  const px = document.querySelector("#px-captcha");
  if (px && visible(px)) {
    const r = px.getBoundingClientRect();
    return line("press-and-hold", "hold", r.left + r.width / 2, r.top + r.height / 2, 9000 + Math.random() * 3000,
      "HUMAN's \"press and hold\" button");
  }

  // AWS WAF; Akamai's and others' outright refusals
  if (document.querySelector("#captcha-container, awswaf-captcha") || /awswaf|token\.awswaf\.com/.test(scripts))
    return /captcha/i.test(text)
      ? line("aws-waf", "agent", 0, 0, 0, "AWS WAF's picture puzzle: look (web screenshot) and click what it asks for")
      : line("aws-waf", "wait", 0, 0, 0, "AWS WAF's check (Amazon...): its script works out a proof of work, then the page loads");
  if (/^access denied$/i.test(title.trim()) && has(/reference #|you don't have permission/i))
    return line("blocked", "blocked", 0, 0, 0, "Access Denied (Akamai): the site refuses this request");

  // search engines' and others' own checks
  if (location.hostname.endsWith("duckduckgo.com") && has(/bots use duckduckgo too|select all squares|anomaly/i))
    return line("duckduckgo-anomaly", "agent", 0, 0, 0, "DuckDuckGo's picture check: look (web screenshot) and click the images it asks for");
  if (has(/verify (that )?you are (a )?human|are you a robot|unusual traffic from your computer|자동입력 방지|로봇이 아닙니다|보안 문자/i))
    return line("check", "agent", 0, 0, 0, "the page asks to prove a person is there: web read / web screenshot shows how");
  return null;
})
