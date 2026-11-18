// The doom add-on's terminal input: the bytes a terminal sends for keys, decoded — legacy escape
// sequences (xterm, VT220, the Linux console), the kitty keyboard protocol and Windows Terminal's
// win32-input-mode (both of which say when a key is let go), answers to the queries.
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#ifndef DOOM_TERM_INPUT_H
#define DOOM_TERM_INPUT_H

#include <stdint.h>

// Keys that are no character: above Unicode.
enum {
    TK_NONE = 0,
    TK_BASE = 0x110000,
    TK_UP, TK_DOWN, TK_LEFT, TK_RIGHT,
    TK_HOME, TK_END, TK_PGUP, TK_PGDN, TK_INSERT, TK_DELETE,
    TK_F1, TK_F2, TK_F3, TK_F4, TK_F5, TK_F6, TK_F7, TK_F8, TK_F9, TK_F10, TK_F11, TK_F12,
    TK_PAUSE,
    TK_SHIFT, TK_CTRL, TK_ALT,  // the modifier keys themselves (kitty, win32-input-mode)
    TK_ESCAPE = 27, TK_ENTER = 13, TK_TAB = 9, TK_BACKSPACE = 127,
};

enum { TM_SHIFT = 1, TM_ALT = 2, TM_CTRL = 4 };

enum {
    TE_PRESS = 1,    // a key went down (legacy: down or repeated, which is all a terminal says)
    TE_REPEAT = 2,   // held down (kitty, win32-input-mode)
    TE_RELEASE = 3,  // let go (kitty, win32-input-mode)
    TE_KITTY = 10,   // the terminal speaks the kitty keyboard protocol (its answer to CSI ? u)
    TE_DA = 11,      // the answer to the device attributes query (CSI c): the queries are answered
    TE_CPR = 12,     // a cursor position report (CSI 6 n): key = row, text = column (1-based)
};

typedef struct {
    int type;        // TE_*
    uint32_t key;    // a character (lower case for letters: the key) or a TK_* key
    uint32_t text;   // the character it types (with shift: 'W'), 0 if none
    int mods;        // TM_*
    int exact;       // 1: the terminal reports releases for this key (kitty, win32)
} tk_event;

typedef struct {
    unsigned char buf[256];
    int len;
    uint32_t esc_since;  // when a lone ESC (or an unfinished sequence) started waiting, ms
} tk_decoder;

void tk_init(tk_decoder *d);
// Adds bytes the terminal sent.
void tk_feed(tk_decoder *d, const unsigned char *bytes, int n, uint32_t now_ms);
// The next event, 1 if there is one. A lone ESC is a key once nothing has followed it for
// `esc_ms` milliseconds (an escape sequence comes in one piece).
int tk_next(tk_decoder *d, tk_event *ev, uint32_t now_ms, uint32_t esc_ms);
const char *tk_key_name(uint32_t key, char buf[16]);

#endif
