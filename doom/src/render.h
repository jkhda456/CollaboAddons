// The doom add-on's terminal renderer: a frame of RGB pixels (DOOM's 320x200) drawn as character
// cells — scaled to the terminal in real time, with only the cells that changed sent again.
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#ifndef DOOM_TERM_RENDER_H
#define DOOM_TERM_RENDER_H

#include <stddef.h>
#include <stdint.h>

// How a cell shows pixels.
enum {
    RT_HALF,   // "▀": two pixels a cell, one above the other (fore- and background colour)
    RT_QUAD,   // quadrant blocks "▘▝▀▖▌▞▛▗▚▐▜▄▙▟█": 2x2 pixels a cell, in two colours
    RT_ASCII,  // a character per cell by its brightness (" .:-=+*#%@"), coloured unless mono
    RT_BLOCKS_N
};

// The colours the terminal takes.
enum {
    RT_TRUECOLOR,  // 24-bit "38;2;r;g;b"
    RT_256,        // xterm's 256 colours (the 6x6x6 cube and the grey ramp)
    RT_16,         // the 16 ANSI colours
    RT_MONO,       // none: characters only (ASCII)
    RT_COLORS_N
};

typedef struct {
    uint32_t a, b;  // colours (0xRRGGBB, or a palette index); for RT_ASCII: a = colour
    uint32_t c;     // RT_QUAD: which quadrants show a (TL=1 TR=2 BL=4 BR=8); RT_ASCII: the char
} rt_cell;

typedef struct {
    // settings: set before rt_layout
    int blocks, color;
    int stretch;          // fill the area, not DOOM's 4:3
    double aspect;        // the picture's width / height (4:3)
    double cell_aspect;   // a column's height / width on the screen (2.0)
    int cellw;            // columns a block character takes: 1, or 2 where the terminal draws
                          // East Asian ambiguous-width characters wide
    int quant;            // bits kept of each colour channel (truecolor; 8 = all)
    int max_cols, max_rows;  // the picture no larger than this (0: no limit)
    size_t budget;        // bytes a frame may take (0: no limit): the cells that look most wrong
                          // go first, the rest in a later frame

    // the source
    int sw, sh;

    // the area to draw in: rt_layout
    int cols, rows, top;
    int ox, oy, cw, ch;   // the picture: first column and row (0-based), its size in cells
    int wide;             // columns a cell takes (cellw, but 1 for ASCII)
    int pw, ph;           // ... and in pixels
    int *xs, *ys;         // source boundaries of the pixels (pw+1, ph+1)
    uint32_t *pix;        // the scaled picture (pw*ph)
    rt_cell *cur;         // what the terminal shows (cw*ch); valid[] says which
    uint8_t *valid;
    rt_cell *want;        // this frame's cells
    uint32_t *err;        // how far what the terminal shows is from them
    double cell_bytes;    // bytes a cell takes, on average (for the budget)

    // the terminal's state while a frame is written
    int tx, ty;           // cursor (-1: unknown)
    uint32_t tfg, tbg;    // colours set (RT_NONE: unknown)

    // the output
    char *out;
    size_t len, cap;

    // statistics
    unsigned long cells_sent;
    int deferred;         // cells of the last frame left for later (the budget)
} rt_t;

#define RT_NONE 0xffffffffu

void rt_init(rt_t *r, int sw, int sh);
void rt_free(rt_t *r);
// The area: cols x rows cells, starting at row `top` (0-based). Forgets what the terminal shows.
void rt_layout(rt_t *r, int cols, int rows, int top);
// Forget what the terminal shows (it was cleared): the next frame is drawn whole.
void rt_invalidate(rt_t *r);
// Bits kept of each colour channel (truecolor) from the next frame on; what is shown stays.
void rt_set_quant(rt_t *r, int bits);
// Appends the escape sequences that bring the terminal to `src` (sw*sh pixels, 0x??RRGGBB) to
// r->out — as far as r->budget goes. Returns how many cells were sent.
int rt_frame(rt_t *r, const uint32_t *src);
// Appends text to the output buffer.
void rt_put(rt_t *r, const char *s, size_t n);
void rt_puts(rt_t *r, const char *s);
void rt_printf(rt_t *r, const char *fmt, ...);
// Sets the terminal's colours to fg/bg (as rt_frame's cells; RT_NONE: the default) at once.
void rt_set_colors(rt_t *r, uint32_t fg, uint32_t bg);
// The 256-colour index of an RGB colour (16..255), the 16-colour one (0..15).
int rt_rgb_to_256(uint32_t rgb);
int rt_rgb_to_16(uint32_t rgb);
const char *rt_blocks_name(int blocks);
const char *rt_color_name(int color);

#endif
