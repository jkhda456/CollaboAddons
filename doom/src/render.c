// The doom add-on's terminal renderer (render.h).
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include "render.h"

#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// ── output buffer ────────────────────────────────────────────────────────────────────────────────
static void reserve(rt_t *r, size_t more)
{
    if (r->len + more <= r->cap)
        return;
    size_t cap = r->cap ? r->cap : 65536;
    while (cap < r->len + more)
        cap *= 2;
    char *p = realloc(r->out, cap);
    if (!p)
        abort();
    r->out = p;
    r->cap = cap;
}

void rt_put(rt_t *r, const char *s, size_t n)
{
    reserve(r, n);
    memcpy(r->out + r->len, s, n);
    r->len += n;
}

void rt_puts(rt_t *r, const char *s) { rt_put(r, s, strlen(s)); }

void rt_printf(rt_t *r, const char *fmt, ...)
{
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (n > 0)
        rt_put(r, buf, (size_t)n < sizeof buf ? (size_t)n : sizeof buf - 1);
}

// A number in decimal, without printf (this runs for every cell).
static inline char *put_uint(char *p, unsigned v)
{
    char tmp[12];
    int n = 0;
    do {
        tmp[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    while (n)
        *p++ = tmp[--n];
    return p;
}

static inline char *put_utf8(char *p, uint32_t c)
{
    if (c < 0x80) {
        *p++ = (char)c;
    } else if (c < 0x800) {
        *p++ = (char)(0xc0 | c >> 6);
        *p++ = (char)(0x80 | (c & 0x3f));
    } else {
        *p++ = (char)(0xe0 | c >> 12);
        *p++ = (char)(0x80 | (c >> 6 & 0x3f));
        *p++ = (char)(0x80 | (c & 0x3f));
    }
    return p;
}

// ── colours ──────────────────────────────────────────────────────────────────────────────────────
// The 16 ANSI colours as most terminals' defaults show them (VGA's).
static const uint32_t ansi16[16] = {
    0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
    0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
};
static const int cube[6] = {0, 95, 135, 175, 215, 255};

static uint32_t xterm256(int i)
{
    if (i < 16)
        return ansi16[i];
    if (i < 232) {
        i -= 16;
        return (uint32_t)(cube[i / 36] << 16 | cube[i / 6 % 6] << 8 | cube[i % 6]);
    }
    int g = 8 + (i - 232) * 10;
    return (uint32_t)(g << 16 | g << 8 | g);
}

static int dist(uint32_t a, uint32_t b)
{
    int dr = (int)(a >> 16 & 255) - (int)(b >> 16 & 255);
    int dg = (int)(a >> 8 & 255) - (int)(b >> 8 & 255);
    int db = (int)(a & 255) - (int)(b & 255);
    return 3 * dr * dr + 4 * dg * dg + 2 * db * db;
}

// Nearest colours of every 15-bit RGB, made on first use.
static uint8_t *lut256, *lut16;

static uint8_t *make_lut(int first, int n)
{
    uint8_t *lut = malloc(32768);
    if (!lut)
        abort();
    for (int i = 0; i < 32768; i++) {
        int r = (i >> 10) & 31, g = (i >> 5) & 31, b = i & 31;
        uint32_t rgb = (uint32_t)((r * 255 / 31) << 16 | (g * 255 / 31) << 8 | (b * 255 / 31));
        int best = first, bd = 1 << 30;
        for (int c = first; c < first + n; c++) {
            int d = dist(rgb, xterm256(c));
            if (d < bd) {
                bd = d;
                best = c;
            }
        }
        lut[i] = (uint8_t)best;
    }
    return lut;
}

static inline int idx15(uint32_t rgb) { return (int)((rgb >> 9 & 0x7c00) | (rgb >> 6 & 0x3e0) | (rgb >> 3 & 31)); }

int rt_rgb_to_256(uint32_t rgb)
{
    if (!lut256)
        lut256 = make_lut(16, 240);  // not 0-15: those are the terminal theme's
    return lut256[idx15(rgb)];
}

int rt_rgb_to_16(uint32_t rgb)
{
    if (!lut16)
        lut16 = make_lut(0, 16);
    return lut16[idx15(rgb)];
}

static uint8_t qtab[256];
static int qtab_bits = -1;

static void make_qtab(int bits)
{
    if (bits == qtab_bits || bits < 1 || bits >= 8)
        return;
    int levels = (1 << bits) - 1;
    for (int v = 0; v < 256; v++)
        qtab[v] = (uint8_t)(((v * levels + 127) / 255) * 255 / levels);
    qtab_bits = bits;
}

void rt_set_quant(rt_t *r, int bits)
{
    r->quant = bits < 1 ? 1 : bits > 8 ? 8 : bits;
    make_qtab(r->quant);
}

static inline uint32_t map_color(rt_t *r, uint32_t rgb)
{
    switch (r->color) {
    case RT_TRUECOLOR:
        if (r->quant >= 8)
            return rgb & 0xffffff;
        return (uint32_t)qtab[rgb >> 16 & 255] << 16 | (uint32_t)qtab[rgb >> 8 & 255] << 8 | qtab[rgb & 255];
    case RT_256:
        return (uint32_t)rt_rgb_to_256(rgb);
    case RT_16:
        return (uint32_t)rt_rgb_to_16(rgb);
    default:
        return 0;
    }
}

static char *put_color(char *p, rt_t *r, uint32_t c, int bg)
{
    if (c == RT_NONE) {
        *p++ = bg ? '4' : '3';
        *p++ = '9';
        return p;
    }
    switch (r->color) {
    case RT_TRUECOLOR:
        *p++ = bg ? '4' : '3';
        memcpy(p, "8;2;", 4);
        p += 4;
        p = put_uint(p, c >> 16 & 255);
        *p++ = ';';
        p = put_uint(p, c >> 8 & 255);
        *p++ = ';';
        return put_uint(p, c & 255);
    case RT_256:
        *p++ = bg ? '4' : '3';
        memcpy(p, "8;5;", 4);
        p += 4;
        return put_uint(p, c);
    default:  // 16
        if (c < 8) {
            *p++ = bg ? '4' : '3';
            *p++ = (char)('0' + c);
        } else {
            if (bg) {
                memcpy(p, "10", 2);
                p += 2;
            } else {
                *p++ = '9';
            }
            *p++ = (char)('0' + c - 8);
        }
        return p;
    }
}

// Sets the colours that differ from the terminal's.
static char *put_sgr(char *p, rt_t *r, uint32_t fg, uint32_t bg)
{
    int f = fg != r->tfg, b = bg != r->tbg;
    if (!f && !b)
        return p;
    *p++ = '\x1b';
    *p++ = '[';
    if (f)
        p = put_color(p, r, fg, 0);
    if (f && b)
        *p++ = ';';
    if (b)
        p = put_color(p, r, bg, 1);
    *p++ = 'm';
    r->tfg = fg;
    r->tbg = bg;
    return p;
}

void rt_set_colors(rt_t *r, uint32_t fg, uint32_t bg)
{
    reserve(r, 64);
    char *p = put_sgr(r->out + r->len, r, fg, bg);
    r->len = (size_t)(p - r->out);
}

const char *rt_blocks_name(int blocks)
{
    static const char *n[] = {"half", "quad", "ascii"};
    return blocks >= 0 && blocks < RT_BLOCKS_N ? n[blocks] : "?";
}

const char *rt_color_name(int color)
{
    static const char *n[] = {"truecolor", "256", "16", "mono"};
    return color >= 0 && color < RT_COLORS_N ? n[color] : "?";
}

// ── layout ───────────────────────────────────────────────────────────────────────────────────────
void rt_init(rt_t *r, int sw, int sh)
{
    memset(r, 0, sizeof *r);
    r->sw = sw;
    r->sh = sh;
    r->blocks = RT_HALF;
    r->color = RT_TRUECOLOR;
    r->aspect = 4.0 / 3.0;
    r->cell_aspect = 2.0;
    r->cellw = 1;
    r->quant = 8;
    r->tx = r->ty = -1;
    r->tfg = r->tbg = RT_NONE;
    r->cell_bytes = 24;
}

void rt_free(rt_t *r)
{
    free(r->xs);
    free(r->ys);
    free(r->pix);
    free(r->cur);
    free(r->valid);
    free(r->want);
    free(r->err);
    free(r->out);
    memset(r, 0, sizeof *r);
}

void rt_invalidate(rt_t *r)
{
    if (r->valid)
        memset(r->valid, 0, (size_t)r->cw * (size_t)r->ch);
    r->tx = r->ty = -1;
    r->tfg = r->tbg = RT_NONE - 1;  // unknown: the first colour is always set
}

static int clampi(int v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : v; }

void rt_layout(rt_t *r, int cols, int rows, int top)
{
    r->cols = cols < 1 ? 1 : cols;
    r->rows = rows < 1 ? 1 : rows;
    r->top = top;
    int wide = r->blocks != RT_ASCII && r->cellw == 2 ? 2 : 1;
    int C = r->cols / wide, R = r->rows;
    if (r->max_cols > 0 && C > r->max_cols / wide)
        C = r->max_cols / wide > 0 ? r->max_cols / wide : 1;
    if (r->max_rows > 0 && R > r->max_rows)
        R = r->max_rows;
    int cw, ch;
    if (r->stretch) {
        cw = C;
        ch = R;
    } else {
        double ca = (r->cell_aspect > 0.2 ? r->cell_aspect : 2.0) / wide;
        double want = r->aspect > 0.1 ? r->aspect : 4.0 / 3.0;
        if ((double)C / (R * ca) > want) {
            ch = R;
            cw = (int)lround(R * ca * want);
        } else {
            cw = C;
            ch = (int)lround(C / (ca * want));
        }
    }
    cw = clampi(cw, 1, C);
    ch = clampi(ch, 1, R);
    r->cw = cw;
    r->ch = ch;
    r->wide = wide;
    r->ox = (r->cols - cw * wide) / 2;
    r->oy = top + (r->rows - ch) / 2;
    switch (r->blocks) {
    case RT_QUAD:
        r->pw = cw * 2;
        r->ph = ch * 2;
        break;
    case RT_ASCII:
        r->pw = cw;
        r->ph = ch;
        break;
    default:
        r->pw = cw;
        r->ph = ch * 2;
        break;
    }
    free(r->xs);
    free(r->ys);
    free(r->pix);
    free(r->cur);
    free(r->valid);
    free(r->want);
    free(r->err);
    r->xs = malloc(sizeof(int) * (size_t)(r->pw + 1));
    r->ys = malloc(sizeof(int) * (size_t)(r->ph + 1));
    r->pix = malloc(sizeof(uint32_t) * (size_t)r->pw * (size_t)r->ph);
    r->cur = calloc((size_t)cw * (size_t)ch, sizeof(rt_cell));
    r->valid = calloc((size_t)cw * (size_t)ch, 1);
    r->want = calloc((size_t)cw * (size_t)ch, sizeof(rt_cell));
    r->err = calloc((size_t)cw * (size_t)ch, sizeof(uint32_t));
    if (!r->xs || !r->ys || !r->pix || !r->cur || !r->valid || !r->want || !r->err)
        abort();
    for (int i = 0; i <= r->pw; i++)
        r->xs[i] = (int)((long long)i * r->sw / r->pw);
    for (int i = 0; i <= r->ph; i++)
        r->ys[i] = (int)((long long)i * r->sh / r->ph);
    make_qtab(r->quant);
    rt_invalidate(r);
}

// ── a frame ──────────────────────────────────────────────────────────────────────────────────────
// The picture: each pixel the mean of the source pixels it covers (a box filter).
static void scale(rt_t *r, const uint32_t *src)
{
    const int sw = r->sw, sh = r->sh;
    uint32_t *o = r->pix;
    for (int y = 0; y < r->ph; y++) {
        int y0 = r->ys[y], y1 = r->ys[y + 1];
        if (y0 >= sh)
            y0 = sh - 1;
        if (y1 <= y0)
            y1 = y0 + 1;
        for (int x = 0; x < r->pw; x++) {
            int x0 = r->xs[x], x1 = r->xs[x + 1];
            if (x0 >= sw)
                x0 = sw - 1;
            if (x1 <= x0)
                x1 = x0 + 1;
            unsigned rs = 0, gs = 0, bs = 0, n = (unsigned)((x1 - x0) * (y1 - y0));
            for (int yy = y0; yy < y1; yy++) {
                const uint32_t *row = src + (size_t)yy * sw;
                for (int xx = x0; xx < x1; xx++) {
                    uint32_t p = row[xx];
                    rs += p >> 16 & 255;
                    gs += p >> 8 & 255;
                    bs += p & 255;
                }
            }
            *o++ = (rs + n / 2) / n << 16 | (gs + n / 2) / n << 8 | (bs + n / 2) / n;
        }
    }
}

static const uint32_t quad_char[16] = {
    ' ', 0x2598, 0x259d, 0x2580, 0x2596, 0x258c, 0x259e, 0x259b,
    0x2597, 0x259a, 0x2590, 0x259c, 0x2584, 0x2599, 0x259f, 0x2588,
};

// Brightness ramp for ASCII cells, dark to light.
static const char ramp[] = " .'`^\",:;Il!i><~+_-?][}{1)(|/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";

static inline int luma(uint32_t p)
{
    return (int)(((p >> 16 & 255) * 2126 + (p >> 8 & 255) * 7152 + (p & 255) * 722) / 10000);
}

// The two colours of four pixels: the split of them in two groups with the least error.
static void quad_cell(rt_t *r, const uint32_t q[4], rt_cell *c)
{
    int best_err = -1, best_m = 0;
    uint32_t best_a = 0, best_b = 0;
    for (int m = 0; m < 8; m++) {  // masks without BR: BR is always in group b
        unsigned sa[3] = {0, 0, 0}, sb[3] = {0, 0, 0}, na = 0, nb = 0;
        for (int i = 0; i < 4; i++) {
            unsigned *s = (m >> i & 1) ? sa : sb;
            s[0] += q[i] >> 16 & 255;
            s[1] += q[i] >> 8 & 255;
            s[2] += q[i] & 255;
            if (m >> i & 1)
                na++;
            else
                nb++;
        }
        uint32_t a = na ? ((sa[0] + na / 2) / na << 16 | (sa[1] + na / 2) / na << 8 | (sa[2] + na / 2) / na) : 0;
        uint32_t b = (sb[0] + nb / 2) / nb << 16 | (sb[1] + nb / 2) / nb << 8 | (sb[2] + nb / 2) / nb;
        int err = 0;
        for (int i = 0; i < 4; i++)
            err += dist(q[i], (m >> i & 1) ? a : b);
        if (best_err < 0 || err < best_err) {
            best_err = err;
            best_m = m;
            best_a = a;
            best_b = b;
        }
    }
    uint32_t b = map_color(r, best_b);
    uint32_t a = best_m ? map_color(r, best_a) : b;
    if (a == b)
        best_m = 0;
    c->a = best_m ? a : b;
    c->b = b;
    c->c = (uint32_t)best_m;
}

static void compute(rt_t *r, int cx, int cy, rt_cell *c)
{
    const uint32_t *pix = r->pix;
    int pw = r->pw;
    switch (r->blocks) {
    case RT_QUAD: {
        const uint32_t *p0 = pix + (size_t)(cy * 2) * pw + cx * 2, *p1 = p0 + pw;
        uint32_t q[4] = {p0[0], p0[1], p1[0], p1[1]};
        if (q[0] == q[1] && q[0] == q[2] && q[0] == q[3]) {
            c->a = c->b = map_color(r, q[0]);
            c->c = 0;
        } else {
            quad_cell(r, q, c);
        }
        break;
    }
    case RT_ASCII: {
        uint32_t p = pix[(size_t)cy * pw + cx];
        int l = luma(p);
        l = (int)(sqrt(l / 255.0) * 255.0 + 0.5);  // lift the darks: DOOM is a dark game
        c->c = (uint32_t)ramp[l * (int)(sizeof ramp - 2) / 255];
        c->a = c->c == ' ' ? 0 : map_color(r, p);
        c->b = 0;
        break;
    }
    default: {
        const uint32_t *p0 = pix + (size_t)(cy * 2) * pw + cx;
        c->a = map_color(r, p0[0]);
        c->b = map_color(r, p0[pw]);
        c->c = 0;
        break;
    }
    }
}

// A cell of one colour: a space on that background, or a full block in that foreground —
// whichever needs no colour change.
static char *put_solid(char *p, rt_t *r, uint32_t col)
{
    if (r->tfg == col && r->tbg != col) {
        p = put_utf8(p, 0x2588);
    } else {
        if (r->tbg != col)
            p = put_sgr(p, r, r->tfg, col);
        *p++ = ' ';
        if (r->wide == 2)  // a space is one column where a block character is two
            *p++ = ' ';
    }
    return p;
}

static char *put_cell(char *p, rt_t *r, const rt_cell *c)
{
    switch (r->blocks) {
    case RT_ASCII:
        if (r->color == RT_MONO || c->c == ' ') {
            if (r->tbg != RT_NONE)
                p = put_sgr(p, r, r->color == RT_MONO ? RT_NONE : r->tfg, RT_NONE);
        } else {
            p = put_sgr(p, r, c->a, RT_NONE);
        }
        *p++ = (char)c->c;
        return p;
    case RT_QUAD: {
        if (c->c == 0)
            return put_solid(p, r, c->b);
        unsigned m = c->c;
        int k1 = (r->tfg != c->a) + (r->tbg != c->b), k2 = (r->tfg != c->b) + (r->tbg != c->a);
        if (k1 <= k2) {
            p = put_sgr(p, r, c->a, c->b);
        } else {
            p = put_sgr(p, r, c->b, c->a);
            m ^= 15;
        }
        return put_utf8(p, quad_char[m]);
    }
    default: {
        if (c->a == c->b)
            return put_solid(p, r, c->a);
        int k1 = (r->tfg != c->a) + (r->tbg != c->b), k2 = (r->tfg != c->b) + (r->tbg != c->a);
        if (k1 <= k2) {
            p = put_sgr(p, r, c->a, c->b);
            return put_utf8(p, 0x2580);  // ▀
        }
        p = put_sgr(p, r, c->b, c->a);
        return put_utf8(p, 0x2584);  // ▄
    }
    }
}

static char *put_move(char *p, rt_t *r, int x, int y)
{
    if (r->ty == y && r->tx == x)
        return p;
    *p++ = '\x1b';
    *p++ = '[';
    if (r->ty == y && r->tx >= 0 && x > r->tx) {
        if (x - r->tx > 1)
            p = put_uint(p, (unsigned)(x - r->tx));
        *p++ = 'C';
    } else {
        p = put_uint(p, (unsigned)y + 1);
        *p++ = ';';
        p = put_uint(p, (unsigned)x + 1);
        *p++ = 'H';
    }
    r->tx = x;
    r->ty = y;
    return p;
}

// What a cell's colour value looks like (RGB).
static inline uint32_t shown(const rt_t *r, uint32_t c)
{
    switch (r->color) {
    case RT_TRUECOLOR: return c;
    case RT_256: return xterm256((int)c);
    case RT_16: return ansi16[c & 15];
    default: return 0xc0c0c0;
    }
}

// How different two cells look: 0 the same.
static uint32_t cell_error(const rt_t *r, const rt_cell *w, const rt_cell *c)
{
    if (w->a == c->a && w->b == c->b && w->c == c->c)
        return 0;
    switch (r->blocks) {
    case RT_QUAD: {
        uint32_t e = 0;
        for (int i = 0; i < 4; i++) {
            uint32_t x = (w->c >> i & 1) ? w->a : w->b, y = (c->c >> i & 1) ? c->a : c->b;
            if (x != y)
                e += (uint32_t)dist(shown(r, x), shown(r, y));
        }
        return e / 2 + 1;
    }
    case RT_ASCII:
        return (w->c != c->c ? 60000u : 0u) + (uint32_t)dist(shown(r, w->a), shown(r, c->a)) + 1;
    default:
        return (uint32_t)dist(shown(r, w->a), shown(r, c->a)) + (uint32_t)dist(shown(r, w->b), shown(r, c->b)) + 1;
    }
}

// Errors in 256 buckets: eight a power of two (0: no error; 256: never drawn)
#define NBUCKETS 257
static inline int bucket_of(uint32_t e)
{
    if (e == 0)
        return 0;
    int x = 31 - __builtin_clz(e);
    unsigned m = x >= 3 ? e >> (x - 3) & 7 : e << (3 - x) & 7;
    return x * 8 + (int)m + 1;
}

int rt_frame(rt_t *r, const uint32_t *src)
{
    scale(r, src);
    size_t n = (size_t)r->cw * r->ch;
    unsigned hist[NBUCKETS] = {0};
    for (int cy = 0; cy < r->ch; cy++) {
        for (int cx = 0; cx < r->cw; cx++) {
            size_t i = (size_t)cy * r->cw + cx;
            compute(r, cx, cy, &r->want[i]);
            uint32_t e = r->valid[i] ? cell_error(r, &r->want[i], &r->cur[i]) : 0xffffffffu;
            r->err[i] = e;
            hist[bucket_of(e)]++;
        }
    }
    // The cells to send: those that look most wrong, as many as the budget takes — all of the
    // buckets above `edge`, a share of `edge`'s, spread over the picture (cells never drawn
    // always: the picture must be whole)
    int edge = 0;
    double share = 1.0;
    if (r->budget > 0) {
        double acc = 0, cost = r->cell_bytes > 4 ? r->cell_bytes : 4;
        for (int b = NBUCKETS - 1; b >= 1; b--) {
            double need = hist[b] * cost;
            if (acc + need > (double)r->budget && b < NBUCKETS - 1) {
                edge = b;
                share = ((double)r->budget - acc) / need;
                if (share < 0)
                    share = 0;
                break;
            }
            acc += need;
        }
    }
    double pick = 0.5;
    int sent = 0, deferred = 0;
    size_t start = r->len;
    for (int cy = 0; cy < r->ch; cy++) {
        for (int cx = 0; cx < r->cw; cx++) {
            size_t i = (size_t)cy * r->cw + cx;
            uint32_t e = r->err[i];
            if (e == 0)
                continue;
            int b = bucket_of(e);
            if (b < edge || (b == edge && (pick += share) < 1.0)) {
                deferred++;
                continue;
            }
            if (b == edge)
                pick -= 1.0;
            rt_cell c = r->want[i];
            r->cur[i] = c;
            r->valid[i] = 1;
            sent++;
            reserve(r, 96);
            char *p = r->out + r->len;
            int x = r->ox + cx * r->wide, y = r->oy + cy;
            p = put_move(p, r, x, y);
            p = put_cell(p, r, &c);
            r->tx = x + r->wide < r->cols ? x + r->wide : -1;  // at the last column the cursor stays put
            r->len = (size_t)(p - r->out);
        }
    }
    (void)n;
    if (sent > 0)
        r->cell_bytes = r->cell_bytes * 0.8 + 0.2 * (double)(r->len - start) / sent;
    r->deferred = deferred;
    r->cells_sent += (unsigned long)sent;
    return sent;
}
