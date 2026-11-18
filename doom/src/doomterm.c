// doomterm: DOOM (doomgeneric) in a terminal — collaboCore's guest shell, or any terminal.
//
// The frame (320x200) is scaled to the terminal's size as it is (and again when the window
// changes) and drawn with block characters in 24-bit colour, only the cells that changed sent
// again. A frame may take what the terminal takes in a frame's time (its pace, seen in how long
// writes wait): the cells that look most wrong go first, the rest in later frames; while cells keep
// waiting, colour precision steps down and then the picture is made smaller (a lower resolution),
// and both come back when the terminal keeps up. The game runs in real time all the while. Keys come as a terminal sends them; where it says when a key is let go (the kitty keyboard
// protocol, Windows Terminal's win32-input-mode) that is used, elsewhere a release is inferred
// from the terminal's key repeat.
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

#include "doomgeneric.h"
#include "doomkeys.h"
#include "doomstat.h"
#include "d_event.h"
#include "i_system.h"
#include "i_timer.h"
#include "m_argv.h"

#include "input.h"
#include "render.h"

#ifndef DOOMTERM_VERSION
#define DOOMTERM_VERSION "1.0"
#endif

extern boolean menuactive;
extern boolean chat_on;

// A doom key code no binding uses: a key that only types a character (cheats read data2).
#define KEY_NOTHING 1

// ── options ──────────────────────────────────────────────────────────────────────────────────────
static struct {
    int blocks, color, stretch, quant, status, kitty, win32, endoom;
    double cell_aspect, aspect;
    int max_cols, max_rows;
    int hold_turn, hold_move, hold_fire, hold_other;
    const char *log;
    int headless, realtime;
    const char *shot, *shotansi;
    int shottics[64], nshots;
    int term_cols, term_rows;
    const char *keys;
    int bench;
    double bandwidth;  // bytes a second the terminal is sent (0: as much as it takes)
    int fixed_quant;   // -quant given: no adapting
    const char *dumpstate;
} opt;

// -trace FILE: what the frontend does (keys given to DOOM, layouts, the terminal's answers), for
// the tests
static FILE *trace;

static void tracef(const char *fmt, ...)
{
    if (!trace)
        return;
    va_list ap;
    va_start(ap, fmt);
    fprintf(trace, "%u ", (unsigned)DG_GetTicksMs());
    vfprintf(trace, fmt, ap);
    fputc('\n', trace);
    va_end(ap);
    fflush(trace);
}

uint32_t DG_GetTicksMs(void);

static const char usage[] =
    "doom: DOOM in the terminal (doomgeneric, Chocolate Doom's engine), drawn with block characters\n"
    "in 24-bit colour and scaled to the terminal's size as it changes.\n"
    "\n"
    "  doom [-iwad FILE] [-file PWAD...] [-warp E M | -warp MAP] [-skill 1-5] [DOOM's options...]\n"
    "\n"
    "IWADs: the add-on has Freedoom (freedoom1.wad, Phase 1, picked by default; freedoom2.wad,\n"
    "Phase 2) in /usr/share/games/doom; doom1.wad, doom.wad, doom2.wad, ... of your own are found\n"
    "in the current folder, $DOOMWADDIR or $DOOMWADPATH, or give -iwad PATH.\n"
    "\n"
    "Keys:  arrows / W S        move, turn          A D  or  , .     strafe\n"
    "       F, Ctrl*, Enter     fire                Space, E          open, use\n"
    "       Shift + key*        run                 R                 always run on/off\n"
    "       Alt + arrows*       strafe              1-7               weapons\n"
    "       Tab                 map                 Esc               menu (Y/N to answer)\n"
    "       F1-F12              help, save, load, ... (as in DOOM)    P, Pause  pause\n"
    "       `                   picture: half blocks, quadrants, ASCII\n"
    "       ~                   colours: 24-bit, 256, 16, none     \\  status line\n"
    "       Ctrl+L              draw again          Ctrl+C            quit\n"
    "  * Ctrl or Shift alone and holding several keys at once need a terminal that reports key\n"
    "    releases (the kitty keyboard protocol: kitty, WezTerm, foot, Ghostty, iTerm2, Alacritty;\n"
    "    Windows Terminal's win32-input-mode); others send repeats only, and a key counts as let go\n"
    "    when its repeats stop. With a Korean input method on, the jamo keys work as their letters.\n"
    "\n"
    "Terminal options:\n"
    "  -blocks half|quad|ascii  how cells show pixels (default half: 2 pixels a cell)\n"
    "  -color auto|truecolor|256|16|mono   the colours the terminal takes (auto: 24-bit; 16 on\n"
    "                       the Linux console)\n"
    "  -size COLSxROWS      the picture no larger than this, always (otherwise it is made smaller\n"
    "                       by itself while the terminal cannot keep up, and larger again)\n"
    "  -stretch             fill the terminal instead of keeping DOOM's 4:3\n"
    "  -cellaspect N        a character cell's height / width (default 2.0)\n"
    "  -quant BITS          bits kept of each colour channel, 1-8 (default: 6, fewer when the\n"
    "                       terminal cannot take more)\n"
    "  -bandwidth KB        send the terminal no more than KB kilobytes a second (default: as much\n"
    "                       as it takes in time; the cells that look most wrong go first)\n"
    "  -fps                 a status line: frames per second, bytes per second, mode\n"
    "  -nokitty, -nowin32   do not ask the terminal for key releases that way\n"
    "  -keyhold MS          (no key releases) how long a tapped turn key turns (default 140)\n"
    "  -log FILE            DOOM's messages while it runs (default: dropped)\n"
    "  -noendoom            no ENDOOM screen at the end\n"
    "Without a terminal (tests, agents):\n"
    "  -headless            no terminal: the game runs, nothing is drawn\n"
    "  -shot FILE           write the frame at -shottic N[,N...] (default 35) and quit: .ppm the\n"
    "                       320x200 picture, anything else as the terminal is sent it;\n"
    "                       %d in FILE: the tic. Time is then simulated: the same frames every run\n"
    "  -shotansi FILE       also the terminal's rendering (-termsize COLSxROWS, default 80x25)\n"
    "  -keys TIC:KEY,...    keys pressed at those tics (+KEY down, -KEY up; up, down, left, right,\n"
    "                       fire, use, run, strafe, enter, esc, tab, space, f1-f12, or a character)\n"
    "  -benchrender         draw every frame for the terminal (-termsize) in memory; time it\n"
    "  -realtime            with -shot/-keys: real time, not simulated\n"
    "  -trace FILE, -dumpstate FILE   (tests) the keys given to DOOM, the layouts; the cells the\n"
    "                       terminal was sent, at the end\n";

static int parse_size(const char *s, int *w, int *h)
{
    int a, b;
    if (sscanf(s, "%dx%d", &a, &b) == 2 || sscanf(s, "%dX%d", &a, &b) == 2) {
        *w = a;
        *h = b;
        return 1;
    }
    return 0;
}

static const char *arg_of(const char *name)
{
    int i = M_CheckParmWithArgs((char *)name, 1);
    return i ? myargv[i + 1] : NULL;
}

static void parse_options(void)
{
    const char *s;
    opt.blocks = RT_HALF;
    opt.color = -1;
    opt.quant = 6;
    opt.kitty = !M_CheckParm("-nokitty");
    opt.win32 = !M_CheckParm("-nowin32");
    opt.endoom = !M_CheckParm("-noendoom");
    opt.status = M_CheckParm("-fps") > 0;
    opt.stretch = M_CheckParm("-stretch") > 0;
    opt.cell_aspect = 2.0;
    opt.aspect = 4.0 / 3.0;
    opt.hold_turn = 140;
    opt.hold_fire = 160;
    opt.hold_other = 120;
    opt.term_cols = 80;
    opt.term_rows = 25;
    if ((s = arg_of("-blocks"))) {
        if (!strcmp(s, "half"))
            opt.blocks = RT_HALF;
        else if (!strcmp(s, "quad"))
            opt.blocks = RT_QUAD;
        else if (!strcmp(s, "ascii"))
            opt.blocks = RT_ASCII;
        else
            I_Error("-blocks: half, quad or ascii, not %s", s);
    }
    if ((s = arg_of("-color")) || (s = arg_of("-colour"))) {
        if (!strcmp(s, "auto"))
            opt.color = -1;
        else if (!strcmp(s, "truecolor") || !strcmp(s, "24bit") || !strcmp(s, "24"))
            opt.color = RT_TRUECOLOR;
        else if (!strcmp(s, "256"))
            opt.color = RT_256;
        else if (!strcmp(s, "16"))
            opt.color = RT_16;
        else if (!strcmp(s, "mono") || !strcmp(s, "none"))
            opt.color = RT_MONO;
        else
            I_Error("-color: auto, truecolor, 256, 16 or mono, not %s", s);
    }
    if (opt.color < 0) {
        const char *term = getenv("TERM"), *ct = getenv("COLORTERM");
        if (ct && (strstr(ct, "truecolor") || strstr(ct, "24bit")))
            opt.color = RT_TRUECOLOR;
        else if (term && (!strcmp(term, "linux") || strstr(term, "-16color")))
            opt.color = RT_16;
        else if (term && (!strcmp(term, "dumb") || !strcmp(term, "vt100") || !strcmp(term, "vt220")))
            opt.color = RT_MONO;
        else
            opt.color = RT_TRUECOLOR;
    }
    if (opt.color == RT_MONO)
        opt.blocks = RT_ASCII;
    if ((s = arg_of("-size")) && !parse_size(s, &opt.max_cols, &opt.max_rows))
        I_Error("-size: COLSxROWS, not %s", s);
    if ((s = arg_of("-termsize")) && !parse_size(s, &opt.term_cols, &opt.term_rows))
        I_Error("-termsize: COLSxROWS, not %s", s);
    if ((s = arg_of("-cellaspect")))
        opt.cell_aspect = atof(s);
    if ((s = arg_of("-quant"))) {
        opt.quant = atoi(s);
        opt.fixed_quant = 1;
    }
    if (opt.quant < 1 || opt.quant > 8)
        opt.quant = 8;
    if ((s = arg_of("-keyhold")))
        opt.hold_turn = atoi(s);
    if ((s = arg_of("-bandwidth")))
        opt.bandwidth = atof(s) * 1024;
    opt.log = arg_of("-log");
    opt.headless = M_CheckParm("-headless") > 0;
    opt.realtime = M_CheckParm("-realtime") > 0;
    opt.shot = arg_of("-shot");
    opt.shotansi = arg_of("-shotansi");
    opt.keys = arg_of("-keys");
    opt.bench = M_CheckParm("-benchrender") > 0;
    opt.dumpstate = arg_of("-dumpstate");
    if ((s = arg_of("-trace")) && !(trace = fopen(s, "w")))
        I_Error("-trace: cannot write %s", s);
    if ((s = arg_of("-shottic"))) {
        char *end;
        for (const char *p = s; *p && opt.nshots < 64;) {
            long v = strtol(p, &end, 10);
            if (end == p)
                break;
            opt.shottics[opt.nshots++] = (int)v;
            p = *end == ',' ? end + 1 : end;
        }
    }
    if ((opt.shot || opt.shotansi) && opt.nshots == 0)
        opt.shottics[opt.nshots++] = 35;
    if (opt.shot || opt.shotansi || opt.keys || opt.bench)
        opt.headless = 1;
}

// ── time ─────────────────────────────────────────────────────────────────────────────────────────
static int virtual_clock;
static uint32_t vclock = 1000;

static uint32_t real_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint32_t)((uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000);
}

static double real_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + ts.tv_nsec / 1e9;
}

uint32_t DG_GetTicksMs(void) { return virtual_clock ? vclock : real_ms(); }

// ── the terminal session ─────────────────────────────────────────────────────────────────────────
static rt_t R;
static tk_decoder dec;
static int tty_fd = -1;
static struct termios saved_tio;
static int saved_stdout = -1, saved_stderr = -1;
static volatile sig_atomic_t session, winch = 1, redraw;
static size_t out_off;           // how much of R.out the terminal has
static int kitty_pushed, kitty_seen, da_seen, cpr_seen, win32_seen;
static int was_interactive;
static char title[128] = "DOOM";
static int term_cols = 80, term_rows = 24;

static void write_all(int fd, const char *s, size_t n)
{
    while (n > 0) {
        ssize_t w = write(fd, s, n);
        if (w > 0) {
            s += w;
            n -= (size_t)w;
        } else if (w < 0 && errno == EAGAIN) {
            struct pollfd p = {fd, POLLOUT, 0};
            poll(&p, 1, 100);
        } else if (!(w < 0 && errno == EINTR)) {
            return;
        }
    }
}

#define LEAVE "\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l\x1b[23;0t"

static void flush_pending(void)
{
    if (out_off < R.len)
        write_all(tty_fd, R.out + out_off, R.len - out_off);
    R.len = out_off = 0;
}

// Back to the terminal as it was (async-signal-safe: also from a signal handler).
static void session_restore(void)
{
    if (!session)
        return;
    session = 0;
    if (out_off < R.len)  // the rest of a frame: an escape sequence must not stay half sent
        write_all(tty_fd, R.out + out_off, R.len - out_off);
    R.len = out_off = 0;
    if (kitty_pushed)
        write_all(tty_fd, "\x1b[<u", 4);
    if (opt.win32)
        write_all(tty_fd, "\x1b[?9001l", 8);
    write_all(tty_fd, LEAVE, sizeof LEAVE - 1);
    tcsetattr(tty_fd, TCSANOW, &saved_tio);
    if (saved_stdout >= 0) {
        dup2(saved_stdout, 1);
        dup2(saved_stderr, 2);
    }
}

// -dumpstate FILE: what the renderer takes the terminal to show (the tests compare it with what a
// terminal shows after the bytes sent)
static void dump_state(void)
{
    FILE *f = opt.dumpstate && R.cur ? fopen(opt.dumpstate, "w") : NULL;
    if (!f)
        return;
    fprintf(f, "layout %d %d %s %s %d %d %d %d %d %d\n", term_cols, term_rows, rt_blocks_name(R.blocks),
            rt_color_name(R.color), R.ox, R.oy, R.cw, R.ch, R.wide, R.quant);
    for (int i = 0; i < R.cw * R.ch; i++)
        fprintf(f, "%d %x %x %x\n", R.valid[i], R.cur[i].a, R.cur[i].b, R.cur[i].c);
    fclose(f);
}

static void session_end(void)
{
    fflush(stdout);
    fflush(stderr);
    if (session) {
        flush_pending();
        dump_state();
    }
    session_restore();
}

static void on_signal(int sig)
{
    session_restore();
    signal(sig, SIG_DFL);
    raise(sig);
}

static void on_winch(int sig)
{
    (void)sig;
    winch = 1;
}

static void query_size(void)
{
    struct winsize ws;
    if (ioctl(tty_fd, TIOCGWINSZ, &ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0) {
        term_cols = ws.ws_col;
        term_rows = ws.ws_row;
    }
}

static void tty_printf(const char *fmt, ...)
{
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (n > 0)
        write_all(tty_fd, buf, (size_t)n < sizeof buf ? (size_t)n : sizeof buf - 1);
}

// Starts drawing on the terminal: raw keys, the alternate screen, DOOM's messages elsewhere.
static int session_start(void)
{
    tty_fd = open("/dev/tty", O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC);
    if (tty_fd < 0 || !isatty(tty_fd) || tcgetattr(tty_fd, &saved_tio) != 0) {
        if (tty_fd >= 0)
            close(tty_fd);
        tty_fd = -1;
        return 0;
    }
    struct termios raw = saved_tio;
    cfmakeraw(&raw);
    raw.c_cc[VMIN] = 0;
    raw.c_cc[VTIME] = 0;
    tcsetattr(tty_fd, TCSANOW, &raw);

    fflush(stdout);
    fflush(stderr);
    saved_stdout = dup(1);
    saved_stderr = dup(2);
    int fd = open(opt.log ? opt.log : "/dev/null", O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
    if (fd >= 0) {
        dup2(fd, 1);
        dup2(fd, 2);
        close(fd);
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_winch;
    sigaction(SIGWINCH, &sa, NULL);
    sa.sa_handler = on_signal;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    sigaction(SIGQUIT, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);

    session = 1;
    was_interactive = 1;
    I_AtExit(session_end, true);
    atexit(session_end);  // also when something calls exit() itself
    // the alternate screen, the title kept, no cursor, no wrapping; the title
    tty_printf("\x1b[?1049h\x1b[22;0t\x1b[?25l\x1b[?7l\x1b[0m\x1b[2J\x1b]2;%s\x07", title);
    // Questions: does a block character take one column or two (the cursor's position after
    // one: East Asian ambiguous width), the kitty keyboard protocol? The device attributes'
    // answer comes after the others' from every terminal.
    tty_printf("\x1b[2;1H\xe2\x96\x80\x1b[6n\x1b[2;1H \x1b[H");
    if (opt.kitty)
        tty_printf("\x1b[?u");
    tty_printf("\x1b[c");
    if (opt.win32)
        tty_printf("\x1b[?9001h");
    winch = 1;
    return 1;
}

// ── keys ─────────────────────────────────────────────────────────────────────────────────────────
enum { ROLE_OTHER, ROLE_MOVE, ROLE_TURN, ROLE_FIRE };

#define MAXK 3
typedef struct {
    uint32_t key;  // the terminal's key (0: a free slot)
    int k[MAXK], n;
    int role, exact, repeating, carried;
    int repeats;          // events since it went down (or was pressed again)
    uint32_t first, last;
    uint32_t delay;       // the first repeat's time after the press: the terminal's repeat delay?
} held_t;

static held_t held[16];
static int refcount[256];
static int autorun;
static uint32_t repeat_delay = 450, repeat_gap = 50;  // the terminal's key repeat, as seen
static uint32_t rel_key, rel_first;                    // the last key let go by its timeout
static uint32_t last_key;                              // the key of the last event

static void post(int type, int data1, int data2)
{
    tracef("%s %d %d", type == ev_keydown ? "down" : type == ev_keyup ? "up" : "event", data1, data2);
    event_t ev;
    memset(&ev, 0, sizeof ev);
    ev.type = type;
    ev.data1 = data1;
    ev.data2 = data2;
    D_PostEvent(&ev);
}

static void key_down(int k, int text)
{
    if (k <= 0 || k > 255)
        return;
    if (refcount[k]++ == 0)
        post(ev_keydown, k, text);
}

static void key_up(int k)
{
    if (k <= 0 || k > 255 || refcount[k] == 0)
        return;
    if (--refcount[k] == 0)
        post(ev_keyup, k, 0);
}

static void hud_message(const char *msg)
{
    static char buf[64];
    if (gamestate == GS_LEVEL) {
        snprintf(buf, sizeof buf, "%s", msg);
        players[consoleplayer].message = buf;
    }
}

// The game itself is being played (the keys move the player), not the menu or a text being typed.
static int playing(void)
{
    return !menuactive && !chat_on && !demoplayback
        && (gamestate == GS_LEVEL || gamestate == GS_INTERMISSION || gamestate == GS_FINALE);
}

// A Korean input method's jamo: the key it is on (two-set layout); shift for the doubled ones.
static uint32_t jamo_key(uint32_t c, int *shift)
{
    static const char keys[] =  // U+3131 ㄱ .. U+3163 ㅣ
        "rRrssseEffffffffaqQqtTdwWczxvgkoiOjpuPhhhhynnnnbmml";
    *shift = 0;
    if (c < 0x3131 || c > 0x3163)
        return 0;
    char k = keys[c - 0x3131];
    if (k >= 'A' && k <= 'Z') {
        *shift = 1;
        k = (char)(k + 32);
    }
    return (uint32_t)k;
}

static int doom_key_of(uint32_t key)
{
    switch (key) {
    case TK_UP: return KEY_UPARROW;
    case TK_DOWN: return KEY_DOWNARROW;
    case TK_LEFT: return KEY_LEFTARROW;
    case TK_RIGHT: return KEY_RIGHTARROW;
    case TK_ESCAPE: return KEY_ESCAPE;
    case TK_ENTER: return KEY_ENTER;
    case TK_TAB: return KEY_TAB;
    case TK_BACKSPACE: return KEY_BACKSPACE;
    case TK_PAUSE: return KEY_PAUSE;
    case TK_HOME: return KEY_HOME;
    case TK_END: return KEY_END;
    case TK_PGUP: return KEY_PGUP;
    case TK_PGDN: return KEY_PGDN;
    case TK_INSERT: return KEY_INS;
    case TK_DELETE: return KEY_DEL;
    case TK_SHIFT: return KEY_RSHIFT;
    case TK_CTRL: return KEY_FIRE;
    case TK_ALT: return KEY_RALT;
    case '+': case '=': return KEY_EQUALS;
    case '-': case '_': return KEY_MINUS;
    }
    if (key >= TK_F1 && key <= TK_F10)
        return KEY_F1 + (int)(key - TK_F1);
    if (key == TK_F11)
        return KEY_F11;
    if (key == TK_F12)
        return KEY_F12;
    if (key >= 0x20 && key < 0x7f)
        return (int)tolower((int)key);
    return 0;
}

static void add_key(held_t *h, int k)
{
    if (k > 0 && h->n < MAXK)
        h->k[h->n++] = k;
}

// The doom keys a terminal key presses, where the player is.
static void map_key(const tk_event *ev, held_t *h)
{
    uint32_t key = ev->key;
    int mods = ev->mods;
    h->n = 0;
    h->role = ROLE_OTHER;
    if (playing()) {
        switch (key) {
        case 'w': case TK_UP:
            add_key(h, KEY_UPARROW);
            h->role = ROLE_MOVE;
            break;
        case 's': case TK_DOWN:
            add_key(h, KEY_DOWNARROW);
            h->role = ROLE_MOVE;
            break;
        case 'a': case ',':
            add_key(h, KEY_STRAFE_L);
            h->role = ROLE_MOVE;
            break;
        case 'd': case '.':
            add_key(h, KEY_STRAFE_R);
            h->role = ROLE_MOVE;
            break;
        case TK_LEFT: case TK_RIGHT:
            if (mods & TM_ALT) {
                add_key(h, key == TK_LEFT ? KEY_STRAFE_L : KEY_STRAFE_R);
                h->role = ROLE_MOVE;
            } else {
                add_key(h, key == TK_LEFT ? KEY_LEFTARROW : KEY_RIGHTARROW);
                h->role = ROLE_TURN;
            }
            break;
        case ' ': case 'e':
            add_key(h, KEY_USE);
            break;
        case 'f':
            if (automapactive) {  // the map's follow mode
                add_key(h, 'f');
                break;
            }
            /* fall through */
        case TK_ENTER: case TK_CTRL:
            add_key(h, KEY_FIRE);
            h->role = ROLE_FIRE;
            break;
        case 'p':
            add_key(h, KEY_PAUSE);
            break;
        default:
            add_key(h, doom_key_of(key));
            break;
        }
        if (h->role == ROLE_MOVE || h->role == ROLE_TURN) {
            if (mods & TM_SHIFT)
                add_key(h, KEY_RSHIFT);
            if ((mods & TM_CTRL) && !ev->exact)  // Ctrl+arrow: Ctrl alone is never sent
                add_key(h, KEY_FIRE);
        }
    } else {
        add_key(h, doom_key_of(key));
    }
}

static held_t *find_held(uint32_t key)
{
    for (int i = 0; i < 16; i++)
        if (held[i].key == key)
            return &held[i];
    return NULL;
}

static void release(held_t *h, uint32_t now)
{
    for (int i = 0; i < h->n; i++)
        key_up(h->k[i]);
    if (!h->exact) {
        rel_key = h->key;
        rel_first = h->first;
    }
    (void)now;
    memset(h, 0, sizeof *h);
}

static uint32_t hold_time(const held_t *h)
{
    if (h->repeating) {
        uint32_t g = repeat_gap * 3;
        return g < 80 ? 80 : g;
    }
    switch (h->role) {
    case ROLE_MOVE: return opt.hold_move ? (uint32_t)opt.hold_move : repeat_delay + 60;
    case ROLE_TURN: return (uint32_t)opt.hold_turn;
    case ROLE_FIRE: return (uint32_t)opt.hold_fire;
    default: return (uint32_t)opt.hold_other;
    }
}

// Keys whose terminal says nothing of releases: let go when their repeats stop.
static void expire_keys(uint32_t now)
{
    int active = 0;
    for (int i = 0; i < 16; i++)
        if (held[i].key && !held[i].exact && !held[i].carried)
            active = 1;
    for (int i = 0; i < 16; i++) {
        held_t *h = &held[i];
        if (!h->key || h->exact)
            continue;
        if (h->carried) {
            // its repeats stopped when a later key took them over: held while that one is
            if (active) {
                h->last = now;
                continue;
            }
            h->carried = 0;
            h->last = now;
        }
        if (now - h->last > hold_time(h))
            release(h, now);
    }
}

static void release_all(void)
{
    for (int i = 0; i < 16; i++)
        if (held[i].key)
            release(&held[i], 0);
}

static void cycle_blocks(void);
static void cycle_color(void);

// A frontend key: 1 if it was one (and is done). Its repeats do nothing: a terminal's (the same key
// again within 150 ms of the last time) or the kitty protocol's.
static int frontend_key(const tk_event *ev)
{
    static uint32_t prev_key, prev_ms;
    uint32_t now = real_ms();
    int repeat = ev->type == TE_REPEAT || (ev->key == prev_key && now - prev_ms < 150);
    prev_key = ev->key;
    prev_ms = now;
    if ((ev->mods & TM_CTRL) && ev->key == 'c') {
        release_all();
        I_Quit();
    }
    if ((ev->mods & TM_CTRL) && ev->key == 'l') {
        redraw = 1;
        return 1;
    }
    if (ev->mods & (TM_CTRL | TM_ALT))
        return 0;
    if (ev->key == '`' && ev->text != '~') {
        if (!repeat)
            cycle_blocks();
        return 1;
    }
    if (ev->key == '~' || (ev->key == '`' && ev->text == '~')) {
        if (!repeat)
            cycle_color();
        return 1;
    }
    if (ev->key == '\\') {
        if (!repeat) {
            opt.status = !opt.status;
            winch = 1;
        }
        return 1;
    }
    if (ev->key == 'r' && playing()) {
        if (repeat)
            return 1;
        autorun = !autorun;
        if (autorun)
            key_down(KEY_RSHIFT, 0);
        else
            key_up(KEY_RSHIFT);
        hud_message(autorun ? "ALWAYS RUN ON" : "ALWAYS RUN OFF");
        post(ev_keydown, KEY_NOTHING, 'r');  // for the cheats
        post(ev_keyup, KEY_NOTHING, 0);
        return 1;
    }
    return 0;
}

static void handle_event(tk_event *ev, uint32_t now)
{
    switch (ev->type) {
    case TE_KITTY:
        if (!kitty_seen && opt.kitty && session) {
            // report every key as an escape sequence (8), with its releases (2), unambiguous (1)
            tty_printf("\x1b[>11u");
            kitty_pushed = 1;
            tracef("kitty");
        }
        kitty_seen = 1;
        return;
    case TE_DA:
        da_seen = 1;
        return;
    case TE_CPR:
        if (!cpr_seen) {
            cpr_seen = 1;
            int cellw = ev->text >= 3 ? 2 : 1;
            if (cellw != R.cellw) {
                R.cellw = cellw;
                tracef("cellw %d", cellw);
                winch = 1;
            }
        }
        return;
    }
    if (ev->exact && ev->key != TK_NONE && !kitty_seen)
        win32_seen = 1;
    if (ev->key >= 0x3131 && ev->key <= 0x3163) {  // a Korean input method's jamo
        int shift;
        uint32_t k = jamo_key(ev->key, &shift);
        ev->key = k;
        ev->text = shift ? k - 32 : k;
        if (shift)
            ev->mods |= TM_SHIFT;
    }
    int text = ev->text > 0 && ev->text < 0x80 ? (int)ev->text : 0;
    if (ev->type == TE_RELEASE) {
        held_t *h = find_held(ev->key);
        if (h)
            release(h, now);
        return;
    }
    held_t *h = find_held(ev->key);
    if (h) {  // held: a repeat, or pressed again
        // Either way DOOM gets a key-down (with its character: cheats, names), as DOS DOOM got a
        // key's typematic repeats; which it was says when the key is let go.
        if (h->n > 0)
            post(ev_keydown, h->k[0], text);
        if (!h->exact) {
            uint32_t since = now - h->first, gap = now - h->last;
            uint32_t early = repeat_delay * 6 / 10 < 200 ? repeat_delay * 6 / 10 : 200;
            if (last_key != ev->key || (h->repeats == 0 && since < early) || (h->repeats > 0 && gap > 150)) {
                // another key in between (a terminal repeats the last key only), too soon for
                // the key's repeat to begin, or after its repeats stopped: pressed again, the
                // key staying down
                h->repeats = 0;
                h->repeating = 0;
                h->first = h->last = now;
                last_key = ev->key;
                return;
            }
            if (h->repeats == 0) {
                h->delay = since;  // the repeat's start, if more follow soon
            } else {
                if (h->repeats == 1 && h->delay >= 150 && h->delay <= 1000)
                    repeat_delay = h->delay;
                if (gap >= 8 && gap <= 150)
                    repeat_gap = (repeat_gap * 3 + gap) / 4;
            }
            h->repeats++;
            h->repeating = 1;
        }
        h->last = now;
        last_key = ev->key;
        return;
    }
    if (frontend_key(ev))
        return;
    if (ev->type == TE_REPEAT)
        ev->type = TE_PRESS;  // a repeat of a key pressed before the game looked
    last_key = ev->key;
    held_t nh;
    memset(&nh, 0, sizeof nh);
    map_key(ev, &nh);
    if (nh.n == 0) {
        if (text) {  // a character no key: still typed, for the cheats
            post(ev_keydown, KEY_NOTHING, text);
            post(ev_keyup, KEY_NOTHING, 0);
        }
        return;
    }
    nh.key = ev->key;
    nh.exact = ev->exact;
    nh.first = nh.last = now;
    if (!nh.exact) {
        // its autorepeat starting after it was taken as let go: it was held all along
        if (ev->key == rel_key && now - rel_first < repeat_delay + 300) {
            nh.repeating = 1;
            nh.first = rel_first;
        }
        // the repeats of a key held before stop now: keep it down while this one is
        for (int i = 0; i < 16; i++)
            if (held[i].key && !held[i].exact && held[i].repeating)
                held[i].carried = 1;
    }
    held_t *slot = NULL;
    for (int i = 0; i < 16 && !slot; i++)
        if (!held[i].key)
            slot = &held[i];
    if (!slot) {  // sixteen keys down: the oldest goes
        slot = &held[0];
        release(slot, now);
    }
    *slot = nh;
    for (int i = 0; i < nh.n; i++)
        key_down(nh.k[i], i == 0 ? text : 0);
}

static void read_input(void)
{
    unsigned char buf[1024];
    for (;;) {
        ssize_t n = read(tty_fd, buf, sizeof buf);
        if (n > 0)
            tk_feed(&dec, buf, (int)n, real_ms());
        else
            break;
    }
}

static void pump_input(void)
{
    read_input();
    uint32_t now = real_ms();
    tk_event ev;
    while (tk_next(&dec, &ev, now, 40))
        handle_event(&ev, now);
    expire_keys(now);
}

// ── keys from -keys (headless) ───────────────────────────────────────────────────────────────────
typedef struct {
    int tic, key, type;
} script_t;
static script_t script[512];
static int nscript, script_pos;

static int script_key(const char *name)
{
    static const struct {
        const char *name;
        int key;
    } names[] = {
        {"up", KEY_UPARROW}, {"down", KEY_DOWNARROW}, {"left", KEY_LEFTARROW}, {"right", KEY_RIGHTARROW},
        {"fire", KEY_FIRE}, {"use", KEY_USE}, {"run", KEY_RSHIFT}, {"strafe", KEY_RALT},
        {"strafeleft", KEY_STRAFE_L}, {"straferight", KEY_STRAFE_R}, {"enter", KEY_ENTER},
        {"esc", KEY_ESCAPE}, {"escape", KEY_ESCAPE}, {"tab", KEY_TAB}, {"space", ' '},
        {"backspace", KEY_BACKSPACE}, {"pause", KEY_PAUSE}, {"minus", KEY_MINUS}, {"equals", KEY_EQUALS},
        {"f1", KEY_F1}, {"f2", KEY_F2}, {"f3", KEY_F3}, {"f4", KEY_F4}, {"f5", KEY_F5}, {"f6", KEY_F6},
        {"f7", KEY_F7}, {"f8", KEY_F8}, {"f9", KEY_F9}, {"f10", KEY_F10}, {"f11", KEY_F11}, {"f12", KEY_F12},
    };
    for (size_t i = 0; i < sizeof names / sizeof *names; i++)
        if (!strcasecmp(name, names[i].name))
            return names[i].key;
    if (strlen(name) == 1)
        return tolower((unsigned char)name[0]);
    I_Error("-keys: no key %s", name);
    return 0;
}

static int cmp_script(const void *a, const void *b)
{
    const script_t *x = a, *y = b;
    return x->tic != y->tic ? x->tic - y->tic : (int)(x - y);
}

static void parse_script(const char *spec)
{
    char *copy = strdup(spec), *save = NULL;
    for (char *item = strtok_r(copy, ",", &save); item && nscript < 510; item = strtok_r(NULL, ",", &save)) {
        char *colon = strchr(item, ':');
        if (!colon)
            I_Error("-keys: TIC:KEY, not %s", item);
        *colon = 0;
        int tic = atoi(item);
        char *name = colon + 1;
        int type = 0;  // 0: down then up a tic later
        if (*name == '+' && name[1]) {
            type = ev_keydown;
            name++;
        } else if (*name == '-' && name[1]) {
            type = ev_keyup;
            name++;
        }
        int key = script_key(name);
        if (type == 0) {
            script[nscript++] = (script_t){tic, key, ev_keydown};
            script[nscript++] = (script_t){tic + 1, key, ev_keyup};
        } else {
            script[nscript++] = (script_t){tic, key, type};
        }
    }
    free(copy);
    qsort(script, (size_t)nscript, sizeof *script, cmp_script);
}

static void run_script(void)
{
    int now = I_GetTime();
    while (script_pos < nscript && script[script_pos].tic <= now) {
        script_t *s = &script[script_pos++];
        int text = s->key < 0x80 ? s->key : 0;
        post(s->type, s->key, s->type == ev_keydown ? text : 0);
    }
}

// ── frames ───────────────────────────────────────────────────────────────────────────────────────
static struct {
    unsigned long frames, dropped, bytes, cells;
    double since, fps, bps;
    unsigned long f0, d0, b0;
    double render_s;
} fs;
static char status_text[256];
static const double scale_of[] = {1.0, 0.75, 0.5, 0.34};
static int scale_level;
static int quant_now = -1;  // the precision chosen (layouts keep it)
static double budget;      // 0: no limit
static int behind;         // the last frame was still being sent when the next came
static size_t last_bytes;  // what the last frame took

static void layout(void)
{
    query_size();
    R.blocks = opt.blocks;
    R.color = opt.color;
    R.stretch = opt.stretch;
    R.cell_aspect = opt.cell_aspect;
    R.aspect = opt.aspect;
    if (quant_now < 0)
        quant_now = opt.quant;
    R.quant = R.color == RT_TRUECOLOR ? quant_now : 8;
    R.max_cols = opt.max_cols;
    R.max_rows = opt.max_rows;
    int rows = term_rows - (opt.status ? 1 : 0);
    if (scale_level > 0) {  // made smaller: the terminal is slow
        R.max_cols = (int)(term_cols * scale_of[scale_level]);
        R.max_rows = (int)((rows > 0 ? rows : 1) * scale_of[scale_level]);
    }
    rt_layout(&R, term_cols, rows > 0 ? rows : 1, 0);
    tracef("layout %d %d %s %s %d %d %d %d %d", term_cols, term_rows, rt_blocks_name(R.blocks),
           rt_color_name(R.color), R.ox, R.oy, R.cw, R.ch, R.wide);
    status_text[0] = 0;
}

static void cycle_blocks(void)
{
    if (opt.color == RT_MONO)
        return;  // characters only
    opt.blocks = (opt.blocks + 1) % RT_BLOCKS_N;
    winch = 1;
}

static void cycle_color(void)
{
    opt.color = (opt.color + 1) % RT_COLORS_N;
    if (opt.color == RT_MONO)
        opt.blocks = RT_ASCII;
    winch = 1;
}

// A terminal that takes bytes more slowly than they come makes writes wait — even on a
// non-blocking descriptor where the console's driver hands them to the host at once (collaboCore's
// guest: a write returns when the host has them). The bytes a write took while it waited, by the
// time it waited, are the terminal's pace.
static double pace;        // bytes a second (0: never seen it wait)
static int waited;         // the last frame's writes waited

static void flush_out(void)
{
    double t0 = real_s();
    size_t before = out_off;
    while (out_off < R.len) {
        ssize_t n = write(tty_fd, R.out + out_off, R.len - out_off);
        if (n > 0) {
            out_off += (size_t)n;
            fs.bytes += (unsigned long)n;
        } else if (!(n < 0 && errno == EINTR)) {
            break;  // EAGAIN: the terminal takes the rest later
        }
    }
    double dt = real_s() - t0;
    if (out_off - before >= 4096 && dt > 0.004) {
        double r = (double)(out_off - before) / dt;
        pace = pace > 0 ? pace * 0.6 + r * 0.4 : r;
        waited = 1;
    }
    if (out_off >= R.len)
        R.len = out_off = 0;
}

static const char *budget_text(void)
{
    static char b[32];
    if (budget <= 0)
        return "whole";
    snprintf(b, sizeof b, "%.0fK", budget / 1024);
    return b;
}

static void draw_status(void)
{
    double now = real_s();
    if (now - fs.since >= 1.0 || !status_text[0]) {
        double dt = now - fs.since;
        if (fs.since > 0 && dt > 0) {
            fs.fps = (fs.frames - fs.f0) / dt;
            fs.bps = (fs.bytes - fs.b0) / dt;
        }
        fs.f0 = fs.frames;
        fs.b0 = fs.bytes;
        fs.d0 = fs.dropped;
        fs.since = now;
        char line[256];
        int n = snprintf(line, sizeof line,
            " DOOM  %4.1f fps  %5.0f KB/s  frame %s  %s %s%s %dx%d  keys: %s  run %s",
            fs.fps, fs.bps / 1024, budget_text(), rt_blocks_name(R.blocks), rt_color_name(R.color),
            R.color == RT_TRUECOLOR ? (const char *[]){"", "/1", "/2", "/3", "/4", "/5", "/6", "/7", ""}[R.quant] : "", R.cw, R.ch,
            kitty_pushed ? "kitty" : win32_seen ? "win32" : "repeat", autorun ? "on" : "off");
        if (n > term_cols)
            line[term_cols] = 0;
        if (strcmp(line, status_text) == 0)
            return;
        snprintf(status_text, sizeof status_text, "%s", line);
        rt_printf(&R, "\x1b[%d;1H\x1b[0;7m%s\x1b[K\x1b[0m", term_rows, status_text);
        R.tx = R.ty = -1;
        R.tfg = R.tbg = RT_NONE;
    }
}

// The bytes a frame may take: what the terminal takes in a frame's time. No limit until it falls
// behind; then cut each time a frame is still being sent when the next comes, and raised a little
// for each frame it took in time (as TCP finds a connection's rate).

// Colour precision follows the budget: a bit less while cells keep being left for later (fewer
// colours: fewer bytes a cell), more again once the terminal keeps up.
static struct {
    int frames, calm, failed, stress, scale_failed;
    double deferred;
} aq;

// And the picture's size: when the precision is down to 3 bits and still most cells wait, the
// picture is made smaller (fewer cells: each frame whole again, if coarser) — 3/4, 1/2, 1/3 of the
// terminal — and larger again when the terminal keeps up.

static void adapt_quality(void)
{
    if (R.cw * R.ch == 0)
        return;
    aq.deferred += (double)R.deferred / (R.cw * R.ch);
    if (++aq.frames < TICRATE)
        return;
    double d = aq.deferred / aq.frames;  // the share of cells left for later, this second
    aq.frames = 0;
    aq.deferred = 0;
    int can_quant = R.color == RT_TRUECOLOR && !opt.fixed_quant;
    int can_scale = !opt.max_cols && !opt.max_rows;
    if (d > 0.10 && can_quant && R.quant > 3) {
        aq.failed = R.quant;
        rt_set_quant(&R, R.quant - 1);
        quant_now = R.quant;
        aq.calm = aq.stress = 0;
        tracef("quant %d", R.quant);
    } else if (d > 0.35 && can_scale && scale_level < 3) {
        if (++aq.stress >= 2) {  // two seconds of it
            aq.scale_failed = scale_level;
            scale_level++;
            winch = 1;  // laid out again
            aq.stress = 0;
            tracef("scale %d", scale_level);
        }
        aq.calm = 0;
    } else if (d < 0.01) {
        aq.stress = 0;
        // back up after 5 calm seconds, or 20 to the size that was too much last; then the
        // precision, after 3 (15)
        if (can_scale && scale_level > 0) {
            if (++aq.calm >= (scale_level - 1 == aq.scale_failed ? 20 : 5)) {
                scale_level--;
                winch = 1;
                aq.calm = 0;
                tracef("scale %d", scale_level);
            }
        } else if (can_quant && R.quant < opt.quant) {
            if (++aq.calm >= (R.quant + 1 == aq.failed ? 15 : 3)) {
                rt_set_quant(&R, R.quant + 1);
                quant_now = R.quant;
                aq.calm = 0;
                tracef("quant %d", R.quant);
            }
        }
    } else {
        aq.calm = aq.stress = 0;
    }
}

// A frame should take the terminal no more than this to take in: the rest of a tic is the game's
#define FRAME_SEND 0.018

static void set_budget(void)
{
    if (opt.bandwidth > 0) {
        budget = opt.bandwidth / TICRATE;
    } else if (behind) {
        double b = budget > 0 && budget < (double)last_bytes ? budget : (double)last_bytes;
        budget = b * 0.7 > 2048 ? b * 0.7 : 2048;
        behind = 0;
    } else if (waited) {
        // the last frame's writes waited: as much as the terminal takes in FRAME_SEND
        double b = pace * FRAME_SEND;
        if (budget <= 0 || b < budget)
            budget = b > 2048 ? b : 2048;
        else
            budget = budget * 1.03 < b ? budget * 1.03 : b;
        waited = 0;
    } else if (budget > 0) {
        // up a little (to find out whether it can take more), no further than a whole frame
        double whole = (double)R.cw * R.ch * (R.cell_bytes > 8 ? R.cell_bytes : 8) * 1.25;
        budget = budget * 1.03 < whole ? budget * 1.03 : whole;
    }
    R.budget = (size_t)budget;
}

static void interactive_frame(void)
{
    static uint32_t traced;
    uint32_t now = real_ms();
    if (trace && now - traced >= 1000) {
        traced = now;
        tracef("second tic %d frames %lu dropped %lu bytes %lu budget %.0f quant %d deferred %d pace %.0f", I_GetTime(),
               fs.frames, fs.dropped, fs.bytes, budget, R.quant, R.deferred, pace);
    }
    flush_out();
    if (R.len > 0) {  // the terminal is still taking the last frame: leave this one out
        fs.dropped++;
        behind = 1;
        return;
    }
    set_budget();
    if (winch || redraw) {
        winch = redraw = 0;
        layout();
        rt_puts(&R, "\x1b[0m\x1b[2J");
        R.tfg = R.tbg = RT_NONE;
        R.tx = R.ty = -1;
    }
    rt_puts(&R, "\x1b[?2026h");
    size_t empty = R.len;
    rt_frame(&R, DG_ScreenBuffer);
    if (opt.status)
        draw_status();
    if (R.len == empty) {
        R.len = 0;
        return;
    }
    rt_puts(&R, "\x1b[?2026l");
    fs.frames++;
    last_bytes = R.len;
    adapt_quality();
    flush_out();
}

static void write_ppm(const char *path)
{
    FILE *f = fopen(path, "wb");
    if (!f)
        I_Error("-shot: cannot write %s", path);
    fprintf(f, "P6\n%d %d\n255\n", DOOMGENERIC_RESX, DOOMGENERIC_RESY);
    for (int i = 0; i < DOOMGENERIC_RESX * DOOMGENERIC_RESY; i++) {
        uint32_t p = DG_ScreenBuffer[i];
        unsigned char rgb[3] = {(unsigned char)(p >> 16), (unsigned char)(p >> 8), (unsigned char)p};
        fwrite(rgb, 1, 3, f);
    }
    fclose(f);
}

static void headless_layout(void)
{
    if (R.cw == 0) {
        term_cols = opt.term_cols;
        term_rows = opt.term_rows;
        R.blocks = opt.blocks;
        R.color = opt.color;
        R.stretch = opt.stretch;
        R.cell_aspect = opt.cell_aspect;
        R.quant = R.color == RT_TRUECOLOR ? opt.quant : 8;
        R.max_cols = opt.max_cols;
        R.max_rows = opt.max_rows;
        rt_layout(&R, term_cols, term_rows, 0);
    }
}

static void write_ansi(const char *path)
{
    headless_layout();
    rt_invalidate(&R);
    R.len = 0;
    rt_puts(&R, "\x1b[0m\x1b[H\x1b[2J");
    R.tfg = R.tbg = RT_NONE;
    rt_frame(&R, DG_ScreenBuffer);
    rt_puts(&R, "\x1b[0m");
    FILE *f = fopen(path, "wb");
    if (!f)
        I_Error("-shotansi: cannot write %s", path);
    fwrite(R.out, 1, R.len, f);
    fclose(f);
    R.len = 0;
}

static void shot_path(char *buf, size_t n, const char *pattern, int tic)
{
    const char *pct = strstr(pattern, "%d");
    if (pct)
        snprintf(buf, n, "%.*s%d%s", (int)(pct - pattern), pattern, tic, pct + 2);
    else
        snprintf(buf, n, "%s", pattern);
}

static int shot_next;

static void headless_frame(void)
{
    if (opt.bench) {
        headless_layout();
        double t0 = real_s();
        R.len = 0;
        R.budget = opt.bandwidth > 0 ? (size_t)(opt.bandwidth / TICRATE) : 0;
        fs.cells += (unsigned long)rt_frame(&R, DG_ScreenBuffer);
        fs.dropped += (unsigned long)R.deferred;
        adapt_quality();
        fs.render_s += real_s() - t0;
        fs.bytes += R.len;
        fs.frames++;
        R.len = 0;
    }
    if (shot_next < opt.nshots && I_GetTime() >= opt.shottics[shot_next]) {
        int tic = opt.shottics[shot_next++];
        char path[1024];
        if (opt.shot) {
            shot_path(path, sizeof path, opt.shot, tic);
            size_t l = strlen(path);
            if (l > 4 && !strcasecmp(path + l - 4, ".ppm"))
                write_ppm(path);
            else {
                headless_layout();
                write_ansi(path);
            }
        }
        if (opt.shotansi) {
            shot_path(path, sizeof path, opt.shotansi, tic);
            write_ansi(path);
        }
        if (opt.dumpstate)
            dump_state();
        if (shot_next >= opt.nshots)
            I_Quit();
    }
}

static void bench_report(void)
{
    if (opt.bench && fs.frames)
        fprintf(stderr, "benchrender: %lu frames at %dx%d (%s, %s, %d bits at the end): %.3f ms a frame, %.0f bytes and %.0f cells a frame, %.0f cells left for later\n",
                fs.frames, R.cw, R.ch, rt_blocks_name(R.blocks), rt_color_name(R.color), R.quant,
                fs.render_s * 1000 / fs.frames, (double)fs.bytes / fs.frames, (double)fs.cells / fs.frames,
                (double)fs.dropped / fs.frames);
}

// ── doomgeneric's interface ──────────────────────────────────────────────────────────────────────
static int started;

void DG_Init(void)
{
    parse_options();
    rt_init(&R, DOOMGENERIC_RESX, DOOMGENERIC_RESY);
    tk_init(&dec);
    if (opt.headless) {
        virtual_clock = (opt.shot || opt.shotansi || opt.keys) && !opt.realtime;
        if (opt.keys)
            parse_script(opt.keys);
        if (opt.bench)
            atexit(bench_report);  // (not I_AtExit: D_Endoom ends that list with exit(0))
    }
}

void DG_DrawFrame(void)
{
    if (opt.headless) {
        headless_frame();
        return;
    }
    if (!started) {
        // the first frame: the terminal is ours from here (DOOM's start-up text stays above)
        started = 1;
        if (!session_start())  // (not on and on with nobody to see it)
            I_Error("doom: no terminal to draw on (/dev/tty): run it in a terminal, or with -headless, -shot");
    }
    if (session) {
        pump_input();
        interactive_frame();
    }
}

void DG_SleepMs(uint32_t ms)
{
    if (virtual_clock) {
        vclock += ms;
        return;
    }
    if (!session) {
        usleep(ms * 1000);
        return;
    }
    uint32_t end = real_ms() + ms;
    for (;;) {
        struct pollfd p = {tty_fd, (short)(POLLIN | (R.len > out_off ? POLLOUT : 0)), 0};
        int32_t left = (int32_t)(end - real_ms());
        if (left <= 0)
            break;
        if (poll(&p, 1, left) > 0) {
            if (p.revents & POLLOUT)
                flush_out();
            if (p.revents & POLLIN)
                read_input();
        }
    }
}

int DG_GetKey(int *pressed, unsigned char *key)
{
    (void)pressed;
    (void)key;
    if (opt.headless) {
        if (nscript)
            run_script();
    } else if (session) {
        pump_input();
    }
    return 0;  // the events are posted already (D_PostEvent): data1 and data2 apart
}

void DG_SetWindowTitle(const char *t)
{
    snprintf(title, sizeof title, "DOOM - %s", t);
    if (session)
        tty_printf("\x1b]2;%s\x07", title);
}

// ── ENDOOM: the text screen DOOM leaves at the end, from the WAD ─────────────────────────────────
static const uint16_t cp437[256] = {
    0x0020, 0x263a, 0x263b, 0x2665, 0x2666, 0x2663, 0x2660, 0x2022, 0x25d8, 0x25cb, 0x25d9, 0x2642, 0x2640, 0x266a, 0x266b, 0x263c,
    0x25ba, 0x25c4, 0x2195, 0x203c, 0x00b6, 0x00a7, 0x25ac, 0x21a8, 0x2191, 0x2193, 0x2192, 0x2190, 0x221f, 0x2194, 0x25b2, 0x25bc,
    0x0020, 0x0021, 0x0022, 0x0023, 0x0024, 0x0025, 0x0026, 0x0027, 0x0028, 0x0029, 0x002a, 0x002b, 0x002c, 0x002d, 0x002e, 0x002f,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, 0x0038, 0x0039, 0x003a, 0x003b, 0x003c, 0x003d, 0x003e, 0x003f,
    0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047, 0x0048, 0x0049, 0x004a, 0x004b, 0x004c, 0x004d, 0x004e, 0x004f,
    0x0050, 0x0051, 0x0052, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057, 0x0058, 0x0059, 0x005a, 0x005b, 0x005c, 0x005d, 0x005e, 0x005f,
    0x0060, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067, 0x0068, 0x0069, 0x006a, 0x006b, 0x006c, 0x006d, 0x006e, 0x006f,
    0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077, 0x0078, 0x0079, 0x007a, 0x007b, 0x007c, 0x007d, 0x007e, 0x2302,
    0x00c7, 0x00fc, 0x00e9, 0x00e2, 0x00e4, 0x00e0, 0x00e5, 0x00e7, 0x00ea, 0x00eb, 0x00e8, 0x00ef, 0x00ee, 0x00ec, 0x00c4, 0x00c5,
    0x00c9, 0x00e6, 0x00c6, 0x00f4, 0x00f6, 0x00f2, 0x00fb, 0x00f9, 0x00ff, 0x00d6, 0x00dc, 0x00a2, 0x00a3, 0x00a5, 0x20a7, 0x0192,
    0x00e1, 0x00ed, 0x00f3, 0x00fa, 0x00f1, 0x00d1, 0x00aa, 0x00ba, 0x00bf, 0x2310, 0x00ac, 0x00bd, 0x00bc, 0x00a1, 0x00ab, 0x00bb,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557, 0x255d, 0x255c, 0x255b, 0x2510,
    0x2514, 0x2534, 0x252c, 0x251c, 0x2500, 0x253c, 0x255e, 0x255f, 0x255a, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256c, 0x2567,
    0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256b, 0x256a, 0x2518, 0x250c, 0x2588, 0x2584, 0x258c, 0x2590, 0x2580,
    0x03b1, 0x00df, 0x0393, 0x03c0, 0x03a3, 0x03c3, 0x00b5, 0x03c4, 0x03a6, 0x0398, 0x03a9, 0x03b4, 0x221e, 0x03c6, 0x03b5, 0x2229,
    0x2261, 0x00b1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00f7, 0x2248, 0x00b0, 0x2219, 0x00b7, 0x221a, 0x207f, 0x00b2, 0x25a0, 0x00a0,
};

// VGA's text colours (attribute order: blue is bit 0)
static const uint32_t vga[16] = {
    0x000000, 0x0000aa, 0x00aa00, 0x00aaaa, 0xaa0000, 0xaa00aa, 0xaa5500, 0xaaaaaa,
    0x555555, 0x5555ff, 0x55ff55, 0x55ffff, 0xff5555, 0xff55ff, 0xffff55, 0xffffff,
};

void I_Endoom(byte *data)
{
    if (!was_interactive || !opt.endoom || !data)
        return;
    rt_t E;
    rt_init(&E, 1, 1);
    E.color = opt.color;
    E.tfg = E.tbg = RT_NONE - 1;
    int cols = term_cols < 80 ? term_cols : 80;
    for (int y = 0; y < 25; y++) {
        for (int x = 0; x < cols; x++) {
            unsigned ch = data[(y * 80 + x) * 2], attr = data[(y * 80 + x) * 2 + 1];
            uint32_t fg = vga[attr & 15], bg = vga[attr >> 4 & 7];
            if (E.color == RT_MONO) {
                if (y == 0 && x == 0)
                    rt_puts(&E, "\x1b[0m");
            } else if (E.color == RT_TRUECOLOR) {
                rt_set_colors(&E, fg, bg);
            } else if (E.color == RT_256) {
                rt_set_colors(&E, (uint32_t)rt_rgb_to_256(fg), (uint32_t)rt_rgb_to_256(bg));
            } else {
                rt_set_colors(&E, (uint32_t)rt_rgb_to_16(fg), (uint32_t)rt_rgb_to_16(bg));
            }
            uint32_t u = cp437[ch];
            char b[4];
            int n;
            if (u < 0x80) {
                b[0] = (char)u;
                n = 1;
            } else if (u < 0x800) {
                b[0] = (char)(0xc0 | u >> 6);
                b[1] = (char)(0x80 | (u & 0x3f));
                n = 2;
            } else {
                b[0] = (char)(0xe0 | u >> 12);
                b[1] = (char)(0x80 | (u >> 6 & 0x3f));
                b[2] = (char)(0x80 | (u & 0x3f));
                n = 3;
            }
            rt_put(&E, b, (size_t)n);
        }
        rt_puts(&E, "\x1b[0m\r\n");
        E.tfg = E.tbg = RT_NONE - 1;
    }
    fflush(stdout);
    write_all(1, E.out, E.len);
    rt_free(&E);
}

int main(int argc, char **argv)
{
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "-h") || !strcmp(argv[i], "-help") || !strcmp(argv[i], "--help")) {
            fputs(usage, stdout);
            return 0;
        }
        if (!strcmp(argv[i], "-version") || !strcmp(argv[i], "--version")) {
            printf("doom (collaboCore add-on) %s: doomgeneric, Chocolate Doom's engine\n", DOOMTERM_VERSION);
            return 0;
        }
    }
    doomgeneric_Create(argc, argv);
    for (;;)
        doomgeneric_Tick();
    return 0;
}
