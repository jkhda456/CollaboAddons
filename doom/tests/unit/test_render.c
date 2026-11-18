// Unit tests of the terminal renderer (src/render.c): run natively and in the guest. (What a
// terminal shows after its output is tested with a terminal emulator: tests/test_guest.py.)
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "render.h"

static int fails, checks;

#define CHECK(cond, ...)                                    \
    do {                                                    \
        checks++;                                           \
        if (!(cond)) {                                      \
            fails++;                                        \
            printf("FAIL %s:%d: ", __FILE__, __LINE__);     \
            printf(__VA_ARGS__);                            \
            printf("\n");                                   \
        }                                                   \
    } while (0)

#define SW 320
#define SH 200
static uint32_t frame[SW * SH];

static void fill(uint32_t c)
{
    for (int i = 0; i < SW * SH; i++)
        frame[i] = c;
}

static uint32_t rnd_state = 12345;
static uint32_t rnd(void)
{
    rnd_state = rnd_state * 1103515245u + 12345u;
    return rnd_state >> 8;
}

static void noise(void)
{
    for (int i = 0; i < SW * SH; i++)
        frame[i] = rnd() & 0xffffff;
}

static void layout_case(int blocks, int cellw, int cols, int rows, int cw, int ch, int ox, int oy)
{
    rt_t r;
    rt_init(&r, SW, SH);
    r.blocks = blocks;
    r.cellw = cellw;
    rt_layout(&r, cols, rows, 0);
    CHECK(r.cw == cw && r.ch == ch && r.ox == ox && r.oy == oy,
          "%s cellw %d %dx%d: picture %dx%d at %d,%d, not %dx%d at %d,%d", rt_blocks_name(blocks), cellw, cols, rows,
          r.cw, r.ch, r.ox, r.oy, cw, ch, ox, oy);
    int px = blocks == RT_QUAD ? 2 : 1, py = blocks == RT_ASCII ? 1 : 2;
    CHECK(r.pw == cw * px && r.ph == ch * py, "pixels %dx%d", r.pw, r.ph);
    rt_free(&r);
}

static size_t count(const char *s, size_t n, const char *what)
{
    size_t k = 0, m = strlen(what);
    for (size_t i = 0; i + m <= n; i++)
        if (memcmp(s + i, what, m) == 0)
            k++;
    return k;
}

int main(void)
{
    // the picture keeps 4:3 with cells twice as high as wide, centred
    layout_case(RT_HALF, 1, 80, 25, 67, 25, 6, 0);
    layout_case(RT_HALF, 1, 80, 24, 64, 24, 8, 0);
    layout_case(RT_HALF, 1, 200, 30, 80, 30, 60, 0);
    layout_case(RT_HALF, 1, 40, 50, 40, 15, 0, 17);
    layout_case(RT_QUAD, 1, 80, 25, 67, 25, 6, 0);
    layout_case(RT_ASCII, 1, 80, 25, 67, 25, 6, 0);
    // a terminal that draws block characters two columns wide: a cell is two columns
    layout_case(RT_HALF, 2, 160, 50, 67, 50, 13, 0);
    layout_case(RT_ASCII, 2, 80, 25, 67, 25, 6, 0);  // ASCII is narrow anyway
    {
        rt_t r;
        rt_init(&r, SW, SH);
        r.stretch = 1;
        rt_layout(&r, 100, 30, 2);
        CHECK(r.cw == 100 && r.ch == 30 && r.oy == 2, "stretch: %dx%d at row %d", r.cw, r.ch, r.oy);
        r.stretch = 0;
        r.max_cols = 40;
        r.max_rows = 12;
        rt_layout(&r, 100, 30, 0);
        CHECK(r.cw == 32 && r.ch == 12, "-size 40x12: %dx%d", r.cw, r.ch);
        rt_free(&r);
    }

    // colours: the nearest of the palettes
    CHECK(rt_rgb_to_256(0xff0000) == 196, "red in 256 colours: %d", rt_rgb_to_256(0xff0000));
    CHECK(rt_rgb_to_256(0x808080) == 244, "grey in 256 colours: %d", rt_rgb_to_256(0x808080));
    CHECK(rt_rgb_to_16(0xff0000) == 1, "red in 16 colours: %d", rt_rgb_to_16(0xff0000));
    CHECK(rt_rgb_to_16(0x000000) == 0, "black in 16 colours: %d", rt_rgb_to_16(0));
    CHECK(rt_rgb_to_16(0xff6060) == 9, "light red in 16 colours: %d", rt_rgb_to_16(0xff6060));

    for (int blocks = 0; blocks < RT_BLOCKS_N; blocks++) {
        for (int color = 0; color < RT_COLORS_N; color++) {
            if (color == RT_MONO && blocks != RT_ASCII)
                continue;
            rt_t r;
            rt_init(&r, SW, SH);
            r.blocks = blocks;
            r.color = color;
            r.quant = 6;
            rt_layout(&r, 80, 25, 0);
            const char *name = rt_blocks_name(blocks), *cname = rt_color_name(color);
            // a frame of one colour: every cell, then nothing more for the same frame
            fill(0x804020);
            int n = rt_frame(&r, frame);
            CHECK(n == r.cw * r.ch || (blocks == RT_ASCII && n > 0), "%s/%s: first frame sends %d cells", name, cname, n);
            r.len = 0;
            n = rt_frame(&r, frame);
            CHECK(n == 0 && r.len == 0, "%s/%s: the same frame again sends %d cells, %zu bytes", name, cname, n, r.len);
            // noise: all of it, then the colour sequences: as many as the colours
            noise();
            r.len = 0;
            n = rt_frame(&r, frame);
            CHECK(n > r.cw * r.ch / 2, "%s/%s: noise sends %d cells", name, cname, n);
            if (color == RT_TRUECOLOR)
                CHECK(count(r.out, r.len, "38;2;") + count(r.out, r.len, "48;2;") > 0, "%s: 24-bit colours", name);
            if (color == RT_256)
                CHECK(count(r.out, r.len, "38;5;") + count(r.out, r.len, "48;5;") > 0, "%s: 256 colours", name);
            if (color == RT_MONO)
                CHECK(count(r.out, r.len, "38;") == 0, "mono: no colours");
            CHECK(count(r.out, r.len, "\n") == 0 && count(r.out, r.len, "\r") == 0, "%s/%s: no line feeds (cursor moves)", name, cname);
            rt_free(&r);
        }
    }

    // the budget: the cells that look most wrong first, the rest later; cells never drawn always
    {
        rt_t r;
        rt_init(&r, SW, SH);
        rt_layout(&r, 160, 50, 0);
        noise();
        r.budget = 1000;
        r.len = 0;
        int n = rt_frame(&r, frame);
        CHECK(n == r.cw * r.ch, "a budget does not stop the first frame: %d cells", n);
        int frames = 0;
        noise();
        do {
            r.len = 0;
            n = rt_frame(&r, frame);
            frames++;
            CHECK(r.len < 1000 * 2 + 200, "frame %d: %zu bytes, budget 1000", frames, r.len);
        } while (r.deferred > 0 && frames < 2000);
        CHECK(frames > 5 && r.deferred == 0, "a new picture comes in %d frames on a budget", frames);
        r.len = 0;
        CHECK(rt_frame(&r, frame) == 0, "then it is whole");

        // a big change and a small one: the big one goes first
        fill(0x000000);
        r.budget = 0;
        r.len = 0;
        rt_frame(&r, frame);
        for (int y = 0; y < SH; y++)
            for (int x = 0; x < SW; x++)
                frame[y * SW + x] = x < SW / 2 ? 0xffffff : 0x080808;
        r.budget = 400;
        r.len = 0;
        rt_frame(&r, frame);
        int left = 0, right = 0;
        for (int y = 0; y < r.ch; y++)
            for (int x = 0; x < r.cw; x++)
                if (r.cur[y * r.cw + x].a != 0) {
                    if (x < r.cw / 2)
                        left++;
                    else
                        right++;
                }
        CHECK(left > 0 && right == 0, "budget: the white half first (%d cells), not the dark grey (%d)", left, right);
        rt_free(&r);
    }

    // rt_set_quant: fewer bits, fewer colours
    {
        rt_t r;
        rt_init(&r, SW, SH);
        r.quant = 8;
        rt_layout(&r, 80, 25, 0);
        fill(0x7b3d11);
        rt_frame(&r, frame);
        CHECK(r.cur[0].a == 0x7b3d11, "8 bits: %06x", r.cur[0].a);
        rt_set_quant(&r, 3);
        r.len = 0;
        rt_frame(&r, frame);
        CHECK(r.cur[0].a == 0x6d4800, "3 bits: %06x, not 6d4800", r.cur[0].a);
        rt_free(&r);
    }

    printf("test_render: %d checks, %d failed\n", checks, fails);
    return fails != 0;
}
