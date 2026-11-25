/*
 * The network process's half of `web intercept` (WebKit patch 0013, CollaboNetIntercept.h): a
 * socket to the browser that started it, over which WebKit's network process asks what to do with
 * what it is about to send, receive or give a page — and the browser (devtools.js) answers.
 *
 * The browser listens on $WPE_BROWSER_NET_SOCKET (browser.c sets it before WebKit starts its
 * network process, which inherits it); each browser (-n NAME) has its own network process and
 * socket, so nothing of one reaches another. Lines, each a JSON object after a letter:
 *
 *   network process → browser   E ID JSON   held: decide it        N JSON   a note (recorded only)
 *                                F ID        what ID held went away   A SERIAL the settings taken
 *   browser → network process   C MASK SERIAL   what to hold now (CollaboNet::Want's bits)
 *                                D ID JSON       the decision for ID
 *
 * Everything runs on the network process's main thread (its GLib main context). Writes are
 * queued and sent as the socket takes them: a body can be large, and the browser may be writing
 * to us at the same time.
 */
#include <errno.h>
#include <fcntl.h>
#include <glib-unix.h>
#include <glib.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

typedef void (*NetDecided)(void *context, const char *decision);

typedef struct {
    NetDecided decided;
    void *context;
} Held;

static struct {
    gboolean tried;
    int fd;
    unsigned wants;
    GHashTable *held;          /* id → Held* */
    unsigned long long next_id;
    GString *in, *out;
    guint read_id, write_id;
} net = { .fd = -1 };

static void net_close(void);

static gboolean net_writable(int fd, GIOCondition condition, gpointer data)
{
    (void)condition;
    (void)data;
    while (net.out->len) {
        ssize_t n = send(fd, net.out->str, net.out->len, MSG_NOSIGNAL); /* no SIGPIPE: a browser gone is a closed socket */
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
            return G_SOURCE_CONTINUE;
        if (n <= 0) {
            net.write_id = 0;
            net_close();
            return G_SOURCE_REMOVE;
        }
        g_string_erase(net.out, 0, n);
    }
    net.write_id = 0;
    return G_SOURCE_REMOVE;
}

static void net_send(const char *text)
{
    if (net.fd < 0)
        return;
    g_string_append(net.out, text);
    if (!net.write_id)
        net.write_id = g_unix_fd_add(net.fd, G_IO_OUT, net_writable, NULL);
}

/* a decision (or, decision NULL: none will come, go on) for what id held */
static void net_decide(unsigned long long id, const char *decision)
{
    Held *held = g_hash_table_lookup(net.held, &id);
    if (!held)
        return;
    Held copy = *held;
    g_hash_table_remove(net.held, &id);
    copy.decided(copy.context, decision);
}

/* the browser is gone: what was held goes on as it was, nothing more is held */
static void net_close(void)
{
    if (net.fd < 0)
        return;
    if (net.read_id)
        g_source_remove(net.read_id);
    if (net.write_id)
        g_source_remove(net.write_id);
    net.read_id = net.write_id = 0;
    close(net.fd);
    net.fd = -1;
    net.wants = 0;
    GList *ids = g_hash_table_get_keys(net.held);
    GList *copies = NULL;
    for (GList *l = ids; l; l = l->next)
        copies = g_list_prepend(copies, g_memdup2(l->data, sizeof(unsigned long long)));
    g_list_free(ids);
    for (GList *l = copies; l; l = l->next)
        net_decide(*(unsigned long long *)l->data, NULL);
    g_list_free_full(copies, g_free);
}

static void net_line(char *line)
{
    if (line[0] == 'C') {
        unsigned wants = 0, serial = 0;
        if (sscanf(line + 1, "%u %u", &wants, &serial) >= 1) {
            net.wants = wants;
            char ack[32];
            snprintf(ack, sizeof ack, "A %u\n", serial);
            net_send(ack);
        }
    } else if (line[0] == 'D') {
        char *end = NULL;
        unsigned long long id = strtoull(line + 1, &end, 10);
        while (end && *end == ' ')
            end++;
        net_decide(id, end && *end ? end : "{}");
    }
}

static void net_lines(void)
{
    char *newline;
    while ((newline = memchr(net.in->str, '\n', net.in->len))) {
        *newline = 0;
        size_t length = newline - net.in->str + 1;
        char *line = g_strndup(net.in->str, length - 1);
        g_string_erase(net.in, 0, length);
        net_line(line);
        g_free(line);
    }
}

static gboolean net_readable(int fd, GIOCondition condition, gpointer data)
{
    (void)data;
    char buffer[65536];
    for (;;) {
        ssize_t n = read(fd, buffer, sizeof buffer);
        if (n > 0) {
            g_string_append_len(net.in, buffer, n);
            continue;
        }
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
            break;
        net.read_id = 0;
        net_lines();
        net_close();
        return G_SOURCE_REMOVE;
    }
    net_lines();
    (void)condition;
    return G_SOURCE_CONTINUE;
}

/* Connects at the network process's start (main.cpp), before WebKit's run loop: the browser's
 * first line (its settings) is waited for here, briefly, so that the first loads already follow
 * them. */
void collabo_net_start(void)
{
    if (net.tried)
        return;
    net.tried = TRUE;
    net.held = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    net.in = g_string_new(NULL);
    net.out = g_string_new(NULL);
    const char *path = getenv("WPE_BROWSER_NET_SOCKET");
    if (!path || !*path)
        return;
    struct sockaddr_un address = { .sun_family = AF_UNIX };
    if (strlen(path) >= sizeof address.sun_path)
        return;
    strcpy(address.sun_path, path);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return;
    if (connect(fd, (struct sockaddr *)&address, sizeof address) < 0) {
        if (getenv("WPE_BROWSER_DEBUG"))
            fprintf(stderr, "WPENetworkProcess %d: no browser at %s (%s): nothing is intercepted\n", getpid(), path, strerror(errno));
        close(fd);
        return;
    }
    net.fd = fd;
    char hello[48];
    snprintf(hello, sizeof hello, "H %d\n", getpid());
    if (send(fd, hello, strlen(hello), MSG_NOSIGNAL) < 0) {
        net.fd = -1;
        close(fd);
        return;
    }
    /* the settings: the browser sends them as it takes the connection */
    struct pollfd p = { .fd = fd, .events = POLLIN };
    gint64 until = g_get_monotonic_time() + 3 * G_USEC_PER_SEC;
    while (!memchr(net.in->str, '\n', net.in->len) && g_get_monotonic_time() < until) {
        int left = (int)((until - g_get_monotonic_time()) / 1000);
        if (poll(&p, 1, left > 0 ? left : 0) <= 0)
            break;
        char buffer[4096];
        ssize_t n = read(fd, buffer, sizeof buffer);
        if (n <= 0)
            break;
        g_string_append_len(net.in, buffer, n);
    }
    fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
    net_lines();
    net.read_id = g_unix_fd_add(fd, G_IO_IN | G_IO_HUP | G_IO_ERR, net_readable, NULL);
}

unsigned collabo_net_wants(void)
{
    return net.fd >= 0 ? net.wants : 0;
}

unsigned long long collabo_net_hold(const char *event, NetDecided decided, void *context)
{
    if (net.fd < 0 || !decided)
        return 0;
    unsigned long long id = ++net.next_id;
    Held *held = g_new(Held, 1);
    held->decided = decided;
    held->context = context;
    g_hash_table_insert(net.held, g_memdup2(&id, sizeof id), held);
    char head[40];
    snprintf(head, sizeof head, "E %llu ", id);
    net_send(head);
    net_send(event);
    net_send("\n");
    return id;
}

void *collabo_net_forget(unsigned long long id)
{
    Held *held = net.held ? g_hash_table_lookup(net.held, &id) : NULL;
    if (!held)
        return NULL;
    void *context = held->context;
    g_hash_table_remove(net.held, &id);
    char line[40];
    snprintf(line, sizeof line, "F %llu\n", id);
    net_send(line);
    return context;
}

void collabo_net_note(const char *event)
{
    if (net.fd < 0)
        return;
    net_send("N ");
    net_send(event);
    net_send("\n");
}
