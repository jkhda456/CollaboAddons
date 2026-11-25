/*
 * wpe-browser: a web browser for agents in the collaboCore guest — WPE WebKit showing one page on a
 * canvas of the guest's display (`gui`), and a control socket the `web` command talks to.
 *
 *   wpe-browser [--name NAME] [--size WxH] [--ephemeral] [--data-dir DIR] [--ignore-tls-errors]
 *               [--user-agent UA|PROFILE] [--header 'NAME: VALUE'] [--header-for HOST 'NAME: VALUE']
 *               [--lang LANGUAGES] [--timezone ZONE|auto] [--humanize on|off] [--inject FILE] [--devtools]
 *               [--proxy URL] [--proxy-bypass HOSTS] [--proxy-ca FILE] [URL]
 *
 * The socket is $COLLABO_WEB_DIR/NAME.sock (/tmp/.collabo-web by default). A request is a list of
 * words: "N\n" then N times "LENGTH\n" and the bytes; the answer is "STATUS LENGTH\n" and the
 * bytes (status 0 done, 1 failed, 2 usage, 3 timed out, 4 not found, 6 a request waits for the agent:
 * `web intercept`). See web.c for the commands.
 *
 * Input an agent asks for (click, type, key, scroll) is sent into the page as the display's own
 * pointer and keyboard events would be, so pages see real events (hover, focus, keydown...), and
 * with a person's paths and timing (human.c; `web humanize off`: at once). With --screenshot FILE an
 * action also leaves a picture of the page with where it acted marked.
 *
 * Who it says it is (`web ua`, `web identity`) is answered by WebKit itself in the page's processes
 * (WebKit patch 0012, through extension.c): navigator, screen, window and time zone, of a real
 * machine drawn from recorded ones, as Camoufox does for Firefox; the browser's own scripts run in a
 * JavaScript world the page cannot see (AGENT_WORLD).
 */
#define _GNU_SOURCE
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib/gstdio.h>
#include <glib-unix.h>
#include <math.h>
#include <png.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
#include <unicode/ucal.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>

#include "browser-js.h" /* snapshot_js, target_js, identity_js, devtools_js, challenge_js: the scripts, as C strings (gen-js.py) */
#include "human.h"      /* a person's paths and timing for the actions */

/* The browser's own scripts (reading the page, finding targets, counting the input it took) run in
 * a JavaScript world of their own, as an extension's content scripts do: they share the page's DOM
 * but not its globals or prototypes, so the page sees no variable, attribute or hook of theirs and
 * cannot change what they call. `web eval` and `web run` are the page's world unless --isolated. */
#define AGENT_WORLD "collabo"

/* An inspector frontend in the program (WebKit patch 0011): the inspector protocol's messages, as
 * text, to `callback` and from webkit_inspector_frontend_send; the developer tools' (devtools.js) */
extern void *webkit_web_view_connect_inspector_frontend(WebKitWebView *view, void (*callback)(const char *message, void *data), void *data);
extern void webkit_inspector_frontend_send(void *frontend, const char *message);
extern void webkit_inspector_frontend_disconnect(void *frontend);

/* tls.c: certificate authorities trusted besides the guest's (web proxy ca) */
extern void wpe_tls_trust_dir(const char *dir);
extern gboolean wpe_tls_trust_file(const char *dir, const char *file, char **subjects, char **error);
extern void wpe_tls_untrust_all(const char *dir);

enum { ST_OK = 0, ST_FAILED = 1, ST_USAGE = 2, ST_TIMEOUT = 3, ST_NOT_FOUND = 4, ST_PAUSED = 6, ST_UNSUPPORTED = 7 };

typedef struct Browser Browser;
typedef struct Request Request;
typedef struct JSCall JSCall;
typedef struct ShotCall ShotCall;
typedef void (*JSDone)(Request *r, JSCValue *value, GError *error);

/* JavaScript running for a request */
struct JSCall {
    Request *r;                 /* NULL once the request has been answered without it */
    JSDone done;
};

/* a snapshot of the page being taken for a request (--screenshot) */
struct ShotCall {
    Request *r;                 /* NULL once the request has gone on without it */
};

/* Who the browser says it is: the User-Agent header and, to match it, what the page's scripts see
 * (identity.js) and, for Chromium's, the client hints it sends (Sec-CH-UA...) */
typedef struct {
    char *profile;              /* a profile's name (ua_profiles), or NULL: a user agent of the caller's own */
    char *user_agent;
    char *platform, *vendor, *oscpu;
    gboolean touch, mobile;
    char *brand, *major;        /* Chromium's: "Google Chrome", "151"; NULL for the others */
    char *ch_platform, *ch_platform_version;
    char *os;                   /* the device's kind: "mac", "windows", "iphone", "android", "linux" */
} Identity;

/* The machine the identity is on, drawn from real devices' (device_presets) by the browser's seed:
 * what WebKit answers for navigator, screen and window (WebKit patch 0012, set in each web process
 * by extension.c) */
typedef struct {
    gboolean set;               /* FALSE: WebKit's own (the guest's display) */
    int screen[7];              /* width, height, availLeft, availTop, availWidth, availHeight, colorDepth */
    int window[4];              /* screenX, screenY, outer width - view width, outer height - view height */
    int cores, touch_points;
} Device;

/* an input event of an action, planned for its time (ms from the action's start) and sent then */
typedef enum { EV_MOVE, EV_DOWN, EV_UP, EV_KEY_DOWN, EV_KEY_UP, EV_MOD_DOWN, EV_MOD_UP, EV_WHEEL } EventKind;
typedef struct {
    EventKind kind;
    double t;
    unsigned seq;               /* order among events of the same time */
    double x, y;                /* MOVE, DOWN, UP, WHEEL: where; WHEEL: x, y and dx, dy */
    double dx, dy;
    guint button, keysym, code; /* DOWN, UP: button; KEY_*, MOD_*: keysym, code */
    unsigned bit;               /* MOD_*: the modifier */
    int count;                  /* DOWN: the click count */
} PlannedEvent;

typedef struct {
    GArray *events;             /* PlannedEvent */
    double t;                   /* where the plan is: ms from its start */
    double px, py;              /* where the pointer will be */
    gboolean have_pointer;
} Plan;

/* a header put on requests (`web header`): host "" every host, value "" takes it off */
typedef struct {
    char *host, *name, *value;
} Header;

/* a page the browser went to (`web history`) */
typedef struct {
    gint64 time;                /* microseconds since 1970 */
    char *uri, *title;
    gboolean saved;             /* in the history file */
} Visit;

/* a script run at the start of every page (`web inject`) */
typedef struct {
    char *name, *source;        /* name: the file's path */
} Script;

struct Browser {
    char *name;
    char *socket_path;
    WebKitWebView *view;
    WebKitNetworkSession *session;
    GMainLoop *loop;
    GSocketService *service;
    GList *load_waiters;        /* Requests waiting for the current navigation to end */
    gboolean loading;
    unsigned load_generation;   /* counts started navigations */
    char *last_load_error;
    char *last_dialog;
    unsigned web_process_crashes;
    unsigned modifiers;         /* WPEModifiers held by injected key events */
    unsigned sent[3];           /* input sent to the page: mousedown, mouseup, keydown (input sync) */
    double pointer_x, pointer_y; /* where the pointer is (the last move sent) */
    gboolean pointer_known;
    gint64 last_release;        /* when the last button was let go (µs, monotonic) */
    GRand *rand;                /* the human input's draws */
    gboolean humanize;          /* input as a person gives it (human.c), or each action at once */
    gboolean serving;           /* answering requests: from the end of the first load */
    WPEView *wpe_view;          /* for the ready line */
    /* settings (start options, `web set`, `web ua`, `web header`, `web lang`, `web inject`) */
    Identity identity;
    char *default_user_agent;   /* WebKit's own, with the machine named as desktop Linux */
    Device device;              /* the machine the identity is on */
    guint32 seed;               /* draws the device: kept in the data folder, so a browser stays the same machine */
    char *seed_file;            /* NULL: ephemeral, a new machine each start */
    int view_width, view_height; /* the page's size, for a device whose screen holds it */
    char *time_zone;            /* NULL: the guest's (UTC); an IANA name */
    char *identity_sent;        /* what the web processes have (printed) */
    GPtrArray *headers;         /* Header */
    GPtrArray *scripts;         /* Script */
    char **languages;           /* NULL: WebKit's, from the environment */
    char *languages_sent;       /* what WebKit has (joined) */
    char *rules_sent;           /* the header rules the web processes have (printed) */
    char *scripts_installed;    /* a checksum of the user scripts the view has */
    GPtrArray *history;         /* Visit, oldest first */
    char *history_file;         /* NULL: an ephemeral browser keeps it in memory only */
    guint history_save_id;
    /* the developer tools (`web devtools`, `web network`, `web har`): devtools.js records the network */
    void *inspector;            /* the inspector frontend, while they are on */
    JSCContext *devtools_context; /* devtools.js's own JavaScript context (not the page's) */
    JSCValue *devtools;         /* its object */
    GQueue *inspector_messages; /* from the inspector, for devtools.js */
    guint devtools_pump_id;
    char *devtools_bodies;      /* the response bodies kept: text, all, none */
    gboolean devtools_wanted;   /* --devtools: on as soon as the page has a process */
    gboolean devtools_reconnect; /* the page's process ended: connect again to the next one */
    gboolean page_started;      /* the first load has begun (the page has a process) */
    /* web intercept: requests that wait for the agent (devtools.js holds them) */
    gboolean intercepting;
    unsigned pause_serial;      /* how many requests paused so far */
    unsigned paused_count;      /* how many wait now */
    unsigned blocking_count;    /* of them, synchronous requests: the page's main thread waits for those */
    guint pause_wake_id;
    GList *pause_waiters;       /* Requests that end when a request pauses (run, intercept wait) */
    /* web intercept in the network process (WebKit patch 0013, netintercept.c): its socket */
    char *net_socket_path;
    int net_listen_fd, net_fd;  /* -1: none */
    GString *net_in, *net_out;
    guint net_read_id, net_write_id;
    unsigned net_connections;   /* network processes that connected (one at a time) */
    /* where the requests go (web proxy): through a proxy, or straight to the sites */
    char *proxy;                /* its address (USER:PASSWORD@ in it for one that asks), NULL: none */
    char **proxy_bypass;        /* the hosts that go straight; NULL: the default (the machine itself) */
    char *proxy_sent;           /* what the network session has (NULL: WebKit's default, which is none) */
    unsigned proxy_refused_load; /* the load (generation) the proxy refused to carry: why, in last_load_error */
    gboolean proxy_refused;     /* the proxy refused (407) since it was set: libsoup does not ask again */
    char *ca_dir;               /* certificate authorities trusted besides the guest's (tls.c) */
    GPtrArray *ca_trusted;      /* their lines (FILE: SUBJECT) */
};

struct Request {
    Browser *browser;
    GSocketConnection *connection;
    GByteArray *input;
    char **argv;
    int argc;
    guint timeout_id;
    guint poll_id;
    unsigned load_generation;   /* navigation started at or after this one ends the wait */
    gboolean wants_load;
    char *done_text;            /* the answer when the load ends */
    char *poll_kind, *poll_arg; /* wait text / selector */
    gint64 deadline;
    /* input sync: what the page had seen and what had been sent before this request's input */
    void (*send)(Request *r);
    void (*synced)(Request *r);
    gboolean have_base, generation_set;
    unsigned seen_base[3], sent_base[3];
    gint64 sync_deadline;
    guint sync_id;
    double x, y;                /* where the request acts, and what is there */
    double target_size;         /* how big what it aims at is (px): how fast a person's pointer gets there */
    char *what;
    JSCall *js;                 /* JavaScript running for it: told when the request is answered first */
    void (*after_bodies)(Request *r); /* network search, har save: once the bodies being read are in */
    unsigned pause_base;        /* the pauses there had been when it came: a later one ends its wait */
    char *decision;             /* intercept send|drop|respond: the decision devtools.js waits on WebKit for */
    GList *cookies;             /* web cookie: SoupCookies still to add or delete */
    unsigned cookies_done;
    /* --screenshot FILE: the page as the action met it (a click) or left it (the others), where
     * it acted marked */
    char *shot_file;
    gboolean acted;             /* its input went to the page (or its load began): a picture is due */
    gboolean shot_taken;        /* taken, being taken, or not to be */
    gboolean have_point;        /* r->x, r->y: where the pointer went */
    gboolean have_box;
    double box[4];              /* the element it acted on: x, y, width, height (view coordinates) */
    double arrow_dx, arrow_dy;  /* scroll: the way it went */
    double scroll_from[2];
    gboolean want_focus;        /* type without a target, key: mark what has the focus */
    char *shot_what;            /* the element, in a few words (button "Greet") */
    char *shot_label;           /* the picture's caption */
    const char *shot_when;      /* "before the click", "after typing"... */
    char *shot_note;            /* the line its answer ends with */
    int held_status;            /* its answer, held while the picture after the action is taken */
    char *held_text;
    ShotCall *shot;
    guint shot_timeout_id;
    void (*after_shot)(Request *r);
    /* input planned for its times (human input), being sent */
    GArray *plan;               /* PlannedEvent, in time order */
    guint plan_next;
    gint64 plan_start;          /* µs, monotonic */
    guint plan_id;
    void (*plan_done)(Request *r);
    /* web challenge: what it saw, how often it clicked */
    char *check_kind, *check_what;
    gint64 check_since;         /* when it first saw this check (µs, monotonic) */
    gint64 check_last_act;      /* when it last clicked or held */
    int check_acts;
    gboolean check_seen;        /* a check was on the page at some point */
    /* a load's answer goes here instead (title, challenge.js's line or NULL): web search */
    void (*loaded)(Request *r, const char *title, const char *check);
    char **engines;             /* web search: the engines to try, in order */
    int engine;
    gboolean settled;           /* its page had a moment to move on by itself */
    unsigned settle_generation;
    char *query;
    GString *skipped;
};

static Browser browser;

/* WPE_BROWSER_DEBUG=1: what happens to loads and waits, on stderr (`gui logs NAME`) */
static gboolean debugging;
#define DEBUG(...) do { if (debugging) g_printerr("wpe-browser: " __VA_ARGS__); } while (0)

/* ── answering ─────────────────────────────────────────────────────────────────────────────── */

static void request_free(Request *r)
{
    if (r->timeout_id)
        g_source_remove(r->timeout_id);
    if (r->poll_id)
        g_source_remove(r->poll_id);
    if (r->sync_id)
        g_source_remove(r->sync_id);
    if (r->js)
        r->js->r = NULL; /* answered (timed out) before its JavaScript was: its end goes nowhere */
    if (r->shot_timeout_id)
        g_source_remove(r->shot_timeout_id);
    if (r->shot)
        r->shot->r = NULL;
    if (r->plan_id) {
        /* its input stopped half way: no key or button stays down */
        g_source_remove(r->plan_id);
        browser.modifiers = 0;
    }
    if (r->plan)
        g_array_free(r->plan, TRUE);
    g_list_free_full(r->cookies, (GDestroyNotify)soup_cookie_free);
    browser.load_waiters = g_list_remove(browser.load_waiters, r);
    browser.pause_waiters = g_list_remove(browser.pause_waiters, r);
    if (r->connection) {
        g_io_stream_close(G_IO_STREAM(r->connection), NULL, NULL);
        g_object_unref(r->connection);
    }
    if (r->input)
        g_byte_array_unref(r->input);
    g_strfreev(r->argv);
    g_free(r->done_text);
    g_free(r->poll_kind);
    g_free(r->poll_arg);
    g_free(r->what);
    g_free(r->decision);
    g_free(r->shot_file);
    g_free(r->shot_what);
    g_free(r->shot_label);
    g_free(r->shot_note);
    g_free(r->held_text);
    g_free(r->check_kind);
    g_free(r->check_what);
    g_strfreev(r->engines);
    g_free(r->query);
    if (r->skipped)
        g_string_free(r->skipped, TRUE);
    g_free(r);
}

static void shot_hold(Request *r, int status, const char *text);

static void reply(Request *r, int status, const char *text)
{
    if (!text)
        text = "";
    /* --screenshot: the answer to an action waits for the picture of what it did */
    if (r->shot_file && r->acted && !r->shot_taken) {
        shot_hold(r, status, text);
        return;
    }
    const char *note = r->shot_note ? r->shot_note : "";
    GOutputStream *out = g_io_stream_get_output_stream(G_IO_STREAM(r->connection));
    char *head = g_strdup_printf("%d %zu\n", status, strlen(text) + strlen(note));
    g_output_stream_write_all(out, head, strlen(head), NULL, NULL, NULL);
    g_output_stream_write_all(out, text, strlen(text), NULL, NULL, NULL);
    g_output_stream_write_all(out, note, strlen(note), NULL, NULL, NULL);
    g_output_stream_flush(out, NULL, NULL);
    g_free(head);
    request_free(r);
}

static void replyf(Request *r, int status, const char *format, ...) G_GNUC_PRINTF(3, 4);
static void replyf(Request *r, int status, const char *format, ...)
{
    va_list args;
    va_start(args, format);
    char *text = g_strdup_vprintf(format, args);
    va_end(args);
    reply(r, status, text);
    g_free(text);
}

static char *page_state(void)
{
    const char *uri = webkit_web_view_get_uri(browser.view);
    const char *title = webkit_web_view_get_title(browser.view);
    return g_strdup_printf("%s\n%s\n", uri ? uri : "about:blank", title ? title : "");
}

/* ── navigation and waiting for it ────────────────────────────────────────────────────────── */

static void call_js(Request *r, const char *body, GVariant *args, JSDone done);

/* web intercept: requests paused while the page loaded (a fetch does not hold its load) */
static const char *paused_note(void)
{
    return !browser.paused_count ? "" : browser.paused_count == 1 ? "(1 request is paused: web intercept lists it)\n"
                                                                  : "(requests are paused: web intercept lists them)\n";
}

/* The answer to a finished load: its address and the document's title (the view's title
 * property can come after the load has finished). */
/* challenge.js's line as the last of a load's answer: "challenge: WHAT (web challenge ...)" */
static char *check_note(const char *line)
{
    char **f = line && *line ? g_strsplit(line, "\t", 6) : NULL;
    char *note = NULL;
    if (f && g_strv_length(f) == 6) {
        const char *how = !strcmp(f[1], "blocked") ? "the site turns this browser away"
                        : !strcmp(f[1], "agent") ? "web screenshot shows it; web challenge says more"
                        : "web challenge solve gets past it";
        note = g_strdup_printf("challenge: %s: %s (%s)\n", f[0], f[5], how);
    }
    g_strfreev(f);
    return note ? note : g_strdup("");
}

static void loaded_title_read(Request *r, JSCValue *value, GError *error)
{
    char *text = !error && value && jsc_value_is_string(value) ? jsc_value_to_string(value) : NULL;
    char *check = text ? strchr(text, '\001') : NULL;
    if (check)
        *check++ = 0;
    if (r->loaded) {
        r->loaded(r, text ? text : "", check && *check ? check : NULL);
        g_free(text);
        return;
    }
    char *note = check_note(check);
    const char *uri = webkit_web_view_get_uri(browser.view);
    const char *view_title = webkit_web_view_get_title(browser.view);
    replyf(r, ST_OK, "%s%s\n%s\n%s%s", r->done_text ? r->done_text : "", uri ? uri : "about:blank",
           text ? text : view_title ? view_title : "", note, paused_note());
    g_free(note);
    g_free(text);
}

static void reply_loaded(Request *r)
{
    browser.load_waiters = g_list_remove(browser.load_waiters, r);
    r->wants_load = FALSE;
    if (r->timeout_id) {
        g_source_remove(r->timeout_id);
        r->timeout_id = 0;
    }
    if (r->poll_id) {
        g_source_remove(r->poll_id);
        r->poll_id = 0;
    }
    char *body = g_strconcat("return document.title + '\\u0001' + ((", challenge_js, ")() || '');", NULL);
    call_js(r, body, NULL, loaded_title_read);
    g_free(body);
}

static void finish_load_waiters(int status, const char *why)
{
    GList *waiters = g_list_copy(browser.load_waiters);
    for (GList *l = waiters; l; l = l->next) {
        Request *r = l->data;
        /* a load that started before the request (the first about:blank, a load the new one
         * replaces) is not the one it waits for */
        if (!r->wants_load || browser.load_generation <= r->load_generation)
            continue;
        DEBUG("%s: answered (%s)\n", r->argv[0], why ? why : "loaded");
        if (status == ST_OK) {
            reply_loaded(r);
            continue;
        }
        char *state = page_state();
        replyf(r, status, "%s\n%s", why, state);
        g_free(state);
    }
    g_list_free(waiters);
}

static gboolean load_timed_out(gpointer data)
{
    Request *r = data;
    r->timeout_id = 0;
    char *state = page_state();
    replyf(r, ST_TIMEOUT, "still loading (%.0f%%); the page may be usable already\n%s",
           webkit_web_view_get_estimated_load_progress(browser.view) * 100, state);
    g_free(state);
    return G_SOURCE_REMOVE;
}

/* Answer `r` when the next navigation ends (or now, if none starts within `start_grace_ms`). */
static gboolean grace_over(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    DEBUG("%s: no load started within the grace (loading %d, generation %u/%u)\n", r->argv[0], browser.loading,
          browser.load_generation, r->load_generation);
    if (browser.loading || browser.load_generation > r->load_generation)
        return G_SOURCE_REMOVE; /* a load started: the waiter answers when it ends */
    char *state = page_state();
    replyf(r, ST_OK, "%s%s", r->done_text ? r->done_text : "", state);
    g_free(state);
    return G_SOURCE_REMOVE;
}

static void reply_loaded(Request *r);
static void reply_paused(Request *r);

static void wait_for_load(Request *r, int timeout_ms, int start_grace_ms)
{
    /* web intercept: a request it made paused already (a click's fetch, while the input went in) */
    if (browser.paused_count && browser.pause_serial > r->pause_base) {
        reply_paused(r);
        return;
    }
    r->wants_load = TRUE;
    /* input commands count from when they sent their input (input_base_read): the load it
     * started may be over already */
    if (!r->generation_set)
        r->load_generation = browser.load_generation;
    if (browser.load_generation > r->load_generation && !browser.loading) {
        if (browser.last_load_error) {
            char *state = page_state();
            replyf(r, ST_FAILED, "%s\n%s", browser.last_load_error, state);
            g_free(state);
        } else
            reply_loaded(r);
        return;
    }
    browser.load_waiters = g_list_append(browser.load_waiters, r);
    r->timeout_id = g_timeout_add(timeout_ms, load_timed_out, r);
    if (start_grace_ms >= 0 && !browser.loading)
        r->poll_id = g_timeout_add(start_grace_ms, grace_over, r);
}

/* Requests are answered from the end of the browser's first load (about:blank or the page it
 * was started with): its events would otherwise end the waits of the first requests. Connections
 * made before wait in the socket's backlog. */
static void start_serving(void)
{
    if (browser.serving)
        return;
    browser.serving = TRUE;
    g_socket_service_start(browser.service);
    g_print("wpe-browser: \"%s\" ready on %s (display: %s)\n", browser.name, browser.socket_path,
            browser.wpe_view ? G_OBJECT_TYPE_NAME(wpe_view_get_display(browser.wpe_view)) : "none");
}

static gboolean start_serving_anyway(gpointer data)
{
    (void)data;
    start_serving();
    return G_SOURCE_REMOVE;
}

/* ── history: every page the browser went to, kept in its data folder (history.tsv) ──────── */

#define HISTORY_KEPT 5000

static void visit_free(Visit *visit)
{
    g_free(visit->uri);
    g_free(visit->title);
    g_free(visit);
}

/* "TIME\tURL\tTITLE": tabs and line ends in a title are spaces */
static void history_save(void)
{
    if (browser.history_save_id) {
        g_source_remove(browser.history_save_id);
        browser.history_save_id = 0;
    }
    if (!browser.history_file)
        return;
    GString *lines = g_string_new(NULL);
    for (guint i = 0; i < browser.history->len; i++) {
        Visit *visit = g_ptr_array_index(browser.history, i);
        if (visit->saved)
            continue;
        char *title = g_strdup(visit->title ? visit->title : "");
        g_strdelimit(title, "\t\r\n", ' ');
        g_string_append_printf(lines, "%" G_GINT64_FORMAT "\t%s\t%s\n", visit->time / 1000, visit->uri, title);
        g_free(title);
        visit->saved = TRUE;
    }
    if (lines->len) {
        FILE *out = fopen(browser.history_file, "a");
        if (out) {
            fputs(lines->str, out);
            fclose(out);
        }
    }
    g_string_free(lines, TRUE);
}

static gboolean history_save_soon(gpointer data)
{
    (void)data;
    browser.history_save_id = 0;
    history_save();
    return G_SOURCE_REMOVE;
}

/* A page the view went to (committed: after its redirects). Saved when its load ends, or a few
 * seconds after a move within the page (a single-page application's pushState: its title tends to
 * come after). */
static void history_visit(const char *uri)
{
    if (!uri || !*uri || g_str_has_prefix(uri, "about:"))
        return;
    history_save();
    Visit *visit = g_new0(Visit, 1);
    visit->time = g_get_real_time();
    visit->uri = g_strdup(uri);
    g_ptr_array_add(browser.history, visit);
    if (browser.history->len > HISTORY_KEPT + 500)
        g_ptr_array_remove_range(browser.history, 0, browser.history->len - HISTORY_KEPT);
}

static void history_load(const char *file)
{
    browser.history_file = g_strdup(file);
    char *text = NULL;
    if (!g_file_get_contents(file, &text, NULL, NULL))
        return;
    char **lines = g_strsplit(text, "\n", -1);
    for (char **line = lines; *line; line++) {
        char **fields = g_strsplit(*line, "\t", 3);
        if (g_strv_length(fields) >= 2 && *fields[1]) {
            Visit *visit = g_new0(Visit, 1);
            visit->time = g_ascii_strtoll(fields[0], NULL, 10) * 1000;
            visit->uri = g_strdup(fields[1]);
            visit->title = g_strdup(fields[2] ? fields[2] : "");
            visit->saved = TRUE;
            g_ptr_array_add(browser.history, visit);
        }
        g_strfreev(fields);
    }
    g_strfreev(lines);
    g_free(text);
    if (browser.history->len > HISTORY_KEPT) {
        /* the file keeps the last HISTORY_KEPT */
        g_ptr_array_remove_range(browser.history, 0, browser.history->len - HISTORY_KEPT);
        for (guint i = 0; i < browser.history->len; i++)
            ((Visit *)g_ptr_array_index(browser.history, i))->saved = FALSE;
        unlink(file);
        history_save();
    }
}

/* the page's title, for its visit */
static void history_title(const char *uri, const char *title)
{
    Visit *last = browser.history->len ? g_ptr_array_index(browser.history, browser.history->len - 1) : NULL;
    if (!last || g_strcmp0(last->uri, uri) || !title || !*title)
        return;
    g_free(last->title);
    last->title = g_strdup(title);
}

/* A move within the page (pushState, a #fragment) adds to the back/forward list without a load */
static void on_back_forward_changed(WebKitBackForwardList *list, WebKitBackForwardListItem *added, gpointer removed, gpointer data)
{
    (void)list; (void)removed; (void)data;
    if (!added || browser.loading)
        return; /* a load's: its commit records it */
    const char *uri = webkit_back_forward_list_item_get_uri(added);
    Visit *last = browser.history->len ? g_ptr_array_index(browser.history, browser.history->len - 1) : NULL;
    if (last && !g_strcmp0(last->uri, uri))
        return;
    history_visit(uri);
    history_title(uri, webkit_web_view_get_title(browser.view));
    if (!browser.history_save_id)
        browser.history_save_id = g_timeout_add_seconds(3, history_save_soon, NULL);
}

static void devtools_connect_again(void);

static void on_load_changed(WebKitWebView *view, WebKitLoadEvent event, gpointer data)
{
    (void)data;
    DEBUG("load %s: %s (generation %u, %u waiting)\n",
          event == WEBKIT_LOAD_STARTED ? "started" : event == WEBKIT_LOAD_REDIRECTED ? "redirected"
          : event == WEBKIT_LOAD_COMMITTED ? "committed" : "finished",
          webkit_web_view_get_uri(view), browser.load_generation, g_list_length(browser.load_waiters));
    switch (event) {
    case WEBKIT_LOAD_STARTED:
        browser.loading = TRUE;
        browser.load_generation++;
        g_clear_pointer(&browser.last_load_error, g_free);
        if (browser.devtools_reconnect) /* a load the browser did not ask for (the page's own) */
            devtools_connect_again();
        break;
    case WEBKIT_LOAD_COMMITTED:
        history_visit(webkit_web_view_get_uri(view));
        break;
    case WEBKIT_LOAD_FINISHED:
        /* saved a little later: a title can come after the load (and the next visit saves it first) */
        if (!browser.history_save_id)
            browser.history_save_id = g_timeout_add_seconds(3, history_save_soon, NULL);
        browser.loading = FALSE;
        finish_load_waiters(browser.last_load_error ? ST_FAILED : ST_OK, browser.last_load_error);
        start_serving();
        break;
    default:
        break;
    }
}

static char *proxy_shown(const char *address);

static gboolean on_load_failed(WebKitWebView *view, WebKitLoadEvent event, const char *uri, GError *error, gpointer data)
{
    (void)view; (void)event; (void)data;
    DEBUG("load failed: %s: %s (%s %d)\n", uri, error->message, g_quark_to_string(error->domain), error->code);
    if (g_error_matches(error, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED))
        return FALSE;
    if (browser.last_load_error && browser.proxy_refused_load == browser.load_generation)
        return TRUE; /* why: the proxy wanted credentials (on_authenticate) */
    g_free(browser.last_load_error);
    if (error->domain == G_TLS_ERROR) {
        /* certificates and the status responses servers staple to them are checked against the
         * clock: a sandbox whose clock is days off fails here on sites that are fine */
        GDateTime *now = g_date_time_new_now_utc();
        char *when = g_date_time_format(now, "%Y-%m-%d %H:%M UTC");
        browser.last_load_error = g_strdup_printf("failed to load %s: %s (checked against this sandbox's clock, %s)",
                                                  uri, error->message, when);
        g_free(when);
        g_date_time_unref(now);
        if (error->code == G_TLS_ERROR_BAD_CERTIFICATE) { /* a certificate authority the guest does not know: a proxy's? */
            char *with = g_strdup_printf("%s; web proxy ca FILE trusts a proxy's or your own certificate authority",
                                         browser.last_load_error);
            g_free(browser.last_load_error);
            browser.last_load_error = with;
        }
    } else if (strstr(error->message, "Blocked by Web Inspector")) /* the inspector's failure: web intercept drop */
        browser.last_load_error = g_strdup_printf("failed to load %s: dropped (web intercept drop)", uri);
    else if (browser.proxy && !strstr(error->message, "Blocked by web intercept")) {
        char *shown = proxy_shown(browser.proxy);
        browser.last_load_error = g_strdup_printf("failed to load %s: %s (through the proxy %s%s: web proxy%s)", uri, error->message, shown,
                                                  browser.proxy_refused ? ", which refused the user and password given (407)" : "",
                                                  browser.proxy_refused ? " SCHEME://USER:PASSWORD@HOST:PORT" : "");
        g_free(shown);
    } else
        browser.last_load_error = g_strdup_printf("failed to load %s: %s", uri, error->message);
    g_printerr("wpe-browser: %s\n", browser.last_load_error);
    /* handled: WebKit's error page would be a load of its own, which would end the waits as a
     * success; the page stays as it was and the error goes to whoever waits (and `web info`) */
    return TRUE;
}

static gboolean on_tls_errors(WebKitWebView *view, const char *uri, GTlsCertificate *certificate, GTlsCertificateFlags errors, gpointer data)
{
    (void)view; (void)certificate; (void)data;
    g_free(browser.last_load_error);
    browser.last_load_error = g_strdup_printf("failed to load %s: the server's certificate is not trusted (TLS errors 0x%x; "
                                              "web proxy ca FILE trusts a proxy's or your own certificate authority, "
                                              "web --ignore-tls-errors at a browser's start any certificate)", uri, errors);
    return FALSE;
}

/* A user and password asked for. The proxy's (407): libsoup answers with the proxy address's own
 * (web proxy http://USER:PASSWORD@HOST:PORT; here when it did not); refused, or none given, the
 * load fails saying so. A site's (401): not answered (WPE would keep the load waiting for an
 * answer that never comes): the page gets the site's answer, as a browser whose dialog was closed.
 * Not answering is going on without credentials (cancelling would cancel the load). */
static gboolean on_authenticate(WebKitWebView *view, WebKitAuthenticationRequest *request, gpointer data)
{
    (void)view; (void)data;
    if (!webkit_authentication_request_is_for_proxy(request)) {
        DEBUG("%s:%u asks for a user and password (realm \"%s\"): not given\n", webkit_authentication_request_get_host(request),
              webkit_authentication_request_get_port(request), webkit_authentication_request_get_realm(request));
        webkit_authentication_request_authenticate(request, NULL);
        return TRUE;
    }
    char *user = NULL, *password = NULL;
    if (browser.proxy)
        g_uri_split_with_user(browser.proxy, G_URI_FLAGS_HAS_PASSWORD, NULL, &user, &password, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
    if (user && !webkit_authentication_request_is_retry(request)) {
        WebKitCredential *credential = webkit_credential_new(user, password ? password : "", WEBKIT_CREDENTIAL_PERSISTENCE_FOR_SESSION);
        webkit_authentication_request_authenticate(request, credential);
        webkit_credential_free(credential);
    } else {
        char *shown = browser.proxy ? proxy_shown(browser.proxy)
                                    : g_strdup_printf("%s:%u", webkit_authentication_request_get_host(request),
                                                      webkit_authentication_request_get_port(request));
        g_free(browser.last_load_error);
        browser.last_load_error = g_strdup_printf("the proxy %s %s (407 Proxy Authentication Required): web proxy "
                                                  "SCHEME://USER:PASSWORD@HOST:PORT", shown,
                                                  user ? "refused the user and password given" : "wants a user and password");
        g_printerr("wpe-browser: %s\n", browser.last_load_error);
        browser.proxy_refused_load = browser.load_generation; /* the load's own failure does not replace it */
        browser.proxy_refused = TRUE;
        g_free(shown);
        webkit_authentication_request_authenticate(request, NULL);
    }
    g_free(user);
    g_free(password);
    return TRUE;
}

static char *normalize_uri(const char *text)
{
    if (strstr(text, "://") || g_str_has_prefix(text, "about:") || g_str_has_prefix(text, "data:") || g_str_has_prefix(text, "file:"))
        return g_strdup(text);
    if (text[0] == '/')
        return g_strconcat("file://", text, NULL);
    if (!strchr(text, ' ') && (strchr(text, '.') || g_str_has_prefix(text, "localhost")))
        return g_strconcat(strncmp(text, "localhost", 9) ? "https://" : "http://", text, NULL);
    /* words: a search */
    char *query = g_uri_escape_string(text, NULL, FALSE);
    char *uri = g_strconcat("https://html.duckduckgo.com/html/?q=", query, NULL);
    g_free(query);
    return uri;
}

/* ── JavaScript ────────────────────────────────────────────────────────────────────────────── */


static JSCall *js_call_new(Request *r, JSDone done)
{
    JSCall *call = g_new0(JSCall, 1);
    call->r = r;
    call->done = done;
    r->js = call;
    return call;
}

static void js_call_end(JSCall *call, JSCValue *value, GError *error)
{
    if (call->r) {
        call->r->js = NULL;
        call->done(call->r, value, error);
    }
    if (value)
        g_object_unref(value);
    g_clear_error(&error);
    g_free(call);
}

static void js_finished(GObject *source, GAsyncResult *result, gpointer data)
{
    GError *error = NULL;
    JSCValue *value = webkit_web_view_call_async_javascript_function_finish(WEBKIT_WEB_VIEW(source), result, &error);
    js_call_end(data, value, error);
}

static void eval_finished(GObject *source, GAsyncResult *result, gpointer data)
{
    GError *error = NULL;
    JSCValue *value = webkit_web_view_evaluate_javascript_finish(WEBKIT_WEB_VIEW(source), result, &error);
    js_call_end(data, value, error);
}

/* Runs `body` (an async function's body) with `args` (a{sv}, floating ok) as its variables, in the
 * browser's own world (AGENT_WORLD): the page's DOM, none of its globals */
static void call_js(Request *r, const char *body, GVariant *args, JSDone done)
{
    webkit_web_view_call_async_javascript_function(browser.view, body, -1, args, AGENT_WORLD, NULL, NULL, js_finished,
                                                   js_call_new(r, done));
}

/* `s` as a JSON (and JavaScript) string literal */
static void json_string(GString *out, const char *s)
{
    g_string_append_c(out, '"');
    for (const unsigned char *p = (const unsigned char *)(s ? s : ""); *p; p++) {
        if (*p == '"' || *p == '\\') {
            g_string_append_c(out, '\\');
            g_string_append_c(out, *p);
        } else if (*p < 0x20)
            g_string_append_printf(out, "\\u%04x", *p);
        else
            g_string_append_c(out, *p);
    }
    g_string_append_c(out, '"');
}

static char *js_value_text(JSCValue *value)
{
    if (!value || jsc_value_is_undefined(value))
        return g_strdup("undefined");
    if (jsc_value_is_string(value))
        return jsc_value_to_string(value);
    char *json = jsc_value_to_json(value, 2);
    return json ? json : jsc_value_to_string(value);
}

static void reply_js_text(Request *r, JSCValue *value, GError *error)
{
    if (error) {
        replyf(r, ST_FAILED, "JavaScript error: %s\n", error->message);
        return;
    }
    char *text = js_value_text(value);
    size_t length = strlen(text);
    replyf(r, ST_OK, "%s%s", text, length && text[length - 1] == '\n' ? "" : "\n");
    g_free(text);
}

static GVariant *args_dict(const char *first_key, ...)
{
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a{sv}"));
    va_list ap;
    va_start(ap, first_key);
    for (const char *key = first_key; key; key = va_arg(ap, const char *))
        g_variant_builder_add(&builder, "{sv}", key, va_arg(ap, GVariant *));
    va_end(ap);
    return g_variant_builder_end(&builder);
}

/* ── input, as the display would send it ──────────────────────────────────────────────────── */

static WPEView *wpe_view(void)
{
    return webkit_web_view_get_wpe_view(browser.view);
}

static guint32 now_ms(void)
{
    return (guint32)(g_get_monotonic_time() / 1000);
}

static void send_event(WPEEvent *event)
{
    wpe_view_event(wpe_view(), event);
    wpe_event_unref(event);
}

static unsigned button_bit(guint button)
{
    return button == 1 ? WPE_MODIFIER_POINTER_BUTTON1 : button == 2 ? WPE_MODIFIER_POINTER_BUTTON2 : WPE_MODIFIER_POINTER_BUTTON3;
}

static void pointer_move(double x, double y)
{
    double dx = browser.pointer_known ? x - browser.pointer_x : 0, dy = browser.pointer_known ? y - browser.pointer_y : 0;
    send_event(wpe_event_pointer_move_new(WPE_EVENT_POINTER_MOVE, wpe_view(), WPE_INPUT_SOURCE_MOUSE, now_ms(),
                                          (WPEModifiers)browser.modifiers, x, y, dx, dy));
    browser.pointer_x = x;
    browser.pointer_y = y;
    browser.pointer_known = TRUE;
}

static void pointer_button(gboolean down, double x, double y, guint button, int count)
{
    unsigned held = button_bit(button);
    if (down) {
        send_event(wpe_event_pointer_button_new(WPE_EVENT_POINTER_DOWN, wpe_view(), WPE_INPUT_SOURCE_MOUSE, now_ms(),
                                                (WPEModifiers)browser.modifiers, button, x, y, count));
        browser.modifiers |= held;
        browser.sent[0]++;
    } else {
        browser.modifiers &= ~held;
        send_event(wpe_event_pointer_button_new(WPE_EVENT_POINTER_UP, wpe_view(), WPE_INPUT_SOURCE_MOUSE, now_ms(),
                                                (WPEModifiers)(browser.modifiers | held), button, x, y, 0));
        browser.sent[1]++;
        browser.last_release = g_get_monotonic_time();
    }
}

typedef struct { const char *name; guint keysym; guint code; } KeyName;
static const KeyName key_names[] = {
    { "Enter", 0xff0d, 28 }, { "Return", 0xff0d, 28 }, { "Tab", 0xff09, 15 }, { "Escape", 0xff1b, 1 },
    { "Esc", 0xff1b, 1 }, { "Backspace", 0xff08, 14 }, { "BackSpace", 0xff08, 14 }, { "Delete", 0xffff, 111 },
    { "Insert", 0xff63, 110 }, { "Home", 0xff50, 102 }, { "End", 0xff57, 107 }, { "PageUp", 0xff55, 104 },
    { "Page_Up", 0xff55, 104 }, { "PageDown", 0xff56, 109 }, { "Page_Down", 0xff56, 109 },
    { "ArrowLeft", 0xff51, 105 }, { "Left", 0xff51, 105 }, { "ArrowUp", 0xff52, 103 }, { "Up", 0xff52, 103 },
    { "ArrowRight", 0xff53, 106 }, { "Right", 0xff53, 106 }, { "ArrowDown", 0xff54, 108 }, { "Down", 0xff54, 108 },
    { "Space", ' ', 57 }, { "space", ' ', 57 }, { "F1", 0xffbe, 59 }, { "F2", 0xffbf, 60 }, { "F3", 0xffc0, 61 },
    { "F4", 0xffc1, 62 }, { "F5", 0xffc2, 63 }, { "F6", 0xffc3, 64 }, { "F7", 0xffc4, 65 }, { "F8", 0xffc5, 66 },
    { "F9", 0xffc6, 67 }, { "F10", 0xffc7, 68 }, { "F11", 0xffc8, 87 }, { "F12", 0xffc9, 88 },
};

/* The US keyboard's key for an ASCII character: its evdev code, and whether it needs Shift. */
static guint ascii_code(gunichar c, gboolean *shift)
{
    static const char row[] = "1234567890-=qwertyuiop[]asdfghjkl;'`\\zxcvbnm,./";
    static const char shifted[] = "!@#$%^&*()_+QWERTYUIOP{}ASDFGHJKL:\"~|ZXCVBNM<>?";
    static const guint codes[] = { 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
                                   30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53 };
    *shift = FALSE;
    if (c == ' ')
        return 57;
    for (size_t i = 0; i < sizeof codes / sizeof *codes; i++) {
        if ((gunichar)row[i] == c)
            return codes[i];
        if ((gunichar)shifted[i] == c) {
            *shift = TRUE;
            return codes[i];
        }
    }
    return 0;
}

static void key_event(gboolean down, guint keysym, guint code)
{
    if (down)
        browser.sent[2]++;
    send_event(wpe_event_keyboard_new(down ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP, wpe_view(),
                                      WPE_INPUT_SOURCE_KEYBOARD, now_ms(), (WPEModifiers)browser.modifiers,
                                      code ? code + 8 : 0, keysym));
}

static void modifier(gboolean down, unsigned bit, guint keysym, guint code)
{
    if (down) {
        key_event(TRUE, keysym, code);
        browser.modifiers |= bit;
    } else {
        browser.modifiers &= ~bit;
        key_event(FALSE, keysym, code);
    }
}

/* ── input planned for its times: a person's (human.c), or all at once ────────────────────────
 * An action's events are planned first — the pointer's path, the dwell before a press, how long a
 * button or key is held, the gaps between keys, a wheel's strokes — and then sent at their times,
 * each as the display sends a real device's event. With humanize off they are all at time 0. */

static Plan *plan_new(void)
{
    Plan *plan = g_new0(Plan, 1);
    plan->events = g_array_new(FALSE, FALSE, sizeof(PlannedEvent));
    plan->px = browser.pointer_x;
    plan->py = browser.pointer_y;
    plan->have_pointer = browser.pointer_known;
    return plan;
}

static PlannedEvent *plan_add(Plan *plan, EventKind kind, double t)
{
    PlannedEvent event = { .kind = kind, .t = t, .seq = plan->events->len };
    g_array_append_val(plan->events, event);
    return &g_array_index(plan->events, PlannedEvent, plan->events->len - 1);
}

/* a delay of a person's (median, p90 factor, bounds), or none with humanize off */
static double human(double median, double spread, double min, double max)
{
    return browser.humanize ? human_delay(browser.rand, median, spread, min, max) : 0;
}

/* the pointer to (x, y), at a target `size` px across */
static void plan_pointer_to(Plan *plan, double x, double y, double size)
{
    x = round(x);
    y = round(y);
    if (plan->have_pointer && plan->px == x && plan->py == y)
        return;
    if (browser.humanize) {
        if (!plan->have_pointer) {
            /* where the hand left it: somewhere in the view, toward its lower right */
            int width = browser.view_width > 0 ? browser.view_width : 1024, height = browser.view_height > 0 ? browser.view_height : 768;
            plan->px = round(width * (0.45 + 0.4 * g_rand_double(browser.rand)));
            plan->py = round(height * (0.5 + 0.4 * g_rand_double(browser.rand)));
            PlannedEvent *e = plan_add(plan, EV_MOVE, plan->t);
            e->x = plan->px;
            e->y = plan->py;
            plan->have_pointer = TRUE;
            plan->t += human(120, 1.5, 60, 300);
        }
        GArray *path = human_path(browser.rand, plan->px, plan->py, x, y, size, 8 + g_rand_double(browser.rand) * 2);
        double end = plan->t;
        for (guint i = 0; i < path->len; i++) {
            HumanPoint *p = &g_array_index(path, HumanPoint, i);
            PlannedEvent *e = plan_add(plan, EV_MOVE, plan->t + p->t);
            e->x = p->x;
            e->y = p->y;
            end = plan->t + p->t;
        }
        g_array_free(path, TRUE);
        plan->t = end;
    } else {
        PlannedEvent *e = plan_add(plan, EV_MOVE, plan->t);
        e->x = x;
        e->y = y;
    }
    plan->px = x;
    plan->py = y;
    plan->have_pointer = TRUE;
}

/* presses of `button` where the pointer is: a click, or `count` of them in a row */
static void plan_press(Plan *plan, guint button, int count, gboolean moved)
{
    /* the pointer comes to rest before a press (more so after a move) */
    plan->t += moved ? human(55, 1.8, 18, 200) : human(25, 2, 5, 120);
    for (int i = 1; i <= count; i++) {
        PlannedEvent *down = plan_add(plan, EV_DOWN, plan->t);
        down->x = plan->px;
        down->y = plan->py;
        down->button = button;
        down->count = i;
        plan->t += i == 1 ? human(96, 1.3, 55, 220) : human(83, 1.36, 45, 190);
        PlannedEvent *up = plan_add(plan, EV_UP, plan->t);
        up->x = plan->px;
        up->y = plan->py;
        up->button = button;
        if (i < count)
            plan->t += human(66, 1.35, 35, 140); /* presses about 150 ms apart */
    }
}

static void plan_key(Plan *plan, gboolean down, guint keysym, guint code, double t)
{
    PlannedEvent *e = plan_add(plan, down ? EV_KEY_DOWN : EV_KEY_UP, t);
    e->keysym = keysym;
    e->code = code;
}

static void plan_modifier(Plan *plan, gboolean down, unsigned bit, guint keysym, guint code, double t)
{
    PlannedEvent *e = plan_add(plan, down ? EV_MOD_DOWN : EV_MOD_UP, t);
    e->bit = bit;
    e->keysym = keysym;
    e->code = code;
}

/* a character typed: its key (with Shift for a shifted one); the next key may go down before this
 * one is up, as a third of a person's do (a key never before its own release) */
static void plan_char(Plan *plan, gunichar c, double pace, double *free_at, guint *last_keysym, double *last_up)
{
    guint keysym, code;
    gboolean shift = FALSE;
    if (c == '\n') {
        keysym = 0xff0d;
        code = 28;
    } else if (c == '\t') {
        keysym = 0xff09;
        code = 15;
    } else {
        code = c < 0x80 ? ascii_code(c, &shift) : 0;
        keysym = (c >= 0x20 && c < 0x7f) ? c : 0x01000000 + c; /* X11: Unicode keysyms */
    }
    gboolean add_shift = code && shift && !(browser.modifiers & WPE_MODIFIER_KEYBOARD_SHIFT);
    double t = plan->t;
    if (keysym == *last_keysym && t < *last_up + 1)
        t = *last_up + human(30, 1.5, 12, 80);
    if (add_shift) {
        /* Shift goes down first and up after, and the keys around it wait for it */
        if (t < *free_at)
            t = *free_at;
        plan_modifier(plan, TRUE, WPE_MODIFIER_KEYBOARD_SHIFT, 0xffe1, 42, t);
        t += human(60, 1.5, 25, 140) * pace;
    }
    double hold = human(105, 1.3, 50, 230) * pace;
    plan_key(plan, TRUE, keysym, code, t);
    plan_key(plan, FALSE, keysym, code, t + hold);
    *last_keysym = keysym;
    *last_up = t + hold;
    if (add_shift) {
        double up = t + hold + human(40, 1.5, 15, 110) * pace;
        plan_modifier(plan, FALSE, WPE_MODIFIER_KEYBOARD_SHIFT, 0xffe1, 42, up);
        *free_at = up + human(20, 1.5, 5, 60);
        plan->t = *free_at;
        return;
    }
    /* the next keydown: a person's gap after this one's (a little longer after a word) */
    double gap = human(c == ' ' ? 175 : 150, 1.55, 55, 600) * pace;
    plan->t = t + gap;
    if (*free_at < t + hold)
        *free_at = t + hold;
}

static int compare_events(gconstpointer a, gconstpointer b)
{
    const PlannedEvent *x = a, *y = b;
    return x->t < y->t ? -1 : x->t > y->t ? 1 : x->seq < y->seq ? -1 : x->seq > y->seq;
}

/* typing `text`: at a person's pace, faster for a long text so that it takes at most ~8 s */
static void plan_text(Plan *plan, const char *text)
{
    long n = g_utf8_strlen(text, -1);
    double pace = 1;
    if (browser.humanize && n > 50)
        pace = MAX(0.3, 50.0 / n);
    double free_at = plan->t, last_up = -1;
    guint last_keysym = 0;
    for (const char *c = text; *c; c = g_utf8_next_char(c))
        plan_char(plan, g_utf8_get_char(c), pace, &free_at, &last_keysym, &last_up);
    if (plan->t < free_at)
        plan->t = free_at;
    if (plan->t < last_up)
        plan->t = last_up;
}

/* Sends what the plan has due, and the rest at their times; done(r) once all went */
static void play_due(Request *r);

static gboolean play_again(gpointer data)
{
    Request *r = data;
    r->plan_id = 0;
    play_due(r);
    return G_SOURCE_REMOVE;
}

static void send_planned(const PlannedEvent *e)
{
    switch (e->kind) {
    case EV_MOVE:
        pointer_move(e->x, e->y);
        break;
    case EV_DOWN:
    case EV_UP:
        pointer_button(e->kind == EV_DOWN, e->x, e->y, e->button, e->count);
        break;
    case EV_KEY_DOWN:
    case EV_KEY_UP:
        key_event(e->kind == EV_KEY_DOWN, e->keysym, e->code);
        break;
    case EV_MOD_DOWN:
    case EV_MOD_UP:
        modifier(e->kind == EV_MOD_DOWN, e->bit, e->keysym, e->code);
        break;
    case EV_WHEEL:
        send_event(wpe_event_scroll_new(wpe_view(), WPE_INPUT_SOURCE_MOUSE, now_ms(), (WPEModifiers)browser.modifiers,
                                        e->dx, e->dy, FALSE, FALSE, e->x, e->y));
        break;
    }
}

static void play_due(Request *r)
{
    double now = (g_get_monotonic_time() - r->plan_start) / 1000.0;
    while (r->plan_next < r->plan->len) {
        const PlannedEvent *e = &g_array_index(r->plan, PlannedEvent, r->plan_next);
        if (e->t > now + 0.5) {
            r->plan_id = g_timeout_add(MAX(1, (guint)(e->t - now)), play_again, r);
            return;
        }
        send_planned(e);
        r->plan_next++;
    }
    g_array_free(r->plan, TRUE);
    r->plan = NULL;
    void (*done)(Request *r) = r->plan_done;
    r->plan_done = NULL;
    if (done)
        done(r);
}

/* plays `plan` (taken) for r, then done(r) */
static void play(Request *r, Plan *plan, void (*done)(Request *r))
{
    g_array_sort(plan->events, compare_events);
    if (r->plan)
        g_array_free(r->plan, TRUE);
    r->plan = plan->events;
    r->plan_next = 0;
    r->plan_start = g_get_monotonic_time();
    r->plan_done = done;
    g_free(plan);
    play_due(r);
}

/* ── input sync ─────────────────────────────────────────────────────────────────────────────
 * Input reaches the page through queues of WebKit's UI process: key events one at a time (each
 * after the page took the one before), pointer events apart from them. A click can overtake keys
 * typed before it, and an answer can come before the page has seen the input. So a command that
 * sends input reads how much the page has seen (INPUT_COUNTER_JS counts it at the window, from
 * the start of each document), sends its own, and goes on once the page has seen that too —
 * or after INPUT_SYNC_MS (a frame of another origin, a page that went away). */
#define INPUT_SYNC_MS 3000
static const char INPUT_COUNTER_JS[] =
    "(() => { let w = window;"
    " try { if (window.top !== window && window.top.__wpeSeen) w = window.top; } catch (e) {}"
    " if (w === window && window.__wpeSeen) return;"
    " if (w === window) window.__wpeSeen = { mousedown: 0, mouseup: 0, keydown: 0 };"
    " for (const t of ['mousedown', 'mouseup', 'keydown']) window.addEventListener(t, () => { w.__wpeSeen[t]++; }, true); })();";
static const char INPUT_SEEN_JS[] =
    "const s = window.__wpeSeen; return s ? `${s.mousedown} ${s.mouseup} ${s.keydown}` : '';";

static gboolean parse_seen(JSCValue *value, unsigned seen[3])
{
    if (!value || !jsc_value_is_string(value))
        return FALSE;
    char *text = jsc_value_to_string(value);
    gboolean ok = sscanf(text, "%u %u %u", &seen[0], &seen[1], &seen[2]) == 3;
    g_free(text);
    return ok;
}

static void input_poll(Request *r);

static gboolean input_poll_again(gpointer data)
{
    Request *r = data;
    r->sync_id = 0;
    input_poll(r);
    return G_SOURCE_REMOVE;
}

static void input_polled(Request *r, JSCValue *value, GError *error)
{
    unsigned seen[3];
    gboolean done = error || !parse_seen(value, seen) || g_get_monotonic_time() > r->sync_deadline;
    for (int i = 0; !done && i < 3; i++) {
        if (seen[i] < r->seen_base[i]) /* another document: the input went to the one before */
            done = TRUE;
    }
    if (!done) {
        done = TRUE;
        for (int i = 0; i < 3; i++) {
            if (seen[i] - r->seen_base[i] < browser.sent[i] - r->sent_base[i])
                done = FALSE;
        }
    }
    if (done)
        r->synced(r);
    else
        r->sync_id = g_timeout_add(25, input_poll_again, r);
}

static void input_poll(Request *r)
{
    if (!r->have_base) {
        r->synced(r);
        return;
    }
    call_js(r, INPUT_SEEN_JS, NULL, input_polled);
}

static void input_base_read(Request *r, JSCValue *value, GError *error)
{
    r->have_base = !error && parse_seen(value, r->seen_base);
    if (!r->generation_set) {
        r->load_generation = browser.load_generation;
        r->generation_set = TRUE;
    }
    memcpy(r->sent_base, browser.sent, sizeof browser.sent);
    r->acted = TRUE;
    r->send(r); /* plays its plan, then input_sent */
}

/* the input has all gone: until the page has seen it */
static void input_sent(Request *r)
{
    r->sync_deadline = g_get_monotonic_time() + INPUT_SYNC_MS * 1000;
    input_poll(r);
}

/* send(r) plays input (ending with input_sent); synced(r) runs when the page has seen it */
static void input_send(Request *r, void (*send)(Request *r), void (*synced)(Request *r))
{
    r->send = send;
    r->synced = synced;
    call_js(r, INPUT_SEEN_JS, NULL, input_base_read);
}

/* "ctrl+shift+Tab", "Enter", "a": FALSE if not understood; planned into `plan` when given */
static gboolean key_spec(const char *spec, Plan *plan)
{
    char **parts = g_strsplit(spec, "+", -1);
    int n = g_strv_length(parts);
    if (!n || !*parts[n - 1]) {
        g_strfreev(parts);
        return FALSE;
    }
    unsigned bits[8] = { 0 };
    guint syms[8] = { 0 }, codes[8] = { 0 };
    int mods = 0;
    for (int i = 0; i < n - 1 && mods < 8; i++) {
        char *m = g_ascii_strdown(parts[i], -1);
        if (!strcmp(m, "ctrl") || !strcmp(m, "control")) { bits[mods] = WPE_MODIFIER_KEYBOARD_CONTROL; syms[mods] = 0xffe3; codes[mods] = 29; }
        else if (!strcmp(m, "shift")) { bits[mods] = WPE_MODIFIER_KEYBOARD_SHIFT; syms[mods] = 0xffe1; codes[mods] = 42; }
        else if (!strcmp(m, "alt")) { bits[mods] = WPE_MODIFIER_KEYBOARD_ALT; syms[mods] = 0xffe9; codes[mods] = 56; }
        else if (!strcmp(m, "meta") || !strcmp(m, "super") || !strcmp(m, "cmd")) { bits[mods] = WPE_MODIFIER_KEYBOARD_META; syms[mods] = 0xffeb; codes[mods] = 125; }
        else { g_free(m); g_strfreev(parts); return FALSE; }
        g_free(m);
        mods++;
    }
    const char *key = parts[n - 1];
    guint keysym = 0, code = 0;
    for (size_t i = 0; i < G_N_ELEMENTS(key_names); i++) {
        if (!g_ascii_strcasecmp(key, key_names[i].name)) {
            keysym = key_names[i].keysym;
            code = key_names[i].code;
            break;
        }
    }
    if (!keysym && g_utf8_strlen(key, -1) == 1) {
        gunichar c = g_utf8_get_char(key);
        gboolean shift;
        if (mods && c < 0x80)
            c = g_ascii_tolower(c);
        code = c < 0x80 ? ascii_code(c, &shift) : 0;
        keysym = (c >= 0x20 && c < 0x7f) ? c : 0x01000000 + c;
    }
    if (!keysym || !plan) {
        g_strfreev(parts);
        return keysym != 0;
    }
    /* a chord as a hand plays it: the modifiers first, each a moment before the next */
    for (int i = 0; i < mods; i++) {
        plan_modifier(plan, TRUE, bits[i], syms[i], codes[i], plan->t);
        plan->t += human(60, 1.5, 25, 140);
    }
    plan_key(plan, TRUE, keysym, code, plan->t);
    plan->t += human(100, 1.3, 50, 220);
    plan_key(plan, FALSE, keysym, code, plan->t);
    for (int i = mods - 1; i >= 0; i--) {
        plan->t += human(40, 1.5, 15, 110);
        plan_modifier(plan, FALSE, bits[i], syms[i], codes[i], plan->t);
    }
    plan->t += human(150, 1.5, 60, 450);
    g_strfreev(parts);
    return TRUE;
}

static void focus_view(void)
{
    wpe_view_focus_in(wpe_view());
}

/* ── settings: who the browser says it is, headers, languages, injected scripts ──────────────
 * Given at the start (options, WEB_USER_AGENT) and changed while it runs (`web set`, `web ua`,
 * `web header`, `web lang`, `web inject`); they apply from the next request or page on. */

/* Recent versions: sites turn away browsers they think are years old. Safari's is the release whose
 * network fingerprint the browser's own matches (TLS, HTTP/2 and the headers of Safari 26.0.1 on
 * macOS: glib-networking and libsoup patches) */
#define CHROME_MAJOR "151"
#define FIREFOX_VERSION "152.0"
#define SAFARI_VERSION "26.0.1"
#define SAFARI_MAC_UA "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/" \
                      SAFARI_VERSION " Safari/605.1.15"

typedef struct {
    const char *name, *about, *user_agent; /* user_agent NULL: WebKit's own */
} UAProfile;

/* the first is a browser's unless it is told otherwise: Safari on a Mac, the engine it is */
static const UAProfile ua_profiles[] = {
    { "safari", "Safari on a Mac: the same engine, the closest match (the default)", SAFARI_MAC_UA },
    { "webkit", "WebKit's own (WPE) on desktop Linux", NULL },
    { "chrome", "Chrome on Windows",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/" CHROME_MAJOR
      ".0.0.0 Safari/537.36" },
    { "edge", "Edge on Windows",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/" CHROME_MAJOR
      ".0.0.0 Safari/537.36 Edg/" CHROME_MAJOR ".0.0.0" },
    { "firefox", "Firefox on Windows",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:" FIREFOX_VERSION ") Gecko/20100101 Firefox/" FIREFOX_VERSION },
    { "iphone", "Safari on an iPhone: mobile sites (with web resize 390x844)",
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/"
      SAFARI_VERSION " Mobile/15E148 Safari/604.1" },
    { "android", "Chrome on an Android phone: mobile sites",
      "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/" CHROME_MAJOR
      ".0.0.0 Mobile Safari/537.36" },
};

/* Real machines' screens at a scale of 1 (the scale this browser draws at: a claimed 2 would not
 * match its pictures), from Camoufox's recorded device presets (python/src/camoufox/
 * fingerprint-presets.json, MPL-2.0): width, height, available width and height, logical cores,
 * touch points */
static const int mac_screens[][6] = {
    { 1920, 1200, 1920, 1170, 8, 0 }, { 1920, 1080, 1920, 985, 2, 0 }, { 1920, 1080, 1920, 1050, 8, 0 },
    { 2560, 1440, 2560, 1440, 8, 0 }, { 1920, 1080, 1920, 987, 8, 0 }, { 2560, 1440, 2560, 1415, 16, 0 },
};
static const int windows_screens[][6] = {
    { 1920, 1080, 1920, 1032, 6, 0 }, { 1920, 1080, 1920, 1032, 8, 0 }, { 3440, 1440, 3440, 1392, 12, 0 },
    { 1920, 1080, 1920, 1040, 4, 0 }, { 1920, 1200, 1920, 1160, 4, 0 }, { 1920, 1080, 1920, 1032, 20, 0 },
    { 1728, 972, 1728, 936, 4, 0 }, { 1024, 768, 1024, 728, 4, 0 }, { 1920, 1080, 1920, 1032, 12, 0 },
    { 1600, 900, 1600, 860, 24, 10 }, { 1366, 768, 1366, 720, 12, 0 }, { 1600, 900, 1600, 900, 8, 10 },
    { 1366, 768, 1366, 720, 4, 0 }, { 1440, 900, 1440, 860, 12, 0 }, { 1366, 768, 1366, 720, 8, 0 },
    { 2400, 1350, 2400, 1290, 8, 1 }, { 1600, 900, 1600, 860, 4, 0 }, { 1920, 1080, 1920, 1032, 16, 0 },
    { 1360, 768, 1360, 720, 4, 10 }, { 1920, 1080, 1920, 1040, 8, 0 },
};
/* phones: the screen is the page's (web resize 390x844) */
static const int iphone_screens[][6] = { { 390, 844, 390, 844, 6, 5 } };
static const int android_screens[][6] = { { 412, 915, 412, 915, 8, 5 } };

static void identity_clear(Identity *id)
{
    g_free(id->profile);
    g_free(id->user_agent);
    g_free(id->platform);
    g_free(id->vendor);
    g_free(id->oscpu);
    g_free(id->brand);
    g_free(id->major);
    g_free(id->ch_platform);
    g_free(id->ch_platform_version);
    g_free(id->os);
    memset(id, 0, sizeof *id);
}

/* What goes with a user agent, read from it: the profiles' and anyone's own */
static void identity_from(Identity *id, const char *profile, const char *user_agent)
{
    char *ua = g_strdup(user_agent); /* which can be the one it replaces */
    identity_clear(id);
    id->profile = g_strdup(profile);
    id->user_agent = ua;
    gboolean windows = strstr(ua, "Windows") != NULL, android = strstr(ua, "Android") != NULL;
    gboolean iphone = strstr(ua, "iPhone") != NULL, ipad = strstr(ua, "iPad") != NULL;
    gboolean mac = !iphone && !ipad && strstr(ua, "Macintosh") != NULL, chromeos = strstr(ua, "CrOS") != NULL;
    gboolean firefox = strstr(ua, "Firefox/") != NULL;
    id->platform = g_strdup(iphone ? "iPhone" : ipad ? "iPad" : android ? "Linux armv81" : windows ? "Win32"
                            : mac ? "MacIntel" : "Linux x86_64");
    id->os = g_strdup(iphone || ipad ? "iphone" : android ? "android" : windows ? "windows" : mac ? "mac" : "linux");
    id->mobile = iphone || android || strstr(ua, "Mobile") != NULL;
    id->touch = iphone || ipad || android;
    const char *chrome = strstr(ua, "Chrome/");
    id->vendor = g_strdup(firefox ? "" : chrome || strstr(ua, "CriOS/") ? "Google Inc." : "Apple Computer, Inc.");
    if (firefox) {
        /* "Windows NT 10.0; Win64; x64" from "(Windows NT 10.0; Win64; x64; rv:152.0)" */
        const char *open = strchr(ua, '('), *close = open ? strchr(open, ')') : NULL;
        if (open && close) {
            char *inside = g_strndup(open + 1, close - open - 1);
            char **parts = g_strsplit(inside, "; ", -1);
            GString *oscpu = g_string_new(NULL);
            for (char **p = parts; *p; p++) {
                if (g_str_has_prefix(*p, "rv:") || !strcmp(*p, "Macintosh") || !strcmp(*p, "X11"))
                    continue;
                g_string_append_printf(oscpu, "%s%s", oscpu->len ? "; " : "", *p);
            }
            id->oscpu = g_string_free(oscpu, FALSE);
            g_strfreev(parts);
            g_free(inside);
        }
    } else if (chrome) {
        /* Chromium: userAgentData and the client hints (Sec-CH-UA...) */
        id->major = g_strndup(chrome + 7, strspn(chrome + 7, "0123456789"));
        if (!*id->major)
            g_clear_pointer(&id->major, g_free);
        else {
            id->brand = g_strdup(strstr(ua, "Edg/") ? "Microsoft Edge" : strstr(ua, "OPR/") ? "Opera" : "Google Chrome");
            id->ch_platform = g_strdup(windows ? "Windows" : android ? "Android" : mac ? "macOS" : chromeos ? "Chrome OS" : "Linux");
            const char *version = windows ? "19.0.0" : mac ? "15.0.0" : "";
            const char *a = android ? strstr(ua, "Android ") : NULL;
            id->ch_platform_version = a ? g_strdup_printf("%d.0.0", atoi(a + 8)) : g_strdup(version);
        }
    }
}

static const UAProfile *ua_profile(const char *name)
{
    if (!g_ascii_strcasecmp(name, "default"))
        return &ua_profiles[0];
    for (size_t i = 0; i < G_N_ELEMENTS(ua_profiles); i++) {
        if (!g_ascii_strcasecmp(name, ua_profiles[i].name))
            return &ua_profiles[i];
    }
    return NULL;
}

/* a profile's name or a user agent; "" or "default": the first profile (Safari on a Mac) */
static gboolean identity_set(const char *text, char **error)
{
    const UAProfile *profile = ua_profile(text && *text ? text : "default");
    if (!profile && strpbrk(text, "\r\n")) {
        *error = g_strdup("a user agent is one line");
        return FALSE;
    }
    if (profile)
        identity_from(&browser.identity, profile->name, profile->user_agent ? profile->user_agent : browser.default_user_agent);
    else
        identity_from(&browser.identity, NULL, text);
    return TRUE;
}

/* The machine the identity is on: one of the real ones of its kind (mac_screens...) whose screen
 * holds the page's window, picked by the browser's seed, so the same browser stays the same
 * machine; where its window is on that screen; what its browser's own bars add around the page.
 * WebKit's own (the guest's display) for a Linux identity. */
static void device_draw(void)
{
    Identity *id = &browser.identity;
    Device *d = &browser.device;
    memset(d, 0, sizeof *d);
    const char *os = id->os ? id->os : "linux";
    const int (*table)[6];
    size_t count;
    int extra_w = 0, extra_h = 0;
    gboolean phone = FALSE, mac = !strcmp(os, "mac"), safari = strstr(id->user_agent, "Chrome/") == NULL && strstr(id->user_agent, "Firefox/") == NULL;
    if (mac) {
        table = mac_screens;
        count = G_N_ELEMENTS(mac_screens);
        /* Safari's toolbar and tab bar over the page */
        extra_h = safari ? 79 : 87;
    } else if (!strcmp(os, "windows")) {
        table = windows_screens;
        count = G_N_ELEMENTS(windows_screens);
        /* the window's resize borders, Chrome's tab strip and toolbar (Firefox's are a little lower) */
        extra_w = 16;
        extra_h = strstr(id->user_agent, "Firefox/") ? 80 : 87;
    } else if (!strcmp(os, "iphone")) {
        table = iphone_screens;
        count = 1;
        phone = TRUE;
    } else if (!strcmp(os, "android")) {
        table = android_screens;
        count = 1;
        phone = TRUE;
    } else
        return;
    int vw = browser.view_width > 0 ? browser.view_width : 1024, vh = browser.view_height > 0 ? browser.view_height : 768;
    GRand *rand = g_rand_new_with_seed(browser.seed ^ g_str_hash(os));
    size_t first = (size_t)g_rand_int_range(rand, 0, (gint32)count), pick = first;
    for (size_t i = 0; i < count; i++) {
        const int *e = table[(first + i) % count];
        if (e[2] >= vw + extra_w && e[3] >= vh + extra_h) {
            pick = (first + i) % count;
            break;
        }
    }
    const int *e = table[pick];
    int width = e[0], height = e[1], avail_w = e[2], avail_h = e[3];
    if (phone) {
        width = avail_w = vw;
        height = avail_h = vh;
    }
    /* a Mac's menu bar is at the top of the screen (the Dock, if any, below); Windows' taskbar below */
    int avail_top = mac && avail_h < height ? MIN(height - avail_h, 25) : 0;
    d->set = TRUE;
    d->screen[0] = width;
    d->screen[1] = height;
    d->screen[2] = 0;
    d->screen[3] = avail_top;
    d->screen[4] = avail_w;
    d->screen[5] = avail_h;
    d->screen[6] = 24;
    /* WebKit tells pages 4 cores, or 8 for a machine with 8 or more (Safari's privacy rule) */
    d->cores = safari ? (e[4] < 8 ? 4 : 8) : e[4];
    d->touch_points = phone ? 5 : e[5];
    d->window[2] = phone ? 0 : extra_w;
    d->window[3] = phone ? 0 : extra_h;
    int outer_w = vw + d->window[2], outer_h = vh + d->window[3];
    d->window[0] = phone || avail_w <= outer_w ? 0 : g_rand_int_range(rand, 0, avail_w - outer_w + 1);
    d->window[1] = phone ? 0 : avail_top + (avail_h <= outer_h ? 0 : g_rand_int_range(rand, 0, avail_h - outer_h + 1));
    g_rand_free(rand);
}

/* the identity as WebKit is to answer it in each web process (extension.c → WebKit patch 0012):
 * a{sms}, nothing for WebKit's own */
static GVariant *identity_values(void)
{
    Identity *id = &browser.identity;
    Device *d = &browser.device;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a{sms}"));
    char *screen = d->set ? g_strdup_printf("%d %d %d %d %d %d %d", d->screen[0], d->screen[1], d->screen[2], d->screen[3],
                                            d->screen[4], d->screen[5], d->screen[6]) : NULL;
    char *window = d->set ? g_strdup_printf("%d %d %d %d", d->window[0], d->window[1], d->window[2], d->window[3]) : NULL;
    char *cores = d->set ? g_strdup_printf("%d", d->cores) : NULL;
    char *touch = d->set ? g_strdup_printf("%d", d->touch_points) : id->touch ? g_strdup("5") : NULL;
    g_variant_builder_add(&builder, "{sms}", "platform", id->platform);
    g_variant_builder_add(&builder, "{sms}", "vendor", id->vendor);
    g_variant_builder_add(&builder, "{sms}", "hardwareConcurrency", cores);
    g_variant_builder_add(&builder, "{sms}", "maxTouchPoints", touch);
    g_variant_builder_add(&builder, "{sms}", "screen", screen);
    g_variant_builder_add(&builder, "{sms}", "window", window);
    g_variant_builder_add(&builder, "{sms}", "timeZone", browser.time_zone);
    g_free(screen);
    g_free(window);
    g_free(cores);
    g_free(touch);
    return g_variant_builder_end(&builder);
}

static char *identity_text(void)
{
    Identity *id = &browser.identity;
    Device *d = &browser.device;
    const UAProfile *profile = id->profile ? ua_profile(id->profile) : NULL;
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "user agent: %s%s%s%s\n  %s\n  navigator.platform \"%s\", vendor \"%s\"", profile ? profile->name : "custom",
                           profile ? " (" : "", profile ? profile->about : "", profile ? ")" : "", id->user_agent, id->platform, id->vendor);
    if (id->oscpu)
        g_string_append_printf(s, ", oscpu \"%s\"", id->oscpu);
    if (id->brand)
        g_string_append_printf(s, ", userAgentData (%s %s, %s); Sec-CH-UA headers to https sites", id->brand, id->major, id->ch_platform);
    g_string_append_c(s, '\n');
    if (d->set)
        g_string_append_printf(s, "  device: screen %dx%d (available %dx%d at %d,%d), %d-bit color, window at %d,%d with %dx%d around the "
                               "page, %d cores, %d touch points (seed %u)\n", d->screen[0], d->screen[1], d->screen[4], d->screen[5],
                               d->screen[2], d->screen[3], d->screen[6], d->window[0], d->window[1], d->window[2], d->window[3],
                               d->cores, d->touch_points, browser.seed);
    else
        g_string_append(s, "  device: WebKit's own (the guest's display)\n");
    return g_string_free(s, FALSE);
}

/* identity.js with what this identity needs of it ("" for none: Safari's) */
static char *identity_script(void)
{
    Identity *id = &browser.identity;
    if (!id->oscpu && !id->brand)
        return g_strdup("");
    GString *s = g_string_new("(");
    g_string_append(s, identity_js);
    g_string_append(s, ")({\"mobile\": ");
    g_string_append(s, id->mobile ? "true" : "false");
    if (id->oscpu) {
        g_string_append(s, ", \"oscpu\": ");
        json_string(s, id->oscpu);
    }
    if (id->brand) {
        g_string_append(s, ", \"brands\": [[\"Chromium\", ");
        json_string(s, id->major);
        g_string_append(s, "], [");
        json_string(s, id->brand);
        g_string_append(s, ", ");
        json_string(s, id->major);
        g_string_append(s, "], [\"Not.A/Brand\", \"99\"]], \"fullVersion\": ");
        char *full = g_strconcat(id->major, ".0.0.0", NULL);
        json_string(s, full);
        g_free(full);
        g_string_append(s, ", \"chPlatform\": ");
        json_string(s, id->ch_platform);
        g_string_append(s, ", \"chPlatformVersion\": ");
        json_string(s, id->ch_platform_version);
    }
    g_string_append(s, "});");
    return g_string_free(s, FALSE);
}

static void header_free(Header *h)
{
    g_free(h->host);
    g_free(h->name);
    g_free(h->value);
    g_free(h);
}

/* "example.com", "*.example.com", "https://www.example.com/x" → "example.com", "www.example.com";
 * NULL, "" or "*": every host ("") */
static char *header_scope(const char *host)
{
    if (!host || !*host || !strcmp(host, "*"))
        return g_strdup("");
    char *text = NULL;
    if (strstr(host, "://")) {
        GUri *uri = g_uri_parse(host, G_URI_FLAGS_NONE, NULL);
        text = g_strdup(uri && g_uri_get_host(uri) ? g_uri_get_host(uri) : "");
        if (uri)
            g_uri_unref(uri);
    } else {
        while (g_str_has_prefix(host, "*.") || *host == '.')
            host += *host == '.' ? 1 : 2;
        text = g_strndup(host, strcspn(host, ":/"));
    }
    char *scope = g_ascii_strdown(text, -1);
    g_free(text);
    return scope;
}

static gboolean header_token(const char *name)
{
    if (!*name)
        return FALSE;
    for (const char *c = name; *c; c++) {
        if (!g_ascii_isalnum(*c) && !strchr("!#$%&'*+-.^_`|~", *c))
            return FALSE;
    }
    return TRUE;
}

/* Puts NAME: VALUE on the requests to HOST (NULL: every host); an empty value takes NAME off them. */
static gboolean header_set(const char *host, const char *name, const char *value, char **error)
{
    static const char *const by_the_network[] = { "Host", "Content-Length", "Transfer-Encoding", "Connection" };
    if (!header_token(name)) {
        *error = g_strdup_printf("\"%s\" is not a header's name", name);
        return FALSE;
    }
    if (strpbrk(value, "\r\n")) {
        *error = g_strdup_printf("the value of %s is one line", name);
        return FALSE;
    }
    if (!g_ascii_strcasecmp(name, "Cookie")) {
        *error = g_strdup("the browser's cookie jar replaces a Cookie header: web cookie set 'NAME=VALUE' [--url URL] "
                          "puts the cookie in the jar instead");
        return FALSE;
    }
    for (size_t i = 0; i < G_N_ELEMENTS(by_the_network); i++) {
        if (!g_ascii_strcasecmp(name, by_the_network[i])) {
            *error = g_strdup_printf("%s is the network stack's to set", by_the_network[i]);
            return FALSE;
        }
    }
    char *scope = header_scope(host);
    if (host && *host && !*scope && strcmp(host, "*")) {
        *error = g_strdup_printf("\"%s\" is not a host", host);
        g_free(scope);
        return FALSE;
    }
    const char *trimmed = value;
    while (*trimmed == ' ' || *trimmed == '\t')
        trimmed++;
    for (guint i = 0; i < browser.headers->len; i++) {
        Header *h = g_ptr_array_index(browser.headers, i);
        if (!strcmp(h->host, scope) && !g_ascii_strcasecmp(h->name, name)) {
            g_free(h->value);
            h->value = g_strdup(trimmed);
            g_free(scope);
            return TRUE;
        }
    }
    Header *h = g_new0(Header, 1);
    h->host = scope;
    h->name = g_strdup(name);
    h->value = g_strdup(trimmed);
    g_ptr_array_add(browser.headers, h);
    return TRUE;
}

/* "NAME: VALUE" (curl's -H: "NAME:" takes NAME off the requests) */
static gboolean header_line(const char *host, const char *line, char **error)
{
    const char *colon = strchr(line, ':');
    if (!colon) {
        *error = g_strdup_printf("\"%s\" is not NAME: VALUE", line);
        return FALSE;
    }
    char *name = g_strstrip(g_strndup(line, colon - line));
    gboolean ok = header_set(host, name, colon + 1, error);
    g_free(name);
    return ok;
}

/* Takes NAME off what was set for HOST (NULL: for every host it was set for); how many it took */
static unsigned header_unset(const char *host, const char *name)
{
    char *scope = host ? header_scope(host) : NULL;
    unsigned removed = 0;
    for (guint i = browser.headers->len; i-- > 0;) {
        Header *h = g_ptr_array_index(browser.headers, i);
        if (!g_ascii_strcasecmp(h->name, name) && (!scope || !strcmp(h->host, scope))) {
            g_ptr_array_remove_index(browser.headers, i);
            removed++;
        }
    }
    g_free(scope);
    return removed;
}

/* The rules the web processes put on requests (extension.c): the profile's client hints, then
 * what was set, which can replace or take them off */
/* Safari: one language for pages (navigator.languages), and to servers that one and its base */
static gboolean identity_is_safari(void)
{
    const char *ua = browser.identity.user_agent;
    const char *os = browser.identity.os;
    return ua && os && (!strcmp(os, "mac") || !strcmp(os, "iphone")) && !strstr(ua, "Chrome/") && !strstr(ua, "CriOS/")
           && !strstr(ua, "Firefox/") && !strstr(ua, "FxiOS/");
}

/* the first preferred language as a tag: "ko-KR" (the environment's, else en-US) */
static char *first_language(void)
{
    const char *first = browser.languages && browser.languages[0] ? browser.languages[0] : NULL;
    for (const char *const *l = g_get_language_names(); !first && *l; l++) {
        if (!strpbrk(*l, ".@") && strcmp(*l, "C") && strcmp(*l, "POSIX"))
            first = *l;
    }
    char *tag = g_strdup(first ? first : "en-US");
    g_strdelimit(tag, "_", '-');
    return tag;
}

static char *safari_accept_language(void)
{
    char *tag = first_language();
    const char *dash = strchr(tag, '-');
    char *value = dash ? g_strdup_printf("%s,%.*s;q=0.9", tag, (int)(dash - tag), tag) : g_strdup(tag);
    g_free(tag);
    return value;
}

static GVariant *header_rules(void)
{
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a(sssb)"));
    Identity *id = &browser.identity;
    if (identity_is_safari()) {
        /* as Safari words it ("ko-KR,ko;q=0.9"), unless a header the caller set says otherwise */
        char *value = safari_accept_language();
        g_variant_builder_add(&builder, "(sssb)", "", "Accept-Language", value, FALSE);
        g_free(value);
    }
    if (id->brand) {
        char *brands = g_strdup_printf("\"Chromium\";v=\"%s\", \"%s\";v=\"%s\", \"Not.A/Brand\";v=\"99\"", id->major, id->brand, id->major);
        char *platform = g_strdup_printf("\"%s\"", id->ch_platform);
        g_variant_builder_add(&builder, "(sssb)", "", "Sec-CH-UA", brands, TRUE);
        g_variant_builder_add(&builder, "(sssb)", "", "Sec-CH-UA-Mobile", id->mobile ? "?1" : "?0", TRUE);
        g_variant_builder_add(&builder, "(sssb)", "", "Sec-CH-UA-Platform", platform, TRUE);
        g_free(brands);
        g_free(platform);
    }
    for (guint i = 0; i < browser.headers->len; i++) {
        Header *h = g_ptr_array_index(browser.headers, i);
        g_variant_builder_add(&builder, "(sssb)", h->host, h->name, h->value, FALSE);
    }
    return g_variant_builder_end(&builder);
}

static char *headers_text(void)
{
    if (!browser.headers->len)
        return g_strdup("headers: none set (web header set NAME VALUE)\n");
    GString *s = g_string_new("headers:\n");
    for (guint i = 0; i < browser.headers->len; i++) {
        Header *h = g_ptr_array_index(browser.headers, i);
        g_string_append_printf(s, "  %s: %s", h->name, *h->value ? h->value : "(taken off the requests)");
        if (*h->host)
            g_string_append_printf(s, "   (to %s and its subdomains)", h->host);
        g_string_append_c(s, '\n');
    }
    return g_string_free(s, FALSE);
}

static void script_free(Script *script)
{
    g_free(script->name);
    g_free(script->source);
    g_free(script);
}

static gboolean script_add(const char *name, const char *source, char **error)
{
    if (!g_utf8_validate(source, -1, NULL)) {
        *error = g_strdup_printf("%s is not UTF-8 text", name);
        return FALSE;
    }
    for (guint i = 0; i < browser.scripts->len; i++) {
        Script *script = g_ptr_array_index(browser.scripts, i);
        if (!strcmp(script->name, name)) {
            g_free(script->source);
            script->source = g_strdup(source);
            return TRUE;
        }
    }
    Script *script = g_new0(Script, 1);
    script->name = g_strdup(name);
    script->source = g_strdup(source);
    g_ptr_array_add(browser.scripts, script);
    return TRUE;
}

static gboolean script_file(const char *path, char **error)
{
    char *source = NULL;
    GError *e = NULL;
    if (!g_file_get_contents(path, &source, NULL, &e)) {
        *error = g_strdup(e->message);
        g_error_free(e);
        return FALSE;
    }
    char *real = realpath(path, NULL); /* the name `web inject remove` gives it (web.c: absolute) */
    char *name = real ? g_strdup(real) : g_canonicalize_filename(path, NULL);
    free(real);
    gboolean ok = script_add(name, source, error);
    g_free(name);
    g_free(source);
    return ok;
}

static char *scripts_text(void)
{
    if (!browser.scripts->len)
        return g_strdup("injected scripts: none (web inject add FILE)\n");
    GString *s = g_string_new("injected scripts (at the start of every page, in every frame):\n");
    for (guint i = 0; i < browser.scripts->len; i++) {
        Script *script = g_ptr_array_index(browser.scripts, i);
        g_string_append_printf(s, "  %s (%zu bytes)\n", script->name, strlen(script->source));
    }
    return g_string_free(s, FALSE);
}

/* "ko-KR,en", "ko_KR en-US", an Accept-Language ("ko-KR,ko;q=0.9,en;q=0.8"); "default": the
 * environment's */
static gboolean languages_set(const char *text, char **error)
{
    GPtrArray *list = g_ptr_array_new_with_free_func(g_free);
    if (!g_ascii_strcasecmp(text, "default")) {
        for (const char *const *l = g_get_language_names(); *l; l++) {
            if (!strpbrk(*l, ".@"))
                g_ptr_array_add(list, g_strdup(*l));
        }
    } else {
        char **parts = g_strsplit_set(text, ", ", -1);
        for (char **p = parts; *p; p++) {
            char *tag = g_strndup(*p, strcspn(*p, ";"));
            gboolean valid = *tag != 0;
            for (const char *c = tag; *c; c++)
                valid = valid && (g_ascii_isalnum(*c) || *c == '-' || *c == '_');
            if (*tag && !valid) {
                *error = g_strdup_printf("\"%s\" is not a language (ko-KR, en-US, ja...)", *p);
                g_free(tag);
                g_strfreev(parts);
                g_ptr_array_free(list, TRUE);
                return FALSE;
            }
            if (*tag)
                g_ptr_array_add(list, tag);
            else
                g_free(tag);
        }
        g_strfreev(parts);
    }
    if (!list->len) {
        *error = g_strdup("no language given");
        g_ptr_array_free(list, TRUE);
        return FALSE;
    }
    g_ptr_array_add(list, NULL);
    g_strfreev(browser.languages);
    browser.languages = (char **)g_ptr_array_free(list, FALSE);
    return TRUE;
}

static char *languages_text(void)
{
    char *list = browser.languages ? g_strjoinv(", ", browser.languages) : g_strdup("the environment's (web lang ko-KR,en)");
    char *text;
    if (identity_is_safari()) {
        char *first = first_language(), *accept = safari_accept_language();
        text = g_strdup_printf("languages: %s; as Safari tells them: navigator.languages [\"%s\"], Accept-Language: %s\n",
                               list, first, accept);
        g_free(first);
        g_free(accept);
    } else
        text = g_strdup_printf("languages: %s (Accept-Language, navigator.languages)\n", list);
    g_free(list);
    return text;
}

/* ── time zone: the identity's, as pages see it (Date, Intl); the guest's own is UTC ──────────── */

/* an IANA time zone ICU knows, in its canonical spelling, or NULL */
static char *time_zone_canonical(const char *name)
{
    UChar id[128], canonical[128];
    int32_t length = 0;
    for (const char *c = name; *c && length < 127; c++) {
        if ((unsigned char)*c >= 0x80)
            return NULL;
        id[length++] = (UChar)*c;
    }
    UErrorCode status = U_ZERO_ERROR;
    UBool system = 0;
    int32_t n = ucal_getCanonicalTimeZoneID(id, length, canonical, 127, &system, &status);
    if (U_FAILURE(status) || !system || n <= 0)
        return NULL;
    char *out = g_malloc(n + 1);
    for (int32_t i = 0; i < n; i++)
        out[i] = (char)canonical[i];
    out[n] = 0;
    return out;
}

/* "name": "value" of a JSON object's text (the few fields of a geolocation answer) */
static char *json_field(const char *text, const char *name)
{
    char *key = g_strdup_printf("\"%s\"", name);
    const char *p = strstr(text, key);
    g_free(key);
    if (!p)
        return NULL;
    p = strchr(p + strlen(name) + 2, ':');
    if (!p)
        return NULL;
    p++;
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')
        p++;
    if (*p != '"')
        return NULL;
    const char *end = strchr(p + 1, '"');
    return end ? g_strndup(p + 1, end - p - 1) : NULL;
}

static const char *const *proxy_bypass_hosts(void);

/* The time zone of the address the browser's requests come from, as Camoufox's geoip does (a page
 * compares the clock it is shown with where the address is): asked of ipinfo.io, else ipapi.co.
 * *about: where it was found. */
static char *time_zone_lookup(char **about, char **error)
{
    static const char *const services[][4] = {
        { "https://ipinfo.io/json", "timezone", "ip", "country" },
        { "https://ipapi.co/json/", "timezone", "ip", "country_code" },
    };
    SoupSession *session = soup_session_new_with_options("timeout", 10, "user-agent", browser.identity.user_agent, NULL);
    /* the address the pages' requests come from: the proxy's, when there is one */
    if (browser.proxy) {
        GProxyResolver *resolver = g_simple_proxy_resolver_new(browser.proxy, (char **)proxy_bypass_hosts());
        soup_session_set_proxy_resolver(session, resolver);
        g_object_unref(resolver);
    }
    char *zone = NULL;
    GString *failures = g_string_new(NULL);
    for (size_t i = 0; i < G_N_ELEMENTS(services) && !zone; i++) {
        SoupMessage *message = soup_message_new("GET", services[i][0]);
        GError *e = NULL;
        GBytes *body = soup_session_send_and_read(session, message, NULL, &e);
        guint status = soup_message_get_status(message);
        if (body && status == 200) {
            char *text = g_strndup(g_bytes_get_data(body, NULL), g_bytes_get_size(body));
            char *found = json_field(text, services[i][1]);
            zone = found ? time_zone_canonical(found) : NULL;
            if (zone) {
                char *ip = json_field(text, services[i][2]), *country = json_field(text, services[i][3]);
                char *host = g_strndup(services[i][0] + 8, strcspn(services[i][0] + 8, "/"));
                *about = g_strdup_printf("the address %s%s%s, by %s", ip ? ip : "?", country ? " in " : "", country ? country : "", host);
                g_free(ip);
                g_free(country);
                g_free(host);
            } else
                g_string_append_printf(failures, "%s%s: no time zone in its answer", failures->len ? "; " : "", services[i][0]);
            g_free(found);
            g_free(text);
        } else
            g_string_append_printf(failures, "%s%s: %s", failures->len ? "; " : "", services[i][0],
                                   e ? e->message : soup_message_get_reason_phrase(message));
        if (body)
            g_bytes_unref(body);
        g_clear_error(&e);
        g_object_unref(message);
    }
    g_object_unref(session);
    if (!zone)
        *error = g_strdup_printf("could not find where the browser's address is (%s)", failures->str);
    g_string_free(failures, TRUE);
    return zone;
}

/* ZONE (Asia/Seoul), auto (the address's), or default / "" (the guest's: UTC) */
static gboolean time_zone_set(const char *text, char **about, char **error)
{
    char *zone = NULL;
    if (!text || !*text || !g_ascii_strcasecmp(text, "default") || !g_ascii_strcasecmp(text, "utc-guest"))
        zone = NULL;
    else if (!g_ascii_strcasecmp(text, "auto")) {
        zone = time_zone_lookup(about, error);
        if (!zone)
            return FALSE;
    } else if (!(zone = time_zone_canonical(text))) {
        *error = g_strdup_printf("\"%s\" is not a time zone (an IANA name: Asia/Seoul, America/New_York, Europe/Berlin, UTC; or auto)", text);
        return FALSE;
    }
    g_free(browser.time_zone);
    browser.time_zone = zone;
    return TRUE;
}

static char *time_zone_text(void)
{
    return g_strdup_printf("time zone: %s (Date, Intl; web tz ZONE | auto | default)\n",
                           browser.time_zone ? browser.time_zone : "the guest's, UTC");
}

static gboolean humanize_set(const char *text, char **error)
{
    if (!g_ascii_strcasecmp(text, "on") || !strcmp(text, "1") || !g_ascii_strcasecmp(text, "true"))
        browser.humanize = TRUE;
    else if (!g_ascii_strcasecmp(text, "off") || !strcmp(text, "0") || !g_ascii_strcasecmp(text, "false"))
        browser.humanize = FALSE;
    else {
        *error = g_strdup_printf("humanize is on or off, not \"%s\"", text);
        return FALSE;
    }
    return TRUE;
}

static char *humanize_text(void)
{
    return g_strdup_printf("humanize: %s\n", browser.humanize
        ? "on (the pointer travels to where it clicks, buttons and keys are held and spaced as a person's, the wheel turns in notches)"
        : "off (each action's events at once)");
}

/* ── where the requests go (web proxy) ─────────────────────────────────────────────────────────
 * Every request of the browser's network session (pages, frames, workers, WebSockets, downloads)
 * goes through the proxy, as WebKit's proxy settings send it: an HTTP proxy gets http:// requests
 * whole and a CONNECT for the others (https, wss), a SOCKS one a connection to each host (named,
 * not looked up here). Per browser (-n NAME: its own network process). */

/* what goes straight unless said: the machine itself, as Chrome and Firefox do (a proxy elsewhere
 * cannot reach the guest's own servers) */
static const char *const proxy_loopback[] = { "localhost", "127.0.0.0/8", "::1", NULL };

static const char *const *proxy_bypass_hosts(void)
{
    return browser.proxy_bypass ? (const char *const *)browser.proxy_bypass : proxy_loopback;
}

/* A proxy's address as given (HOST:PORT alone: an HTTP proxy's), checked and written out whole;
 * NULL with *error */
static char *proxy_address(const char *text, char **error)
{
    static const char *const schemes[] = { "http", "https", "socks", "socks4", "socks4a", "socks5", "socks5h", NULL };
    char *whole = strstr(text, "://") ? g_strdup(text) : g_strconcat("http://", text, NULL);
    char *scheme = NULL, *userinfo = NULL, *host = NULL, *path = NULL, *query = NULL, *fragment = NULL;
    int port = -1;
    GError *e = NULL;
    char *result = NULL;
    if (!g_uri_split(whole, G_URI_FLAGS_ENCODED, &scheme, &userinfo, &host, &port, &path, &query, &fragment, &e)) {
        *error = g_strdup_printf("%s is not a proxy's address (%s): http://HOST:PORT, socks5://HOST:PORT...", text, e->message);
        g_error_free(e);
    } else {
        char *lower = g_ascii_strdown(scheme, -1);
        if (!g_strv_contains(schemes, lower))
            *error = g_strdup_printf("a proxy is http://, https://, socks5:// (socks5h://), socks4://, socks4a:// or socks:// (any of the"
                                     " SOCKS ones), not %s://", scheme);
        else if (!host || !*host)
            *error = g_strdup_printf("%s names no host: %s://HOST:PORT", text, lower);
        else if (port <= 0)
            *error = g_strdup_printf("give the proxy's port: %s://%s:PORT", lower, host);
        else if ((path && *path && strcmp(path, "/")) || query || fragment)
            *error = g_strdup_printf("a proxy's address is SCHEME://[USER:PASSWORD@]HOST:PORT, without a path (%s)", text);
        else /* socks5h (curl's: the proxy looks the names up) is what socks5 does here */
            result = g_uri_join(G_URI_FLAGS_ENCODED, !strcmp(lower, "socks5h") ? "socks5" : lower, userinfo, host, port, "", NULL, NULL);
        g_free(lower);
    }
    g_free(whole);
    g_free(scheme);
    g_free(userinfo);
    g_free(host);
    g_free(path);
    g_free(query);
    g_free(fragment);
    return result;
}

/* an address as it is shown: its password hidden */
static char *proxy_shown(const char *address)
{
    char *scheme = NULL, *userinfo = NULL, *host = NULL;
    int port = -1;
    if (!g_uri_split(address, G_URI_FLAGS_ENCODED, &scheme, &userinfo, &host, &port, NULL, NULL, NULL, NULL))
        return g_strdup(address);
    char *user = userinfo ? (strchr(userinfo, ':') ? g_strdup_printf("%.*s:****", (int)strcspn(userinfo, ":"), userinfo)
                                                   : g_strdup(userinfo))
                          : NULL;
    char *shown = g_uri_join(G_URI_FLAGS_ENCODED, scheme, user, host, port, "", NULL, NULL);
    g_free(scheme);
    g_free(userinfo);
    g_free(host);
    g_free(user);
    return shown;
}

/* URL, or off (none, direct, ""): straight to the sites (*address NULL); FALSE with *error */
static gboolean proxy_parse(const char *text, char **address, char **error)
{
    *address = NULL;
    if (!text || !*text || !g_ascii_strcasecmp(text, "off") || !g_ascii_strcasecmp(text, "none") || !g_ascii_strcasecmp(text, "direct"))
        return TRUE;
    return (*address = proxy_address(text, error)) != NULL;
}

static gboolean proxy_set(const char *text, char **error)
{
    char *address = NULL;
    if (!proxy_parse(text, &address, error))
        return FALSE;
    g_free(browser.proxy);
    browser.proxy = address;
    return TRUE;
}

/* HOST[,HOST...] (names, *.domain, .domain, ADDRESS/BITS, HOST:PORT), none (every host through
 * the proxy), or default (the machine itself) */
static gboolean proxy_bypass_set(const char *text, char **error)
{
    if (!g_ascii_strcasecmp(text, "default")) {
        g_strfreev(browser.proxy_bypass);
        browser.proxy_bypass = NULL;
        return TRUE;
    }
    GPtrArray *hosts = g_ptr_array_new();
    if (g_ascii_strcasecmp(text, "none")) {
        char **words = g_strsplit_set(text, ", ", -1);
        for (char **w = words; *w; w++) {
            if (**w)
                g_ptr_array_add(hosts, g_strdup(*w));
        }
        g_strfreev(words);
        if (!hosts->len) {
            g_ptr_array_free(hosts, TRUE);
            *error = g_strdup("--bypass takes hosts (a,b,*.c,10.0.0.0/8), none or default");
            return FALSE;
        }
    }
    g_ptr_array_add(hosts, NULL);
    g_strfreev(browser.proxy_bypass);
    browser.proxy_bypass = (char **)g_ptr_array_free(hosts, FALSE);
    return TRUE;
}

/* the folder of the certificate authorities this browser trusts besides the guest's, beside its
 * socket: made empty at the start, before WebKit's network process starts (it inherits where) */
static void ca_start(void)
{
    const char *dir = g_getenv("COLLABO_WEB_DIR");
    browser.ca_dir = g_strdup_printf("%s/%s.ca", dir && *dir ? dir : "/tmp/.collabo-web", browser.name);
    g_mkdir_with_parents(browser.ca_dir, 0700);
    wpe_tls_untrust_all(browser.ca_dir); /* a browser of this name before */
    wpe_tls_trust_dir(browser.ca_dir);
    browser.ca_trusted = g_ptr_array_new_with_free_func(g_free);
}

/* trusts FILE's certificates from the next connection on; *about: their subjects */
static gboolean ca_add(const char *file, char **about, char **error)
{
    char *subjects = NULL;
    if (!wpe_tls_trust_file(browser.ca_dir, file, &subjects, error))
        return FALSE;
    char **lines = g_strsplit(subjects, "\n", -1);
    for (char **l = lines; *l; l++) {
        if (!**l)
            continue;
        char *line = g_strdup_printf("%s: %s", file, *l);
        gboolean known = FALSE;
        for (guint i = 0; i < browser.ca_trusted->len && !known; i++)
            known = !strcmp(g_ptr_array_index(browser.ca_trusted, i), line);
        if (known)
            g_free(line);
        else
            g_ptr_array_add(browser.ca_trusted, line);
    }
    g_strfreev(lines);
    if (about)
        *about = subjects;
    else
        g_free(subjects);
    return TRUE;
}

static char *ca_text(void)
{
    if (!browser.ca_trusted || !browser.ca_trusted->len)
        return g_strdup("certificate authorities trusted besides the guest's: none (web proxy ca FILE: a TLS-reading proxy's)\n");
    GString *s = g_string_new("certificate authorities trusted besides the guest's (web proxy ca):\n");
    for (guint i = 0; i < browser.ca_trusted->len; i++)
        g_string_append_printf(s, "  %s\n", (char *)g_ptr_array_index(browser.ca_trusted, i));
    return g_string_free(s, FALSE);
}

static char *proxy_text(void)
{
    GString *s = g_string_new(NULL);
    if (browser.proxy) {
        char *shown = proxy_shown(browser.proxy);
        const char *const *hosts = proxy_bypass_hosts();
        char *joined = g_strjoinv(", ", (char **)hosts);
        g_string_append_printf(s, "proxy: %s (every request of the browser: pages, frames, workers, WebSockets, downloads;"
                                  " web tz auto)\n  straight to the sites: %s (--bypass HOSTS | none | default)\n",
                               shown, *hosts ? joined : "none, every host through the proxy");
        g_free(shown);
        g_free(joined);
    } else
        g_string_append(s, "proxy: none (requests go straight to the sites; web proxy URL)\n");
    char *ca = ca_text();
    g_string_append(s, ca);
    g_free(ca);
    return g_string_free(s, FALSE);
}

/* the proxy to the network session, when it changed: WebKit stops the loads under way (soup
 * aborts its session) and the next requests go the new way */
static gboolean proxy_apply(void)
{
    if (!browser.session)
        return FALSE;
    char *hosts = g_strjoinv(",", (char **)proxy_bypass_hosts());
    char *now = browser.proxy ? g_strdup_printf("%s %s", browser.proxy, hosts) : g_strdup("");
    g_free(hosts);
    if (!strcmp(now, browser.proxy_sent ? browser.proxy_sent : "")) {
        g_free(now);
        return FALSE;
    }
    if (browser.proxy) {
        WebKitNetworkProxySettings *settings = webkit_network_proxy_settings_new(browser.proxy, proxy_bypass_hosts());
        webkit_network_session_set_proxy_settings(browser.session, WEBKIT_NETWORK_PROXY_MODE_CUSTOM, settings);
        webkit_network_proxy_settings_free(settings);
    } else
        webkit_network_session_set_proxy_settings(browser.session, WEBKIT_NETWORK_PROXY_MODE_NO_PROXY, NULL);
    if (debugging) {
        char *shown = browser.proxy ? proxy_shown(browser.proxy) : NULL;
        DEBUG("proxy: %s\n", shown ? shown : "none");
        g_free(shown);
    }
    browser.proxy_refused = FALSE;
    g_free(browser.proxy_sent);
    browser.proxy_sent = now;
    return TRUE;
}

/* one of the proxy's options; FALSE with *error */
static gboolean proxy_option(const char *option, const char *value, char **error)
{
    if (!strcmp(option, "--proxy"))
        return proxy_set(value, error);
    if (!strcmp(option, "--proxy-bypass"))
        return proxy_bypass_set(value, error);
    if (!strcmp(option, "--proxy-ca")) {
        /* several: FILE:FILE (WEB_PROXY_CA), or the option again */
        char **files = g_strsplit(value, ":", -1);
        gboolean ok = TRUE;
        for (char **f = files; *f && ok; f++) {
            if (**f)
                ok = ca_add(*f, NULL, error);
        }
        g_strfreev(files);
        return ok;
    }
    return TRUE;
}

static gboolean is_proxy_option(const char *option)
{
    return !strcmp(option, "--proxy") || !strcmp(option, "--proxy-bypass") || !strcmp(option, "--proxy-ca");
}

/* The proxy's options among the arguments, before the other settings: a --timezone auto with them
 * asks where the browser's address is through the proxy. FALSE with *error. */
static gboolean proxy_options(int argc, char **argv, char **error)
{
    for (int i = 1; i < argc; i++) {
        if (!is_proxy_option(argv[i]))
            continue;
        if (i + 1 >= argc) {
            *error = g_strdup_printf("%s needs a value", argv[i]);
            return FALSE;
        }
        if (!proxy_option(argv[i], argv[i + 1], error))
            return FALSE;
        i++;
    }
    return TRUE;
}

/* A setting among the arguments at argv[i] (the browser's options, `web set`): applied (the view
 * sees it at settings_changed); how many words it took, 0 if it is not one, -1 with *error. The
 * proxy's are applied before (proxy_options): taken here, not applied again. */
static int setting_option(int argc, char **argv, int i, char **error)
{
    const char *option = argv[i];
    char *about = NULL;
    if (is_proxy_option(option)) {
        if (i + 2 > argc) {
            *error = g_strdup_printf("%s needs a value", option);
            return -1;
        }
        return 2;
    }
    int words = !strcmp(option, "--header-for") ? 3
              : !strcmp(option, "--user-agent") || !strcmp(option, "--header") || !strcmp(option, "--lang")
                || !strcmp(option, "--inject") || !strcmp(option, "--timezone") || !strcmp(option, "--humanize") ? 2
              : !strcmp(option, "--devtools") ? 1 : 0;
    if (!words)
        return 0;
    if (words == 1) {
        browser.devtools_wanted = TRUE; /* on at settings_changed (the view is there then) */
        return 1;
    }
    if (i + words > argc) {
        *error = g_strdup_printf("%s needs %s", option, words == 3 ? "a host and NAME: VALUE" : "a value");
        return -1;
    }
    gboolean ok = !strcmp(option, "--user-agent") ? identity_set(argv[i + 1], error)
                : !strcmp(option, "--header") ? header_line(NULL, argv[i + 1], error)
                : !strcmp(option, "--header-for") ? header_line(argv[i + 1], argv[i + 2], error)
                : !strcmp(option, "--lang") ? languages_set(argv[i + 1], error)
                : !strcmp(option, "--humanize") ? humanize_set(argv[i + 1], error)
                : !strcmp(option, "--timezone") ? time_zone_set(argv[i + 1], &about, error)
                : script_file(argv[i + 1], error);
    g_free(about);
    return ok ? words : -1;
}

static gboolean devtools_on(void);

/* What the settings say, to the view and the web processes (each only when it changed) */
static void settings_changed(void)
{
    proxy_apply(); /* before the first load: its requests go through it too */
    if (browser.devtools_wanted && browser.page_started) {
        browser.devtools_wanted = FALSE;
        devtools_on();
    }
    WebKitSettings *settings = webkit_web_view_get_settings(browser.view);
    if (g_strcmp0(webkit_settings_get_user_agent(settings), browser.identity.user_agent))
        webkit_settings_set_user_agent(settings, browser.identity.user_agent);

    /* Safari tells pages one language (navigator.languages, Intl's default); the others the list */
    WebKitWebContext *context = webkit_web_context_get_default();
    char **languages;
    if (identity_is_safari()) {
        languages = g_new0(char *, 2);
        languages[0] = first_language();
    } else if (browser.languages)
        languages = g_strdupv(browser.languages);
    else {
        GPtrArray *list = g_ptr_array_new();
        for (const char *const *l = g_get_language_names(); *l; l++) {
            if (!strpbrk(*l, ".@"))
                g_ptr_array_add(list, g_strdup(*l));
        }
        g_ptr_array_add(list, NULL);
        languages = (char **)g_ptr_array_free(list, FALSE);
    }
    char *joined = g_strjoinv(",", languages);
    if (g_strcmp0(joined, browser.languages_sent)) {
        webkit_web_context_set_preferred_languages(context, (const char *const *)languages);
        g_free(browser.languages_sent);
        browser.languages_sent = g_steal_pointer(&joined);
    }
    g_free(joined);
    g_strfreev(languages);

    /* the header rules and the identity: a web process started from now on gets them as it starts
     * (its extension's initialization data), the ones running at once (a message) */
    device_draw();
    GVariant *rules = g_variant_ref_sink(header_rules());
    GVariant *identity = g_variant_ref_sink(identity_values());
    char *printed = g_variant_print(rules, FALSE), *printed_identity = g_variant_print(identity, FALSE);
    gboolean rules_changed = g_strcmp0(printed, browser.rules_sent) != 0;
    gboolean identity_changed = g_strcmp0(printed_identity, browser.identity_sent) != 0;
    if (rules_changed || identity_changed) {
        GVariantDict dict;
        g_variant_dict_init(&dict, NULL);
        g_variant_dict_insert_value(&dict, "headers", rules);
        g_variant_dict_insert_value(&dict, "identity", identity);
        webkit_web_context_set_web_process_extensions_initialization_user_data(context, g_variant_dict_end(&dict));
    }
    if (rules_changed) {
        webkit_web_context_send_message_to_all_extensions(context, webkit_user_message_new("headers", rules));
        g_free(browser.rules_sent);
        browser.rules_sent = g_steal_pointer(&printed);
    }
    if (identity_changed) {
        webkit_web_context_send_message_to_all_extensions(context, webkit_user_message_new("identity", identity));
        g_free(browser.identity_sent);
        browser.identity_sent = g_steal_pointer(&printed_identity);
    }
    g_free(printed);
    g_free(printed_identity);
    g_variant_unref(rules);
    g_variant_unref(identity);

    /* user scripts, at the start of every document: the input counter (in the browser's own world),
     * what of the identity WebKit does not answer itself (identity.js), the injected */
    GPtrArray *sources = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(sources, g_strdup(INPUT_COUNTER_JS));
    g_ptr_array_add(sources, identity_script());
    for (guint i = 0; i < browser.scripts->len; i++) {
        Script *script = g_ptr_array_index(browser.scripts, i);
        char *uri = script->name[0] == '/' ? g_filename_to_uri(script->name, NULL, NULL) : g_strdup(script->name);
        g_ptr_array_add(sources, g_strdup_printf("%s\n//# sourceURL=%s\n", script->source, uri));
        g_free(uri);
    }
    GChecksum *checksum = g_checksum_new(G_CHECKSUM_SHA256);
    for (guint i = 0; i < sources->len; i++)
        g_checksum_update(checksum, g_ptr_array_index(sources, i), -1);
    if (g_strcmp0(g_checksum_get_string(checksum), browser.scripts_installed)) {
        WebKitUserContentManager *content = webkit_web_view_get_user_content_manager(browser.view);
        webkit_user_content_manager_remove_all_scripts(content);
        for (guint i = 0; i < sources->len; i++) {
            const char *source = g_ptr_array_index(sources, i);
            if (!*source)
                continue;
            WebKitUserScript *script = i == 0
                ? webkit_user_script_new_for_world(source, WEBKIT_USER_CONTENT_INJECT_ALL_FRAMES, WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START,
                                                   AGENT_WORLD, NULL, NULL)
                : webkit_user_script_new(source, WEBKIT_USER_CONTENT_INJECT_ALL_FRAMES, WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START,
                                         NULL, NULL);
            webkit_user_content_manager_add_script(content, script);
            webkit_user_script_unref(script);
        }
        g_free(browser.scripts_installed);
        browser.scripts_installed = g_strdup(g_checksum_get_string(checksum));
    }
    g_checksum_free(checksum);
    g_ptr_array_free(sources, TRUE);
}

/* ── developer tools: the network as the Web Inspector records it ────────────────────────────
 * devtools.js runs in a JavaScript context of the browser's own and keeps every request the page
 * makes as an HTTP Archive entry. While the tools are on, an inspector frontend in the program
 * (WebKit patch 0011) gives it the inspector protocol's messages — WebKit's Network, Page and
 * Worker domains, per page process (target) — and sends what it asks for. Messages go both ways
 * from the run loop (devtools_pump), not from inside WebKit's calls that bring them. */

static char *devtools_call(const char *method, const char *arg1, const char *arg2)
{
    if (!browser.devtools)
        return NULL;
    JSCValue *result = arg2 ? jsc_value_object_invoke_method(browser.devtools, method, G_TYPE_STRING, arg1, G_TYPE_STRING, arg2, G_TYPE_NONE)
                     : arg1 ? jsc_value_object_invoke_method(browser.devtools, method, G_TYPE_STRING, arg1, G_TYPE_NONE)
                     : jsc_value_object_invoke_method(browser.devtools, method, G_TYPE_NONE);
    JSCException *exception = jsc_context_get_exception(browser.devtools_context);
    if (exception) {
        char *report = jsc_exception_report(exception);
        g_printerr("wpe-browser: devtools.js %s: %s", method, report);
        g_free(report);
        jsc_context_clear_exception(browser.devtools_context);
    }
    char *text = result && !jsc_value_is_undefined(result) && !jsc_value_is_null(result) ? jsc_value_to_string(result) : NULL;
    g_clear_object(&result);
    return text;
}

static void pauses_check(void);
static void net_flush_outbox(void);
static void devtools_pump(void);

/* ── web intercept in the network process ────────────────────────────────────────────────────
 * WebKit's network process (patch 0013) connects to $WPE_BROWSER_NET_SOCKET at its start
 * (netintercept.c) and holds what the browser asks it to — every HTTP hop, responses with their
 * bodies, WebSockets' handshakes and messages — until devtools.js decides. Lines both ways:
 * E ID JSON (held), N JSON (a note), F ID (gone), A SERIAL (settings taken) from it; C MASK SERIAL
 * (what to hold) and D ID JSON (a decision) to it. One network process per browser: its own
 * requests only. The socket is $COLLABO_WEB_DIR/NAME.network. */

static void net_drop_connection(void);

static gboolean net_writable(int fd, GIOCondition condition, gpointer data)
{
    (void)condition;
    (void)data;
    while (browser.net_out->len) {
        ssize_t n = send(fd, browser.net_out->str, browser.net_out->len, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
            return G_SOURCE_CONTINUE;
        if (n <= 0) {
            browser.net_write_id = 0;
            net_drop_connection();
            return G_SOURCE_REMOVE;
        }
        g_string_erase(browser.net_out, 0, n);
    }
    browser.net_write_id = 0;
    return G_SOURCE_REMOVE;
}

static void net_send(const char *text)
{
    if (browser.net_fd < 0 || !text || !*text)
        return;
    g_string_append(browser.net_out, text);
    if (text[strlen(text) - 1] != '\n')
        g_string_append_c(browser.net_out, '\n');
    if (!browser.net_write_id)
        browser.net_write_id = g_unix_fd_add(browser.net_fd, G_IO_OUT, net_writable, NULL);
}

/* what devtools.js has for the network process (settings, decisions) */
static void net_flush_outbox(void)
{
    char *out = browser.devtools ? devtools_call("takeNetOutbox", NULL, NULL) : NULL;
    if (out && *out)
        net_send(out);
    g_free(out);
}

static void net_drop_connection(void)
{
    if (browser.net_fd < 0)
        return;
    if (browser.net_read_id)
        g_source_remove(browser.net_read_id);
    if (browser.net_write_id)
        g_source_remove(browser.net_write_id);
    browser.net_read_id = browser.net_write_id = 0;
    close(browser.net_fd);
    browser.net_fd = -1;
    g_string_truncate(browser.net_in, 0);
    g_string_truncate(browser.net_out, 0);
    /* what it held went on as it was (its process ended, or it lets go when the socket closes) */
    g_free(devtools_call("netGone", NULL, NULL));
    pauses_check();
}

static void net_lines(void)
{
    char *newline;
    gboolean any = FALSE;
    while ((newline = memchr(browser.net_in->str, '\n', browser.net_in->len))) {
        size_t length = newline - browser.net_in->str;
        char *line = g_strndup(browser.net_in->str, length);
        g_string_erase(browser.net_in, 0, length + 1);
        if (!browser.devtools) {
            /* nothing to decide it: on as it was */
            unsigned long long id = 0;
            if (line[0] == 'E' && sscanf(line + 1, "%llu", &id) == 1) {
                char *decision = g_strdup_printf("D %llu {}", id);
                net_send(decision);
                g_free(decision);
            }
        } else {
            if (!any)
                devtools_pump(); /* what the inspector said first: a record it made is then there for this */
            g_free(devtools_call("netReceive", line, NULL));
            any = TRUE;
        }
        g_free(line);
    }
    if (any) {
        net_flush_outbox();
        pauses_check();
    }
}

static gboolean net_readable(int fd, GIOCondition condition, gpointer data)
{
    (void)condition;
    (void)data;
    char buffer[65536];
    for (;;) {
        ssize_t n = read(fd, buffer, sizeof buffer);
        if (n > 0) {
            g_string_append_len(browser.net_in, buffer, n);
            continue;
        }
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
            break;
        net_lines();
        browser.net_read_id = 0;
        net_drop_connection();
        return G_SOURCE_REMOVE;
    }
    net_lines();
    return G_SOURCE_CONTINUE;
}

/* a network process connects (WebKit starts another after a crash): it takes the settings now */
static gboolean net_accept(int fd, GIOCondition condition, gpointer data)
{
    (void)condition;
    (void)data;
    int connection = accept(fd, NULL, NULL);
    if (connection < 0)
        return G_SOURCE_CONTINUE;
    net_drop_connection();
    fcntl(connection, F_SETFD, FD_CLOEXEC);
    fcntl(connection, F_SETFL, fcntl(connection, F_GETFL) | O_NONBLOCK);
    browser.net_fd = connection;
    browser.net_connections++;
    browser.net_read_id = g_unix_fd_add(connection, G_IO_IN | G_IO_HUP | G_IO_ERR, net_readable, NULL);
    char *config = browser.devtools ? devtools_call("netConnected", NULL, NULL) : NULL;
    net_send(config ? config : "C 0 0");
    g_free(config);
    return G_SOURCE_CONTINUE;
}

/* the socket WebKit's network process (started later, by WebKit) finds in its environment */
static void start_net_socket(void)
{
    browser.net_fd = browser.net_listen_fd = -1;
    browser.net_in = g_string_new(NULL);
    browser.net_out = g_string_new(NULL);
    const char *dir = g_getenv("COLLABO_WEB_DIR");
    /* NAME.network: not NAME.sock, which web list takes for a browser's (and would connect to) */
    browser.net_socket_path = g_strdup_printf("%s/%s.network", dir && *dir ? dir : "/tmp/.collabo-web", browser.name);
    struct sockaddr_un address = { .sun_family = AF_UNIX };
    if (strlen(browser.net_socket_path) >= sizeof address.sun_path)
        return;
    strcpy(address.sun_path, browser.net_socket_path);
    unlink(browser.net_socket_path);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return;
    if (bind(fd, (struct sockaddr *)&address, sizeof address) < 0 || listen(fd, 4) < 0) {
        g_printerr("wpe-browser: no socket for the network process (%s): web intercept holds the page's requests only\n", g_strerror(errno));
        close(fd);
        return;
    }
    chmod(browser.net_socket_path, 0600);
    browser.net_listen_fd = fd;
    g_unix_fd_add(fd, G_IO_IN, net_accept, NULL);
    g_setenv("WPE_BROWSER_NET_SOCKET", browser.net_socket_path, TRUE);
}

/* what the inspector said, to devtools.js; what devtools.js has to say, to the inspector */
static void devtools_pump(void)
{
    for (int round = 0; browser.devtools && round < 100; round++) {
        gboolean any = FALSE;
        char *message;
        while ((message = g_queue_pop_head(browser.inspector_messages))) {
            g_free(devtools_call("receive", message, NULL));
            g_free(message);
            any = TRUE;
        }
        char *out = devtools_call("takeOutbox", NULL, NULL);
        if (out && *out && browser.inspector) {
            char **lines = g_strsplit(out, "\n", -1);
            for (char **line = lines; *line; line++) {
                if (**line)
                    webkit_inspector_frontend_send(browser.inspector, *line);
            }
            g_strfreev(lines);
            any = TRUE;
        }
        g_free(out);
        net_flush_outbox();
        if (!any)
            break;
    }
    if (browser.devtools)
        pauses_check();
}

static gboolean devtools_pump_soon(gpointer data)
{
    (void)data;
    browser.devtools_pump_id = 0;
    devtools_pump();
    return G_SOURCE_REMOVE;
}

static void on_inspector_message(const char *message, void *data)
{
    (void)data;
    g_queue_push_tail(browser.inspector_messages, g_strdup(message));
    if (!browser.devtools_pump_id)
        browser.devtools_pump_id = g_idle_add_full(G_PRIORITY_DEFAULT, devtools_pump_soon, NULL, NULL);
}

static gboolean devtools_load(void)
{
    if (browser.devtools)
        return TRUE;
    browser.devtools_context = jsc_context_new();
    JSCValue *value = jsc_context_evaluate_with_source_uri(browser.devtools_context, devtools_js, -1, "wpe-browser:///devtools.js", 1);
    JSCException *exception = jsc_context_get_exception(browser.devtools_context);
    if (exception || !value || !jsc_value_is_object(value)) {
        char *report = exception ? jsc_exception_report(exception) : g_strdup("not an object\n");
        g_printerr("wpe-browser: devtools.js: %s", report);
        g_free(report);
        g_clear_object(&value);
        g_clear_object(&browser.devtools_context);
        return FALSE;
    }
    browser.devtools = value;
    browser.inspector_messages = g_queue_new();
    return TRUE;
}

/* devtools.js's start: the frontend's first commands (again for each connection) */
static void devtools_start(void)
{
    GString *options = g_string_new("{\"bodies\": ");
    json_string(options, browser.devtools_bodies);
    g_string_append(options, ", \"uri\": ");
    json_string(options, webkit_web_view_get_uri(browser.view));
    g_string_append(options, ", \"title\": ");
    json_string(options, webkit_web_view_get_title(browser.view));
    g_string_append_printf(options, ", \"version\": \"%u.%u.%u\"}", webkit_get_major_version(), webkit_get_minor_version(),
                           webkit_get_micro_version());
    g_free(devtools_call("start", options->str, NULL));
    g_string_free(options, TRUE);
}

/* The page's targets come at once (as messages), each one the page goes to later paused until
 * devtools.js has turned the recording on in it. A page target without a process (before the
 * first load, after a crash) takes no messages, and does not get them when its process starts:
 * the browser connects once the page has one. */
static gboolean devtools_on(void)
{
    if (browser.inspector)
        return TRUE;
    if (!devtools_load())
        return FALSE;
    devtools_start();
    /* the network process connected already: devtools.js tells it what to hold (and record) */
    if (browser.net_fd >= 0) {
        char *config = devtools_call("netConnected", NULL, NULL);
        net_send(config);
        g_free(config);
    }
    browser.inspector = webkit_web_view_connect_inspector_frontend(browser.view, on_inspector_message, NULL);
    devtools_pump();
    return TRUE;
}

/* The page's process ended and the next one started: the frontend again (the recording goes on).
 * Right after the browser asks for the load, its messages come before the load's own requests. */
static void devtools_connect_again(void)
{
    browser.devtools_reconnect = FALSE;
    if (!browser.inspector)
        return;
    devtools_pump();
    webkit_inspector_frontend_disconnect(browser.inspector);
    g_queue_clear_full(browser.inspector_messages, g_free);
    devtools_start();
    browser.inspector = webkit_web_view_connect_inspector_frontend(browser.view, on_inspector_message, NULL);
    devtools_pump();
}

/* Off: the frontend goes, and with it the interception (WebKit sends on what it held) */
static void devtools_off(void)
{
    if (!browser.inspector)
        return;
    devtools_pump();
    webkit_inspector_frontend_disconnect(browser.inspector);
    browser.inspector = NULL;
    g_queue_clear_full(browser.inspector_messages, g_free);
    g_free(devtools_call("stop", NULL, NULL));
    net_flush_outbox(); /* the network process holds nothing more, and lets go of what it held */
    pauses_check();
}

static gboolean devtools_bodies_mode(const char *mode)
{
    return !g_strcmp0(mode, "text") || !g_strcmp0(mode, "all") || !g_strcmp0(mode, "none");
}

static char *devtools_text(void)
{
    char *status = browser.devtools ? devtools_call("status", NULL, NULL) : NULL;
    char *intercept = browser.intercepting ? devtools_call("interceptStatus", NULL, NULL) : NULL;
    char *text = browser.inspector ? g_strdup_printf("developer tools: on, recording the network (response bodies: %s); %s\n%s",
                                                     browser.devtools_bodies, status ? status : "", intercept ? intercept : "")
               : status ? g_strdup_printf("developer tools: off (web devtools on); recorded before: %s\n", status)
               : g_strdup("developer tools: off (web devtools on records the network)\n");
    g_free(intercept);
    g_free(status);
    return text;
}

/* ── web intercept: the page's requests wait for the agent ─────────────────────────────────
 * devtools.js has WebKit hold the requests the agent asked for (and their responses, if so) until
 * it sends, drops or answers each. A command that waits on the page (a load, `wait`, `run`) would
 * wait for nothing then: it ends as a request pauses, with status 6 and the paused ones listed. */

/* the answer of a command whose wait a paused request ended (intercept wait: its own answer) */
static void reply_paused(Request *r)
{
    gboolean waited_for = !strcmp(r->argv[0], "intercept");
    char *list = devtools_call("pausedList", waited_for ? "" : "paused: ", NULL);
    const char *after = waited_for ? ""
                      : !strcmp(r->argv[0], "run") ? "the script goes on in the page; what it returns is lost (run it once they are decided)\n"
                      : !strcmp(r->argv[0], "eval") ? "the script goes on in the page once they are decided; what it returns is lost\n"
                      : browser.loading ? "the page waits for them (web wait load, once they are decided)\n" : "";
    replyf(r, waited_for ? ST_OK : ST_PAUSED, "%s%s%s", r->done_text ? r->done_text : "", list ? list : "", after);
    g_free(list);
}

static gboolean pauses_wake(gpointer data)
{
    (void)data;
    browser.pause_wake_id = 0;
    if (!browser.paused_count)
        return G_SOURCE_REMOVE; /* decided already */
    GList *waiting = g_list_concat(g_list_copy(browser.load_waiters), g_list_copy(browser.pause_waiters));
    for (GList *l = waiting; l; l = l->next) {
        Request *r = l->data;
        /* eval is not held up by what it starts (a fetch), only by a synchronous request that stops the page */
        if (!strcmp(r->argv[0], "eval") && !browser.blocking_count)
            continue;
        if (r->pause_base < browser.pause_serial && (g_list_find(browser.load_waiters, r) || g_list_find(browser.pause_waiters, r)))
            reply_paused(r);
    }
    g_list_free(waiting);
    return G_SOURCE_REMOVE;
}

/* after devtools.js has had its messages: whether a request paused (the waits end, from the run loop) */
static void pauses_check(void)
{
    char *state = devtools_call("pauseState", NULL, NULL);
    unsigned serial = 0, count = 0, on = 0, blocking = 0;
    if (state && sscanf(state, "%u %u %u %u", &serial, &count, &on, &blocking) >= 3) {
        browser.paused_count = count;
        browser.intercepting = on;
        browser.blocking_count = blocking;
        if (serial != browser.pause_serial) {
            browser.pause_serial = serial;
            if (!browser.pause_wake_id)
                browser.pause_wake_id = g_idle_add(pauses_wake, NULL);
        }
    }
    g_free(state);
}

/* ["a", "b"]: words for devtools.js */
static char *json_words(char **words, int count)
{
    GString *s = g_string_new("[");
    for (int i = 0; i < count; i++) {
        if (i)
            g_string_append(s, ", ");
        json_string(s, words[i]);
    }
    g_string_append_c(s, ']');
    return g_string_free(s, FALSE);
}

/* ── commands ──────────────────────────────────────────────────────────────────────────────── */

static int timeout_arg(Request *r, int index, int fallback_ms)
{
    if (r->argc > index) {
        char *end;
        double seconds = g_ascii_strtod(r->argv[index], &end);
        if (end != r->argv[index] && seconds > 0)
            return (int)(seconds * 1000);
    }
    return fallback_ms;
}

static void cmd_open(Request *r)
{
    if (r->argc < 2) {
        reply(r, ST_USAGE, "usage: open URL [TIMEOUT_SECONDS]\n");
        return;
    }
    char *uri = normalize_uri(r->argv[1]);
    webkit_web_view_load_uri(browser.view, uri);
    g_free(uri);
    r->acted = TRUE;
    r->shot_when = "after the load";
    if (browser.devtools_reconnect)
        devtools_connect_again(); /* a page whose process had ended has one again */
    /* a load follows (unless the address only moves within the page): its start can take a while
     * when the page's process is started first */
    wait_for_load(r, timeout_arg(r, 2, 60000), 15000);
}

static void cmd_history(Request *r, const char *which)
{
    if (!strcmp(which, "back")) {
        if (!webkit_web_view_can_go_back(browser.view)) {
            reply(r, ST_FAILED, "no page to go back to\n");
            return;
        }
        webkit_web_view_go_back(browser.view);
    } else if (!strcmp(which, "forward")) {
        if (!webkit_web_view_can_go_forward(browser.view)) {
            reply(r, ST_FAILED, "no page to go forward to\n");
            return;
        }
        webkit_web_view_go_forward(browser.view);
    } else
        webkit_web_view_reload(browser.view);
    r->acted = TRUE;
    r->shot_when = "after the load";
    if (browser.devtools_reconnect)
        devtools_connect_again();
    wait_for_load(r, timeout_arg(r, 1, 60000), 1500);
}

static void cmd_info(Request *r)
{
    const char *uri = webkit_web_view_get_uri(browser.view);
    const char *title = webkit_web_view_get_title(browser.view);
    int width = 0, height = 0;
    if (wpe_view())
        wpe_toplevel_get_size(wpe_view_get_toplevel(wpe_view()), &width, &height);
    char *intercept = browser.intercepting || browser.paused_count
        ? g_strdup_printf("intercept: %s, %u request%s paused%s\n", browser.intercepting ? "on" : "off", browser.paused_count,
                          browser.paused_count == 1 ? "" : "s", browser.paused_count ? " (web intercept lists them)" : "")
        : NULL;
    replyf(r, ST_OK,
           "url: %s\ntitle: %s\nloading: %s (%.0f%%)\nback: %s\nforward: %s\nsize: %dx%d\nuser agent: %s\ndevtools: %s\n%s%s%s%s%s%s%s",
           uri ? uri : "about:blank", title ? title : "",
           browser.loading ? "yes" : "no", webkit_web_view_get_estimated_load_progress(browser.view) * 100,
           webkit_web_view_can_go_back(browser.view) ? "yes" : "no",
           webkit_web_view_can_go_forward(browser.view) ? "yes" : "no", width, height,
           browser.identity.profile ? browser.identity.profile : browser.identity.user_agent,
           browser.inspector ? "on (web network)" : "off", intercept ? intercept : "",
           browser.last_load_error ? "error: " : "", browser.last_load_error ? browser.last_load_error : "",
           browser.last_load_error ? "\n" : "",
           browser.last_dialog ? "last dialog: " : "", browser.last_dialog ? browser.last_dialog : "",
           browser.last_dialog ? "\n" : "");
    g_free(intercept);
}

/* eval [--isolated] JS: in the page's world (its globals), or the browser's own (the DOM only,
 * where nothing the page did to its globals and prototypes is in the way, and nothing shows) */
static void cmd_eval(Request *r)
{
    gboolean isolated = r->argc > 2 && !strcmp(r->argv[1], "--isolated");
    if (r->argc < 2 + isolated) {
        reply(r, ST_USAGE, "usage: eval [--isolated] JAVASCRIPT\n");
        return;
    }
    webkit_web_view_evaluate_javascript(browser.view, r->argv[1 + isolated], -1, isolated ? AGENT_WORLD : NULL, NULL, NULL,
                                        eval_finished, js_call_new(r, reply_js_text));
    /* a synchronous XMLHttpRequest it makes may pause (the page waits for it: so would this) */
    browser.pause_waiters = g_list_prepend(browser.pause_waiters, r);
}

/* run NAME SOURCE TIMEOUT_SECONDS [ARG...] (web.c reads the file): the file as the body of an async
 * function in the page, `args` its arguments; `return` gives the answer, `await` waits */
static const char RUN_PREFIX[] = "const args = JSON.parse(__webArgs); try { return await (async () => {";
/* an error: its message and stack, without the wrapper's frames (the file's, past its last line) */
static const char RUN_SUFFIX[] =
    "\n})(); } catch (e) { if (!e || typeof e !== 'object' || !e.stack) throw e;"
    " const own = l => { const m = l.match(/^(.*):(\\d+):\\d+$/); return !(m && m[1].endsWith(__webFile) && +m[2] > __webLines); };"
    " throw `${e}\\n${e.stack.split('\\n').filter(own).join('\\n')}`; }";

static gboolean run_timed_out(gpointer data)
{
    Request *r = data;
    r->timeout_id = 0;
    replyf(r, ST_TIMEOUT, "the script has not finished within %g s (it goes on in the page)\n",
           timeout_arg(r, 3, 120000) / 1000.0);
    return G_SOURCE_REMOVE;
}

static void cmd_run(Request *r)
{
    gboolean isolated = r->argc > 1 && !strcmp(r->argv[1], "--isolated");
    if (isolated) {
        /* the words after it as the rest expect them */
        g_free(r->argv[1]);
        memmove(r->argv + 1, r->argv + 2, (r->argc - 1) * sizeof *r->argv);
        r->argc--;
    }
    if (r->argc < 4) {
        reply(r, ST_USAGE, "usage: run [--isolated] FILE [ARG...] (web run reads the file)\n");
        return;
    }
    const char *name = r->argv[1];
    char *source = g_strdup(r->argv[2]);
    if (!g_utf8_validate(source, -1, NULL)) {
        replyf(r, ST_USAGE, "%s is not UTF-8 text\n", name);
        g_free(source);
        return;
    }
    if (g_str_has_prefix(source, "#!"))
        memcpy(source, "//", 2); /* a script's #! line: a comment, keeping the lines where they are */
    /* the file's first line goes on the wrapper's (WebKit's, too): line numbers stay the file's */
    char *body = g_strconcat(RUN_PREFIX, source, RUN_SUFFIX, NULL);
    GString *args = g_string_new("[");
    for (int i = 4; i < r->argc; i++) {
        if (i > 4)
            g_string_append(args, ", ");
        json_string(args, r->argv[i]);
    }
    g_string_append_c(args, ']');
    char *uri = name[0] == '/' ? g_filename_to_uri(name, NULL, NULL) : g_strdup(name);
    int lines = 1;
    for (const char *c = source; *c; c++)
        lines += *c == '\n';
    webkit_web_view_call_async_javascript_function(browser.view, body, -1,
                                                   args_dict("__webArgs", g_variant_new_string(args->str), "__webFile", g_variant_new_string(uri),
                                                             "__webLines", g_variant_new_int32(lines), NULL),
                                                   isolated ? AGENT_WORLD : NULL, uri, NULL, js_finished, js_call_new(r, reply_js_text));
    r->timeout_id = g_timeout_add(timeout_arg(r, 3, 120000), run_timed_out, r);
    browser.pause_waiters = g_list_prepend(browser.pause_waiters, r); /* a request it awaits may pause */
    g_free(uri);
    g_string_free(args, TRUE);
    g_free(body);
    g_free(source);
}

static void cmd_read(Request *r)
{
    int max = 40000;
    gboolean hidden = FALSE;
    for (int i = 1; i < r->argc; i++) {
        if (!strcmp(r->argv[i], "--all") || !strcmp(r->argv[i], "--hidden"))
            hidden = TRUE;
        else if (atoi(r->argv[i]) > 0)
            max = atoi(r->argv[i]);
    }
    /* a number: WebKit passes no booleans to JavaScript (numbers, strings, dictionaries only) */
    char *body = g_strconcat("return (", snapshot_js, ")(maxChars, !!includeHidden);", NULL);
    call_js(r, body, args_dict("maxChars", g_variant_new_int32(max), "includeHidden", g_variant_new_int32(hidden), NULL), reply_js_text);
    g_free(body);
}

static void cmd_text(Request *r)
{
    call_js(r, "return document.body ? document.body.innerText : '';", NULL, reply_js_text);
}

static void cmd_html(Request *r)
{
    call_js(r, "const e = selector ? document.querySelector(selector) : document.documentElement; "
               "return e ? e.outerHTML : null;",
            args_dict("selector", g_variant_new_string(r->argc > 1 ? r->argv[1] : ""), NULL), reply_js_text);
}

static void cmd_links(Request *r)
{
    call_js(r, "const seen = new Set(); const lines = [];"
               "for (const a of document.querySelectorAll('a[href]')) {"
               "  if (a.href.startsWith('javascript:') || seen.has(a.href)) continue; seen.add(a.href);"
               "  const text = (a.innerText || a.getAttribute('aria-label') || a.title || '').replace(/\\s+/g, ' ').trim();"
               "  lines.push((text ? text.slice(0, 100) + '  ' : '') + a.href); }"
               "return lines.join('\\n');",
            NULL, reply_js_text);
}

/* ── screenshots of the actions (--screenshot FILE) ─────────────────────────────────────────
 * WebKit's own snapshot of what the view shows (painted for it, so it is the page as it is now:
 * a scroll into view or a hover's style included), as PNG, with where the action went marked: a
 * ring at the point, a frame around the element, an arrow for a scroll, a caption. A click is
 * pictured as it goes in (the page it may load is another), the other actions once done. */

#define SHOT_TIMEOUT_MS 10000
#define CAPTION_PX 15

/* an RGB picture to draw on */
typedef struct {
    guint8 *px;
    int w, h;
} Canvas;

static const guint8 MARK_RGB[3] = { 255, 32, 80 }, HALO_RGB[3] = { 255, 255, 255 }, CAPTION_RGB[3] = { 24, 24, 28 };

static void blend(Canvas *c, int x, int y, const guint8 rgb[3], double alpha)
{
    if (x < 0 || y < 0 || x >= c->w || y >= c->h || alpha <= 0)
        return;
    if (alpha > 1)
        alpha = 1;
    guint8 *p = c->px + ((size_t)y * c->w + x) * 3;
    for (int i = 0; i < 3; i++)
        p[i] = (guint8)(p[i] + (rgb[i] - p[i]) * alpha + 0.5);
}

/* x0..x1, y0..y1, its edges antialiased */
static void fill_rect(Canvas *c, double x0, double y0, double x1, double y1, const guint8 rgb[3], double alpha)
{
    for (int y = (int)floor(y0); y < (int)ceil(y1); y++) {
        double cover_y = fmin(y + 1, y1) - fmax(y, y0);
        for (int x = (int)floor(x0); x < (int)ceil(x1); x++)
            blend(c, x, y, rgb, alpha * cover_y * (fmin(x + 1, x1) - fmax(x, x0)));
    }
}

/* the ring between radii inner and outer (inner < 0: a disc) */
static void fill_ring(Canvas *c, double cx, double cy, double inner, double outer, const guint8 rgb[3], double alpha)
{
    for (int y = (int)floor(cy - outer - 1); y <= (int)ceil(cy + outer + 1); y++) {
        for (int x = (int)floor(cx - outer - 1); x <= (int)ceil(cx + outer + 1); x++) {
            double d = hypot(x + 0.5 - cx, y + 0.5 - cy);
            double cover = fmin(d - inner, outer - d) + 0.5;
            blend(c, x, y, rgb, alpha * fmin(fmax(cover, 0), 1));
        }
    }
}

/* a triangle, 4x4 samples a pixel */
static void fill_triangle(Canvas *c, const double p[6], const guint8 rgb[3], double alpha)
{
    double x0 = fmin(p[0], fmin(p[2], p[4])), x1 = fmax(p[0], fmax(p[2], p[4]));
    double y0 = fmin(p[1], fmin(p[3], p[5])), y1 = fmax(p[1], fmax(p[3], p[5]));
    for (int y = (int)floor(y0); y <= (int)ceil(y1); y++) {
        for (int x = (int)floor(x0); x <= (int)ceil(x1); x++) {
            int in = 0;
            for (int s = 0; s < 16; s++) {
                double sx = x + (s % 4 + 0.5) / 4, sy = y + (s / 4 + 0.5) / 4;
                double d1 = (sx - p[2]) * (p[1] - p[3]) - (p[0] - p[2]) * (sy - p[3]);
                double d2 = (sx - p[4]) * (p[3] - p[5]) - (p[2] - p[4]) * (sy - p[5]);
                double d3 = (sx - p[0]) * (p[5] - p[1]) - (p[4] - p[0]) * (sy - p[1]);
                in += !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0));
            }
            blend(c, x, y, rgb, alpha * in / 16);
        }
    }
}

/* the marks, in two passes: a white halo under them all (pass 0), then the marks (pass 1), so
 * they show on any page */
static void draw_frame(Canvas *c, const double box[4], int pass)
{
    double grow = pass ? 1 : 2.5, width = pass ? 2 : 5;
    double x0 = box[0] - grow, y0 = box[1] - grow, x1 = box[0] + box[2] + grow, y1 = box[1] + box[3] + grow;
    const guint8 *rgb = pass ? MARK_RGB : HALO_RGB;
    double alpha = pass ? 1 : 0.8;
    fill_rect(c, x0, y0, x1, y0 + width, rgb, alpha);
    fill_rect(c, x0, y1 - width, x1, y1, rgb, alpha);
    fill_rect(c, x0, y0 + width, x0 + width, y1 - width, rgb, alpha);
    fill_rect(c, x1 - width, y0 + width, x1, y1 - width, rgb, alpha);
}

/* a ring (two for a double click) with ticks out of it and, for a press, a dot in the middle */
static void draw_point(Canvas *c, double x, double y, int rings, gboolean dot, int pass)
{
    double halo = pass ? 0 : 1.75;
    const guint8 *rgb = pass ? MARK_RGB : HALO_RGB;
    double alpha = pass ? 1 : 0.85;
    double outer = 13;
    for (int i = 0; i < rings; i++) {
        double radius = 12 + 6 * i;
        fill_ring(c, x, y, radius - 1.5 - halo, radius + 1.5 + halo, rgb, alpha);
        outer = radius + 1.5;
    }
    double from = outer + 4, to = outer + 14, half = 1 + halo;
    fill_rect(c, x + from - halo, y - half, x + to + halo, y + half, rgb, alpha);
    fill_rect(c, x - to - halo, y - half, x - from + halo, y + half, rgb, alpha);
    fill_rect(c, x - half, y + from - halo, x + half, y + to + halo, rgb, alpha);
    fill_rect(c, x - half, y - to - halo, x + half, y - from + halo, rgb, alpha);
    if (dot)
        fill_ring(c, x, y, -1, 2.5 + halo, rgb, alpha);
}

/* scroll: an arrow from the point the way it went (dx, dy: a unit along an axis) */
static void draw_arrow(Canvas *c, double x, double y, double dx, double dy, int pass)
{
    double halo = pass ? 0 : 1.75;
    const guint8 *rgb = pass ? MARK_RGB : HALO_RGB;
    double alpha = pass ? 1 : 0.85;
    double start = 22, end = 64, head = 18, half = 2 + halo, wing = 10 + halo * 1.8;
    double sx = x + dx * start, sy = y + dy * start, ex = x + dx * (end - head), ey = y + dy * (end - head);
    fill_rect(c, fmin(sx, ex) - (dx ? halo : half), fmin(sy, ey) - (dy ? halo : half),
              fmax(sx, ex) + (dx ? halo : half), fmax(sy, ey) + (dy ? halo : half), rgb, alpha);
    double tip = end + halo * 2;
    double p[6] = { x + dx * tip, y + dy * tip, ex - dy * wing - dx * halo, ey + dx * wing - dy * halo,
                    ex + dy * wing - dx * halo, ey - dx * wing - dy * halo };
    fill_triangle(c, p, rgb, alpha);
}

/* the caption's faces: DejaVu Sans Bold, Nanum Gothic Bold for Korean (the add-on's fonts) */
static FT_Library caption_library;
static FT_Face caption_faces[2];
static int caption_face_count = -1;

static void caption_fonts(void)
{
    if (caption_face_count >= 0)
        return;
    caption_face_count = 0;
    if (FT_Init_FreeType(&caption_library))
        return;
    static const char *const files[] = { "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf", "/usr/share/fonts/nanum/NanumGothic-Bold.ttf" };
    for (size_t i = 0; i < G_N_ELEMENTS(files); i++) {
        FT_Face face;
        if (FT_New_Face(caption_library, files[i], 0, &face))
            continue;
        FT_Set_Pixel_Sizes(face, 0, CAPTION_PX);
        caption_faces[caption_face_count++] = face;
    }
}

/* Draws `text` from x on the baseline (draw FALSE: only measures); returns its width */
static int caption_text(Canvas *c, const char *text, int x, int baseline, gboolean draw)
{
    int pen = x;
    for (const char *p = text; *p; p = g_utf8_next_char(p)) {
        gunichar ch = g_utf8_get_char(p);
        FT_Face face = caption_faces[0];
        FT_UInt index = 0;
        for (int i = 0; i < caption_face_count && !index; i++) {
            index = FT_Get_Char_Index(caption_faces[i], ch);
            if (index)
                face = caption_faces[i];
        }
        if (FT_Load_Glyph(face, index, draw ? FT_LOAD_RENDER : FT_LOAD_DEFAULT))
            continue;
        FT_GlyphSlot glyph = face->glyph;
        if (draw && glyph->bitmap.pixel_mode == FT_PIXEL_MODE_GRAY) {
            for (unsigned row = 0; row < glyph->bitmap.rows; row++)
                for (unsigned col = 0; col < glyph->bitmap.width; col++)
                    blend(c, pen + glyph->bitmap_left + (int)col, baseline - glyph->bitmap_top + (int)row, HALO_RGB,
                          glyph->bitmap.buffer[row * glyph->bitmap.pitch + col] / 255.0);
        }
        pen += (int)(glyph->advance.x >> 6);
    }
    return pen - x;
}

/* the caption on a dark band, in a corner clear of the marks (x0, y0, x1, y1; NULL: none) — not
 * beside them, where it would hide what is around the element */
static void draw_caption(Canvas *c, const char *text, const double *marks)
{
    caption_fonts();
    if (!caption_face_count || !text || !*text)
        return;
    int ascent = (int)(caption_faces[0]->size->metrics.ascender >> 6);
    int descent = (int)(-caption_faces[0]->size->metrics.descender >> 6);
    int pad = 6, w = caption_text(c, text, 0, 0, FALSE) + 2 * pad, h = ascent + descent + 2 * pad - 2;
    double corners[4][2] = { { 8, c->h - h - 8 }, { 8, 8 }, { c->w - w - 8, c->h - h - 8 }, { c->w - w - 8, 8 } };
    double x = corners[0][0], y = corners[0][1];
    for (int i = 0; marks && i < 4; i++) {
        double cx = corners[i][0], cy = corners[i][1];
        if (cx + w < marks[0] || cx > marks[2] || cy + h < marks[1] || cy > marks[3]) {
            x = cx;
            y = cy;
            break;
        }
    }
    x = fmax(2, x);
    y = fmax(2, y);
    fill_rect(c, x, y, x + w, y + h, CAPTION_RGB, 0.82);
    caption_text(c, text, (int)x + pad, (int)y + pad - 1 + ascent, TRUE);
}

/* the marks of what the request did, in the picture's pixels (sx, sy: the view's scale in it) */
static void shot_mark(Canvas *c, Request *r, double sx, double sy)
{
    double box[4] = { r->box[0] * sx, r->box[1] * sy, r->box[2] * sx, r->box[3] * sy };
    double x = r->x * sx, y = r->y * sy;
    const char *cmd = r->argv[0];
    for (int pass = 0; pass < 2; pass++) {
        if (r->have_box)
            draw_frame(c, box, pass);
        if (r->have_point && (r->arrow_dx || r->arrow_dy))
            draw_arrow(c, x, y, r->arrow_dx, r->arrow_dy, pass);
        if (r->have_point)
            draw_point(c, x, y, !strcmp(cmd, "dblclick") ? 2 : 1, strcmp(cmd, "hover") && strcmp(cmd, "scroll"), pass);
    }
    /* what the marks cover: the caption keeps out of it */
    double marks[4] = { G_MAXDOUBLE, G_MAXDOUBLE, -G_MAXDOUBLE, -G_MAXDOUBLE };
    if (r->have_box) {
        marks[0] = box[0] - 6, marks[1] = box[1] - 6;
        marks[2] = box[0] + box[2] + 6, marks[3] = box[1] + box[3] + 6;
    }
    if (r->have_point) {
        double reach = !strcmp(cmd, "dblclick") ? 40 : 34;
        double ax = x + r->arrow_dx * 70, ay = y + r->arrow_dy * 70;
        marks[0] = fmin(marks[0], fmin(x, ax) - reach), marks[1] = fmin(marks[1], fmin(y, ay) - reach);
        marks[2] = fmax(marks[2], fmax(x, ax) + reach), marks[3] = fmax(marks[3], fmax(y, ay) + reach);
    }
    draw_caption(c, r->shot_label, r->have_box || r->have_point ? marks : NULL);
}

static char *png_save(const char *file, Canvas *c)
{
    FILE *out = fopen(file, "wb");
    if (!out)
        return g_strdup_printf("%s: %s", file, g_strerror(errno));
    png_image image;
    memset(&image, 0, sizeof image);
    image.version = PNG_IMAGE_VERSION;
    image.width = c->w;
    image.height = c->h;
    image.format = PNG_FORMAT_RGB;
    char *problem = NULL;
    if (!png_image_write_to_stdio(&image, out, 0, c->px, c->w * 3, NULL))
        problem = g_strdup_printf("%s: %s", file, image.message);
    png_image_free(&image);
    if (fclose(out) && !problem)
        problem = g_strdup_printf("%s: %s", file, g_strerror(errno));
    return problem;
}

/* The snapshot (BGRA, premultiplied: opaque here) marked, into the file; NULL, or what went wrong */
static char *shot_save(Request *r, WebKitImage *image)
{
    int w = webkit_image_get_width(image), h = webkit_image_get_height(image);
    guint stride = webkit_image_get_stride(image);
    gsize size = 0;
    const guint8 *bgra = g_bytes_get_data(webkit_image_as_bytes(image), &size);
    if (w <= 0 || h <= 0 || !bgra || size < (gsize)stride * h)
        return g_strdup("WebKit gave an empty picture");
    Canvas c = { g_malloc((gsize)w * h * 3), w, h };
    for (int y = 0; y < h; y++) {
        const guint8 *from = bgra + (gsize)y * stride;
        guint8 *to = c.px + (gsize)y * w * 3;
        for (int x = 0; x < w; x++, from += 4, to += 3) {
            to[0] = from[2];
            to[1] = from[1];
            to[2] = from[0];
        }
    }
    int view_w = wpe_view_get_width(wpe_view()), view_h = wpe_view_get_height(wpe_view());
    shot_mark(&c, r, view_w > 0 ? (double)w / view_w : 1, view_h > 0 ? (double)h / view_h : 1);
    char *problem = png_save(r->shot_file, &c);
    g_free(c.px);
    if (problem)
        return problem;
    const char *marked = r->have_point && r->have_box ? "; marked: the point and the element"
                       : r->have_point ? "; marked: the point" : r->have_box ? "; marked: the element" : "";
    r->shot_note = g_strdup_printf("screenshot: %s (%dx%d, %s%s)\n", r->shot_file, w, h, r->shot_when ? r->shot_when : "after it", marked);
    return NULL;
}

/* NULL if the display's settings let agents see its pictures (gui's `capture`), else why not */
static char *shot_forbidden(void)
{
    const char *file = g_getenv("COLLABO_GUI_CONFIG");
    char *text = NULL;
    if (!g_file_get_contents(file && *file ? file : "/etc/collabo/gui.json", &text, NULL, NULL))
        return NULL;
    gboolean off = FALSE;
    const char *p = strstr(text, "\"capture\"");
    if (p) {
        for (p += 9; *p == ' ' || *p == '\t' || *p == '\r' || *p == '\n' || *p == ':'; p++) {}
        off = !strncmp(p, "false", 5);
    }
    g_free(text);
    return off ? g_strdup("the display's settings do not allow screenshots (capture: false, gui status)") : NULL;
}

static void shot_done(Request *r)
{
    if (r->shot_timeout_id) {
        g_source_remove(r->shot_timeout_id);
        r->shot_timeout_id = 0;
    }
    void (*then)(Request *r) = r->after_shot;
    r->after_shot = NULL;
    then(r);
}

static void shot_snapshot_done(GObject *source, GAsyncResult *result, gpointer data)
{
    ShotCall *call = data;
    Request *r = call->r;
    GError *error = NULL;
    WebKitImage *image = webkit_web_view_get_snapshot_finish(WEBKIT_WEB_VIEW(source), result, &error);
    g_free(call);
    if (r) {
        r->shot = NULL;
        char *problem = image ? shot_save(r, image) : g_strdup(error ? error->message : "WebKit gave no picture");
        if (problem)
            r->shot_note = g_strdup_printf("screenshot not taken: %s\n", problem);
        g_free(problem);
        shot_done(r);
    }
    g_clear_object(&image);
    g_clear_error(&error);
}

static gboolean shot_timed_out(gpointer data)
{
    Request *r = data;
    r->shot_timeout_id = 0;
    if (r->js) {
        r->js->r = NULL;
        r->js = NULL;
    }
    if (r->shot) {
        r->shot->r = NULL;
        r->shot = NULL;
    }
    g_free(r->shot_note);
    r->shot_note = g_strdup_printf("screenshot not taken: the page did not answer within %d s\n", SHOT_TIMEOUT_MS / 1000);
    shot_done(r);
    return G_SOURCE_REMOVE;
}

/* Takes the picture, then(r) */
static void shot_take(Request *r, void (*then)(Request *r))
{
    r->shot_taken = TRUE;
    r->after_shot = then;
    char *why = shot_forbidden();
    if (why) {
        r->shot_note = g_strdup_printf("screenshot not taken: %s\n", why);
        g_free(why);
        shot_done(r);
        return;
    }
    if (!r->shot_timeout_id)
        r->shot_timeout_id = g_timeout_add(SHOT_TIMEOUT_MS, shot_timed_out, r);
    r->shot = g_new0(ShotCall, 1);
    r->shot->r = r;
    webkit_web_view_get_snapshot(browser.view, WEBKIT_SNAPSHOT_REGION_VISIBLE, WEBKIT_SNAPSHOT_OPTIONS_NONE, NULL,
                                 shot_snapshot_done, r->shot);
}

static void shot_answer(Request *r)
{
    reply(r, r->held_status, r->held_text);
}

/* after an action: a frame for what it changed to be drawn (a hover's style, a menu), then, with
 * `focus`, the box of what has the focus ("X Y W H", view coordinates; "" for none) */
static const char SHOT_SETTLE_JS[] =
    "await new Promise(done => { requestAnimationFrame(() => requestAnimationFrame(done)); setTimeout(done, 500); });"
    "const e = document.activeElement;"
    "if (!focus || !e || e === document.body || e === document.documentElement) return '';"
    "const r = e.getBoundingClientRect(), s = window.devicePixelRatio || 1;"
    "return [r.left, r.top, r.width, r.height].map(v => Math.round(v * s)).join(' ');";

static void shot_settled(Request *r, JSCValue *value, GError *error)
{
    if (!error && value && jsc_value_is_string(value)) {
        char *box = jsc_value_to_string(value);
        double b[4];
        if (sscanf(box, "%lf %lf %lf %lf", &b[0], &b[1], &b[2], &b[3]) == 4) {
            memcpy(r->box, b, sizeof b);
            r->have_box = TRUE;
        }
        g_free(box);
    }
    shot_take(r, shot_answer);
}

/* The answer to an action that went in, held (nothing else answers it meanwhile) until its
 * picture is taken */
static void shot_hold(Request *r, int status, const char *text)
{
    r->shot_taken = TRUE;
    r->held_status = status;
    r->held_text = g_strdup(text);
    if (r->timeout_id) {
        g_source_remove(r->timeout_id);
        r->timeout_id = 0;
    }
    if (r->poll_id) {
        g_source_remove(r->poll_id);
        r->poll_id = 0;
    }
    if (r->sync_id) {
        g_source_remove(r->sync_id);
        r->sync_id = 0;
    }
    if (r->js) {
        r->js->r = NULL;
        r->js = NULL;
    }
    browser.load_waiters = g_list_remove(browser.load_waiters, r);
    browser.pause_waiters = g_list_remove(browser.pause_waiters, r);
    r->after_shot = shot_answer;
    r->shot_timeout_id = g_timeout_add(SHOT_TIMEOUT_MS, shot_timed_out, r);
    /* (WebKit passes numbers, strings and dictionaries, not booleans) */
    call_js(r, SHOT_SETTLE_JS, args_dict("focus", g_variant_new_int32(r->want_focus && !r->have_box), NULL), shot_settled);
}

/* What target.js (or SHOT_POINT_JS) found, for the picture: its box and a few words on it */
static void shot_element(Request *r, JSCValue *value, const char *json)
{
    JSCValue *found = jsc_value_new_from_json(jsc_value_get_context(value), json);
    if (!found || !jsc_value_is_object(found)) {
        g_clear_object(&found);
        return;
    }
    JSCValue *box = jsc_value_object_get_property(found, "box");
    if (jsc_value_is_array(box)) {
        for (int i = 0; i < 4; i++) {
            JSCValue *v = jsc_value_object_get_property_at_index(box, i);
            r->box[i] = jsc_value_to_double(v);
            g_object_unref(v);
        }
        r->have_box = r->box[2] > 0 && r->box[3] > 0;
    }
    g_object_unref(box);
    JSCValue *tag = jsc_value_object_get_property(found, "tag"), *text = jsc_value_object_get_property(found, "text");
    char *tag_text = jsc_value_is_string(tag) ? jsc_value_to_string(tag) : g_strdup("element");
    char *words = jsc_value_is_string(text) ? jsc_value_to_string(text) : g_strdup("");
    if (g_utf8_strlen(words, -1) > 40) {
        char *cut = g_utf8_substring(words, 0, 39);
        g_free(words);
        words = g_strconcat(cut, "…", NULL);
        g_free(cut);
    }
    g_free(r->shot_what);
    r->shot_what = *words ? g_strdup_printf("%s \"%s\"", tag_text, words) : g_strdup(tag_text);
    g_free(tag_text);
    g_free(words);
    g_object_unref(tag);
    g_object_unref(text);
    g_object_unref(found);
}

/* the element at a point (click X Y --screenshot): JSON as target.js gives, or null */
static const char SHOT_POINT_JS[] =
    "const s = window.devicePixelRatio || 1, hit = document.elementFromPoint(x / s, y / s);"
    "if (!hit) return null;"
    "const el = hit.closest('a, button, input, select, textarea, label, summary, option, [role=button], [role=link],"
    " [role=tab], [role=menuitem], [role=checkbox], [role=option], [onclick], [contenteditable]') || hit;"
    "if (el === document.body || el === document.documentElement) return null;"
    "const r = el.getBoundingClientRect();"
    "return JSON.stringify({ box: [r.left, r.top, r.width, r.height].map(v => Math.round(v * s)), tag: el.tagName.toLowerCase(),"
    "  text: (el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '')"
    "    .replace(/\\s+/g, ' ').trim().slice(0, 80) });";

/* the caption: "click (512, 300) · button "Greet"" */
static void shot_caption(Request *r, const char *action)
{
    g_free(r->shot_label);
    if (r->shot_what)
        r->shot_label = g_strdup_printf("%s · %s", action, r->shot_what);
    else
        r->shot_label = g_strdup(action);
}

/* click / hover / type / select: find the target (target.js), then act on its coordinates */
typedef enum { ACT_CLICK, ACT_DOUBLE_CLICK, ACT_RIGHT_CLICK, ACT_HOVER, ACT_TYPE, ACT_SELECT } Action;

typedef struct {
    Action action;
} TargetAction;

static void act_at(Request *r, Action action, double x, double y, const char *what);

static Action action_of(Request *r)
{
    const char *cmd = r->argv[0];
    if (!strcmp(cmd, "dblclick"))
        return ACT_DOUBLE_CLICK;
    if (!strcmp(cmd, "rightclick"))
        return ACT_RIGHT_CLICK;
    if (!strcmp(cmd, "hover"))
        return ACT_HOVER;
    if (!strcmp(cmd, "type"))
        return ACT_TYPE;
    if (!strcmp(cmd, "select"))
        return ACT_SELECT;
    return ACT_CLICK;
}

static void target_found(Request *r, JSCValue *value, GError *error)
{
    if (error) {
        replyf(r, ST_FAILED, "JavaScript error: %s\n", error->message);
        return;
    }
    if (!value || jsc_value_is_null(value) || jsc_value_is_undefined(value)) {
        const char *target = r->argv[0][0] == 't' ? r->argv[2] : r->argv[1];
        replyf(r, ST_NOT_FOUND, "nothing matches \"%s\" (a number from `web read`, a CSS selector or a link's or button's text)\n", target);
        return;
    }
    char *json = jsc_value_to_string(value);
    /* the fields we need, read without a JSON library: "x":N,"y":N,"tag":"...","text":"..." */
    double x = 0, y = 0;
    const char *p = strstr(json, "\"x\":");
    if (p)
        x = g_ascii_strtod(p + 4, NULL);
    p = strstr(json, "\"y\":");
    if (p)
        y = g_ascii_strtod(p + 4, NULL);
    double box[4];
    p = strstr(json, "\"box\":[");
    if (p && sscanf(p + 7, "%lf,%lf,%lf,%lf", &box[0], &box[1], &box[2], &box[3]) == 4)
        r->target_size = MIN(box[2], box[3]);
    if (r->shot_file)
        shot_element(r, value, json);
    act_at(r, action_of(r), x, y, json);
    g_free(json);
}

/* click X Y --screenshot: what is there, for the picture */
static void point_found(Request *r, JSCValue *value, GError *error)
{
    if (!error && value && jsc_value_is_string(value)) {
        char *json = jsc_value_to_string(value);
        shot_element(r, value, json);
        g_free(json);
    }
    char *what = g_strdup_printf("(%g, %g)", r->x, r->y);
    act_at(r, action_of(r), r->x, r->y, what);
    g_free(what);
}

static void select_done(Request *r, JSCValue *value, GError *error)
{
    if (error) {
        replyf(r, ST_FAILED, "JavaScript error: %s\n", error->message);
        return;
    }
    char *text = js_value_text(value);
    if (!strcmp(text, "null"))
        replyf(r, ST_NOT_FOUND, "no option \"%s\"\n", r->argv[2]);
    else {
        char *action = g_strdup_printf("select %s", text);
        shot_caption(r, action);
        g_free(action);
        replyf(r, ST_OK, "selected %s\n", text);
    }
    g_free(text);
}

static void hovered(Request *r)
{
    r->acted = TRUE;
    r->shot_when = "after the hover";
    replyf(r, ST_OK, "hovering %s\n", r->what);
}

/* select: the click on the <select> went in; the option is chosen as the page's own script would */
static void select_clicked(Request *r)
{
    r->acted = TRUE;
    r->shot_when = "after selecting";
    call_js(r, "const t = /^\\d+$/.test(target) ? (globalThis.__wpeRefs || [])[+target] : document.querySelector(target);"
               "if (!t || t.tagName !== 'SELECT') return null;"
               "const want = value.trim().toLowerCase();"
               "const o = Array.from(t.options).find(o => o.value === value || o.text.trim().toLowerCase() === want)"
               "  || Array.from(t.options).find(o => o.text.trim().toLowerCase().includes(want));"
               "if (!o) return null; t.value = o.value;"
               "t.dispatchEvent(new Event('input', { bubbles: true })); t.dispatchEvent(new Event('change', { bubbles: true }));"
               "return o.text.trim();",
            args_dict("target", g_variant_new_string(r->argv[1]), "value", g_variant_new_string(r->argv[2]), NULL), select_done);
}

/* the pointer to the request's point (r->x, r->y) and `count` presses of `button` there */
static void send_press(Request *r, guint button, int count)
{
    Plan *plan = plan_new();
    gboolean moved = !(plan->have_pointer && plan->px == round(r->x) && plan->py == round(r->y));
    plan_pointer_to(plan, r->x, r->y, r->target_size);
    plan_press(plan, button, count, moved);
    play(r, plan, input_sent);
}

static void send_click(Request *r)
{
    send_press(r, 1, 1);
}

static void send_double_click(Request *r)
{
    send_press(r, 1, 2);
}

static void send_right_click(Request *r)
{
    send_press(r, 3, 1);
}

static void wait_after_click(Request *r)
{
    /* a click that navigates answers when the new page is there */
    wait_for_load(r, timeout_arg(r, 3, 30000), 800);
}

/* click --screenshot: the picture has been taken, the click goes in */
static void click_after_shot(Request *r)
{
    input_send(r, r->send, wait_after_click);
}

/* the text of `type` (argv[1]) as key events */
static void send_text(Request *r)
{
    Plan *plan = plan_new();
    /* a moment between the click that focused the field and the first key */
    plan->t += r->have_point ? human(250, 1.6, 120, 700) : 0;
    plan_text(plan, r->argv[1]);
    play(r, plan, input_sent);
}

static void typed(Request *r)
{
    long n = g_utf8_strlen(r->argv[1], -1);
    if (r->what)
        replyf(r, ST_OK, "typed %ld characters into %s\n", n, r->what);
    else
        replyf(r, ST_OK, "typed %ld characters\n", n);
}

static void send_typed_text(Request *r)
{
    input_send(r, send_text, typed);
}

static void act_at(Request *r, Action action, double x, double y, const char *what)
{
    focus_view();
    /* for --screenshot: where it acts, and the caption ("click (512, 300) · button "Greet"") */
    r->x = x;
    r->y = y;
    r->have_point = TRUE;
    if (r->shot_file) {
        char *caption = action == ACT_TYPE ? g_strdup_printf("type %ld characters", g_utf8_strlen(r->argv[1], -1))
                                           : g_strdup_printf("%s (%g, %g)", r->argv[0], x, y);
        shot_caption(r, caption);
        g_free(caption);
    }
    switch (action) {
    case ACT_HOVER: {
        r->what = g_strdup(what);
        Plan *plan = plan_new();
        plan_pointer_to(plan, x, y, r->target_size);
        play(r, plan, hovered);
        return;
    }
    case ACT_SELECT: {
        Plan *plan = plan_new();
        gboolean moved = !(plan->have_pointer && plan->px == round(x) && plan->py == round(y));
        plan_pointer_to(plan, x, y, r->target_size);
        plan_press(plan, 1, 1, moved);
        play(r, plan, select_clicked);
        return;
    }
    case ACT_TYPE:
        /* the click first, and the keys once it has focused the field */
        r->what = g_strdup(what);
        r->shot_when = "after typing";
        input_send(r, send_click, send_typed_text);
        return;
    default:
        break;
    }
    r->done_text = g_strdup_printf("clicked %s\n", what);
    r->send = action == ACT_RIGHT_CLICK ? send_right_click : action == ACT_DOUBLE_CLICK ? send_double_click : send_click;
    if (r->shot_file) {
        /* the picture first: the page as the click meets it (a page it loads is another) */
        r->shot_when = action == ACT_RIGHT_CLICK ? "before the right click" : action == ACT_DOUBLE_CLICK ? "before the double click"
                                                                                                        : "before the click";
        shot_take(r, click_after_shot);
        return;
    }
    input_send(r, r->send, wait_after_click);
}

static gboolean parse_point(Request *r, int index, double *x, double *y)
{
    if (r->argc <= index + 1)
        return FALSE;
    char *end1, *end2;
    *x = g_ascii_strtod(r->argv[index], &end1);
    *y = g_ascii_strtod(r->argv[index + 1], &end2);
    return *end1 == 0 && *end2 == 0 && end1 != r->argv[index] && end2 != r->argv[index + 1];
}

static void cmd_target(Request *r)
{
    Action action = action_of(r);
    int target_index = action == ACT_TYPE ? 2 : 1;
    if (action == ACT_TYPE && r->argc == 2) {
        /* into whatever has the focus */
        focus_view();
        if (r->shot_file) {
            r->shot_label = g_strdup_printf("type %ld characters", g_utf8_strlen(r->argv[1], -1));
            r->shot_when = "after typing";
            r->want_focus = TRUE;
        }
        input_send(r, send_text, typed);
        return;
    }
    if (r->argc <= target_index || (action == ACT_SELECT && r->argc < 3)) {
        reply(r, ST_USAGE, action == ACT_TYPE ? "usage: type TEXT [TARGET]\n"
                           : action == ACT_SELECT ? "usage: select TARGET VALUE\n"
                           : "usage: click|dblclick|rightclick|hover TARGET | X Y\n");
        return;
    }
    double x, y;
    if (action != ACT_TYPE && action != ACT_SELECT && parse_point(r, 1, &x, &y)) {
        if (r->shot_file) {
            /* what is there first, for the picture */
            r->x = x;
            r->y = y;
            call_js(r, SHOT_POINT_JS, args_dict("x", g_variant_new_double(x), "y", g_variant_new_double(y), NULL), point_found);
            return;
        }
        char *what = g_strdup_printf("(%g, %g)", x, y);
        act_at(r, action, x, y, what);
        g_free(what);
        return;
    }
    char *body = g_strconcat("return (", target_js, ")(target, !!spread);", NULL);
    call_js(r, body, args_dict("target", g_variant_new_string(r->argv[target_index]),
                               "spread", g_variant_new_int32(browser.humanize && action != ACT_TYPE && action != ACT_SELECT), NULL),
            target_found);
    g_free(body);
}

static void send_keys(Request *r)
{
    Plan *plan = plan_new();
    for (int i = 1; i < r->argc; i++)
        key_spec(r->argv[i], plan);
    play(r, plan, input_sent);
}

static void wait_after_keys(Request *r)
{
    wait_for_load(r, 30000, 600); /* Enter in a form may navigate */
}

static void cmd_key(Request *r)
{
    if (r->argc < 2) {
        reply(r, ST_USAGE, "usage: key KEY... (Enter, Tab, Escape, ArrowDown, ctrl+a, shift+Tab, ...)\n");
        return;
    }
    for (int i = 1; i < r->argc; i++) {
        if (!key_spec(r->argv[i], NULL)) {
            replyf(r, ST_USAGE, "unknown key \"%s\"\n", r->argv[i]);
            return;
        }
    }
    focus_view();
    r->done_text = g_strdup_printf("pressed %d key%s\n", r->argc - 1, r->argc > 2 ? "s" : "");
    if (r->shot_file) {
        char *keys = g_strjoinv(" ", r->argv + 1);
        r->shot_label = g_strdup_printf("key %s", keys);
        g_free(keys);
        r->shot_when = "after the keys";
        r->want_focus = TRUE;
    }
    input_send(r, send_keys, wait_after_keys);
}

/* scroll --screenshot: until the page has stopped moving (or has not moved: 600 ms), from x0, y0;
 * "X Y" where it is */
static const char SCROLL_SETTLE_JS[] =
    "const start = performance.now(), at = () => `${Math.round(scrollX)} ${Math.round(scrollY)}`, from = `${x0} ${y0}`;"
    "let last = at(), since = start;"
    "while (performance.now() - start < 2000) {"
    "  await new Promise(done => setTimeout(done, 40));"
    "  const now = at(), t = performance.now();"
    "  if (now !== last) { last = now; since = t; }"
    "  else if (now !== from ? t - since >= 150 : t - start >= 600) break;"
    "}"
    "return last;";

static void scroll_reply(Request *r, const char *how, int notches, const char *to)
{
    double x = 0, y = 0;
    if (to && sscanf(to, "%lf %lf", &x, &y) == 2) {
        gboolean across = !strcmp(how, "left") || !strcmp(how, "right");
        double from = across ? r->scroll_from[0] : r->scroll_from[1], now = across ? x : y;
        char *caption = g_strdup_printf("scroll %s %d · %s %g → %g", how, notches, across ? "x" : "y", from, now);
        shot_caption(r, caption);
        g_free(caption);
        replyf(r, ST_OK, "scrolled %s %d (the page's %s from %g to %g)\n", how, notches, across ? "x" : "y", from, now);
    } else
        replyf(r, ST_OK, "scrolled %s %d\n", how, notches);
}

static int scroll_notches(Request *r)
{
    return r->argc > 2 && atoi(r->argv[2]) > 0 ? atoi(r->argv[2]) : 3;
}

static void scroll_settled(Request *r, JSCValue *value, GError *error)
{
    char *to = !error && value && jsc_value_is_string(value) ? jsc_value_to_string(value) : NULL;
    scroll_reply(r, r->argc > 1 ? r->argv[1] : "down", scroll_notches(r), to);
    g_free(to);
}

/* The wheel turned dx, dy notches where the pointer is (in the view; else its middle): with
 * humanize, a notch per event in strokes of a few, as a hand turns a notched wheel; then done(r) */
static void scroll_send(Request *r, double dx, double dy, void (*done)(Request *r))
{
    int width = 0, height = 0;
    wpe_toplevel_get_size(wpe_view_get_toplevel(wpe_view()), &width, &height);
    Plan *plan = plan_new();
    double x = width / 2.0, y = height / 2.0;
    if (plan->have_pointer && plan->px >= 0 && plan->py >= 0 && plan->px < width && plan->py < height) {
        x = plan->px;
        y = plan->py;
    } else if (browser.humanize)
        plan_pointer_to(plan, x + (g_rand_double(browser.rand) - 0.5) * width * 0.3,
                        y + (g_rand_double(browser.rand) - 0.5) * height * 0.3, 200);
    if (browser.humanize) {
        x = plan->px;
        y = plan->py;
        int notches = (int)fabs(dx != 0 ? dx : dy);
        double sx = dx > 0 ? 1 : dx < 0 ? -1 : 0, sy = dy > 0 ? 1 : dy < 0 ? -1 : 0;
        plan->t += human(120, 1.6, 40, 400);
        /* a short scroll opens with a lone notch; then strokes of 1-5 (2 most often) */
        for (int done_notches = 0, stroke = 1; done_notches < notches; stroke = 1 + (int)(g_rand_double(browser.rand) * 4.2)) {
            if (stroke > notches - done_notches)
                stroke = notches - done_notches;
            for (int i = 0; i < stroke; i++) {
                PlannedEvent *e = plan_add(plan, EV_WHEEL, plan->t);
                e->x = x;
                e->y = y;
                e->dx = sx;
                e->dy = sy;
                plan->t += human(16, 1.3, 8, 30);
            }
            done_notches += stroke;
            if (done_notches < notches)
                plan->t += human(60, 1.3, 40, 120);
        }
    } else {
        PlannedEvent *e = plan_add(plan, EV_WHEEL, 0);
        e->x = x;
        e->y = y;
        e->dx = dx;
        e->dy = dy;
    }
    /* --screenshot: the arrow in the view's middle (the wheel turned wherever the pointer was) */
    r->acted = TRUE;
    r->x = width / 2.0;
    r->y = height / 2.0;
    r->have_point = TRUE;
    play(r, plan, done);
}

static void scrolled(Request *r)
{
    replyf(r, ST_OK, "scrolled %s %d\n", r->argc > 1 ? r->argv[1] : "down", scroll_notches(r));
}

static void scroll_settle(Request *r)
{
    call_js(r, SCROLL_SETTLE_JS, args_dict("x0", g_variant_new_double(r->scroll_from[0]), "y0", g_variant_new_double(r->scroll_from[1]), NULL),
            scroll_settled);
}

/* scroll --screenshot: where the page was, then the wheel, then wait for it to settle */
static void scroll_from_read(Request *r, JSCValue *value, GError *error)
{
    char *from = !error && value && jsc_value_is_string(value) ? jsc_value_to_string(value) : NULL;
    if (!from || sscanf(from, "%lf %lf", &r->scroll_from[0], &r->scroll_from[1]) != 2)
        r->scroll_from[0] = r->scroll_from[1] = 0;
    g_free(from);
    /* the arrow points the way the page goes; the wheel's notches the other way (+: up, left) */
    int notches = scroll_notches(r);
    scroll_send(r, -r->arrow_dx * notches, -r->arrow_dy * notches, scroll_settle);
}

static void cmd_scroll(Request *r)
{
    const char *how = r->argc > 1 ? r->argv[1] : "down";
    if (!strcmp(how, "top") || !strcmp(how, "bottom")) {
        r->acted = TRUE;
        r->shot_when = "after the scroll";
        r->shot_label = g_strdup_printf("scroll %s", how);
        call_js(r, !strcmp(how, "top") ? "window.scrollTo(0, 0); return `at ${scrollY}`;"
                                       : "window.scrollTo(0, document.documentElement.scrollHeight); return `at ${scrollY}`;",
                NULL, reply_js_text);
        return;
    }
    int notches = scroll_notches(r);
    double dx = 0, dy = 0;
    if (!strcmp(how, "down")) dy = -notches;
    else if (!strcmp(how, "up")) dy = notches;
    else if (!strcmp(how, "left")) dx = notches;
    else if (!strcmp(how, "right")) dx = -notches;
    else {
        reply(r, ST_USAGE, "usage: scroll [down|up|left|right [NOTCHES]] | top | bottom\n");
        return;
    }
    if (r->shot_file) {
        /* the arrow: a unit the way the page goes, down for "down" */
        r->arrow_dx = dx > 0 ? -1 : dx < 0 ? 1 : 0;
        r->arrow_dy = dy > 0 ? -1 : dy < 0 ? 1 : 0;
        r->shot_when = "after the scroll";
        call_js(r, "return `${Math.round(scrollX)} ${Math.round(scrollY)}`;", NULL, scroll_from_read);
        return;
    }
    scroll_send(r, dx, dy, scrolled);
}

/* ── robot checks (`web challenge`) ─────────────────────────────────────────────────────────────
 * challenge.js recognizes the check on the page; `solve` waits for one that passes by itself
 * (Cloudflare's), clicks a checkbox or presses and holds a button as a person does (once it has
 * been there a moment, at most three times, some seconds apart), and hands a picture puzzle to the
 * agent. A check that passes usually loads the page it stood in front of. */

#define CHECK_POLL_MS 1000

static void check_step(Request *r);

static gboolean check_step_again(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    check_step(r);
    return G_SOURCE_REMOVE;
}

static void check_later(Request *r, int ms)
{
    r->poll_id = g_timeout_add(ms, check_step_again, r);
}

static void check_acted(Request *r)
{
    r->check_last_act = g_get_monotonic_time();
    check_later(r, 2500);
}

static const char *check_advice(const char *action)
{
    return !strcmp(action, "wait") ? "it passes by itself in a few seconds: web challenge solve waits for it"
         : !strcmp(action, "click") ? "web challenge solve clicks its box as a person would"
         : !strcmp(action, "hold") ? "web challenge solve presses and holds it as a person would"
         : !strcmp(action, "agent") ? "a puzzle for whoever looks: web screenshot, then web click X Y on what it asks for"
         : "nothing on the page gets past it: later, another site, or another machine (web identity new)";
}

/* a press held `hold` ms at the pointer, the hand not quite still */
static void plan_hold(Plan *plan, double hold)
{
    plan->t += human(80, 1.6, 30, 250);
    PlannedEvent *down = plan_add(plan, EV_DOWN, plan->t);
    down->x = plan->px;
    down->y = plan->py;
    down->button = 1;
    down->count = 1;
    double end = plan->t + hold;
    for (double t = plan->t + human(300, 1.6, 120, 900); t < end - 50; t += human(260, 1.7, 90, 800)) {
        PlannedEvent *e = plan_add(plan, EV_MOVE, t);
        e->x = plan->px + (g_rand_double(browser.rand) < 0.5 ? -1 : 1) * (g_rand_double(browser.rand) < 0.7 ? 1 : 2);
        e->y = plan->py + (g_rand_double(browser.rand) < 0.5 ? -1 : 1) * (g_rand_double(browser.rand) < 0.8 ? 0 : 1);
    }
    PlannedEvent *up = plan_add(plan, EV_UP, end);
    up->x = plan->px;
    up->y = plan->py;
    up->button = 1;
    plan->t = end;
}

/* An empty checkbox around (x, y) in the view's picture, as a person sees it before clicking: a
 * square 14-32 px across whose frame is darker than its light inside, the same on several rows and
 * columns (a spinner's dots, a tick, a blank are not). *cx, *cy: its middle (view coordinates). */
static gboolean find_checkbox(WebKitImage *image, double x, double y, double *cx, double *cy)
{
    int w = webkit_image_get_width(image), h = webkit_image_get_height(image);
    guint stride = webkit_image_get_stride(image);
    gsize size = 0;
    const guint8 *px = g_bytes_get_data(webkit_image_as_bytes(image), &size);
    int view_w = wpe_view_get_width(wpe_view()), view_h = wpe_view_get_height(wpe_view());
    double sx = view_w > 0 ? (double)w / view_w : 1, sy = view_h > 0 ? (double)h / view_h : 1;
    if (!px || w <= 0 || h <= 0 || size < (gsize)stride * h)
        return FALSE;
#define LUMA(X, Y) ((X) < 0 || (Y) < 0 || (X) >= w || (Y) >= h ? -1 \
    : (px[(gsize)(Y) * stride + (X) * 4 + 2] * 3 + px[(gsize)(Y) * stride + (X) * 4 + 1] * 6 + px[(gsize)(Y) * stride + (X) * 4]) / 10)
#define DARK(X, Y) (LUMA(X, Y) >= 0 && LUMA(X, Y) < inside - 50)
    /* the box may be off the point a little: start from a few points around it */
    for (int oy = -8; oy <= 8; oy += 4) {
        for (int ox = -16; ox <= 8; ox += 4) {
            int x0 = (int)(x * sx) + ox, y0 = (int)(y * sy) + oy, inside = LUMA(x0, y0);
            if (inside < 190)
                continue;
            int left = -1, right = -1, top = -1, bottom = -1;
            for (int d = 1; d <= 34; d++) {
                if (left < 0 && DARK(x0 - d, y0)) left = x0 - d;
                if (right < 0 && DARK(x0 + d, y0)) right = x0 + d;
                if (top < 0 && DARK(x0, y0 - d)) top = y0 - d;
                if (bottom < 0 && DARK(x0, y0 + d)) bottom = y0 + d;
            }
            int bw = right - left, bh = bottom - top;
            if (left < 0 || right < 0 || top < 0 || bottom < 0 || bw < 14 * sx || bw > 32 * sx || abs(bw - bh) > 3 * sx)
                continue;
            /* a frame: the same edges on other rows and columns, the inside light throughout */
            gboolean frame = TRUE;
            int mx = (left + right) / 2, my = (top + bottom) / 2;
            for (int k = -bh / 3; k <= bh / 3 && frame; k += MAX(1, bh / 6))
                frame = DARK(left, my + k) && DARK(right, my + k) && DARK(mx + k, top) && DARK(mx + k, bottom)
                     && LUMA(mx + k / 2, my) >= inside - 25 && LUMA(mx, my + k / 2) >= inside - 25;
            if (!frame)
                continue;
            *cx = mx / sx;
            *cy = my / sy;
            return TRUE;
        }
    }
#undef DARK
#undef LUMA
    return FALSE;
}

static void check_click(Request *r, double x, double y)
{
    /* as a person: a moment to look, the pointer over, then the click */
    r->check_acts++;
    r->x = x;
    r->y = y;
    r->have_point = TRUE;
    focus_view();
    Plan *plan = plan_new();
    gboolean moved = !(plan->have_pointer && plan->px == round(x) && plan->py == round(y));
    plan->t += human(500, 1.8, 200, 1600);
    plan_pointer_to(plan, x, y, 24);
    plan_press(plan, 1, 1, moved);
    play(r, plan, check_acted);
}

/* the picture of the view: a checkbox is clicked once it shows (not while its widget is still at
 * work, which takes no clicks), else looked for again a moment later */
static void check_looked(GObject *source, GAsyncResult *result, gpointer data)
{
    ShotCall *call = data;
    Request *r = call->r;
    WebKitImage *image = webkit_web_view_get_snapshot_finish(WEBKIT_WEB_VIEW(source), result, NULL);
    g_free(call);
    if (r) {
        r->shot = NULL;
        double x, y;
        if (image && find_checkbox(image, r->x, r->y, &x, &y))
            check_click(r, x, y);
        else
            check_later(r, CHECK_POLL_MS);
    }
    if (image)
        g_object_unref(image);
}

static void check_seen(Request *r, JSCValue *value, GError *error)
{
    gboolean solve = r->argc > 1 && (!strcmp(r->argv[1], "solve") || !strcmp(r->argv[1], "pass"));
    char *line = !error && value && jsc_value_is_string(value) ? jsc_value_to_string(value) : NULL;
    char **f = line ? g_strsplit(line, "\t", 6) : NULL;
    gint64 now = g_get_monotonic_time();
    const char *uri = webkit_web_view_get_uri(browser.view), *title = webkit_web_view_get_title(browser.view);
    if (!f || g_strv_length(f) != 6) {
        if (solve && (browser.loading || error) && now < r->deadline)
            check_later(r, CHECK_POLL_MS); /* the page it let through is loading */
        else if (r->check_seen)
            replyf(r, ST_OK, "passed (%s)\n%s\n%s\n", r->check_kind, uri ? uri : "", title ? title : "");
        else
            replyf(r, ST_OK, "no robot check on this page (%s)\n", uri ? uri : "about:blank");
        g_strfreev(f);
        g_free(line);
        return;
    }
    const char *kind = f[0], *action = f[1], *what = f[5];
    double x = g_ascii_strtod(f[2], NULL), y = g_ascii_strtod(f[3], NULL), hold = g_ascii_strtod(f[4], NULL);
    if (g_strcmp0(kind, r->check_kind)) {
        g_free(r->check_kind);
        r->check_kind = g_strdup(kind);
        r->check_since = now;
        r->check_acts = 0;
    }
    g_free(r->check_what);
    r->check_what = g_strdup(what);
    r->check_seen = TRUE;
    gboolean acts = !strcmp(action, "click") || !strcmp(action, "hold");
    if (!solve) {
        char *at = acts ? g_strdup_printf(" (at %g, %g)", x, y) : g_strdup("");
        replyf(r, ST_OK, "%s: %s\n  %s%s\n", kind, what, check_advice(action), at);
        g_free(at);
    } else if (!strcmp(action, "blocked") || !strcmp(action, "agent"))
        replyf(r, ST_FAILED, "%s: %s\n  %s\n", kind, what, check_advice(action));
    else if (now >= r->deadline) {
        char *tried = r->check_acts ? g_strdup_printf(" and %d %s", r->check_acts, !strcmp(action, "hold") ? "holds" : "clicks")
                                    : g_strdup("");
        replyf(r, ST_TIMEOUT, "still there after %d s%s: %s: %s\n", timeout_arg(r, 2, 90000) / 1000, tried, kind, what);
        g_free(tried);
    } else if (acts && !browser.loading && r->check_acts < 3 && now - r->check_since >= 2500000 && now - r->check_last_act >= 8000000) {
        if (!strcmp(action, "hold")) {
            /* as a person: a moment to look, the pointer over, then the hold */
            r->check_acts++;
            r->x = x;
            r->y = y;
            r->have_point = TRUE;
            focus_view();
            Plan *plan = plan_new();
            plan->t += human(500, 1.8, 200, 1600);
            plan_pointer_to(plan, x, y, 40);
            plan_hold(plan, hold);
            play(r, plan, check_acted);
        } else {
            /* a checkbox: once the picture shows it (check_looked) */
            r->x = x;
            r->y = y;
            r->shot = g_new0(ShotCall, 1);
            r->shot->r = r;
            webkit_web_view_get_snapshot(browser.view, WEBKIT_SNAPSHOT_REGION_VISIBLE, WEBKIT_SNAPSHOT_OPTIONS_NONE, NULL,
                                         check_looked, r->shot);
        }
    } else
        check_later(r, CHECK_POLL_MS);
    g_strfreev(f);
    g_free(line);
}

static void check_step(Request *r)
{
    char *body = g_strconcat("return (", challenge_js, ")();", NULL);
    call_js(r, body, NULL, check_seen);
    g_free(body);
}

/* challenge [status] | challenge solve [TIMEOUT] */
static void cmd_challenge(Request *r)
{
    if (r->argc > 1 && strcmp(r->argv[1], "solve") && strcmp(r->argv[1], "pass") && strcmp(r->argv[1], "status")) {
        reply(r, ST_USAGE, "usage: challenge [status] | challenge solve [TIMEOUT]\n");
        return;
    }
    r->deadline = g_get_monotonic_time() + (gint64)timeout_arg(r, 2, 90000) * 1000;
    check_step(r);
}

/* ── web search: one search engine after the other, past those that turn the browser away ────── */

typedef struct {
    const char *name, *format; /* format: the query, escaped, at %s */
} Engine;

static const Engine engines[] = {
    { "duckduckgo", "https://html.duckduckgo.com/html/?q=%s" },
    { "bing", "https://www.bing.com/search?q=%s" },
    { "google", "https://www.google.com/search?q=%s" },
    { "brave", "https://search.brave.com/search?q=%s" },
    { "naver", "https://search.naver.com/search.naver?query=%s" },
};

static const Engine *engine_named(const char *name)
{
    for (size_t i = 0; i < G_N_ELEMENTS(engines); i++) {
        if (!g_ascii_strcasecmp(name, engines[i].name) || (!g_ascii_strcasecmp(name, "ddg") && i == 0))
            return &engines[i];
    }
    return NULL;
}

static void search_next(Request *r);

/* a moment after the results page loaded: a page that moved on by itself (Google's first answer
 * runs a script that loads the results) is waited for, else the page is read again */
static gboolean search_settle(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    if (browser.loading || browser.load_generation > r->settle_generation) {
        r->generation_set = TRUE;
        r->load_generation = r->settle_generation;
        wait_for_load(r, 60000, -1);
    } else
        reply_loaded(r);
    return G_SOURCE_REMOVE;
}

static void search_loaded(Request *r, const char *title, const char *check)
{
    if (!r->settled) {
        r->settled = TRUE;
        r->settle_generation = browser.load_generation;
        r->poll_id = g_timeout_add(1500, search_settle, r);
        return;
    }
    const char *uri = webkit_web_view_get_uri(browser.view);
    char **f = check ? g_strsplit(check, "\t", 6) : NULL;
    gboolean turned_away = f && g_strv_length(f) == 6 && (!strcmp(f[1], "blocked") || !strcmp(f[1], "agent"));
    if (turned_away && r->engines[r->engine + 1]) {
        g_string_append_printf(r->skipped, "  %s: %s (%s)\n", r->engines[r->engine], f[0], f[5]);
        g_strfreev(f);
        r->engine++;
        search_next(r);
        return;
    }
    char *note = check_note(check);
    replyf(r, ST_OK, "search: %s \"%s\"%s\n%s%s\n%s\n%s%s", r->engines[r->engine], r->query,
           r->skipped->len ? " (the engines before it turned the browser away:)" : "", r->skipped->str,
           uri ? uri : "", title, note, paused_note());
    g_free(note);
    g_strfreev(f);
}

static void search_next(Request *r)
{
    const Engine *engine = engine_named(r->engines[r->engine]);
    char *query = g_uri_escape_string(r->query, NULL, FALSE);
    char *uri = g_strdup_printf(engine->format, query);
    webkit_web_view_load_uri(browser.view, uri);
    g_free(uri);
    g_free(query);
    r->acted = TRUE;
    r->shot_when = "after the load";
    r->generation_set = FALSE;
    r->settled = FALSE;
    wait_for_load(r, 60000, 15000);
}

/* search WORDS... [--engine NAME[,NAME...]]: the results page of the first engine that lets the
 * browser in (duckduckgo, bing, google, brave by default); the ones that showed a robot check or
 * refused are said */
static void cmd_search(Request *r)
{
    GString *words = g_string_new(NULL);
    char *list = NULL;
    for (int i = 1; i < r->argc; i++) {
        if (!strcmp(r->argv[i], "--engine") && i + 1 < r->argc) {
            g_free(list);
            list = g_strdup(r->argv[++i]);
        } else
            g_string_append_printf(words, "%s%s", words->len ? " " : "", r->argv[i]);
    }
    if (!words->len) {
        g_string_free(words, TRUE);
        g_free(list);
        reply(r, ST_USAGE, "usage: search WORDS... [--engine duckduckgo|bing|google|brave|naver[,...]]\n");
        return;
    }
    r->engines = g_strsplit(list ? list : "duckduckgo,bing,google,brave", ",", -1);
    g_free(list);
    for (char **e = r->engines; *e; e++) {
        if (!engine_named(*e)) {
            replyf(r, ST_USAGE, "no search engine \"%s\" (duckduckgo, bing, google, brave, naver)\n", *e);
            g_string_free(words, TRUE);
            return;
        }
    }
    r->query = g_string_free(words, FALSE);
    r->skipped = g_string_new(NULL);
    r->loaded = search_loaded;
    if (browser.devtools_reconnect)
        devtools_connect_again();
    search_next(r);
}

/* wait load | text STRING | selector CSS | gone CSS [TIMEOUT] */
static void poll_check(Request *r);

static void poll_result(Request *r, JSCValue *value, GError *error)
{
    if (!error && value && jsc_value_to_boolean(value)) {
        char *state = page_state();
        replyf(r, ST_OK, "found\n%s", state);
        g_free(state);
        return;
    }
    if (browser.paused_count) {
        reply_paused(r); /* what it waits for may wait for them */
        return;
    }
    if (g_get_monotonic_time() >= r->deadline) {
        replyf(r, ST_TIMEOUT, "not found in time: %s %s\n", r->poll_kind, r->poll_arg);
        return;
    }
    r->poll_id = g_timeout_add(300, (GSourceFunc)(void (*)(void))poll_check, r);
}

static void poll_check(Request *r)
{
    r->poll_id = 0;
    const char *body = !strcmp(r->poll_kind, "text") ? "return !!document.body && document.body.innerText.includes(arg);"
                     : !strcmp(r->poll_kind, "gone") ? "return !document.querySelector(arg);"
                     : "return !!document.querySelector(arg);";
    call_js(r, body, args_dict("arg", g_variant_new_string(r->poll_arg), NULL), poll_result);
}

static gboolean poll_check_source(gpointer data)
{
    poll_check(data);
    return G_SOURCE_REMOVE;
}

static void cmd_wait(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "load";
    r->shot_when = "as the wait ended";
    if (!strcmp(what, "load")) {
        r->acted = TRUE;
        if (!browser.loading) {
            char *state = page_state();
            replyf(r, ST_OK, "loaded\n%s%s", state, paused_note());
            g_free(state);
            return;
        }
        r->pause_base = 0; /* the load waits for those paused already */
        r->load_generation = browser.load_generation - 1; /* the load going on is the one it waits for */
        r->generation_set = TRUE;
        wait_for_load(r, timeout_arg(r, 2, 60000), -1);
        return;
    }
    if ((!strcmp(what, "text") || !strcmp(what, "selector") || !strcmp(what, "gone")) && r->argc > 2) {
        r->acted = TRUE;
        r->poll_kind = g_strdup(what);
        r->poll_arg = g_strdup(r->argv[2]);
        r->deadline = g_get_monotonic_time() + (gint64)timeout_arg(r, 3, 30000) * 1000;
        r->poll_id = g_idle_add(poll_check_source, r);
        return;
    }
    reply(r, ST_USAGE, "usage: wait [load | text STRING | selector CSS | gone CSS] [TIMEOUT_SECONDS]\n");
}

static void cmd_resize(Request *r)
{
    int w, h;
    if (r->argc < 2 || sscanf(r->argv[1], "%dx%d", &w, &h) != 2 || w <= 0 || h <= 0) {
        reply(r, ST_USAGE, "usage: resize WIDTHxHEIGHT\n");
        return;
    }
    wpe_toplevel_resize(wpe_view_get_toplevel(wpe_view()), w, h);
    browser.view_width = w;
    browser.view_height = h;
    settings_changed(); /* a screen that holds the window, the window's size around the page */
    replyf(r, ST_OK, "asked for %dx%d\n", w, h);
}

/* set OPTION...: the browser's setting options (--user-agent, --header, --header-for, --lang,
 * --inject, --devtools, --proxy...), as `web` gives them to a browser that is running already */
static void cmd_set(Request *r)
{
    char *proxy_error = NULL;
    if (!proxy_options(r->argc, r->argv, &proxy_error)) {
        settings_changed();
        replyf(r, ST_USAGE, "%s\n", proxy_error);
        g_free(proxy_error);
        return;
    }
    for (int i = 1; i < r->argc;) {
        char *error = NULL;
        int n = setting_option(r->argc, r->argv, i, &error);
        if (n <= 0) {
            settings_changed(); /* the ones before it */
            if (n == 0)
                replyf(r, ST_USAGE, "unknown setting %s (--user-agent, --header, --header-for, --lang, --timezone, --humanize, --inject, --devtools,"
                       " --proxy, --proxy-bypass, --proxy-ca)\n", r->argv[i]);
            else
                replyf(r, ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
        i += n;
    }
    settings_changed();
    reply(r, ST_OK, "");
}

static void reply_setting(Request *r, char *text, gboolean changed)
{
    replyf(r, ST_OK, "%s%s", text, changed ? "(from the next request on: web reload shows this page with it)\n" : "");
    g_free(text);
}

/* ua [PROFILE | USER_AGENT | list] */
static void cmd_ua(Request *r)
{
    if (r->argc == 2 && !strcmp(r->argv[1], "list")) {
        GString *s = g_string_new("profiles (web ua NAME), or any user agent of your own (web ua 'Mozilla/5.0 ...'):\n");
        for (size_t i = 0; i < G_N_ELEMENTS(ua_profiles); i++)
            g_string_append_printf(s, "  %-8s %s\n           %s\n", ua_profiles[i].name, ua_profiles[i].about,
                                   ua_profiles[i].user_agent ? ua_profiles[i].user_agent : browser.default_user_agent);
        reply_setting(r, g_string_free(s, FALSE), FALSE);
        return;
    }
    if (r->argc > 1) {
        char *text = g_strjoinv(" ", r->argv + 1); /* the words of a user agent not given as one */
        char *error = NULL;
        gboolean ok = identity_set(text, &error);
        g_free(text);
        if (!ok) {
            replyf(r, ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
        settings_changed();
    }
    reply_setting(r, identity_text(), r->argc > 1);
}

/* identity [new [SEED]]: who the browser says it is, and the machine it is on; new: another machine */
static void cmd_identity(Request *r)
{
    if (r->argc > 1 && strcmp(r->argv[1], "new")) {
        reply(r, ST_USAGE, "usage: identity [new [SEED]]\n");
        return;
    }
    if (r->argc > 1) {
        browser.seed = r->argc > 2 ? (guint32)strtoul(r->argv[2], NULL, 10) : g_random_int();
        if (browser.seed_file) {
            char *text = g_strdup_printf("%u\n", browser.seed);
            g_file_set_contents(browser.seed_file, text, -1, NULL);
            g_free(text);
        }
        settings_changed();
    }
    char *parts[] = { identity_text(), languages_text(), time_zone_text(), humanize_text() };
    GString *s = g_string_new(NULL);
    for (size_t i = 0; i < G_N_ELEMENTS(parts); i++) {
        g_string_append(s, parts[i]);
        g_free(parts[i]);
    }
    reply_setting(r, g_string_free(s, FALSE), r->argc > 1);
}

/* tz [ZONE | auto | default] */
static void cmd_tz(Request *r)
{
    char *about = NULL;
    if (r->argc > 1) {
        char *error = NULL;
        if (!time_zone_set(r->argv[1], &about, &error)) {
            replyf(r, !g_ascii_strcasecmp(r->argv[1], "auto") ? ST_FAILED : ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
        settings_changed();
    }
    char *text = time_zone_text();
    if (about) {
        char *with = g_strdup_printf("%s  from %s\n", text, about);
        g_free(text);
        text = with;
    }
    reply_setting(r, text, r->argc > 1);
    g_free(about);
}

/* humanize [on | off] */
static void cmd_humanize(Request *r)
{
    if (r->argc > 1) {
        char *error = NULL;
        if (!humanize_set(r->argv[1], &error)) {
            replyf(r, ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
    }
    replyf(r, ST_OK, "%s", humanize_text());
}

/* header [list] | set NAME VALUE [--host HOST] | set 'NAME: VALUE' [--host HOST] | unset NAME [--host HOST] | clear */
static void cmd_header(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "list";
    const char *host = NULL;
    GPtrArray *words = g_ptr_array_new();
    for (int i = 2; i < r->argc; i++) {
        if ((!strcmp(r->argv[i], "--host") || !strcmp(r->argv[i], "--for")) && i + 1 < r->argc)
            host = r->argv[++i];
        else
            g_ptr_array_add(words, r->argv[i]);
    }
    char *error = NULL;
    if (!strcmp(what, "set") || !strcmp(what, "add")) {
        gboolean ok;
        if (words->len == 1 && strchr(g_ptr_array_index(words, 0), ':'))
            ok = header_line(host, g_ptr_array_index(words, 0), &error);
        else if (words->len == 2)
            ok = header_set(host, g_ptr_array_index(words, 0), g_ptr_array_index(words, 1), &error);
        else {
            ok = FALSE;
            error = g_strdup("usage: header set NAME VALUE [--host HOST] (an empty VALUE takes NAME off the requests)");
        }
        if (ok) {
            settings_changed();
            reply_setting(r, headers_text(), TRUE);
        } else
            replyf(r, ST_USAGE, "%s\n", error);
    } else if (!strcmp(what, "unset") || !strcmp(what, "remove") || !strcmp(what, "delete")) {
        if (words->len != 1)
            reply(r, ST_USAGE, "usage: header unset NAME [--host HOST]\n");
        else if (!header_unset(host, g_ptr_array_index(words, 0)))
            replyf(r, ST_NOT_FOUND, "no header %s was set%s%s\n", (char *)g_ptr_array_index(words, 0), host ? " for " : "", host ? host : "");
        else {
            settings_changed();
            reply_setting(r, headers_text(), TRUE);
        }
    } else if (!strcmp(what, "clear")) {
        g_ptr_array_set_size(browser.headers, 0);
        settings_changed();
        reply_setting(r, headers_text(), TRUE);
    } else if (!strcmp(what, "list"))
        reply_setting(r, headers_text(), FALSE);
    else
        reply(r, ST_USAGE, "usage: header [list] | set NAME VALUE [--host HOST] | unset NAME [--host HOST] | clear\n");
    g_free(error);
    g_ptr_array_free(words, TRUE);
}

/* lang [LANGUAGES | default] */
static void cmd_lang(Request *r)
{
    if (r->argc > 1) {
        char *text = g_strjoinv(",", r->argv + 1);
        char *error = NULL;
        gboolean ok = languages_set(text, &error);
        g_free(text);
        if (!ok) {
            replyf(r, ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
        settings_changed();
    }
    reply_setting(r, languages_text(), r->argc > 1);
}

/* inject [list] | add NAME SOURCE (web.c reads the file) | remove NAME | clear */
static void cmd_inject(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "list";
    if (!strcmp(what, "add") && r->argc == 4) {
        char *error = NULL;
        if (!script_add(r->argv[2], r->argv[3], &error)) {
            replyf(r, ST_USAGE, "%s\n", error);
            g_free(error);
            return;
        }
        settings_changed();
        reply_setting(r, scripts_text(), TRUE);
    } else if (!strcmp(what, "remove") && r->argc == 3) {
        for (guint i = 0; i < browser.scripts->len; i++) {
            Script *script = g_ptr_array_index(browser.scripts, i);
            if (!strcmp(script->name, r->argv[2])) {
                g_ptr_array_remove_index(browser.scripts, i);
                settings_changed();
                reply_setting(r, scripts_text(), TRUE);
                return;
            }
        }
        replyf(r, ST_NOT_FOUND, "%s is not injected\n%s", r->argv[2], browser.scripts->len ? "" : "(nothing is)\n");
    } else if (!strcmp(what, "clear")) {
        g_ptr_array_set_size(browser.scripts, 0);
        settings_changed();
        reply_setting(r, scripts_text(), TRUE);
    } else if (!strcmp(what, "list"))
        reply_setting(r, scripts_text(), FALSE);
    else
        reply(r, ST_USAGE, "usage: inject [list] | add FILE | remove FILE | clear\n");
}

/* history [N | --all] [TEXT] | history clear: the pages visited (oldest first), the last N (30),
 * those whose address or title has TEXT */
static void cmd_visits(Request *r)
{
    if (r->argc > 1 && !strcmp(r->argv[1], "clear")) {
        g_ptr_array_set_size(browser.history, 0);
        if (browser.history_file)
            unlink(browser.history_file);
        reply(r, ST_OK, "history cleared\n");
        return;
    }
    guint max = 30;
    gboolean all = FALSE, have_max = FALSE;
    const char *text = NULL;
    for (int i = 1; i < r->argc; i++) {
        const char *word = r->argv[i];
        if (!strcmp(word, "--all"))
            all = TRUE;
        else if (!have_max && !text && *word && !word[strspn(word, "0123456789")] && atoi(word) > 0) {
            max = atoi(word); /* the first number; a later one is TEXT */
            have_max = TRUE;
        } else
            text = word;
    }
    GArray *matches = g_array_new(FALSE, FALSE, sizeof(guint));
    for (guint i = 0; i < browser.history->len; i++) {
        Visit *visit = g_ptr_array_index(browser.history, i);
        if (!text || strcasestr(visit->uri, text) || (visit->title && strcasestr(visit->title, text)))
            g_array_append_val(matches, i);
    }
    if (!matches->len) {
        if (text)
            replyf(r, ST_NOT_FOUND, "no visit's address or title has \"%s\"\n", text);
        else
            reply(r, ST_OK, "no history yet\n");
        g_array_free(matches, TRUE);
        return;
    }
    guint from = all || matches->len <= max ? 0 : matches->len - max;
    GString *s = g_string_new(NULL);
    if (from)
        g_string_append_printf(s, "(%u earlier: web history --all%s%s)\n", from, text ? " " : "", text ? text : "");
    for (guint i = from; i < matches->len; i++) {
        guint index = g_array_index(matches, guint, i);
        Visit *visit = g_ptr_array_index(browser.history, index);
        GDateTime *when = g_date_time_new_from_unix_local(visit->time / G_USEC_PER_SEC);
        char *stamp = when ? g_date_time_format(when, "%Y-%m-%d %H:%M:%S") : g_strdup("?");
        g_string_append_printf(s, "%5u  %s  %s  <%s>\n", index + 1, stamp,
                               visit->title && *visit->title ? visit->title : "(no title)", visit->uri);
        g_free(stamp);
        if (when)
            g_date_time_unref(when);
    }
    reply(r, ST_OK, s->str);
    g_string_free(s, TRUE);
    g_array_free(matches, TRUE);
}

/* proxy [status] | [set] URL [--bypass HOSTS|none|default] | off | bypass HOSTS|none|default
 * | ca [list] | ca FILE (web.c makes it whole) | ca clear */
static void cmd_proxy(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "status";
    static const char *const usage_text = "usage: proxy [status] | URL [--bypass HOSTS|none|default] | off | bypass HOSTS|none|default"
                                          " | ca [FILE | clear]\n";
    char *error = NULL;
    if (!strcmp(what, "status") || !strcmp(what, "show") || !strcmp(what, "list")) {
        if (r->argc > 2)
            reply(r, ST_USAGE, usage_text);
        else
            reply_setting(r, proxy_text(), FALSE);
        return;
    }
    if (!strcmp(what, "ca")) {
        if (r->argc == 2 || (r->argc == 3 && !strcmp(r->argv[2], "list")))
            reply_setting(r, ca_text(), FALSE);
        else if (r->argc == 3 && !strcmp(r->argv[2], "clear")) {
            wpe_tls_untrust_all(browser.ca_dir);
            g_ptr_array_set_size(browser.ca_trusted, 0);
            reply(r, ST_OK, "certificate authorities trusted besides the guest's: none from the next connection on (one used already"
                            " stays trusted until the browser starts again: web close)\n");
        } else if (r->argc == 3) {
            char *about = NULL;
            if (!ca_add(r->argv[2], &about, &error)) {
                replyf(r, ST_FAILED, "%s\n", error);
                g_free(error);
                return;
            }
            char *text = ca_text();
            replyf(r, ST_OK, "trusted from the next connection on:\n%s%s", about, text);
            g_free(about);
            g_free(text);
        } else
            reply(r, ST_USAGE, usage_text);
        return;
    }
    const char *address = NULL, *bypass = NULL;
    gboolean bypass_only = !strcmp(what, "bypass");
    if (bypass_only) {
        if (r->argc != 3) {
            reply(r, ST_USAGE, usage_text);
            return;
        }
        bypass = r->argv[2];
    } else {
        for (int i = !strcmp(what, "set") ? 2 : 1; i < r->argc; i++) {
            if (!strcmp(r->argv[i], "--bypass") && i + 1 < r->argc)
                bypass = r->argv[++i];
            else if (!address && r->argv[i][0] != '-')
                address = r->argv[i];
            else {
                reply(r, ST_USAGE, usage_text);
                return;
            }
        }
        if (!address) {
            reply(r, ST_USAGE, usage_text);
            return;
        }
    }
    /* the address checked before the hosts are taken */
    char *checked = NULL;
    if ((address && !proxy_parse(address, &checked, &error)) || (bypass && !proxy_bypass_set(bypass, &error))) {
        g_free(checked);
        replyf(r, ST_USAGE, "%s\n", error);
        g_free(error);
        return;
    }
    if (address) {
        g_free(browser.proxy);
        browser.proxy = checked;
    }
    char *was = g_strdup(browser.proxy_sent ? browser.proxy_sent : "");
    settings_changed();
    gboolean changed = g_strcmp0(was, browser.proxy_sent ? browser.proxy_sent : "") != 0;
    g_free(was);
    char *text = proxy_text();
    replyf(r, ST_OK, "%s%s", text, changed ? "(from now on: the loads under way were stopped; web reload loads this page the new way)\n" : "");
    g_free(text);
}

static void cmd_settings(Request *r)
{
    char *parts[] = { identity_text(), languages_text(), time_zone_text(), humanize_text(), headers_text(), scripts_text(), devtools_text(),
                      proxy_text() };
    replyf(r, ST_OK, "%s%s%s%s%s%s%s%s", parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6], parts[7]);
    for (size_t i = 0; i < G_N_ELEMENTS(parts); i++)
        g_free(parts[i]);
}

/* devtools [status] | on [--bodies text|all|none] | off | toggle */
static void cmd_devtools(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "status";
    const char *bodies = NULL;
    for (int i = 2; i < r->argc; i++) {
        if (!strcmp(r->argv[i], "--bodies") && i + 1 < r->argc)
            bodies = r->argv[++i];
        else if (g_str_has_prefix(r->argv[i], "--bodies="))
            bodies = r->argv[i] + 9;
        else {
            replyf(r, ST_USAGE, "unknown option %s (devtools on --bodies text|all|none)\n", r->argv[i]);
            return;
        }
    }
    if (bodies && !devtools_bodies_mode(bodies)) {
        replyf(r, ST_USAGE, "--bodies is text (the default: documents, scripts, styles, JSON, XML...), all or none, not %s\n", bodies);
        return;
    }
    if (!strcmp(what, "toggle"))
        what = browser.inspector ? "off" : "on";
    if (!strcmp(what, "on") || !strcmp(what, "start")) {
        gboolean was_on = browser.inspector != NULL;
        if (bodies) {
            g_free(browser.devtools_bodies);
            browser.devtools_bodies = g_strdup(bodies);
            if (browser.devtools)
                g_free(devtools_call("setBodies", bodies, NULL));
        }
        if (!devtools_on()) {
            reply(r, ST_FAILED, "the developer tools did not start (gui logs NAME says why)\n");
            return;
        }
        char *text = devtools_text();
        replyf(r, ST_OK, "%s%s", text, was_on ? "" : "web network lists the requests, web network show N one of them, web har FILE saves them all\n");
        g_free(text);
    } else if (!strcmp(what, "off") || !strcmp(what, "stop")) {
        gboolean intercepting = browser.intercepting;
        unsigned paused = browser.paused_count;
        devtools_off();
        char *text = devtools_text();
        replyf(r, ST_OK, "%s%s%s", text, browser.devtools ? "(kept: web network, web har; web network clear forgets it)\n" : "",
               !intercepting ? "" : paused ? "intercept: off too; the paused requests went on as they were\n" : "intercept: off too\n");
        g_free(text);
    } else if (!strcmp(what, "status"))
        reply_setting(r, devtools_text(), FALSE);
    else
        reply(r, ST_USAGE, "usage: devtools [status] | on [--bodies text|all|none] | off | toggle\n");
}

/* then(r) once the bodies still being read from the page's processes are in (5 s at most) */
static gboolean bodies_read(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    devtools_pump();
    char *busy = devtools_call("busy", NULL, NULL);
    gboolean waiting = busy && atoi(busy) > 0 && g_get_monotonic_time() < r->deadline;
    g_free(busy);
    if (waiting)
        r->poll_id = g_timeout_add(50, bodies_read, r);
    else
        r->after_bodies(r);
    return G_SOURCE_REMOVE;
}

static void after_bodies_read(Request *r, void (*then)(Request *r))
{
    r->after_bodies = then;
    r->deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
    bodies_read(r);
}

/* "T" text, "N" text (nothing found), "E" what is wrong: devtools.js's search answers */
static void reply_search(Request *r, const char *name, char *answer)
{
    if (answer && (answer[0] == 'T' || answer[0] == 'N'))
        reply(r, answer[0] == 'T' ? ST_OK : ST_NOT_FOUND, answer + 1);
    else if (name)
        replyf(r, ST_USAGE, "%s: %s\n", name, answer ? answer + 1 : "devtools.js failed (gui logs NAME)");
    else
        replyf(r, ST_USAGE, "%s\n", answer ? answer + 1 : "devtools.js failed (gui logs NAME)");
    g_free(answer);
}

static void network_search(Request *r)
{
    char *words = json_words(r->argv + 3, r->argc - 3);
    reply_search(r, NULL, devtools_call("search", r->argv[2], words));
    g_free(words);
}

/* network body N|TEXT [FILE]: devtools.js may have to read it from the page's process first */
static void network_body(Request *r);

static gboolean network_body_again(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    network_body(r);
    return G_SOURCE_REMOVE;
}

static void network_body(Request *r)
{
    devtools_pump();
    char *answer = devtools_call("body", r->argv[2], NULL);
    char kind = answer ? answer[0] : 'E';
    const char *file = r->argc > 3 ? r->argv[3] : NULL;
    if (kind == 'P') {
        if (g_get_monotonic_time() < r->deadline)
            r->poll_id = g_timeout_add(50, network_body_again, r);
        else
            reply(r, ST_TIMEOUT, "the body did not come from the page's process in time\n");
    } else if (kind == 'E')
        replyf(r, ST_NOT_FOUND, "%s\n", answer ? answer + 1 : "no body");
    else if (!file && kind == 'B')
        replyf(r, ST_FAILED, "the body is binary (%zu bytes of base64): web network body %s FILE writes it\n", strlen(answer + 1), r->argv[2]);
    else if (!file)
        reply(r, ST_OK, answer + 1);
    else {
        gsize length = 0;
        guchar *bytes = kind == 'B' ? g_base64_decode(answer + 1, &length) : NULL;
        GError *error = NULL;
        if (g_file_set_contents(file, bytes ? (const char *)bytes : answer + 1, bytes ? (gssize)length : -1, &error))
            replyf(r, ST_OK, "%s: %zu bytes\n", file, bytes ? (size_t)length : strlen(answer + 1));
        else {
            replyf(r, ST_FAILED, "cannot write %s: %s\n", file, error->message);
            g_error_free(error);
        }
        g_free(bytes);
    }
    g_free(answer);
}

/* devtools.js's export (exported()): the text as the answer, or into FILE (web.c makes it absolute):
 * a script's #! line first, and executable */
static void reply_export(Request *r, char *answer, const char *file)
{
    unsigned count = 0, notes = 0;
    char format[32] = "";
    char *shebang = answer && answer[0] == 'T' ? strchr(answer, '\n') : NULL;
    if (!shebang || sscanf(answer + 1, "%u %31s %u", &count, format, &notes) != 3) {
        int status = !answer ? ST_FAILED : answer[0] == 'N' ? ST_NOT_FOUND : answer[0] == 'E' ? ST_USAGE : ST_FAILED;
        replyf(r, status, "%s\n", answer && answer[0] != 'T' ? answer + 1 : "devtools.js failed (gui logs NAME)");
        g_free(answer);
        return;
    }
    shebang++;
    char *said = strchr(shebang, '\n');
    *said++ = 0;
    char *text = said; /* past the notes: what is left out, said in the file's answer */
    for (unsigned i = 0; i < notes && strchr(text, '\n'); i++)
        text = strchr(text, '\n') + 1;
    if (!file) {
        reply(r, ST_OK, text);
        g_free(answer);
        return;
    }
    char *contents = *shebang ? g_strconcat(shebang, "\n", text, NULL) : g_strdup(text);
    GError *error = NULL;
    if (!g_file_set_contents(file, contents, -1, &error)) {
        replyf(r, ST_FAILED, "cannot write %s: %s\n", file, error->message);
        g_error_free(error);
    } else {
        if (*shebang)
            chmod(file, 0755);
        replyf(r, ST_OK, "%s: %u request%s as %s (%zu bytes%s)\n%.*s", file, count, count == 1 ? "" : "s", format, strlen(contents),
               *shebang ? ", executable" : "", (int)(text - said), said);
    }
    g_free(contents);
    g_free(answer);
}

/* the words of an export but -o FILE (into *file) */
static char *export_words(Request *r, int first, const char **file)
{
    char **words = g_new0(char *, r->argc + 1);
    int n = 0;
    *file = NULL;
    for (int i = first; i < r->argc; i++) {
        if ((!strcmp(r->argv[i], "-o") || !strcmp(r->argv[i], "--output")) && i + 1 < r->argc)
            *file = r->argv[++i];
        else
            words[n++] = r->argv[i];
    }
    char *json = json_words(words, n);
    g_free(words);
    return json;
}

/* network export WHICH... [--as FORMAT] [--no-cookies] [-o FILE]: once the bodies being read are in
 * (a HAR has them) */
static void network_export(Request *r)
{
    const char *file;
    char *words = export_words(r, 2, &file);
    reply_export(r, devtools_call("exportRequests", words, NULL), file);
    g_free(words);
}

/* network [list] [TEXT] [--type T] [--status S] [--failed] [--last N | --all] | show N|TEXT
 * | body N|TEXT [FILE] | search PATTERN [OPTION...] | export WHICH... [OPTION...]
 * | poll CURSOR [WORDS...] (web network watch) | clear */
static void cmd_network(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "list";
    gboolean known = !strcmp(what, "list") || !strcmp(what, "show") || !strcmp(what, "body") || !strcmp(what, "poll")
                     || !strcmp(what, "search") || !strcmp(what, "export") || !strcmp(what, "clear");
    int first = known ? 2 : 1; /* network TEXT: a listing */
    if (!known)
        what = "list";
    if (!browser.devtools) {
        reply(r, ST_FAILED, "nothing recorded: the developer tools are off (web devtools on records the network from then on; "
                            "web --devtools open URL from the browser's start)\n");
        return;
    }
    devtools_pump();
    if (!strcmp(what, "list") || !strcmp(what, "poll")) {
        gboolean polling = !strcmp(what, "poll");
        if (polling && r->argc < 3) {
            reply(r, ST_USAGE, "usage: network poll CURSOR [WORDS...]\n");
            return;
        }
        char *words = json_words(r->argv + first + polling, r->argc - first - polling);
        char *text = polling ? devtools_call("poll", r->argv[2], words) : devtools_call("list", words, NULL);
        g_free(words);
        if (polling || browser.inspector)
            reply(r, text ? ST_OK : ST_FAILED, text ? text : "devtools.js failed (gui logs NAME)\n");
        else
            replyf(r, text ? ST_OK : ST_FAILED, "%s(the developer tools are off: nothing new is recorded)\n", text ? text : "");
        g_free(text);
    } else if (!strcmp(what, "show")) {
        if (r->argc != 3) {
            reply(r, ST_USAGE, "usage: network show N|TEXT (a number from web network, or a part of the address: the last such)\n");
            return;
        }
        char *text = devtools_call("show", r->argv[2], NULL);
        if (text)
            reply(r, ST_OK, text);
        else
            replyf(r, ST_NOT_FOUND, "no request %s (web network lists them)\n", r->argv[2]);
        g_free(text);
    } else if (!strcmp(what, "body")) {
        if (r->argc < 3 || r->argc > 4) {
            reply(r, ST_USAGE, "usage: network body N|TEXT [FILE]\n");
            return;
        }
        r->deadline = g_get_monotonic_time() + 10 * G_USEC_PER_SEC;
        network_body(r);
    } else if (!strcmp(what, "search")) {
        if (r->argc < 3) {
            reply(r, ST_USAGE, "usage: network search TEXT [--case] [--regex] [--in url,headers,body,messages] [--url TEXT] "
                               "[--type T] [--status S] [--failed] [--max N]\n");
            return;
        }
        after_bodies_read(r, network_search);
    } else if (!strcmp(what, "export"))
        after_bodies_read(r, network_export);
    else {
        g_free(devtools_call("clear", NULL, NULL));
        reply(r, ST_OK, "the network log is cleared\n");
    }
}

/* har save FILE [--no-bodies] | har view NAME TEXT [WORDS...] | har search NAME TEXT PATTERN [WORDS...]
 * | har export NAME TEXT WHICH... [OPTION...] (web.c reads the files) */
static void har_save(Request *r)
{
    gboolean bodies = TRUE;
    for (int i = 3; i < r->argc; i++)
        bodies = bodies && strcmp(r->argv[i], "--no-bodies");
    char *options = g_strdup_printf("{\"bodies\": %s, \"version\": \"%u.%u.%u\"}", bodies ? "true" : "false",
                                    webkit_get_major_version(), webkit_get_minor_version(), webkit_get_micro_version());
    char *har = devtools_call("har", options, NULL);
    g_free(options);
    char *status = devtools_call("status", NULL, NULL);
    GError *error = NULL;
    if (!har)
        reply(r, ST_FAILED, "devtools.js failed (gui logs NAME)\n");
    else if (g_file_set_contents(r->argv[2], har, -1, &error))
        replyf(r, ST_OK, "%s: %s (HAR 1.2, %zu bytes)\n", r->argv[2], status ? status : "", strlen(har));
    else {
        replyf(r, ST_FAILED, "cannot write %s: %s\n", r->argv[2], error->message);
        g_error_free(error);
    }
    g_free(status);
    g_free(har);
}

static void cmd_har(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "";
    if (!strcmp(what, "save") && r->argc >= 3) {
        if (!browser.devtools) {
            reply(r, ST_FAILED, "nothing recorded: the developer tools are off (web devtools on)\n");
            return;
        }
        after_bodies_read(r, har_save);
    } else if (!strcmp(what, "search") && r->argc >= 5) {
        if (!devtools_load()) {
            reply(r, ST_FAILED, "devtools.js failed (gui logs NAME)\n");
            return;
        }
        char *words = json_words(r->argv + 4, r->argc - 4); /* the pattern first */
        reply_search(r, r->argv[2], devtools_call("searchHar", r->argv[3], words));
        g_free(words);
    } else if (!strcmp(what, "export") && r->argc >= 4) {
        if (!devtools_load()) {
            reply(r, ST_FAILED, "devtools.js failed (gui logs NAME)\n");
            return;
        }
        const char *file;
        char *words = export_words(r, 4, &file);
        char *answer = devtools_call("exportHar", r->argv[3], words);
        g_free(words);
        if (answer && answer[0] == 'E' && !strstr(answer, "export"))
            replyf(r, ST_FAILED, "%s: %s\n", r->argv[2], answer + 1); /* not a HAR file */
        else
            reply_export(r, answer, file);
    } else if (!strcmp(what, "view") && r->argc >= 4) {
        if (!devtools_load()) {
            reply(r, ST_FAILED, "devtools.js failed (gui logs NAME)\n");
            return;
        }
        char *words = json_words(r->argv + 4, r->argc - 4);
        char *answer = devtools_call("view", r->argv[3], words);
        g_free(words);
        if (answer && answer[0] == 'T')
            reply(r, ST_OK, answer + 1);
        else
            replyf(r, answer && strstr(answer, "no entry") ? ST_NOT_FOUND : ST_FAILED, "%s: %s\n", r->argv[2], answer ? answer + 1 : "devtools.js failed");
        g_free(answer);
    } else
        reply(r, ST_USAGE, "usage: har save FILE [--no-bodies] | har view FILE [N | TEXT ...] | har search FILE TEXT [OPTION...] "
                           "| har export FILE WHICH... [--as FORMAT] [-o OUT] (web har reads and writes the files)\n");
}

/* intercept send|drop|respond: devtools.js's answer once WebKit has taken the decision (5 s at most) */
static gboolean decision_poll(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    devtools_pump();
    gboolean late = g_get_monotonic_time() >= r->deadline;
    char *answer = devtools_call("decision", r->decision, late ? "late" : NULL);
    if (answer && answer[0] == 'P')
        r->poll_id = g_timeout_add(20, decision_poll, r);
    else
        reply(r, answer && answer[0] == 'T' ? ST_OK : ST_FAILED, answer ? answer + 1 : "devtools.js failed (gui logs NAME)\n");
    g_free(answer);
    return G_SOURCE_REMOVE;
}

/* intercept on: answered once the network process holds what the rules take (2 s at most) */
static gboolean intercept_on_settled(gpointer data)
{
    Request *r = data;
    r->poll_id = 0;
    devtools_pump();
    char *settled = devtools_call("netSettled", NULL, NULL);
    gboolean done = !settled || settled[0] == '1' || g_get_monotonic_time() >= r->deadline;
    g_free(settled);
    if (!done) {
        r->poll_id = g_timeout_add(20, intercept_on_settled, r);
        return G_SOURCE_REMOVE;
    }
    reply(r, ST_OK, r->done_text);
    return G_SOURCE_REMOVE;
}

/* intercept body N|TEXT [FILE]: a paused request's body, a response's (--stage body), a message's data */
static void intercept_body(Request *r)
{
    char *answer = devtools_call("pausedBody", r->argv[2], NULL);
    char kind = answer ? answer[0] : 'E';
    const char *file = r->argc > 3 ? r->argv[3] : NULL;
    if (kind == 'E' || kind == 'N')
        replyf(r, ST_NOT_FOUND, "%s\n", answer ? answer + 1 : "devtools.js failed");
    else if (!file && kind == 'B')
        replyf(r, ST_FAILED, "it is binary (%zu bytes of base64): web intercept body %s FILE writes it\n", strlen(answer + 1), r->argv[2]);
    else if (!file)
        reply(r, ST_OK, answer + 1);
    else {
        gsize length = 0;
        guchar *bytes = kind == 'B' ? g_base64_decode(answer + 1, &length) : NULL;
        GError *error = NULL;
        if (g_file_set_contents(file, bytes ? (const char *)bytes : answer + 1, bytes ? (gssize)length : -1, &error))
            replyf(r, ST_OK, "%s: %zu bytes\n", file, bytes ? (size_t)length : strlen(answer + 1));
        else {
            replyf(r, ST_FAILED, "cannot write %s: %s\n", file, error->message);
            g_error_free(error);
        }
        g_free(bytes);
    }
    g_free(answer);
}

static gboolean intercept_wait_over(gpointer data)
{
    Request *r = data;
    r->timeout_id = 0;
    replyf(r, ST_TIMEOUT, "no request paused within %g s (the page: %s)\n", timeout_arg(r, 2, 30000) / 1000.0,
           browser.loading ? "still loading" : "loaded");
    return G_SOURCE_REMOVE;
}

/* intercept [status] | on [TEXT...] [OPTION...] | off | list | show N|TEXT | wait [TIMEOUT]
 * | send|drop|respond N...|all|TEXT [OPTION...] (web.c gives a --body-file as --body64) */
static void cmd_intercept(Request *r)
{
    const char *what = r->argc > 1 ? r->argv[1] : "status";
    /* other words for the same: pause, stop and send (the requests paused, each stopped or sent
     * on), Burp's, the Chrome DevTools protocol's */
    if (!strcmp(what, "pause"))
        what = "on";
    else if (!strcmp(what, "continue") || !strcmp(what, "forward") || !strcmp(what, "approve"))
        what = "send";
    else if (!strcmp(what, "stop") || !strcmp(what, "abort") || !strcmp(what, "block") || !strcmp(what, "fail"))
        what = "drop";
    else if (!strcmp(what, "fulfill") || !strcmp(what, "answer") || !strcmp(what, "mock"))
        what = "respond";
    else if (!strcmp(what, "ls") || !strcmp(what, "paused"))
        what = "list";
    if (!strcmp(what, "on")) {
        gboolean was_on = browser.inspector != NULL;
        if (!devtools_on()) {
            reply(r, ST_FAILED, "the developer tools did not start (gui logs NAME says why)\n");
            return;
        }
        char *words = json_words(r->argv + 2, r->argc - 2);
        char *answer = devtools_call("interceptOn", words, NULL);
        g_free(words);
        devtools_pump();
        if (answer && answer[0] == 'T') {
            /* the answer once the network process holds what the rules take too */
            r->done_text = g_strdup_printf("%s%s%s", was_on ? "" : "developer tools: on (web network records the requests too)\n", answer + 1,
                                           "web intercept lists the paused requests; a command waiting on the page (open, click, wait, run) ends as one pauses (exit 6)\n");
            r->deadline = g_get_monotonic_time() + 2 * G_USEC_PER_SEC;
            intercept_on_settled(r);
        } else
            replyf(r, answer && answer[0] == 'U' ? ST_UNSUPPORTED : ST_USAGE, "%s\n", answer ? answer + 1 : "devtools.js failed (gui logs NAME)");
        g_free(answer);
        return;
    }
    if (!browser.devtools) {
        if (!strcmp(what, "status") || !strcmp(what, "off"))
            reply(r, ST_OK, "intercept: off (web intercept on: the page's requests pause until sent, dropped or answered)\n");
        else if (!strcmp(what, "list") || !strcmp(what, "show") || !strcmp(what, "wait") || !strcmp(what, "send")
                 || !strcmp(what, "drop") || !strcmp(what, "respond") || !strcmp(what, "body"))
            reply(r, ST_FAILED, "intercept: off, so nothing is paused (web intercept on)\n");
        else
            replyf(r, ST_USAGE, "unknown intercept command %s (web help)\n", what);
        return;
    }
    devtools_pump();
    gboolean json = r->argc > 2 && !strcmp(r->argv[r->argc - 1], "--json");
    if (!strcmp(what, "status")) {
        char *text = devtools_call("interceptStatus", NULL, NULL);
        char *net = devtools_call("netStatus", NULL, NULL);
        replyf(r, ST_OK, "%s%s", text ? text : "", net ? net : "");
        g_free(net);
        g_free(text);
    } else if (!strcmp(what, "off")) {
        char *text = devtools_call("interceptOff", NULL, NULL);
        devtools_pump();
        reply(r, text ? ST_OK : ST_FAILED, text ? text : "devtools.js failed (gui logs NAME)\n");
        g_free(text);
    } else if (!strcmp(what, "list") && json) {
        char *text = devtools_call("pausedJson", "all", "0");
        reply(r, text ? ST_OK : ST_FAILED, text ? text + 1 : "devtools.js failed (gui logs NAME)\n");
        g_free(text);
    } else if (!strcmp(what, "show") && json) {
        if (r->argc != 4) {
            reply(r, ST_USAGE, "usage: intercept show N|TEXT --json\n");
            return;
        }
        char *answer = devtools_call("pausedJson", r->argv[2], "1");
        replyf(r, answer && answer[0] == 'T' ? ST_OK : ST_NOT_FOUND, "%s%s", answer ? answer + 1 : "devtools.js failed", answer && answer[0] == 'T' ? "" : "\n");
        g_free(answer);
    } else if (!strcmp(what, "body")) {
        if (r->argc != 3 && r->argc != 4) {
            reply(r, ST_USAGE, "usage: intercept body N|TEXT [FILE] (a paused request's body, a response's with --stage body, a WebSocket message's data)\n");
            return;
        }
        intercept_body(r);
    } else if (!strcmp(what, "list")) {
        char *text = devtools_call("pausedList", "", NULL);
        char *state = devtools_call("interceptStatus", NULL, NULL);
        replyf(r, ST_OK, "%s%s", browser.paused_count || !state ? "" : state, text ? text : "");
        g_free(state);
        g_free(text);
    } else if (!strcmp(what, "show")) {
        if (r->argc != 3) {
            reply(r, ST_USAGE, "usage: intercept show N|TEXT (a number web intercept lists, or a part of the address)\n");
            return;
        }
        char *answer = devtools_call("pausedShow", r->argv[2], NULL);
        replyf(r, answer && answer[0] == 'T' ? ST_OK : ST_NOT_FOUND, "%s%s", answer ? answer + 1 : "devtools.js failed", answer && answer[0] == 'T' ? "" : "\n");
        g_free(answer);
    } else if (!strcmp(what, "wait")) {
        if (browser.paused_count)
            reply_paused(r);
        else if (!browser.intercepting)
            reply(r, ST_FAILED, "intercept: off, so nothing will pause (web intercept on)\n");
        else {
            r->pause_base = browser.pause_serial;
            browser.pause_waiters = g_list_prepend(browser.pause_waiters, r);
            r->timeout_id = g_timeout_add(timeout_arg(r, 2, 30000), intercept_wait_over, r);
        }
    } else if (!strcmp(what, "send") || !strcmp(what, "drop") || !strcmp(what, "respond")) {
        char *words = json_words(r->argv + 2, r->argc - 2);
        char *answer = devtools_call("decide", what, words);
        g_free(words);
        if (answer && answer[0] == 'P') {
            r->decision = g_strdup(answer + 1);
            r->deadline = g_get_monotonic_time() + 5 * G_USEC_PER_SEC;
            devtools_pump();
            decision_poll(r);
        } else
            replyf(r, !answer ? ST_USAGE : answer[0] == 'N' ? ST_NOT_FOUND : answer[0] == 'U' ? ST_UNSUPPORTED : ST_USAGE, "%s\n",
                   answer ? answer + 1 : "devtools.js failed (gui logs NAME)");
        g_free(answer);
    } else
        reply(r, ST_USAGE, "usage: intercept [status] | on [TEXT...] [--regex] [--case] [--type T[,T]] [--method M[,M]] "
                           "[--stage request|response|body|send|receive|both|messages|all] | off | list [--json] | show N [--json] | body N [FILE] "
                           "| wait [TIMEOUT] | send N... [CHANGES] | drop N... | respond N... [--status S] [--header 'NAME: VALUE'] [--mime TYPE] [--body TEXT]\n");
}

/* ── cookies: cookie [list [URL] [--all]] | set COOKIE... [--url URL] | delete NAME [--url URL] | clear
 * The browser's cookie jar (it replaces any Cookie header put on a request). COOKIE is a
 * Set-Cookie ("sid=1; Domain=example.com; Path=/; Secure") or a Cookie header ("a=1; b=2"). */

static const char *const cookie_attributes[] = { "expires", "max-age", "domain", "path", "secure", "httponly", "samesite",
                                                 "partitioned", "priority" };

/* "example.com", "https://example.com/x" → a site's address */
static char *site_uri(const char *text)
{
    return strstr(text, "://") ? g_strdup(text) : g_strconcat("https://", text, NULL);
}

/* the page's address when it is a site's (http, https), else NULL */
static char *page_site_uri(void)
{
    const char *uri = webkit_web_view_get_uri(browser.view);
    return uri && (g_str_has_prefix(uri, "http://") || g_str_has_prefix(uri, "https://")) ? g_strdup(uri) : NULL;
}

/* The cookies in `text`, each with the attributes in it, for `origin` (NULL: Domain= must say) */
static gboolean cookies_parse(const char *text, GUri *origin, GList **out, char **error)
{
    char **parts = g_strsplit(text, ";", -1);
    GPtrArray *pairs = g_ptr_array_new_with_free_func(g_free);
    GString *attributes = g_string_new(NULL);
    gboolean ok = TRUE, path = FALSE;
    for (char **p = parts; *p && ok; p++) {
        char *part = g_strstrip(*p);
        if (!*part)
            continue;
        char *eq = strchr(part, '=');
        char *key = g_ascii_strdown(part, eq ? eq - part : -1);
        g_strstrip(key);
        gboolean attribute = FALSE;
        for (size_t i = 0; p != parts && i < G_N_ELEMENTS(cookie_attributes); i++)
            attribute = attribute || !strcmp(key, cookie_attributes[i]);
        if (attribute) {
            g_string_append_printf(attributes, "; %s", part);
            path = path || !strcmp(key, "path");
        }
        else if (!eq || eq == part) {
            *error = g_strdup_printf("\"%s\" is not NAME=VALUE", part);
            ok = FALSE;
        } else
            g_ptr_array_add(pairs, g_strdup(part));
        g_free(key);
    }
    if (ok && !pairs->len) {
        *error = g_strdup_printf("no NAME=VALUE in \"%s\"", text);
        ok = FALSE;
    }
    if (!path)
        g_string_append(attributes, "; Path=/"); /* the whole site's (a page's script would get its directory's) */
    for (guint i = 0; ok && i < pairs->len; i++) {
        char *header = g_strconcat(g_ptr_array_index(pairs, i), attributes->str, NULL);
        SoupCookie *cookie = soup_cookie_parse(header, origin);
        if (!cookie || !soup_cookie_get_domain(cookie) || !*soup_cookie_get_domain(cookie)) {
            *error = origin ? g_strdup_printf("\"%s\" is not a cookie %s can have (a Domain= of another site? --url gives the site)",
                                              header, g_uri_get_host(origin))
                            : g_strdup_printf("\"%s\": for which site? (--url URL, or Domain=)", header);
            ok = FALSE;
            if (cookie)
                soup_cookie_free(cookie);
        } else {
            if (!soup_cookie_get_path(cookie)) /* a Path= not starting with / */
                soup_cookie_set_path(cookie, "/");
            *out = g_list_append(*out, cookie);
        }
        g_free(header);
    }
    g_string_free(attributes, TRUE);
    g_ptr_array_free(pairs, TRUE);
    g_strfreev(parts);
    return ok;
}

static void reply_cookies(Request *r, GList *cookies, GError *error)
{
    if (error) {
        replyf(r, ST_FAILED, "cannot read the cookies: %s\n", error->message);
        g_error_free(error);
        return;
    }
    GString *s = g_string_new(NULL);
    for (GList *l = cookies; l; l = l->next) {
        char *line = soup_cookie_to_set_cookie_header(l->data);
        g_string_append_printf(s, "%s\n", line);
        g_free(line);
    }
    if (!cookies)
        g_string_append_printf(s, "no cookies%s%s\n", r->what ? " for " : "", r->what ? r->what : "");
    g_list_free_full(cookies, (GDestroyNotify)soup_cookie_free);
    reply(r, ST_OK, s->str);
    g_string_free(s, TRUE);
}

static void cookie_step(Request *r);

static void cookie_step_done(GObject *source, GAsyncResult *result, gpointer data)
{
    Request *r = data;
    GError *error = NULL;
    gboolean adding = !strcmp(r->poll_kind, "set");
    gboolean ok = adding ? webkit_cookie_manager_add_cookie_finish(WEBKIT_COOKIE_MANAGER(source), result, &error)
                         : webkit_cookie_manager_delete_cookie_finish(WEBKIT_COOKIE_MANAGER(source), result, &error);
    if (!ok) {
        replyf(r, ST_FAILED, "cannot %s a cookie: %s\n%s", adding ? "set" : "delete", error ? error->message : "",
               r->done_text ? r->done_text : "");
        g_clear_error(&error);
        return;
    }
    r->cookies_done++;
    cookie_step(r);
}

/* adds (set) or deletes the cookies in r->cookies one after another, then answers */
static void cookie_step(Request *r)
{
    gboolean adding = !strcmp(r->poll_kind, "set");
    if (!r->cookies) {
        replyf(r, ST_OK, "%s %u cookie%s\n%s", adding ? "set" : "deleted", r->cookies_done, r->cookies_done == 1 ? "" : "s",
               r->done_text ? r->done_text : "");
        return;
    }
    SoupCookie *cookie = r->cookies->data;
    r->cookies = g_list_delete_link(r->cookies, r->cookies);
    char *line = soup_cookie_to_set_cookie_header(cookie);
    char *text = g_strdup_printf("%s  %s\n", r->done_text ? r->done_text : "", line);
    g_free(r->done_text);
    r->done_text = text;
    g_free(line);
    WebKitCookieManager *manager = webkit_network_session_get_cookie_manager(browser.session);
    if (adding)
        webkit_cookie_manager_add_cookie(manager, cookie, NULL, cookie_step_done, r);
    else
        webkit_cookie_manager_delete_cookie(manager, cookie, NULL, cookie_step_done, r);
    soup_cookie_free(cookie);
}

static void cookies_listed(GObject *source, GAsyncResult *result, gpointer data)
{
    Request *r = data;
    GError *error = NULL;
    GList *cookies = webkit_cookie_manager_get_cookies_finish(WEBKIT_COOKIE_MANAGER(source), result, &error);
    if (strcmp(r->poll_kind, "delete")) {
        reply_cookies(r, cookies, error);
        return;
    }
    if (error) {
        reply_cookies(r, NULL, error);
        return;
    }
    /* delete NAME: the ones of that name */
    for (GList *l = cookies; l; l = l->next) {
        if (!strcmp(soup_cookie_get_name(l->data), r->poll_arg))
            r->cookies = g_list_append(r->cookies, soup_cookie_copy(l->data));
    }
    g_list_free_full(cookies, (GDestroyNotify)soup_cookie_free);
    if (!r->cookies) {
        replyf(r, ST_NOT_FOUND, "no cookie %s%s%s\n", r->poll_arg, r->what ? " for " : "", r->what ? r->what : "");
        return;
    }
    cookie_step(r);
}

static void all_cookies_listed(GObject *source, GAsyncResult *result, gpointer data)
{
    Request *r = data;
    GError *error = NULL;
    GList *cookies = webkit_cookie_manager_get_all_cookies_finish(WEBKIT_COOKIE_MANAGER(source), result, &error);
    if (!strcmp(r->poll_kind, "delete") && !error) {
        for (GList *l = cookies; l; l = l->next) {
            if (!strcmp(soup_cookie_get_name(l->data), r->poll_arg))
                r->cookies = g_list_append(r->cookies, soup_cookie_copy(l->data));
        }
        g_list_free_full(cookies, (GDestroyNotify)soup_cookie_free);
        if (!r->cookies) {
            replyf(r, ST_NOT_FOUND, "no cookie %s\n", r->poll_arg);
            return;
        }
        cookie_step(r);
        return;
    }
    reply_cookies(r, cookies, error);
}

static void cookies_cleared(GObject *source, GAsyncResult *result, gpointer data)
{
    Request *r = data;
    GError *error = NULL;
    if (webkit_website_data_manager_clear_finish(WEBKIT_WEBSITE_DATA_MANAGER(source), result, &error))
        reply(r, ST_OK, "cleared every cookie\n");
    else {
        replyf(r, ST_FAILED, "cannot clear the cookies: %s\n", error ? error->message : "");
        g_clear_error(&error);
    }
}

static void cmd_cookie(Request *r)
{
    static const char *const kinds[] = { "list", "set", "add", "delete", "remove", "clear" };
    const char *what = r->argc > 1 ? r->argv[1] : "list", *url = NULL;
    int first = 2;
    gboolean known = FALSE;
    for (size_t i = 0; i < G_N_ELEMENTS(kinds); i++)
        known = known || !strcmp(what, kinds[i]);
    if (!known && r->argc > 1 && (strchr(what, '.') || strstr(what, "://") || !strcmp(what, "--all"))) {
        what = "list"; /* cookies URL */
        first = 1;
    }
    what = !strcmp(what, "add") ? "set" : !strcmp(what, "remove") ? "delete" : what;
    r->poll_kind = g_strdup(what);
    gboolean all = FALSE;
    GPtrArray *words = g_ptr_array_new();
    for (int i = first; i < r->argc; i++) {
        if (!strcmp(r->argv[i], "--url") && i + 1 < r->argc)
            url = r->argv[++i];
        else if (!strcmp(r->argv[i], "--all"))
            all = TRUE;
        else
            g_ptr_array_add(words, r->argv[i]);
    }
    WebKitCookieManager *manager = webkit_network_session_get_cookie_manager(browser.session);
    char *page = page_site_uri();
    if (!strcmp(what, "list") || !strcmp(what, "delete")) {
        gboolean deleting = !strcmp(what, "delete");
        if (deleting && words->len < 1)
            reply(r, ST_USAGE, "usage: cookie delete NAME [--url URL | --all]\n");
        else {
            if (deleting)
                r->poll_arg = g_strdup(g_ptr_array_index(words, 0));
            if (!url && !deleting && words->len)
                url = g_ptr_array_index(words, 0);
            r->what = url ? site_uri(url) : all ? NULL : g_strdup(page);
            if (r->what)
                webkit_cookie_manager_get_cookies(manager, r->what, NULL, cookies_listed, r);
            else
                webkit_cookie_manager_get_all_cookies(manager, NULL, all_cookies_listed, r);
        }
    } else if (!strcmp(what, "set")) {
        char *origin_uri = url ? site_uri(url) : g_strdup(page);
        GUri *origin = origin_uri ? g_uri_parse(origin_uri, G_URI_FLAGS_NONE, NULL) : NULL;
        char *error = NULL;
        gboolean ok = words->len > 0;
        if (!ok)
            error = g_strdup("usage: cookie set 'NAME=VALUE[; Domain=...; Path=...; Secure; Max-Age=...]'... [--url URL]");
        for (guint i = 0; ok && i < words->len; i++)
            ok = cookies_parse(g_ptr_array_index(words, i), origin, &r->cookies, &error);
        if (ok)
            cookie_step(r);
        else
            replyf(r, ST_USAGE, "%s\n", error);
        g_free(error);
        if (origin)
            g_uri_unref(origin);
        g_free(origin_uri);
    } else if (!strcmp(what, "clear"))
        webkit_website_data_manager_clear(webkit_network_session_get_website_data_manager(browser.session),
                                          WEBKIT_WEBSITE_DATA_COOKIES, 0, NULL, cookies_cleared, r);
    else
        reply(r, ST_USAGE, "usage: cookie [list [URL] [--all]] | set COOKIE... [--url URL] | delete NAME [--url URL] | clear\n");
    g_free(page);
    g_ptr_array_free(words, TRUE);
}

static gboolean quit_soon(gpointer data)
{
    (void)data;
    g_main_loop_quit(browser.loop);
    return G_SOURCE_REMOVE;
}

/* the commands --screenshot FILE is for: the actions, and the loads (a picture once loaded) */
static const char *const shot_commands[] = { "click", "dblclick", "rightclick", "hover", "type", "select", "key", "press",
                                             "scroll", "open", "goto", "back", "forward", "reload", "wait", "search", NULL };

static void run_request(Request *r)
{
    if (!r->argc) {
        reply(r, ST_USAGE, "empty request\n");
        return;
    }
    const char *cmd = r->argv[0];
    r->pause_base = browser.pause_serial;
    /* --screenshot FILE, anywhere after the command's name (web made FILE absolute) */
    if (g_strv_contains(shot_commands, cmd)) {
        for (int i = 1; i < r->argc; i++) {
            if (strcmp(r->argv[i], "--screenshot"))
                continue;
            if (i + 1 >= r->argc || !*r->argv[i + 1]) {
                reply(r, ST_USAGE, "--screenshot needs a file (a PNG is written there)\n");
                return;
            }
            g_free(r->argv[i]);
            g_free(r->shot_file);
            r->shot_file = r->argv[i + 1];
            memmove(r->argv + i, r->argv + i + 2, (r->argc - i - 1) * sizeof *r->argv);
            r->argc -= 2;
            i--;
        }
    }
    /* web intercept: a synchronous request of the page is paused, so the page's main thread waits for
     * it: what needs the page would wait as long. It ends at once, the paused ones listed (exit 6). */
    static const char *const page_commands[] = { "open", "goto", "back", "forward", "reload", "wait", "read", "text", "html", "links",
                                                 "eval", "run", "click", "dblclick", "rightclick", "hover", "type", "select", "key", "press",
                                                 "scroll", "search", "challenge", NULL };
    if (browser.blocking_count && g_strv_contains(page_commands, cmd)) {
        char *list = devtools_call("pausedList", "paused: ", NULL);
        replyf(r, ST_PAUSED, "%sthe page waits for a synchronous request that is paused (its main thread is stopped): "
                             "decide it first (web intercept send|drop|respond N)\n", list ? list : "");
        g_free(list);
        return;
    }
    if (!strcmp(cmd, "open") || !strcmp(cmd, "goto"))
        cmd_open(r);
    else if (!strcmp(cmd, "back") || !strcmp(cmd, "forward") || !strcmp(cmd, "reload"))
        cmd_history(r, cmd);
    else if (!strcmp(cmd, "stop")) {
        webkit_web_view_stop_loading(browser.view);
        reply(r, ST_OK, "stopped\n");
    } else if (!strcmp(cmd, "info") || !strcmp(cmd, "status"))
        cmd_info(r);
    else if (!strcmp(cmd, "url")) {
        const char *uri = webkit_web_view_get_uri(browser.view);
        replyf(r, ST_OK, "%s\n", uri ? uri : "about:blank");
    } else if (!strcmp(cmd, "title")) {
        const char *title = webkit_web_view_get_title(browser.view);
        replyf(r, ST_OK, "%s\n", title ? title : "");
    } else if (!strcmp(cmd, "eval") || !strcmp(cmd, "js"))
        cmd_eval(r);
    else if (!strcmp(cmd, "run"))
        cmd_run(r);
    else if (!strcmp(cmd, "set"))
        cmd_set(r);
    else if (!strcmp(cmd, "ua") || !strcmp(cmd, "user-agent"))
        cmd_ua(r);
    else if (!strcmp(cmd, "identity") || !strcmp(cmd, "whoami"))
        cmd_identity(r);
    else if (!strcmp(cmd, "tz") || !strcmp(cmd, "timezone"))
        cmd_tz(r);
    else if (!strcmp(cmd, "humanize"))
        cmd_humanize(r);
    else if (!strcmp(cmd, "challenge") || !strcmp(cmd, "captcha"))
        cmd_challenge(r);
    else if (!strcmp(cmd, "search"))
        cmd_search(r);
    else if (!strcmp(cmd, "header") || !strcmp(cmd, "headers"))
        cmd_header(r);
    else if (!strcmp(cmd, "lang") || !strcmp(cmd, "languages"))
        cmd_lang(r);
    else if (!strcmp(cmd, "inject"))
        cmd_inject(r);
    else if (!strcmp(cmd, "cookie") || !strcmp(cmd, "cookies"))
        cmd_cookie(r);
    else if (!strcmp(cmd, "settings"))
        cmd_settings(r);
    else if (!strcmp(cmd, "proxy"))
        cmd_proxy(r);
    else if (!strcmp(cmd, "history"))
        cmd_visits(r);
    else if (!strcmp(cmd, "devtools"))
        cmd_devtools(r);
    else if (!strcmp(cmd, "network") || !strcmp(cmd, "net"))
        cmd_network(r);
    else if (!strcmp(cmd, "har"))
        cmd_har(r);
    else if (!strcmp(cmd, "intercept"))
        cmd_intercept(r);
    else if (!strcmp(cmd, "read") || !strcmp(cmd, "snapshot"))
        cmd_read(r);
    else if (!strcmp(cmd, "text"))
        cmd_text(r);
    else if (!strcmp(cmd, "html"))
        cmd_html(r);
    else if (!strcmp(cmd, "links"))
        cmd_links(r);
    else if (!strcmp(cmd, "click") || !strcmp(cmd, "dblclick") || !strcmp(cmd, "rightclick") || !strcmp(cmd, "hover")
             || !strcmp(cmd, "type") || !strcmp(cmd, "select"))
        cmd_target(r);
    else if (!strcmp(cmd, "key") || !strcmp(cmd, "press"))
        cmd_key(r);
    else if (!strcmp(cmd, "scroll"))
        cmd_scroll(r);
    else if (!strcmp(cmd, "wait"))
        cmd_wait(r);
    else if (!strcmp(cmd, "resize"))
        cmd_resize(r);
    else if (!strcmp(cmd, "close") || !strcmp(cmd, "quit")) {
        reply(r, ST_OK, "closing\n");
        g_idle_add(quit_soon, NULL);
    } else if (!strcmp(cmd, "ping"))
        reply(r, ST_OK, "pong\n");
    else
        replyf(r, ST_USAGE, "unknown command \"%s\"\n", cmd);
}

/* ── the control socket ────────────────────────────────────────────────────────────────────── */

/* Parses "N\n" + N × ("LEN\n" bytes): 1 complete, 0 more needed, -1 malformed */
static int parse_request(Request *r)
{
    const char *p = (const char *)r->input->data, *end = p + r->input->len;
    const char *nl = memchr(p, '\n', end - p);
    if (!nl)
        return r->input->len > 32 ? -1 : 0;
    long count = strtol(p, NULL, 10);
    if (count < 0 || count > 1000)
        return -1;
    p = nl + 1;
    GPtrArray *words = g_ptr_array_new_with_free_func(g_free);
    for (long i = 0; i < count; i++) {
        nl = memchr(p, '\n', end - p);
        if (!nl) {
            g_ptr_array_free(words, TRUE);
            return 0;
        }
        long length = strtol(p, NULL, 10);
        if (length < 0 || length > 64 * 1024 * 1024) {
            g_ptr_array_free(words, TRUE);
            return -1;
        }
        p = nl + 1;
        if (end - p < length) {
            g_ptr_array_free(words, TRUE);
            return 0;
        }
        g_ptr_array_add(words, g_strndup(p, length));
        p += length;
    }
    g_ptr_array_add(words, NULL);
    r->argc = (int)count;
    r->argv = (char **)g_ptr_array_free(words, FALSE);
    return 1;
}

static void read_more(Request *r);

static void read_done(GObject *source, GAsyncResult *result, gpointer data)
{
    Request *r = data;
    GBytes *bytes = g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, NULL);
    gsize size = 0;
    const guint8 *chunk = bytes ? g_bytes_get_data(bytes, &size) : NULL;
    if (!size) {
        if (bytes)
            g_bytes_unref(bytes);
        request_free(r);
        return;
    }
    g_byte_array_append(r->input, chunk, size);
    g_bytes_unref(bytes);
    switch (parse_request(r)) {
    case 1:
        run_request(r);
        break;
    case 0:
        read_more(r);
        break;
    default:
        reply(r, ST_USAGE, "malformed request\n");
        break;
    }
}

static void read_more(Request *r)
{
    GInputStream *in = g_io_stream_get_input_stream(G_IO_STREAM(r->connection));
    g_input_stream_read_bytes_async(in, 65536, G_PRIORITY_DEFAULT, NULL, read_done, r);
}

static gboolean on_incoming(GSocketService *service, GSocketConnection *connection, GObject *source, gpointer data)
{
    (void)service; (void)source; (void)data;
    Request *r = g_new0(Request, 1);
    r->browser = &browser;
    r->connection = g_object_ref(connection);
    r->input = g_byte_array_new();
    read_more(r);
    return TRUE;
}

static gboolean start_control_socket(GError **error)
{
    const char *dir = g_getenv("COLLABO_WEB_DIR");
    char *socket_dir = g_strdup(dir && *dir ? dir : "/tmp/.collabo-web");
    g_mkdir_with_parents(socket_dir, 0700);
    browser.socket_path = g_strdup_printf("%s/%s.sock", socket_dir, browser.name);
    g_free(socket_dir);

    /* a socket nobody answers on is left over from a browser that is gone */
    GSocketClient *client = g_socket_client_new();
    GSocketAddress *address = g_unix_socket_address_new(browser.socket_path);
    GSocketConnection *existing = g_socket_client_connect(client, G_SOCKET_CONNECTABLE(address), NULL, NULL);
    g_object_unref(client);
    if (existing) {
        g_object_unref(existing);
        g_object_unref(address);
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "a browser named \"%s\" is running already (%s)", browser.name, browser.socket_path);
        return FALSE;
    }
    unlink(browser.socket_path);

    browser.service = g_socket_service_new();
    g_socket_service_stop(browser.service); /* until start_serving */
    gboolean ok = g_socket_listener_add_address(G_SOCKET_LISTENER(browser.service), address, G_SOCKET_TYPE_STREAM,
                                                G_SOCKET_PROTOCOL_DEFAULT, NULL, NULL, error);
    g_object_unref(address);
    if (!ok)
        return FALSE;
    g_signal_connect(browser.service, "incoming", G_CALLBACK(on_incoming), NULL);
    return TRUE;
}

/* ── the page's own requests ───────────────────────────────────────────────────────────────── */

static WebKitWebView *on_create(WebKitWebView *view, WebKitNavigationAction *action, gpointer data)
{
    (void)data;
    /* one page: what would open a window opens here */
    webkit_web_view_load_request(view, webkit_navigation_action_get_request(action));
    return NULL;
}

static gboolean on_script_dialog(WebKitWebView *view, WebKitScriptDialog *dialog, gpointer data)
{
    (void)view; (void)data;
    const char *kinds[] = { "alert", "confirm", "prompt", "before unload" };
    WebKitScriptDialogType type = webkit_script_dialog_get_dialog_type(dialog);
    g_free(browser.last_dialog);
    browser.last_dialog = g_strdup_printf("%s: %s", type < 4 ? kinds[type] : "dialog", webkit_script_dialog_get_message(dialog));
    g_print("wpe-browser: %s\n", browser.last_dialog);
    /* nobody is there to answer: accept (confirm: OK, prompt: its default, leave the page) */
    if (type == WEBKIT_SCRIPT_DIALOG_CONFIRM || type == WEBKIT_SCRIPT_DIALOG_BEFORE_UNLOAD_CONFIRM)
        webkit_script_dialog_confirm_set_confirmed(dialog, TRUE);
    else if (type == WEBKIT_SCRIPT_DIALOG_PROMPT)
        webkit_script_dialog_prompt_set_text(dialog, webkit_script_dialog_prompt_get_default_text(dialog));
    return TRUE;
}

static gboolean on_permission_request(WebKitWebView *view, WebKitPermissionRequest *request, gpointer data)
{
    (void)view; (void)data;
    webkit_permission_request_deny(request);
    return TRUE;
}

static void on_title(WebKitWebView *view, GParamSpec *spec, gpointer data)
{
    (void)spec; (void)data;
    const char *title = webkit_web_view_get_title(view);
    if (wpe_view())
        wpe_toplevel_set_title(wpe_view_get_toplevel(wpe_view()), title && *title ? title : "web");
    history_title(webkit_web_view_get_uri(view), title);
    if (browser.devtools)
        g_free(devtools_call("title", title, NULL));
}

static void on_web_process_terminated(WebKitWebView *view, WebKitWebProcessTerminationReason reason, gpointer data)
{
    (void)data;
    browser.web_process_crashes++;
    g_free(browser.last_load_error);
    browser.last_load_error = g_strdup_printf("the page's process ended (%s); `web reload` loads it again",
                                              reason == WEBKIT_WEB_PROCESS_EXCEEDED_MEMORY_LIMIT ? "out of memory" : "crashed");
    g_printerr("wpe-browser: %s\n", browser.last_load_error);
    browser.loading = FALSE;
    browser.load_generation++;  /* ends every wait, whenever it began */
    browser.devtools_reconnect = browser.inspector != NULL;
    finish_load_waiters(ST_FAILED, browser.last_load_error);
    start_serving();
    (void)view;
}

static gboolean on_decide_policy(WebKitWebView *view, WebKitPolicyDecision *decision, WebKitPolicyDecisionType type, gpointer data)
{
    (void)view; (void)data;
    if (type != WEBKIT_POLICY_DECISION_TYPE_RESPONSE)
        return FALSE;
    WebKitResponsePolicyDecision *response = WEBKIT_RESPONSE_POLICY_DECISION(decision);
    if (!webkit_response_policy_decision_is_mime_type_supported(response)) {
        webkit_policy_decision_download(decision);
        return TRUE;
    }
    return FALSE;
}

static gboolean on_decide_destination(WebKitDownload *download, const char *suggested, gpointer data)
{
    (void)data;
    char *dir = g_build_filename(g_get_home_dir(), "Downloads", NULL);
    g_mkdir_with_parents(dir, 0755);
    char *base = g_path_get_basename(suggested && *suggested ? suggested : "download");
    char *path = g_build_filename(dir, base, NULL);
    webkit_download_set_destination(download, path);
    g_print("wpe-browser: downloading to %s\n", path);
    g_free(base);
    g_free(dir);
    g_free(path);
    return TRUE;
}

static void on_download_started(WebKitNetworkSession *session, WebKitDownload *download, gpointer data)
{
    (void)session; (void)data;
    g_signal_connect(download, "decide-destination", G_CALLBACK(on_decide_destination), NULL);
}

static void on_close(WebKitWebView *view, gpointer data)
{
    (void)view; (void)data;
    /* window.close(): an agent's page is closed by the agent (web close) */
}

/* the canvas was closed (`gui close`) or the display went away */
static void on_view_closed(WPEView *view, gpointer data)
{
    (void)view; (void)data;
    g_main_loop_quit(browser.loop);
}

/* ── main ──────────────────────────────────────────────────────────────────────────────────── */

static void set_feature(WebKitSettings *settings, const char *identifier, gboolean enabled)
{
    WebKitFeatureList *features = webkit_settings_get_all_features();
    for (gsize i = 0; i < webkit_feature_list_get_length(features); i++) {
        WebKitFeature *feature = webkit_feature_list_get(features, i);
        if (!g_strcmp0(webkit_feature_get_identifier(feature), identifier)) {
            webkit_settings_set_feature_enabled(settings, feature, enabled);
            break;
        }
    }
    webkit_feature_list_unref(features);
}

static void usage(void)
{
    g_printerr("usage: wpe-browser [--name NAME] [--size WxH] [--ephemeral] [--data-dir DIR] [--ignore-tls-errors]\n"
               "                   [--user-agent UA|PROFILE] [--header 'NAME: VALUE'] [--header-for HOST 'NAME: VALUE']\n"
               "                   [--lang LANGUAGES] [--timezone ZONE|auto] [--humanize on|off] [--inject FILE]\n"
               "                   [--devtools] [--proxy URL] [--proxy-bypass HOSTS] [--proxy-ca FILE] [URL]\n"
               "Usually started by the `web` command; `web help` says more.\n");
}

int wpe_browser_main(int argc, char **argv)
{
    const char *url = NULL, *size = NULL, *data_dir = NULL;
    gboolean ephemeral = FALSE, ignore_tls = FALSE;
    browser.name = g_strdup("web");
    debugging = g_strcmp0(g_getenv("WPE_BROWSER_DEBUG"), "1") == 0;
    browser.headers = g_ptr_array_new_with_free_func((GDestroyNotify)header_free);
    browser.scripts = g_ptr_array_new_with_free_func((GDestroyNotify)script_free);
    browser.history = g_ptr_array_new_with_free_func((GDestroyNotify)visit_free);
    /* WEB_DEVTOOLS=1 (or text, all, none: the bodies kept): the developer tools on from the start */
    const char *devtools_env = g_getenv("WEB_DEVTOOLS");
    browser.devtools_bodies = g_strdup(devtools_bodies_mode(devtools_env) ? devtools_env : "text");
    browser.devtools_wanted = devtools_env && *devtools_env && strcmp(devtools_env, "0") && strcmp(devtools_env, "off");
    /* human input unless WEB_HUMANIZE=0 (or off) */
    browser.rand = g_rand_new();
    browser.humanize = TRUE;

    /* WebKit names the machine as uname does: "Linux wasm" is a platform no site knows, and some
     * take it for a robot; the desktop Linux most of them have seen instead */
    WebKitSettings *settings = webkit_settings_new();
    char **parts = g_strsplit(webkit_settings_get_user_agent(settings), "Linux wasm32", -1);
    char *joined = g_strjoinv("Linux x86_64", parts);
    g_strfreev(parts);
    parts = g_strsplit(joined, "Linux wasm", -1);
    g_free(joined);
    browser.default_user_agent = g_strjoinv("Linux x86_64", parts);
    g_strfreev(parts);
    /* its name first: the folder of the certificate authorities it trusts is named by it */
    for (int i = 1; i + 1 < argc; i++) {
        if (!strcmp(argv[i], "--name")) {
            g_free(browser.name);
            browser.name = g_strdup(argv[++i]);
        }
    }
    ca_start();
    /* the proxy before what may ask the network (WEB_TIMEZONE=auto, --timezone auto): WEB_PROXY,
     * WEB_PROXY_BYPASS, WEB_PROXY_CA, then the options */
    char *error_text = NULL;
    static const char *const proxy_env[][2] = { { "WEB_PROXY", "--proxy" }, { "WEB_PROXY_BYPASS", "--proxy-bypass" },
                                                { "WEB_PROXY_CA", "--proxy-ca" } };
    for (size_t i = 0; i < G_N_ELEMENTS(proxy_env); i++) {
        const char *value = g_getenv(proxy_env[i][0]);
        if (value && *value && !proxy_option(proxy_env[i][1], value, &error_text)) {
            g_printerr("wpe-browser: %s: %s\n", proxy_env[i][0], error_text);
            return 2;
        }
    }
    if (!proxy_options(argc, argv, &error_text)) {
        g_printerr("wpe-browser: %s\n", error_text);
        return 2;
    }
    /* the environment's settings, before the options': WEB_USER_AGENT, WEB_HEADERS (lines
     * NAME: VALUE), WEB_LANG (the Dart package's WebTools gives a browser its first ones so) */
    if (!identity_set(g_getenv("WEB_USER_AGENT"), &error_text)) {
        g_printerr("wpe-browser: WEB_USER_AGENT: %s\n", error_text);
        return 2;
    }
    char **header_lines = g_strsplit(g_getenv("WEB_HEADERS") ? g_getenv("WEB_HEADERS") : "", "\n", -1);
    for (char **line = header_lines; *line; line++) {
        g_strstrip(*line);
        if (**line && !header_line(NULL, *line, &error_text)) {
            g_printerr("wpe-browser: WEB_HEADERS: %s\n", error_text);
            return 2;
        }
    }
    g_strfreev(header_lines);
    if (g_getenv("WEB_LANG") && *g_getenv("WEB_LANG") && !languages_set(g_getenv("WEB_LANG"), &error_text)) {
        g_printerr("wpe-browser: WEB_LANG: %s\n", error_text);
        return 2;
    }
    if (g_getenv("WEB_HUMANIZE") && *g_getenv("WEB_HUMANIZE") && !humanize_set(g_getenv("WEB_HUMANIZE"), &error_text)) {
        g_printerr("wpe-browser: WEB_HUMANIZE: %s\n", error_text);
        return 2;
    }
    char *about = NULL;
    if (g_getenv("WEB_TIMEZONE") && *g_getenv("WEB_TIMEZONE") && !time_zone_set(g_getenv("WEB_TIMEZONE"), &about, &error_text)) {
        g_printerr("wpe-browser: WEB_TIMEZONE: %s\n", error_text);
        return 2;
    }
    g_free(about);

    for (int i = 1; i < argc; i++) {
        int taken = setting_option(argc, argv, i, &error_text);
        if (taken < 0) {
            g_printerr("wpe-browser: %s\n", error_text);
            return 2;
        }
        if (taken > 0) {
            i += taken - 1;
            continue;
        }
        if (!strcmp(argv[i], "--name") && i + 1 < argc) {
            g_free(browser.name);
            browser.name = g_strdup(argv[++i]);
        } else if (!strcmp(argv[i], "--size") && i + 1 < argc)
            size = argv[++i];
        else if (!strcmp(argv[i], "--data-dir") && i + 1 < argc)
            data_dir = argv[++i];
        else if (!strcmp(argv[i], "--ephemeral"))
            ephemeral = TRUE;
        else if (!strcmp(argv[i], "--ignore-tls-errors"))
            ignore_tls = TRUE;
        else if (!strcmp(argv[i], "-h") || !strcmp(argv[i], "--help")) {
            usage();
            return 0;
        } else if (argv[i][0] == '-') {
            usage();
            return 2;
        } else
            url = argv[i];
    }

    g_set_prgname(browser.name);
    g_set_application_name(browser.name);
    /* the guest's trust store (with the sandbox's own CA for the hosts it adds secrets to) */
    if (!g_getenv("SSL_CERT_FILE") && g_file_test("/etc/ssl/cert.pem", G_FILE_TEST_EXISTS))
        g_setenv("SSL_CERT_FILE", "/etc/ssl/cert.pem", TRUE);

    GError *error = NULL;
    if (!start_control_socket(&error)) {
        g_printerr("wpe-browser: %s\n", error->message);
        return 1;
    }
    /* before WebKit starts its network process: it inherits where to find the browser */
    start_net_socket();

    if (ephemeral) {
        browser.session = webkit_network_session_new_ephemeral();
        browser.seed = g_random_int();
    }
    else {
        char *data = data_dir ? g_strdup(data_dir) : g_build_filename(g_get_user_data_dir(), "collabo-web", browser.name, NULL);
        char *cache = g_build_filename(g_get_user_cache_dir(), "collabo-web", browser.name, NULL);
        browser.session = webkit_network_session_new(data, cache);
        g_mkdir_with_parents(data, 0700);
        char *history = g_build_filename(data, "history.tsv", NULL);
        history_load(history);
        g_free(history);
        /* the machine it is on, the same each start (web identity new: another) */
        browser.seed_file = g_build_filename(data, "identity-seed", NULL);
        char *seed_text = NULL;
        if (g_file_get_contents(browser.seed_file, &seed_text, NULL, NULL) && g_ascii_isdigit(*seed_text))
            browser.seed = (guint32)strtoul(seed_text, NULL, 10);
        else {
            browser.seed = g_random_int();
            char *text = g_strdup_printf("%u\n", browser.seed);
            g_file_set_contents(browser.seed_file, text, -1, NULL);
            g_free(text);
        }
        g_free(seed_text);
        g_free(data);
        g_free(cache);
    }
    if (ignore_tls)
        webkit_network_session_set_tls_errors_policy(browser.session, WEBKIT_TLS_ERRORS_POLICY_IGNORE);
    g_signal_connect(browser.session, "download-started", G_CALLBACK(on_download_started), NULL);

    /* Each page's process is a program of its own in the guest, with ~100 MB of memory or more:
     * no cache of processes for sites left (WebKit's default keeps one per site), and the two
     * pages before for `back` (the back/forward cache) */
    webkit_web_context_set_cache_model(webkit_web_context_get_default(), WEBKIT_CACHE_MODEL_DOCUMENT_BROWSER);

    /* the guest has no GPU: Skia on the CPU, buffers in shared memory (WPE has no API for the
     * policy GTK has: the internal feature it sets; the web process also takes the CPU path by
     * itself when there is no EGL: patch 0004) */
    set_feature(settings, "HardwareAccelerationEnabled", FALSE);
    webkit_settings_set_enable_write_console_messages_to_stdout(settings, TRUE);
    webkit_settings_set_javascript_can_open_windows_automatically(settings, TRUE);
    webkit_settings_set_enable_webgl(settings, FALSE);
    webkit_settings_set_user_agent(settings, browser.identity.user_agent);

    browser.view = WEBKIT_WEB_VIEW(g_object_new(WEBKIT_TYPE_WEB_VIEW,
                                                "network-session", browser.session,
                                                "settings", settings,
                                                NULL));
    g_object_unref(settings);
    /* the page's size, which the identity's screen must hold */
    int size_w, size_h;
    if (size && sscanf(size, "%dx%d", &size_w, &size_h) == 2 && size_w > 0 && size_h > 0) {
        browser.view_width = size_w;
        browser.view_height = size_h;
    } else if (wpe_view() && wpe_view_get_width(wpe_view()) > 0) {
        browser.view_width = wpe_view_get_width(wpe_view());
        browser.view_height = wpe_view_get_height(wpe_view());
    }
    /* the user scripts (the input counter, the identity, the injected) and, before a web process
     * starts, the headers it puts on requests */
    settings_changed();
    g_signal_connect(browser.view, "load-changed", G_CALLBACK(on_load_changed), NULL);
    g_signal_connect(webkit_web_view_get_back_forward_list(browser.view), "changed", G_CALLBACK(on_back_forward_changed), NULL);
    g_signal_connect(browser.view, "load-failed", G_CALLBACK(on_load_failed), NULL);
    g_signal_connect(browser.view, "load-failed-with-tls-errors", G_CALLBACK(on_tls_errors), NULL);
    g_signal_connect(browser.view, "authenticate", G_CALLBACK(on_authenticate), NULL);
    g_signal_connect(browser.view, "create", G_CALLBACK(on_create), NULL);
    g_signal_connect(browser.view, "script-dialog", G_CALLBACK(on_script_dialog), NULL);
    g_signal_connect(browser.view, "permission-request", G_CALLBACK(on_permission_request), NULL);
    g_signal_connect(browser.view, "notify::title", G_CALLBACK(on_title), NULL);
    g_signal_connect(browser.view, "web-process-terminated", G_CALLBACK(on_web_process_terminated), NULL);
    g_signal_connect(browser.view, "decide-policy", G_CALLBACK(on_decide_policy), NULL);
    g_signal_connect(browser.view, "close", G_CALLBACK(on_close), NULL);

    WPEView *view = wpe_view();
    if (view) {
        WPEToplevel *toplevel = wpe_view_get_toplevel(view);
        if (toplevel) {
            int w, h;
            if (size && sscanf(size, "%dx%d", &w, &h) == 2 && w > 0 && h > 0)
                wpe_toplevel_resize(toplevel, w, h);
            wpe_toplevel_set_title(toplevel, browser.name);
        }
        g_signal_connect(view, "closed", G_CALLBACK(on_view_closed), NULL);
        wpe_view_focus_in(view);
    }

    browser.wpe_view = view;
    /* the browser's input is a mouse's and a keyboard's: pages are told so before their first
     * process starts (a display that says none answers (pointer: none) and (hover: none), as no
     * desktop browser does) */
    WPEDisplay *display = view ? wpe_view_get_display(view) : wpe_display_get_primary();
    if (display)
        wpe_display_set_available_input_devices(display, WPE_AVAILABLE_INPUT_DEVICE_MOUSE | WPE_AVAILABLE_INPUT_DEVICE_KEYBOARD);
    char *start = normalize_uri(url ? url : "about:blank");
    webkit_web_view_load_uri(browser.view, start);
    g_free(start);
    /* the page has a process now (it starts with the load): the developer tools can be on from its
     * first request (its messages come before the load's own, which waits for a policy decision) */
    browser.page_started = TRUE;
    settings_changed();
    /* a first page that does not end still lets the browser be driven */
    g_timeout_add_seconds(60, start_serving_anyway, NULL);

    browser.loop = g_main_loop_new(NULL, FALSE);
    g_main_loop_run(browser.loop);

    g_socket_service_stop(browser.service);
    unlink(browser.socket_path);
    if (browser.net_socket_path)
        unlink(browser.net_socket_path);
    wpe_tls_untrust_all(browser.ca_dir);
    g_rmdir(browser.ca_dir);
    history_save();
    devtools_off();
    /* the web and network processes end when their connections close */
    g_object_unref(browser.view);
    return 0;
}
