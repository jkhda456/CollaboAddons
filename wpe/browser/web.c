/*
 * web: drive the sandbox's web browser (WPE WebKit, the wpe add-on) from the command line — what an
 * agent uses to browse. Each command talks to a running browser (wpe-browser) through its socket,
 * and starts one first if there is none. `web help` lists the commands; `web --agent-help` is a
 * guide for an LLM agent (agent-help.txt).
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "agent-help.h" /* agent_help_txt: agent-help.txt, by gen-js.py */

extern char **environ;

static const char *help_text =
    "web: browse the web from the sandbox (WPE WebKit). A browser starts by itself the first time\n"
    "(it shows on the guest's display as the program NAME, `web` by default: gui list, gui screenshot).\n"
    "\n"
    "usage: web [-n NAME] [BROWSER OPTION...] COMMAND [ARG...]\n"
    "       web --agent-help [TOPIC...]   a guide for agents: how to work with it, real examples,\n"
    "                                     pitfalls, recipes (web --agent-help topics)\n"
    "\n"
    "Pages\n"
    "  open URL [TIMEOUT]       load a page (words: a web search); waits until it has loaded\n"
    "  back | forward | reload  history; waits for the page\n"
    "  info                     url, title, loading, history, size, last error or dialog\n"
    "  history [N | --all] [TEXT]   the pages visited, the last N (30): time, title, address;\n"
    "                           TEXT: those whose address or title has it; kept across restarts\n"
    "                           (not by --ephemeral); history clear forgets them\n"
    "  wait [load | text STR | selector CSS | gone CSS] [TIMEOUT]   wait for the page (seconds)\n"
    "\n"
    "Reading\n"
    "  read [MAX] [--all]       the page as text, every link, button and field numbered [N]\n"
    "  text                     the page's text (innerText)\n"
    "  links                    every link: text and address\n"
    "  html [CSS]               the HTML of the page or of an element\n"
    "  eval JS                  run JavaScript in the page; prints the result (JSON)\n"
    "  run [--timeout S] FILE|- [ARG...]   run a JavaScript file (-: stdin) in the page as the body\n"
    "                           of an async function: `return` gives the result (JSON), `await`\n"
    "                           waits (up to S seconds, 120), `args` holds the ARGs (strings)\n"
    "  screenshot [FILE] [--max-width N]   the page as PNG (default /tmp/NAME.png)\n"
    "\n"
    "Acting (TARGET: a number from `web read`, a CSS selector, or a link's or button's text)\n"
    "  click TARGET | X Y       click (waits for a page it opens); also dblclick, rightclick\n"
    "  type TEXT [TARGET]       type into TARGET (clicked first) or what has the focus\n"
    "  key KEY...               Enter, Tab, Escape, ArrowDown, Backspace, ctrl+a, shift+Tab...\n"
    "  select TARGET VALUE      pick an option of a <select>\n"
    "  hover TARGET | X Y       move the pointer there\n"
    "  scroll [down|up|left|right [N] | top | bottom]\n"
    "  ... --screenshot FILE    with any of these (and open, back, forward, reload, wait): a PNG of\n"
    "                           the page with where it acted marked — a ring at the point, a frame\n"
    "                           around the element, a caption (click (512, 300) · button \"Go\"); a\n"
    "                           click's as it goes in, the others' once done; scroll: an arrow\n"
    "\n"
    "How the browser presents itself (from the next request on; `web reload` for this page)\n"
    "  ua [PROFILE | 'USER AGENT' | list]   the user agent: default, safari, chrome, edge, firefox,\n"
    "                           iphone, android, or any string; the page's navigator (platform,\n"
    "                           vendor, userAgentData) and Chrome's Sec-CH-UA headers follow it\n"
    "  header [list]            the headers put on every request (the page, its parts, fetch)\n"
    "  header set NAME VALUE [--host HOST]   add or replace one; --host: only to HOST and its\n"
    "                           subdomains; an empty VALUE takes the header off the requests\n"
    "  header unset NAME [--host HOST] | header clear\n"
    "  lang [LANGUAGES]         preferred languages (ko-KR,en): Accept-Language, navigator.languages\n"
    "  cookie [list [URL] [--all]]   the cookies for URL (default the page's)\n"
    "  cookie set 'NAME=VALUE[; Domain=..; Path=..; Max-Age=..; Secure; HttpOnly]'... [--url URL]\n"
    "  cookie delete NAME [--url URL | --all] | cookie clear\n"
    "  inject [list] | inject add FILE|- | inject remove FILE | inject clear\n"
    "                           JavaScript run at the start of every page, before the page's own\n"
    "  settings                 all of the above\n"
    "\n"
    "Developer tools: the network, as a browser's developer tools record it (off by default)\n"
    "  devtools [on [--bodies text|all|none] | off | toggle]   record every request the page\n"
    "                           and its frames and workers make: headers as sent (cookies too),\n"
    "                           request and response bodies (text ones by default), timings, the\n"
    "                           server's address, WebSocket messages; devtools: on or off\n"
    "  network [TEXT] [--type T] [--status S] [--failed] [--last N | --all]   the requests\n"
    "                           recorded (TEXT: in the address; T: document, script, xhr, fetch...)\n"
    "  network show N|TEXT      one in detail: headers, query, bodies, timing, initiator\n"
    "  network body N|TEXT [FILE]   its response body (a binary one into FILE)\n"
    "  network search TEXT [--case] [--regex] [--in url,headers,body,messages] [--url T2]\n"
    "                           [--type T] [--status S] [--failed] [--max N]   TEXT in every\n"
    "                           request's address, headers, bodies, WebSocket messages\n"
    "  network watch [TEXT...] [--timeout S]   each request as it finishes (devtools on first)\n"
    "  network clear            forget what was recorded\n"
    "  har [FILE] [--no-bodies]   save it as an HTTP Archive, HAR 1.2 (default /tmp/NAME.har)\n"
    "  har view FILE [N | TEXT...]   a HAR file of any browser: its requests, or one in detail\n"
    "  har search FILE TEXT [OPTION...]   search a HAR file as network search does\n"
    "  network export N... [--as FORMAT] [-o FILE] [--no-cookies]   requests as commands or\n"
    "                           files of other tools, printed or into FILE: curl (the default),\n"
    "                           fetch (a browser's console), node, python (urllib), http (the\n"
    "                           .http of REST clients), raw (the HTTP message: Burp, nc), har;\n"
    "                           N: a number, N-M, all, or a part of the address (the last with\n"
    "                           it); --type T, --status S, --failed narrow them\n"
    "  har export FILE N... [--as FORMAT] [-o OUT]   the same from a HAR file of any browser\n"
    "\n"
    "Intercept: the page's requests wait for approval, each sent, dropped or answered here\n"
    "  intercept on [TEXT...] [--regex] [--case] [--type T[,T]] [--method M[,M]]\n"
    "               [--stage request|response|both]   from now on, the requests the page\n"
    "                           makes (TEXT: whose address has it; T: document, script, fetch...)\n"
    "                           pause before they go (response: their responses, before the\n"
    "                           page gets them); turns the developer tools on (also: pause)\n"
    "  intercept [status] | off   the rules; off: the paused ones go as they are\n"
    "  intercept list           the paused requests (N: the number web network has too)\n"
    "  intercept show N         one: what would go (method, address, headers, body), or the\n"
    "                           response that would reach the page (status, headers)\n"
    "  intercept send N... [--url U] [--method M] [--header 'NAME: VALUE']... [--unset NAME]...\n"
    "                 [--body TEXT | --body-file FILE]   let it go, changed so; a response:\n"
    "                           [--status S] [--header ...] [--mime TYPE] --body TEXT\n"
    "  intercept drop N... [--error general|access|cancel|timeout]   stop it: the page gets a\n"
    "                           network error (also: stop)\n"
    "  intercept respond N... [--status S] [--header 'NAME: VALUE']... [--mime TYPE]\n"
    "                 [--body TEXT | --body-file FILE]   answer it here, nothing sent\n"
    "  intercept wait [TIMEOUT]   until a request pauses (seconds, 30)\n"
    "                           N: a number, all, or a part of the address. While requests are\n"
    "                           paused, a command waiting on the page (open, click, wait, run)\n"
    "                           ends with them listed, exit 6; web wait load after deciding\n"
    "\n"
    "Browsers\n"
    "  list                     the running browsers\n"
    "  resize WxH               the page's size\n"
    "  close                    end the browser NAME\n"
    "\n"
    "Browser options (before COMMAND): the browser this command starts gets them; one running\n"
    "already gets the settings among them (the others are for a start: `web close` first)\n"
    "  --user-agent PROFILE|UA   --header 'NAME: VALUE'   --header-for HOST 'NAME: VALUE'\n"
    "  --lang LANGUAGES   --inject FILE   --devtools   (settings: each but --user-agent and\n"
    "  --devtools can be given more than once)\n"
    "  --ephemeral (no cookies or storage kept)   --ignore-tls-errors   --size WxH\n"
    "\n"
    "Exit status: 0 done, 1 failed, 2 usage, 3 timed out, 4 not found, 5 no browser,\n"
    "6 requests are paused for approval (web intercept).\n"
    "WEB_START_TIMEOUT (seconds, 300) bounds the start of a browser; WEB_BROWSER_ARGS adds\n"
    "options to it (words). The first settings of a browser this command starts, unless options\n"
    "say: WEB_USER_AGENT (a profile or a user agent), WEB_HEADERS (lines NAME: VALUE), WEB_LANG,\n"
    "WEB_DEVTOOLS (1, or the bodies kept: text, all, none).\n"
    "WPE_BROWSER_DEBUG=1 logs its loads (`gui logs NAME`).\n";

static const char *name = "web";

/* browser options from the command line: settings (given to a running browser too: `set`), and
 * the ones only a start takes */
static char **settings, **start_options;
static int settings_count, start_count;

static void add_word(char ***list, int *count, const char *word)
{
    *list = realloc(*list, (*count + 2) * sizeof **list);
    (*list)[(*count)++] = (char *)word;
    (*list)[*count] = NULL;
}

static void die(int status, const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    fputs("web: ", stderr);
    vfprintf(stderr, format, ap);
    fputc('\n', stderr);
    va_end(ap);
    exit(status);
}

static const char *socket_dir(void)
{
    const char *dir = getenv("COLLABO_WEB_DIR");
    return dir && *dir ? dir : "/tmp/.collabo-web";
}

static int connect_to(const char *who)
{
    struct sockaddr_un address = { .sun_family = AF_UNIX };
    snprintf(address.sun_path, sizeof address.sun_path, "%s/%s.sock", socket_dir(), who);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return -1;
    if (connect(fd, (struct sockaddr *)&address, sizeof address) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static void write_all(int fd, const void *data, size_t size)
{
    const char *p = data;
    while (size) {
        ssize_t n = write(fd, p, size);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            die(1, "lost the browser while sending: %s", strerror(errno));
        p += n;
        size -= n;
    }
}

static void send_words(int fd, int argc, char **argv)
{
    char head[32];
    snprintf(head, sizeof head, "%d\n", argc);
    write_all(fd, head, strlen(head));
    for (int i = 0; i < argc; i++) {
        snprintf(head, sizeof head, "%zu\n", strlen(argv[i]));
        write_all(fd, head, strlen(head));
        write_all(fd, argv[i], strlen(argv[i]));
    }
}

/* The answer to the words sent: "STATUS LENGTH\n" then LENGTH bytes. out NULL: into *text
 * (malloc'd); else printed there (stderr for a failure), unless quiet. Returns the status. */
static int read_answer(int fd, FILE *out, char **text, int quiet)
{
    char line[64];
    size_t used = 0;
    while (used < sizeof line - 1) {
        ssize_t n = read(fd, line + used, 1);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            die(1, "the browser went away before answering (did it crash? `gui logs %s`)", name);
        if (line[used++] == '\n')
            break;
    }
    line[used] = 0;
    int status = 1;
    size_t length = 0;
    if (sscanf(line, "%d %zu", &status, &length) != 2)
        die(1, "the browser answered something else: %s", line);
    if (out && status)
        out = stderr;
    size_t got = 0;
    if (text)
        *text = malloc(length + 1);
    char buffer[65536];
    while (got < length) {
        char *into = text ? *text + got : buffer;
        size_t want = length - got;
        if (!text && want > sizeof buffer)
            want = sizeof buffer;
        ssize_t n = read(fd, into, want);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            break;
        if (out && !quiet)
            fwrite(into, 1, n, out);
        got += n;
    }
    if (text)
        (*text)[got] = 0;
    if (out)
        fflush(out);
    return status;
}

/* Sends the words, prints the answer; returns its status. */
static int request(int fd, int argc, char **argv, int quiet)
{
    send_words(fd, argc, argv);
    return read_answer(fd, stdout, NULL, quiet);
}

/* Runs argv; quiet: its stdout to /dev/null (its errors still show) */
static int run_quietly(char *const argv[], int wait_for_it, int quiet)
{
    pid_t pid;
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    if (quiet)
        posix_spawn_file_actions_addopen(&actions, 1, "/dev/null", O_WRONLY, 0);
    int error = posix_spawnp(&pid, argv[0], &actions, NULL, argv, environ);
    posix_spawn_file_actions_destroy(&actions);
    if (error)
        return -1;
    if (!wait_for_it)
        return 0;
    int status;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {}
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}

static int run(char *const argv[], int wait_for_it)
{
    return run_quietly(argv, wait_for_it, 0);
}

static int have(const char *program)
{
    const char *path = getenv("PATH");
    if (!path)
        path = "/usr/bin:/bin";
    char *copy = strdup(path), *save = NULL;
    for (char *dir = strtok_r(copy, ":", &save); dir; dir = strtok_r(NULL, ":", &save)) {
        char file[4096];
        snprintf(file, sizeof file, "%s/%s", dir, program);
        if (!access(file, X_OK)) {
            free(copy);
            return 1;
        }
    }
    free(copy);
    return 0;
}

/* An absolute path for `path` (the browser runs elsewhere), its links resolved when it exists */
static char *absolute(const char *path)
{
    char *real = realpath(path, NULL);
    if (real)
        return real;
    if (path[0] == '/')
        return strdup(path);
    char cwd[4096];
    if (!getcwd(cwd, sizeof cwd))
        return strdup(path);
    char *full = malloc(strlen(cwd) + strlen(path) + 2);
    sprintf(full, "%s/%s", cwd, path);
    return full;
}

/* The text of a file ("-": stdin), for the browser to run */
static char *read_script(const char *path)
{
    FILE *in = strcmp(path, "-") ? fopen(path, "rb") : stdin;
    if (!in)
        die(2, "cannot read %s: %s", path, strerror(errno));
    size_t size = 0, capacity = 65536;
    char *text = malloc(capacity + 1);
    size_t n;
    while ((n = fread(text + size, 1, capacity - size, in)) > 0) {
        size += n;
        if (size == capacity)
            text = realloc(text, (capacity *= 2) + 1);
    }
    if (ferror(in))
        die(1, "cannot read %s: %s", path, strerror(errno));
    if (in != stdin)
        fclose(in);
    text[size] = 0;
    if (strlen(text) != size)
        die(2, "%s has NUL bytes: not a script", path);
    return text;
}

/* A file as base64 (any bytes: a body `web intercept` gives the page or sends) */
static char *read_base64(const char *path)
{
    static const char digits[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    FILE *in = strcmp(path, "-") ? fopen(path, "rb") : stdin;
    if (!in)
        die(2, "cannot read %s: %s", path, strerror(errno));
    size_t size = 0, capacity = 65536;
    unsigned char *data = malloc(capacity);
    size_t n;
    while ((n = fread(data + size, 1, capacity - size, in)) > 0) {
        size += n;
        if (size == capacity)
            data = realloc(data, capacity *= 2);
    }
    if (ferror(in))
        die(1, "cannot read %s: %s", path, strerror(errno));
    if (in != stdin)
        fclose(in);
    if (size > 45u * 1024 * 1024) /* the browser takes a word of 64 MiB at most */
        die(2, "%s is too large to send (%zu MB; 45 at most)", path, size >> 20);
    char *text = malloc((size + 2) / 3 * 4 + 1), *out = text;
    for (size_t i = 0; i < size; i += 3) {
        unsigned v = data[i] << 16 | (i + 1 < size ? data[i + 1] << 8 : 0) | (i + 2 < size ? data[i + 2] : 0);
        *out++ = digits[v >> 18 & 63];
        *out++ = digits[v >> 12 & 63];
        *out++ = i + 1 < size ? digits[v >> 6 & 63] : '=';
        *out++ = i + 2 < size ? digits[v & 63] : '=';
    }
    *out = 0;
    free(data);
    return text;
}

/* Starts a browser named `name`: on the display (gui run) when there is one, else on its own. */
static void start_browser(void)
{
    char **args = calloc(48 + settings_count + start_count, sizeof *args);
    int n = 0;
    char *extra = getenv("WEB_BROWSER_ARGS") ? strdup(getenv("WEB_BROWSER_ARGS")) : NULL;
    if (have("gui")) {
        args[n++] = "gui";
        args[n++] = "run";
        args[n++] = "--name";
        args[n++] = (char *)name;
        args[n++] = "--";
    }
    args[n++] = "wpe-browser";
    args[n++] = "--name";
    args[n++] = (char *)name;
    if (extra) {
        char *save = NULL;
        for (char *word = strtok_r(extra, " ", &save); word && n < 40; word = strtok_r(NULL, " ", &save))
            args[n++] = word;
    }
    /* the command line's after WEB_BROWSER_ARGS: a later --user-agent wins */
    for (int i = 0; i < start_count; i++)
        args[n++] = start_options[i];
    for (int i = 0; i < settings_count; i++)
        args[n++] = settings[i];
    args[n] = NULL;
    if (!strcmp(args[0], "gui")) {
        int status = run_quietly(args, 1, 1);
        if (status == 0)
            return;
        fprintf(stderr, "web: gui run failed (%d); starting the browser without a display\n", status);
        memmove(args, args + 5, (n - 4) * sizeof *args);
    }
    /* on its own: in the background, its output in a log beside the socket */
    char log[4096];
    snprintf(log, sizeof log, "%s/%s.log", socket_dir(), name);
    mkdir(socket_dir(), 0700);
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0);
    posix_spawn_file_actions_addopen(&actions, 1, log, O_WRONLY | O_CREAT | O_APPEND, 0600);
    posix_spawn_file_actions_adddup2(&actions, 1, 2);
    posix_spawnattr_t attributes;
    posix_spawnattr_init(&attributes);
    posix_spawnattr_setflags(&attributes, POSIX_SPAWN_SETSID);
    pid_t pid;
    if (posix_spawnp(&pid, "wpe-browser", &actions, &attributes, args, environ))
        die(5, "cannot start wpe-browser: %s (is the wpe add-on on?)", strerror(errno));
}

/* *started: whether this command started it (with the options) */
static int connect_or_start(int *started)
{
    int fd = connect_to(name);
    *started = fd < 0;
    if (fd >= 0)
        return fd;
    start_browser();
    const char *limit = getenv("WEB_START_TIMEOUT");
    double timeout = limit ? atof(limit) : 300;
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    int said = 0;
    for (;;) {
        fd = connect_to(name);
        if (fd >= 0)
            return fd;
        clock_gettime(CLOCK_MONOTONIC, &now);
        double elapsed = (now.tv_sec - start.tv_sec) + (now.tv_nsec - start.tv_nsec) / 1e9;
        if (elapsed > timeout)
            die(5, "the browser did not start within %.0f s (gui logs %s)", timeout, name);
        if (!said && elapsed > 3) {
            fprintf(stderr, "web: starting the browser \"%s\" (its first start on this computer compiles it: about 20 s)...\n", name);
            said = 1;
        }
        usleep(200000);
    }
}

static int cmd_list(void)
{
    DIR *dir = opendir(socket_dir());
    int found = 0;
    if (dir) {
        struct dirent *entry;
        while ((entry = readdir(dir))) {
            size_t length = strlen(entry->d_name);
            if (length < 6 || strcmp(entry->d_name + length - 5, ".sock"))
                continue;
            char who[256];
            snprintf(who, sizeof who, "%.*s", (int)(length - 5), entry->d_name);
            int fd = connect_to(who);
            if (fd < 0)
                continue;
            char *words[] = { "url" };
            printf("%s\t", who);
            fflush(stdout);
            request(fd, 1, words, 0);
            close(fd);
            found++;
        }
        closedir(dir);
    }
    if (!found)
        printf("no browser is running (web open URL starts one)\n");
    return 0;
}

static double now_seconds(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

/* network watch [WORDS...] [--timeout S]: what finishes from now on, as it does (the browser's
 * `network poll CURSOR WORDS...` four times a second) */
static int cmd_watch(int argc, char **argv)
{
    double timeout = -1;
    char **poll = calloc(argc + 4, sizeof *poll);
    int n = 0;
    poll[n++] = "network";
    poll[n++] = "poll";
    poll[n++] = NULL; /* the cursor */
    for (int i = 2; i < argc; i++) {
        if (!strcmp(argv[i], "--timeout") && i + 1 < argc)
            timeout = atof(argv[++i]);
        else
            poll[n++] = argv[i];
    }
    int started;
    int fd = connect_or_start(&started);
    char *on[] = { "devtools", "on" };
    send_words(fd, 2, on);
    char *answer = NULL;
    int status = read_answer(fd, NULL, &answer, 1);
    close(fd);
    if (status) {
        fputs(answer, stderr);
        return status;
    }
    if (strstr(answer, "web network lists")) /* it was off */
        fprintf(stderr, "web: the developer tools were off: on now, recording from here\n");
    free(answer);
    char cursor[32] = "-1";
    double until = timeout >= 0 ? now_seconds() + timeout : 0;
    for (;;) {
        fd = connect_to(name);
        if (fd < 0)
            die(5, "lost the browser \"%s\"", name);
        poll[2] = cursor;
        send_words(fd, n, poll);
        status = read_answer(fd, NULL, &answer, 1);
        close(fd);
        if (status) {
            fputs(answer, stderr);
            return status;
        }
        /* "CURSOR\n" and the requests that finished since the one before */
        char *rows = strchr(answer, '\n');
        snprintf(cursor, sizeof cursor, "%.*s", rows ? (int)(rows - answer) : 0, answer);
        if (rows && rows[1]) {
            fputs(rows + 1, stdout);
            fflush(stdout);
        }
        free(answer);
        if (until && now_seconds() >= until)
            return 0;
        usleep(250000);
    }
}

static int cmd_screenshot(int argc, char **argv)
{
    char file[4096];
    snprintf(file, sizeof file, "/tmp/%s.png", name);
    char *args[16];
    int n = 0;
    args[n++] = "gui";
    args[n++] = "screenshot";
    args[n++] = (char *)name;
    args[n++] = "--idle";
    args[n++] = "300";
    for (int i = 1; i < argc && n < 12; i++) {
        if (argv[i][0] != '-' && (i == 1 || argv[i - 1][0] != '-'))
            snprintf(file, sizeof file, "%s", argv[i]);
        else
            args[n++] = argv[i];
    }
    args[n++] = "-o";
    args[n++] = file;
    args[n] = NULL;
    if (!have("gui"))
        die(1, "no display (the gui command of the tools image) to take a screenshot from; `web read` reads the page as text");
    int started;
    int fd = connect_or_start(&started); /* the browser must be there */
    close(fd);
    int status = run(args, 1);
    if (status == 0)
        printf("%s\n", file);
    return status;
}

/* web --agent-help [TOPIC...]: the guide (agent-help.txt), or the parts of it named — a part starts
 * with a line "## NAME: TITLE", and a NAME's first letters name it too — or `topics`: the list */
static int agent_help(int argc, char **argv)
{
    if (!argc) {
        fputs(agent_help_txt, stdout);
        return 0;
    }
    int listing = argc == 1 && !strcmp(argv[0], "topics");
    for (int i = 0; i < argc; i++) {
        int found = 0;
        const char *next;
        for (const char *part = strstr(agent_help_txt, "\n## "); part; part = next) {
            part++;
            next = strstr(part, "\n## ");
            const char *name = part + 3;
            size_t length = strcspn(name, ":\n"), wanted = strlen(argv[i]);
            if (listing) {
                const char *title = name + length + (name[length] == ':' ? 2 : 0);
                printf("  %-11.*s %.*s\n", (int)length, name, (int)strcspn(title, "\n"), title);
                continue;
            }
            if (!wanted || wanted > length || strncmp(name, argv[i], wanted))
                continue;
            fwrite(part, 1, next ? (size_t)(next - part + 1) : strlen(part), stdout);
            found = 1;
        }
        if (!found && !listing)
            die(2, "no topic \"%s\" in the guide (web --agent-help topics lists them)", argv[i]);
    }
    if (listing)
        printf("(web --agent-help TOPIC... prints those parts; web --agent-help, all of it)\n");
    return 0;
}

int main(int argc, char **argv)
{
    int i = 1;
    for (; i < argc && argv[i][0] == '-'; i++) {
        const char *option = argv[i];
        int values = !strcmp(option, "--header-for") ? 2
                   : !strcmp(option, "--user-agent") || !strcmp(option, "--header") || !strcmp(option, "--lang")
                     || !strcmp(option, "--inject") || !strcmp(option, "--size") ? 1 : 0;
        if (values && i + values >= argc)
            die(2, "%s needs %s", option, values == 2 ? "a host and 'NAME: VALUE'" : "a value");
        if ((!strcmp(option, "-n") || !strcmp(option, "--name")) && i + 1 < argc)
            name = argv[++i];
        else if (!strcmp(option, "--size")) {
            add_word(&start_options, &start_count, option);
            add_word(&start_options, &start_count, argv[++i]);
        } else if (!strcmp(option, "--ephemeral") || !strcmp(option, "--ignore-tls-errors"))
            add_word(&start_options, &start_count, option);
        else if (!strcmp(option, "--devtools"))
            add_word(&settings, &settings_count, option);
        else if (!strcmp(option, "--inject")) {
            add_word(&settings, &settings_count, option);
            add_word(&settings, &settings_count, absolute(argv[++i]));
        } else if (values) {
            add_word(&settings, &settings_count, option);
            for (int k = 0; k < values; k++)
                add_word(&settings, &settings_count, argv[++i]);
        } else if (!strcmp(option, "-h") || !strcmp(option, "--help")) {
            fputs(help_text, stdout);
            return 0;
        } else if (!strcmp(option, "--agent-help"))
            return agent_help(argc - i - 1, argv + i + 1);
        else
            die(2, "unknown option %s (web help)", argv[i]);
    }
    if (i >= argc || !strcmp(argv[i], "help")) {
        fputs(help_text, i >= argc ? stderr : stdout);
        return i >= argc ? 2 : 0;
    }
    if (!strcmp(argv[i], "agent-help"))
        return agent_help(argc - i - 1, argv + i + 1);
    if (!getenv("COLLABO_WEB_NAME") && strchr(name, '/'))
        die(2, "a browser's name has no '/'");
    char **words = argv + i;
    int count = argc - i;
    const char *command = words[0];
    signal(SIGPIPE, SIG_IGN);

    /* --screenshot FILE (or =FILE): the browser writes it, so its path as the caller means it */
    static const char *const shot_commands[] = { "click", "dblclick", "rightclick", "hover", "type", "select", "key", "press",
                                                 "scroll", "open", "goto", "back", "forward", "reload", "wait", NULL };
    int shot_command = 0;
    for (const char *const *c = shot_commands; *c; c++)
        shot_command |= !strcmp(command, *c);
    char **shot_words = calloc(2 * count + 1, sizeof *shot_words);
    int shot_count = 0;
    for (int j = 0; j < count; j++) {
        const char *word = words[j];
        if (!j || (strcmp(word, "--screenshot") && strncmp(word, "--screenshot=", 13))) {
            shot_words[shot_count++] = words[j];
            continue;
        }
        if (!shot_command)
            die(2, "--screenshot is for click, dblclick, rightclick, hover, type, select, key, scroll, open, back, forward,"
                   " reload and wait (web screenshot takes a picture of the page as it is)");
        const char *file = word[12] == '=' ? word + 13 : j + 1 < count ? words[++j] : "";
        if (!*file)
            die(2, "--screenshot needs a file (a PNG is written there)");
        shot_words[shot_count++] = "--screenshot";
        shot_words[shot_count++] = absolute(file);
    }
    words = shot_words;
    count = shot_count;

    if (!strcmp(command, "list"))
        return cmd_list();
    if (!strcmp(command, "screenshot"))
        return cmd_screenshot(count, words);
    if (!strcmp(command, "view") || !strcmp(command, "logs")) {
        char *args[] = { "gui", (char *)command, (char *)name, NULL };
        return run(args, 1);
    }
    if (!strcmp(command, "close") || !strcmp(command, "quit") || !strcmp(command, "stop")) {
        int fd = connect_to(name);
        if (fd < 0) {
            printf("no browser \"%s\" is running\n", name);
            return 0;
        }
        char *close_words[] = { "close" };
        return request(fd, 1, close_words, 0);
    }
    /* scripts: web reads the file (the caller's: its directory, its stdin) and sends the text */
    if (!strcmp(command, "run")) {
        int k = 1;
        const char *timeout = "";
        if (k + 1 < count && !strcmp(words[k], "--timeout")) {
            timeout = words[k + 1];
            k += 2;
        }
        if (k >= count)
            die(2, "usage: web run [--timeout SECONDS] FILE|- [ARG...]");
        char **run = calloc(count + 4, sizeof *run);
        run[0] = "run";
        run[1] = strcmp(words[k], "-") ? absolute(words[k]) : "stdin";
        run[2] = read_script(words[k]);
        run[3] = (char *)timeout;
        int n = 4;
        for (int j = k + 1; j < count; j++)
            run[n++] = words[j];
        words = run;
        count = n;
    } else if ((!strcmp(command, "network") || !strcmp(command, "net")) && count > 1 && !strcmp(words[1], "watch")) {
        return cmd_watch(count, words);
    } else if ((!strcmp(command, "network") || !strcmp(command, "net")) && count == 4 && !strcmp(words[1], "body")) {
        words[3] = absolute(words[3]); /* the browser writes it */
    } else if ((!strcmp(command, "network") || !strcmp(command, "net")) && count > 1 && !strcmp(words[1], "export")) {
        for (int j = 2; j + 1 < count; j++) {
            if (!strcmp(words[j], "-o") || !strcmp(words[j], "--output"))
                words[j + 1] = absolute(words[j + 1]), j++;
        }
    } else if (!strcmp(command, "har")) {
        /* har [save] [FILE] [--no-bodies] | har view FILE [WORDS...]: web reads and writes the files' names
         * as the caller means them */
        int k = 1;
        char **har = calloc(count + 4, sizeof *har);
        har[0] = "har";
        if (count > 1 && (!strcmp(words[1], "view") || !strcmp(words[1], "search") || !strcmp(words[1], "export"))) {
            if (count < (strcmp(words[1], "view") ? 4 : 3))
                die(2, "usage: web har view FILE [N | TEXT ...] | web har search FILE TEXT [OPTION...] | web har export FILE WHICH... [--as FORMAT] [-o OUT]");
            har[1] = words[1];
            har[2] = absolute(words[2]);
            har[3] = read_script(words[2]);
            if (strlen(har[3]) > 60u * 1024 * 1024) /* the browser takes a word of 64 MiB at most */
                die(2, "%s is too large to view here (%zu MB; 60 at most)", words[2], strlen(har[3]) >> 20);
            int n = 4;
            for (int j = 3; j < count; j++) {
                har[n++] = words[j];
                if ((!strcmp(words[j], "-o") || !strcmp(words[j], "--output")) && j + 1 < count)
                    har[n++] = absolute(words[++j]);
            }
            words = har;
            count = n;
        } else {
            if (count > 1 && !strcmp(words[1], "save"))
                k = 2;
            char file[4096];
            snprintf(file, sizeof file, "/tmp/%s.har", name);
            const char *path = file;
            int n = 3;
            for (int j = k; j < count; j++) {
                if (words[j][0] == '-')
                    har[n++] = words[j];
                else
                    path = words[j];
            }
            har[1] = "save";
            har[2] = absolute(path);
            words = har;
            count = n;
        }
    } else if (!strcmp(command, "intercept")) {
        /* --body-file FILE (-: stdin): web reads it, the browser gets its bytes as base64 */
        for (int j = 2; j + 1 < count; j++) {
            if (!strcmp(words[j], "--body-file")) {
                words[j] = "--body64";
                words[j + 1] = read_base64(words[j + 1]);
                j++;
            }
        }
    } else if (!strcmp(command, "inject") && count > 1 && (!strcmp(words[1], "add") || !strcmp(words[1], "remove"))) {
        int adding = !strcmp(words[1], "add");
        if (count != 3)
            die(2, "usage: web inject %s FILE%s", words[1], adding ? "|-" : "");
        char **inject = calloc(5, sizeof *inject);
        inject[0] = "inject";
        inject[1] = words[1];
        inject[2] = strcmp(words[2], "-") ? absolute(words[2]) : "stdin";
        if (adding)
            inject[3] = read_script(words[2]);
        words = inject;
        count = adding ? 4 : 3;
    }

    int started;
    int fd = connect_or_start(&started);
    if (!started && settings_count) {
        /* running already: the settings given now apply to it as well */
        char **set = calloc(settings_count + 2, sizeof *set);
        set[0] = "set";
        memcpy(set + 1, settings, settings_count * sizeof *set);
        int status = request(fd, settings_count + 1, set, 0);
        close(fd);
        if (status)
            return status;
        fd = connect_to(name);
        if (fd < 0)
            die(5, "lost the browser \"%s\"", name);
    }
    if (!started && start_count)
        fprintf(stderr, "web: the browser \"%s\" is running already; %s is for its start (web close first)\n", name, start_options[0]);
    int status = request(fd, count, words, 0);
    close(fd);
    return status;
}
