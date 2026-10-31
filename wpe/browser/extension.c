/*
 * The browser's web process extension: linked into the program like the rest (patch 0010 calls its
 * entry point in each web process), it puts the headers the browser was given (`web header`, the
 * user agent profile's client hints) on every request a page makes — the document, its resources,
 * fetch and XMLHttpRequest — as the request leaves the web process.
 *
 * The rules come as a(sssb): host (empty: every host; else that host and its subdomains), name,
 * value (empty: the header is taken off the request), https only. A new web process gets them
 * with its initialization data ({"headers": rules}); the ones running get a "headers" message
 * whenever they change (browser.c: headers_changed).
 */
#include <string.h>
#include <wpe/webkit-web-process-extension.h>

static GVariant *rules; /* a(sssb) */

static void set_rules(GVariant *value)
{
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE("a(sssb)")))
        return;
    g_clear_pointer(&rules, g_variant_unref);
    rules = g_variant_ref_sink(value);
}

/* the host itself or one of its subdomains: "example.com" takes "www.example.com" */
static gboolean host_matches(const char *host, const char *scope)
{
    if (!*scope)
        return TRUE;
    if (!host)
        return FALSE;
    size_t h = strlen(host), s = strlen(scope);
    if (h < s || g_ascii_strcasecmp(host + h - s, scope))
        return FALSE;
    return h == s || host[h - s - 1] == '.';
}

static gboolean on_send_request(WebKitWebPage *page, WebKitURIRequest *request, WebKitURIResponse *redirect, gpointer data)
{
    (void)page; (void)redirect; (void)data;
    if (!rules || !g_variant_n_children(rules))
        return FALSE;
    SoupMessageHeaders *headers = webkit_uri_request_get_http_headers(request); /* NULL: not http(s) */
    if (!headers)
        return FALSE;
    GUri *uri = g_uri_parse(webkit_uri_request_get_uri(request), G_URI_FLAGS_NONE, NULL);
    if (!uri)
        return FALSE;
    const char *host = g_uri_get_host(uri);
    gboolean secure = !g_strcmp0(g_uri_get_scheme(uri), "https");
    GVariantIter iter;
    const char *scope, *name, *value;
    gboolean https_only;
    g_variant_iter_init(&iter, rules);
    while (g_variant_iter_next(&iter, "(&s&s&sb)", &scope, &name, &value, &https_only)) {
        if ((https_only && !secure) || !host_matches(host, scope))
            continue;
        if (*value)
            soup_message_headers_replace(headers, name, value);
        else
            soup_message_headers_remove(headers, name);
    }
    g_uri_unref(uri);
    return FALSE;
}

static void on_page_created(WebKitWebProcessExtension *extension, WebKitWebPage *page, gpointer data)
{
    (void)extension; (void)data;
    g_signal_connect(page, "send-request", G_CALLBACK(on_send_request), NULL);
}

static gboolean on_message(WebKitWebProcessExtension *extension, WebKitUserMessage *message, gpointer data)
{
    (void)extension; (void)data;
    if (g_strcmp0(webkit_user_message_get_name(message), "headers"))
        return FALSE;
    set_rules(webkit_user_message_get_parameters(message));
    return TRUE;
}

void webkit_web_process_extension_initialize_with_user_data(WebKitWebProcessExtension *extension, const GVariant *user_data)
{
    GVariant *data = (GVariant *)user_data;
    if (data && g_variant_is_of_type(data, G_VARIANT_TYPE_VARDICT)) {
        GVariant *headers = g_variant_lookup_value(data, "headers", G_VARIANT_TYPE("a(sssb)"));
        if (headers) {
            set_rules(headers);
            g_variant_unref(headers);
        }
    }
    g_signal_connect(extension, "page-created", G_CALLBACK(on_page_created), NULL);
    g_signal_connect(extension, "user-message-received", G_CALLBACK(on_message), NULL);
}
