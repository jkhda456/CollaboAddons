// Unit tests of the terminal input decoder (src/input.c): run natively and in the guest.
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include <stdio.h>
#include <string.h>

#include "input.h"

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

// Decodes s (fed in pieces of `step` bytes, 0: at once) at time 0, then at `later` ms; up to 16 events.
static int decode(const char *s, int step, uint32_t later, tk_event *ev)
{
    tk_decoder d;
    tk_init(&d);
    int n = 0, len = (int)strlen(s);
    if (step <= 0)
        step = len ? len : 1;
    for (int i = 0; i < len; i += step) {
        int k = len - i < step ? len - i : step;
        tk_feed(&d, (const unsigned char *)s + i, k, 0);
        while (n < 16 && tk_next(&d, &ev[n], 0, 40))
            n++;
    }
    while (n < 16 && tk_next(&d, &ev[n], later, 40))
        n++;
    return n;
}

static void one(const char *what, const char *s, int type, uint32_t key, uint32_t text, int mods, int exact)
{
    for (int step = 0; step <= 1; step++) {  // at once, and a byte at a time
        tk_event ev[16];
        int n = decode(s, step, 100, ev);
        CHECK(n == 1, "%s (step %d): %d events", what, step, n);
        if (n < 1)
            continue;
        CHECK(ev[0].type == type, "%s: type %d, not %d", what, ev[0].type, type);
        CHECK(ev[0].key == key, "%s: key %#x, not %#x", what, ev[0].key, key);
        CHECK(ev[0].text == text, "%s: text %#x, not %#x", what, ev[0].text, text);
        CHECK(ev[0].mods == mods, "%s: mods %d, not %d", what, ev[0].mods, mods);
        CHECK(ev[0].exact == exact, "%s: exact %d, not %d", what, ev[0].exact, exact);
    }
}

int main(void)
{
    // characters
    one("w", "w", TE_PRESS, 'w', 'w', 0, 0);
    one("W: shift", "W", TE_PRESS, 'w', 'W', TM_SHIFT, 0);
    one("space", " ", TE_PRESS, ' ', ' ', 0, 0);
    one("digit", "3", TE_PRESS, '3', '3', 0, 0);
    one("enter (CR)", "\r", TE_PRESS, TK_ENTER, '\r', 0, 0);
    one("enter (LF)", "\n", TE_PRESS, TK_ENTER, '\r', 0, 0);
    one("tab", "\t", TE_PRESS, TK_TAB, '\t', 0, 0);
    one("backspace (DEL)", "\x7f", TE_PRESS, TK_BACKSPACE, 0, 0, 0);
    one("backspace (BS)", "\x08", TE_PRESS, TK_BACKSPACE, 0, 0, 0);
    one("ctrl-c", "\x03", TE_PRESS, 'c', 0, TM_CTRL, 0);
    one("ctrl-l", "\x0c", TE_PRESS, 'l', 0, TM_CTRL, 0);
    one("hangul jamo (UTF-8)", "\xe3\x85\x88", TE_PRESS, 0x3148, 0x3148, 0, 0);  // ㅈ
    one("alt-x", "\x1bx", TE_PRESS, 'x', 'x', TM_ALT, 0);

    // escape: alone only after the wait, at once in a sequence
    {
        tk_decoder d;
        tk_event ev;
        tk_init(&d);
        tk_feed(&d, (const unsigned char *)"\x1b", 1, 1000);
        CHECK(!tk_next(&d, &ev, 1010, 40), "a lone ESC waits");
        CHECK(tk_next(&d, &ev, 1041, 40) && ev.key == TK_ESCAPE, "a lone ESC is the key after 40 ms");
        tk_feed(&d, (const unsigned char *)"\x1b", 1, 2000);
        tk_feed(&d, (const unsigned char *)"[A", 2, 2005);
        CHECK(tk_next(&d, &ev, 2005, 40) && ev.key == TK_UP, "ESC then [A in another read: up");
        tk_feed(&d, (const unsigned char *)"\x1b\x1b", 2, 3000);
        CHECK(tk_next(&d, &ev, 3000, 40) && ev.key == TK_ESCAPE, "ESC ESC: the first an escape");
        CHECK(tk_next(&d, &ev, 3050, 40) && ev.key == TK_ESCAPE, "... and the second");
    }

    // legacy cursor and function keys
    one("up (CSI)", "\x1b[A", TE_PRESS, TK_UP, 0, 0, 0);
    one("down (CSI)", "\x1b[B", TE_PRESS, TK_DOWN, 0, 0, 0);
    one("right (CSI)", "\x1b[C", TE_PRESS, TK_RIGHT, 0, 0, 0);
    one("left (CSI)", "\x1b[D", TE_PRESS, TK_LEFT, 0, 0, 0);
    one("up (SS3: application mode)", "\x1bOA", TE_PRESS, TK_UP, 0, 0, 0);
    one("shift-up", "\x1b[1;2A", TE_PRESS, TK_UP, 0, TM_SHIFT, 0);
    one("alt-left", "\x1b[1;3D", TE_PRESS, TK_LEFT, 0, TM_ALT, 0);
    one("ctrl-right", "\x1b[1;5C", TE_PRESS, TK_RIGHT, 0, TM_CTRL, 0);
    one("home", "\x1b[H", TE_PRESS, TK_HOME, 0, 0, 0);
    one("end (~)", "\x1b[4~", TE_PRESS, TK_END, 0, 0, 0);
    one("page up", "\x1b[5~", TE_PRESS, TK_PGUP, 0, 0, 0);
    one("delete", "\x1b[3~", TE_PRESS, TK_DELETE, 0, 0, 0);
    one("F1 (SS3)", "\x1bOP", TE_PRESS, TK_F1, 0, 0, 0);
    one("F2 (CSI)", "\x1b[1;2Q", TE_PRESS, TK_F2, 0, TM_SHIFT, 0);
    one("F3 (CSI 13~)", "\x1b[13~", TE_PRESS, TK_F3, 0, 0, 0);
    one("F5", "\x1b[15~", TE_PRESS, TK_F5, 0, 0, 0);
    one("F10", "\x1b[21~", TE_PRESS, TK_F10, 0, 0, 0);
    one("F11", "\x1b[23~", TE_PRESS, TK_F11, 0, 0, 0);
    one("F12", "\x1b[24~", TE_PRESS, TK_F12, 0, 0, 0);
    one("F1 (Linux console)", "\x1b[[A", TE_PRESS, TK_F1, 0, 0, 0);
    one("F5 (Linux console)", "\x1b[[E", TE_PRESS, TK_F5, 0, 0, 0);
    one("shift-tab", "\x1b[Z", TE_PRESS, TK_TAB, 0, TM_SHIFT, 0);
    one("keypad enter (SS3)", "\x1bOM", TE_PRESS, TK_ENTER, '\r', 0, 0);
    one("modifyOtherKeys ctrl-w", "\x1b[27;5;119~", TE_PRESS, 'w', 'w', TM_CTRL, 0);

    // the kitty keyboard protocol (flags 1|2|8)
    one("kitty w press", "\x1b[119u", TE_PRESS, 'w', 'w', 0, 1);
    one("kitty w release", "\x1b[119;1:3u", TE_RELEASE, 'w', 'w', 0, 1);
    one("kitty w repeat", "\x1b[119;1:2u", TE_REPEAT, 'w', 'w', 0, 1);
    one("kitty shift-w", "\x1b[119;2u", TE_PRESS, 'w', 'W', TM_SHIFT, 1);
    one("kitty shift-1 with its shifted key", "\x1b[49:33;2u", TE_PRESS, '1', '!', TM_SHIFT, 1);
    one("kitty ctrl-c", "\x1b[99;5u", TE_PRESS, 'c', 'c', TM_CTRL, 1);
    one("kitty left shift press", "\x1b[57441;2u", TE_PRESS, TK_SHIFT, 0, TM_SHIFT, 1);
    one("kitty left ctrl release", "\x1b[57442;1:3u", TE_RELEASE, TK_CTRL, 0, 0, 1);
    one("kitty right alt", "\x1b[57449;3u", TE_PRESS, TK_ALT, 0, TM_ALT, 1);
    one("kitty enter", "\x1b[13u", TE_PRESS, TK_ENTER, '\r', 0, 1);
    one("kitty escape", "\x1b[27u", TE_PRESS, TK_ESCAPE, 0, 0, 1);
    one("kitty up release", "\x1b[1;1:3A", TE_RELEASE, TK_UP, 0, 0, 1);
    one("kitty up press with shift", "\x1b[1;2:1A", TE_PRESS, TK_UP, 0, TM_SHIFT, 1);
    one("kitty F5 release", "\x1b[15;1:3~", TE_RELEASE, TK_F5, 0, 0, 1);
    one("kitty keypad 8", "\x1b[57407u", TE_PRESS, '8', '8', 0, 1);
    one("kitty keypad up", "\x1b[57419u", TE_PRESS, TK_UP, 0, 0, 1);

    // Windows Terminal's win32-input-mode: CSI Vk;Sc;Uc;Kd;Cs;Rc _
    one("win32 W down", "\x1b[87;17;119;1;0;1_", TE_PRESS, 'w', 'w', 0, 1);
    one("win32 W up", "\x1b[87;17;119;0;0;1_", TE_RELEASE, 'w', 'w', 0, 1);
    one("win32 shift-W", "\x1b[87;17;87;1;16;1_", TE_PRESS, 'w', 'W', TM_SHIFT, 1);
    one("win32 ctrl alone", "\x1b[17;29;0;1;8;1_", TE_PRESS, TK_CTRL, 0, TM_CTRL, 1);
    one("win32 left arrow up", "\x1b[37;75;0;0;256;1_", TE_RELEASE, TK_LEFT, 0, 0, 1);
    one("win32 F12", "\x1b[123;88;0;1;0;1_", TE_PRESS, TK_F12, 0, 0, 1);
    one("win32 enter", "\x1b[13;28;13;1;0;1_", TE_PRESS, TK_ENTER, '\r', 0, 1);
    one("win32 char only (Vk 0)", "\x1b[0;0;97;1;0;1_", TE_PRESS, 'a', 'a', 0, 1);

    // answers to the queries
    one("kitty flags answer", "\x1b[?0u", TE_KITTY, TK_NONE, 0, 0, 1);
    one("device attributes", "\x1b[?62;22c", TE_DA, TK_NONE, 0, 0, 0);
    one("cursor position report", "\x1b[2;3R", TE_CPR, 2, 3, 0, 0);
    {
        tk_event ev[16];
        CHECK(decode("\x1b[>1;10;0c", 0, 100, ev) == 0, "secondary device attributes are no key");
        CHECK(decode("\x1b[<0;10;5M", 0, 100, ev) == 0, "an SGR mouse report is no key");
        CHECK(decode("\x1b[I\x1b[O", 0, 100, ev) == 0, "focus reports are no keys");
        CHECK(decode("\x1b]11;rgb:0000/0000/0000\x07", 0, 100, ev) == 0, "an OSC answer (BEL) is no key");
        CHECK(decode("\x1b]11;rgb:0000/0000/0000\x1b\\", 1, 100, ev) == 0, "an OSC answer (ST) is no key");
        CHECK(decode("\x1bP1$r0m\x1b\\w", 0, 100, ev) == 1 && ev[0].key == 'w', "a DCS answer is no key");
    }

    // several at once, in order
    {
        tk_event ev[16];
        int n = decode("\x1b[A\x1b[Bw\x1b[119;1:3u\x1b[2;2R", 0, 100, ev);
        CHECK(n == 5 && ev[0].key == TK_UP && ev[1].key == TK_DOWN && ev[2].key == 'w' && ev[3].type == TE_RELEASE
                  && ev[4].type == TE_CPR,
              "a burst: %d events", n);
        n = decode("\x1b[A\x1b[B", 1, 100, ev);
        CHECK(n == 2 && ev[0].key == TK_UP && ev[1].key == TK_DOWN, "a burst a byte at a time: %d events", n);
    }
    // junk does not stop what comes after
    {
        tk_event ev[16];
        int n = decode("\x80\xff\x1b[999;999;999;999;999;999;999;999;999;999;999zw", 0, 100, ev);
        CHECK(n == 1 && ev[0].key == 'w', "after junk: %d events", n);
    }
    // a sequence that never ends goes after a while
    {
        tk_decoder d;
        tk_event ev;
        tk_init(&d);
        tk_feed(&d, (const unsigned char *)"\x1b[12;", 5, 0);
        CHECK(!tk_next(&d, &ev, 100, 40), "an unfinished sequence waits");
        tk_feed(&d, (const unsigned char *)"", 0, 0);
        int got = 0;
        for (uint32_t t = 400; t < 1000 && !got; t += 100)
            got = tk_next(&d, &ev, t, 40) && ev.type == TE_PRESS;
        tk_feed(&d, (const unsigned char *)"x", 1, 1000);
        while (tk_next(&d, &ev, 1000, 40))
            if (ev.key == 'x')
                got = 2;
        CHECK(got == 2, "after an unfinished sequence, the next key comes through");
    }

    char buf[16];
    CHECK(strcmp(tk_key_name(TK_LEFT, buf), "left") == 0, "name of left");
    CHECK(strcmp(tk_key_name('q', buf), "q") == 0, "name of q");

    printf("test_input: %d checks, %d failed\n", checks, fails);
    return fails != 0;
}
