// The doom add-on's terminal input decoder (input.h).
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include "input.h"

#include <stdio.h>
#include <string.h>

void tk_init(tk_decoder *d) { memset(d, 0, sizeof *d); }

void tk_feed(tk_decoder *d, const unsigned char *bytes, int n, uint32_t now_ms)
{
    if (d->len == 0)
        d->esc_since = now_ms;
    if (n > (int)sizeof d->buf - d->len) {
        // more than a sequence can be: the oldest bytes go (keys mashed while the game hung)
        int drop = n - ((int)sizeof d->buf - d->len);
        if (drop >= d->len) {
            bytes += drop - d->len;
            n -= drop - d->len;
            d->len = 0;
        } else {
            memmove(d->buf, d->buf + drop, (size_t)(d->len - drop));
            d->len -= drop;
        }
    }
    memcpy(d->buf + d->len, bytes, (size_t)n);
    d->len += n;
}

static void consume(tk_decoder *d, int n, uint32_t now_ms)
{
    if (n > d->len)
        n = d->len;
    memmove(d->buf, d->buf + n, (size_t)(d->len - n));
    d->len -= n;
    d->esc_since = now_ms;
}

static uint32_t lower(uint32_t c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }

static void key_event(tk_event *ev, uint32_t key, uint32_t text, int mods, int type, int exact)
{
    ev->type = type;
    ev->key = key;
    ev->text = text;
    ev->mods = mods;
    ev->exact = exact;
}

// A character typed: its key (letters in lower case), shift for capitals.
static void char_event(tk_event *ev, uint32_t c, int mods)
{
    if (c == '\r' || c == '\n')
        key_event(ev, TK_ENTER, '\r', mods, TE_PRESS, 0);
    else if (c == '\t')
        key_event(ev, TK_TAB, '\t', mods, TE_PRESS, 0);
    else if (c == 0x7f || c == 0x08)
        key_event(ev, TK_BACKSPACE, 0, mods, TE_PRESS, 0);
    else if (c == 0)
        key_event(ev, ' ', 0, mods | TM_CTRL, TE_PRESS, 0);
    else if (c < 0x1b)
        key_event(ev, 'a' + c - 1, 0, mods | TM_CTRL, TE_PRESS, 0);
    else if (c < 0x20)
        key_event(ev, "\\]^_"[c - 0x1c], 0, mods | TM_CTRL, TE_PRESS, 0);
    else if (c >= 'A' && c <= 'Z')
        key_event(ev, c + 32, c, mods | TM_SHIFT, TE_PRESS, 0);
    else
        key_event(ev, c, c, mods, TE_PRESS, 0);
}

// UTF-8: the length of the sequence starting with b (0: not a start byte).
static int utf8_len(unsigned char b)
{
    if (b < 0x80)
        return 1;
    if (b >= 0xc2 && b < 0xe0)
        return 2;
    if (b >= 0xe0 && b < 0xf0)
        return 3;
    if (b >= 0xf0 && b < 0xf5)
        return 4;
    return 0;
}

static uint32_t utf8_decode(const unsigned char *s, int n)
{
    if (n == 1)
        return s[0];
    uint32_t c = s[0] & (0x7f >> n);
    for (int i = 1; i < n; i++)
        c = c << 6 | (s[i] & 0x3f);
    return c;
}

// ── CSI ──────────────────────────────────────────────────────────────────────────────────────────
#define MAXP 8
#define MAXS 4
typedef struct {
    int prefix;          // '<' '=' '>' '?' or 0
    int final;
    int n;               // parameters given
    int p[MAXP][MAXS];   // -1: left out
} csi_t;

static int P(const csi_t *c, int i, int s, int dflt)
{
    return i < c->n && c->p[i][s] >= 0 ? c->p[i][s] : dflt;
}

// The parameters of a CSI sequence whose bytes are s[0..n) (after "ESC [", before the final).
static void parse_params(csi_t *c, const unsigned char *s, int n)
{
    memset(c->p, -1, sizeof c->p);
    c->n = 0;
    c->prefix = 0;
    int i = 0;
    if (n > 0 && s[0] >= 0x3c && s[0] <= 0x3f)
        c->prefix = s[i++];
    if (i >= n)
        return;
    int pi = 0, si = 0;
    c->n = 1;
    for (; i < n; i++) {
        unsigned char b = s[i];
        if (b >= '0' && b <= '9') {
            if (pi < MAXP && si < MAXS) {
                int v = c->p[pi][si] < 0 ? 0 : c->p[pi][si];
                if (v < 10000000)
                    c->p[pi][si] = v * 10 + (b - '0');
            }
        } else if (b == ';') {
            pi++;
            si = 0;
            if (pi < MAXP)
                c->n = pi + 1;
        } else if (b == ':') {
            si++;
        }
    }
}

// The kitty keyboard protocol's codes of keys that are no character.
static uint32_t kitty_key(int code)
{
    switch (code) {
    case 13: return TK_ENTER;
    case 9: return TK_TAB;
    case 127: case 8: return TK_BACKSPACE;
    case 27: return TK_ESCAPE;
    case 57441: case 57447: return TK_SHIFT;
    case 57442: case 57448: return TK_CTRL;
    case 57443: case 57449: return TK_ALT;
    case 57362: return TK_PAUSE;
    case 57414: return TK_ENTER;          // keypad
    case 57417: return TK_LEFT;
    case 57418: return TK_RIGHT;
    case 57419: return TK_UP;
    case 57420: return TK_DOWN;
    case 57421: return TK_PGUP;
    case 57422: return TK_PGDN;
    case 57423: return TK_HOME;
    case 57424: return TK_END;
    case 57425: return TK_INSERT;
    case 57426: return TK_DELETE;
    case 57409: return '.';
    case 57410: return '/';
    case 57411: return '*';
    case 57412: return '-';
    case 57413: return '+';
    case 57415: return '=';
    }
    if (code >= 57399 && code <= 57408)
        return (uint32_t)('0' + code - 57399);
    if (code >= 57344 && code < 63744)
        return TK_NONE;  // other keys of the private use area: media keys, F13..., ...
    return (uint32_t)code;
}

static uint32_t tilde_key(int n)
{
    switch (n) {
    case 1: case 7: return TK_HOME;
    case 2: return TK_INSERT;
    case 3: return TK_DELETE;
    case 4: case 8: return TK_END;
    case 5: return TK_PGUP;
    case 6: return TK_PGDN;
    case 11: return TK_F1;
    case 12: return TK_F2;
    case 13: return TK_F3;
    case 14: return TK_F4;
    case 15: return TK_F5;
    case 17: return TK_F6;
    case 18: return TK_F7;
    case 19: return TK_F8;
    case 20: return TK_F9;
    case 21: return TK_F10;
    case 23: return TK_F11;
    case 24: return TK_F12;
    }
    return TK_NONE;
}

static uint32_t letter_key(int final)
{
    switch (final) {
    case 'A': return TK_UP;
    case 'B': return TK_DOWN;
    case 'C': return TK_RIGHT;
    case 'D': return TK_LEFT;
    case 'H': return TK_HOME;
    case 'F': return TK_END;
    case 'P': return TK_F1;
    case 'Q': return TK_F2;
    case 'R': return TK_F3;
    case 'S': return TK_F4;
    }
    return TK_NONE;
}

// Windows' virtual key codes (win32-input-mode).
static uint32_t vk_key(int vk, int uc)
{
    if (vk >= 'A' && vk <= 'Z')
        return (uint32_t)(vk + 32);
    if (vk >= '0' && vk <= '9')
        return (uint32_t)vk;
    if (vk >= 0x60 && vk <= 0x69)
        return (uint32_t)('0' + vk - 0x60);
    if (vk >= 0x70 && vk <= 0x7b)
        return TK_F1 + (uint32_t)(vk - 0x70);
    switch (vk) {
    case 0x08: return TK_BACKSPACE;
    case 0x09: return TK_TAB;
    case 0x0d: return TK_ENTER;
    case 0x10: case 0xa0: case 0xa1: return TK_SHIFT;
    case 0x11: case 0xa2: case 0xa3: return TK_CTRL;
    case 0x12: case 0xa4: case 0xa5: return TK_ALT;
    case 0x13: return TK_PAUSE;
    case 0x1b: return TK_ESCAPE;
    case 0x20: return ' ';
    case 0x21: return TK_PGUP;
    case 0x22: return TK_PGDN;
    case 0x23: return TK_END;
    case 0x24: return TK_HOME;
    case 0x25: return TK_LEFT;
    case 0x26: return TK_UP;
    case 0x27: return TK_RIGHT;
    case 0x28: return TK_DOWN;
    case 0x2d: return TK_INSERT;
    case 0x2e: return TK_DELETE;
    case 0x6a: return '*';
    case 0x6b: return '+';
    case 0x6d: return '-';
    case 0x6e: return '.';
    case 0x6f: return '/';
    case 0xba: return ';';
    case 0xbb: return '=';
    case 0xbc: return ',';
    case 0xbd: return '-';
    case 0xbe: return '.';
    case 0xbf: return '/';
    case 0xc0: return '`';
    case 0xdb: return '[';
    case 0xdc: return '\\';
    case 0xdd: return ']';
    case 0xde: return '\'';
    }
    if (uc >= 0x20)
        return lower((uint32_t)uc);
    return TK_NONE;
}

static int kitty_mods(int m)
{
    m = m > 0 ? m - 1 : 0;
    return (m & 1 ? TM_SHIFT : 0) | (m & 2 ? TM_ALT : 0) | (m & 4 ? TM_CTRL : 0);
}

// A whole CSI sequence: 1 and the event if it is a key or an answer, 0 if it is something else.
static int csi_event(const csi_t *c, tk_event *ev)
{
    int f = c->final;
    if (c->prefix == '?') {
        if (f == 'u') {
            key_event(ev, TK_NONE, 0, 0, TE_KITTY, 1);
            ev->text = (uint32_t)P(c, 0, 0, 0);  // the flags in force
            return 1;
        }
        if (f == 'c') {
            key_event(ev, TK_NONE, 0, 0, TE_DA, 0);
            return 1;
        }
        return 0;
    }
    if (c->prefix)
        return 0;  // mouse reports, the secondary device attributes, ...
    if (f == 'u') {  // kitty: CSI code[:shifted[:base]] ; mods[:event] ; text u
        int code = P(c, 0, 0, 0);
        uint32_t key = kitty_key(code);
        if (key == TK_NONE)
            return 0;
        int mods = kitty_mods(P(c, 1, 0, 1));
        int type = P(c, 1, 1, 1);
        if (type < 1 || type > 3)
            type = 1;
        uint32_t text = 0;
        if (key < TK_BASE && key >= 0x20 && key != 127) {
            text = P(c, 2, 0, -1) > 0 ? (uint32_t)P(c, 2, 0, 0) : key;
            if ((mods & TM_SHIFT) && key >= 'a' && key <= 'z')
                text = key - 32;
            if (P(c, 0, 1, -1) > 0 && (mods & TM_SHIFT))
                text = (uint32_t)P(c, 0, 1, 0);  // the shifted key, when the terminal says
        }
        if (key == TK_ENTER)
            text = '\r';
        key_event(ev, lower(key), text, mods, type, 1);
        return 1;
    }
    if (f == '_') {  // win32-input-mode: CSI Vk;Sc;Uc;Kd;Cs;Rc _
        int vk = P(c, 0, 0, 0), uc = P(c, 2, 0, 0), kd = P(c, 3, 0, 0), cs = P(c, 4, 0, 0);
        uint32_t key = vk_key(vk, uc);
        if (key == TK_NONE)
            return 0;
        int mods = (cs & 0x10 ? TM_SHIFT : 0) | (cs & 0x3 ? TM_ALT : 0) | (cs & 0xc ? TM_CTRL : 0);
        uint32_t text = uc >= 0x20 && uc != 127 ? (uint32_t)uc : 0;
        if (key == TK_ENTER)
            text = '\r';
        key_event(ev, key, text, mods, kd ? TE_PRESS : TE_RELEASE, 1);
        return 1;
    }
    if (f == '~') {
        int n = P(c, 0, 0, 0);
        if (n == 27) {  // xterm's modifyOtherKeys: CSI 27 ; mods ; code ~
            int code = P(c, 2, 0, 0);
            if (code <= 0)
                return 0;
            int mods = kitty_mods(P(c, 1, 0, 1));
            tk_event e;
            char_event(&e, (uint32_t)code, mods);
            *ev = e;
            return 1;
        }
        uint32_t key = tilde_key(n);
        if (key == TK_NONE)
            return 0;
        int type = P(c, 1, 1, 1);
        key_event(ev, key, 0, kitty_mods(P(c, 1, 0, 1)), type >= 1 && type <= 3 ? type : 1, P(c, 1, 1, -1) > 0);
        return 1;
    }
    if (f == 'Z') {
        key_event(ev, TK_TAB, 0, TM_SHIFT, TE_PRESS, 0);
        return 1;
    }
    uint32_t key = letter_key(f);
    if (key == TK_NONE)
        return 0;
    if (f == 'R' && c->n >= 2 && P(c, 0, 0, 1) != 1) {  // a cursor position report, not F3
        key_event(ev, (uint32_t)P(c, 0, 0, 1), (uint32_t)P(c, 1, 0, 1), 0, TE_CPR, 0);
        return 1;
    }
    int type = P(c, 1, 1, 1);
    key_event(ev, key, 0, kitty_mods(P(c, 1, 0, 1)), type >= 1 && type <= 3 ? type : 1, P(c, 1, 1, -1) > 0);
    return 1;
}

int tk_next(tk_decoder *d, tk_event *ev, uint32_t now_ms, uint32_t esc_ms)
{
    while (d->len > 0) {
        unsigned char *b = d->buf;
        uint32_t waited = now_ms - d->esc_since;
        if (b[0] != 0x1b) {
            int n = utf8_len(b[0]);
            if (n == 0) {
                consume(d, 1, now_ms);
                continue;
            }
            if (n > d->len) {
                if (waited > 200)
                    consume(d, d->len, now_ms);
                return 0;
            }
            uint32_t c = utf8_decode(b, n);
            consume(d, n, now_ms);
            char_event(ev, c, 0);
            return 1;
        }
        if (d->len == 1) {
            if (waited < esc_ms)
                return 0;
            consume(d, 1, now_ms);
            key_event(ev, TK_ESCAPE, 0x1b, 0, TE_PRESS, 0);
            return 1;
        }
        if (b[1] == '[') {
            if (d->len >= 3 && b[2] == '[') {  // the Linux console's F1-F5: ESC [ [ A..E
                if (d->len < 4)
                    goto incomplete;
                int k = b[3];
                consume(d, 4, now_ms);
                if (k >= 'A' && k <= 'E') {
                    key_event(ev, TK_F1 + (uint32_t)(k - 'A'), 0, 0, TE_PRESS, 0);
                    return 1;
                }
                continue;
            }
            int i = 2;
            while (i < d->len && b[i] >= 0x20 && b[i] <= 0x3f)
                i++;
            if (i >= d->len)
                goto incomplete;
            if (b[i] < 0x40 || b[i] > 0x7e) {  // not a sequence: drop the ESC [
                consume(d, 2, now_ms);
                continue;
            }
            csi_t c;
            parse_params(&c, b + 2, i - 2);
            c.final = b[i];
            consume(d, i + 1, now_ms);
            if (csi_event(&c, ev))
                return 1;
            continue;
        }
        if (b[1] == 'O') {  // SS3
            if (d->len < 3)
                goto incomplete;
            int i = 2, mods = 0;
            while (i < d->len && b[i] >= '0' && b[i] <= '9')
                mods = mods * 10 + (b[i++] - '0');
            if (i >= d->len)
                goto incomplete;
            int k = b[i];
            consume(d, i + 1, now_ms);
            uint32_t key = letter_key(k);
            if (key != TK_NONE) {
                key_event(ev, key, 0, kitty_mods(mods ? mods : 1), TE_PRESS, 0);
                return 1;
            }
            if (k == 'M') {
                key_event(ev, TK_ENTER, '\r', 0, TE_PRESS, 0);
                return 1;
            }
            static const char kp[] = "*+,-./0123456789";  // ESC O j .. y: the keypad
            if (k >= 'j' && k <= 'y') {
                char_event(ev, (uint32_t)kp[k - 'j'], 0);
                return 1;
            }
            continue;
        }
        if (b[1] == ']' || b[1] == 'P' || b[1] == '_' || b[1] == '^' || b[1] == 'X') {
            // a string (OSC, DCS, APC, PM, SOS): an answer, no key; it ends with BEL or ST (ESC \)
            int i = 2;
            while (i < d->len && b[i] != 0x07 && !(b[i] == 0x1b && i + 1 < d->len && b[i + 1] == '\\'))
                i++;
            if (i >= d->len || (b[i] == 0x1b && i + 1 >= d->len))
                goto incomplete;
            consume(d, b[i] == 0x07 ? i + 1 : i + 2, now_ms);
            continue;
        }
        if (b[1] == 0x1b) {  // ESC ESC: the first is the key
            consume(d, 1, now_ms);
            key_event(ev, TK_ESCAPE, 0x1b, 0, TE_PRESS, 0);
            return 1;
        }
        // ESC and a character: that character with Alt
        {
            int n = utf8_len(b[1]);
            if (n == 0) {
                consume(d, 2, now_ms);
                continue;
            }
            if (1 + n > d->len)
                goto incomplete;
            uint32_t c = utf8_decode(b + 1, n);
            consume(d, 1 + n, now_ms);
            char_event(ev, c, TM_ALT);
            return 1;
        }
    incomplete:
        if (waited > 300 || d->len >= (int)sizeof d->buf) {
            consume(d, 1, now_ms);  // a sequence that never ended: drop its ESC
            continue;
        }
        return 0;
    }
    return 0;
}

const char *tk_key_name(uint32_t key, char buf[16])
{
    static const char *names[] = {"up", "down", "left", "right", "home", "end", "pgup", "pgdn", "insert",
        "delete", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "pause",
        "shift", "ctrl", "alt"};
    if (key > TK_BASE && key - TK_BASE - 1 < sizeof names / sizeof *names)
        return names[key - TK_BASE - 1];
    switch (key) {
    case TK_ESCAPE: return "esc";
    case TK_ENTER: return "enter";
    case TK_TAB: return "tab";
    case TK_BACKSPACE: return "backspace";
    case ' ': return "space";
    }
    if (key < 0x80 && key > 0x20) {
        buf[0] = (char)key;
        buf[1] = 0;
    } else {
        snprintf(buf, 16, "U+%04X", (unsigned)key);
    }
    return buf;
}
