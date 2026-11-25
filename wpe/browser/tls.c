/* glib-networking's OpenSSL backend, linked into the program: GIO would load it as a module
 * (libgioopenssl.so) from its module directory, which a static program cannot. A type module whose
 * "load" registers the backend's types plays that module. */
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <openssl/err.h>
#include <openssl/pem.h>
#include <openssl/x509.h>
#include <openssl/x509v3.h>

void g_io_openssl_load(GIOModule *module);

typedef GTypeModule WpeStaticModule;
typedef GTypeModuleClass WpeStaticModuleClass;
G_DEFINE_TYPE(WpeStaticModule, wpe_static_module, G_TYPE_TYPE_MODULE)

static gboolean wpe_static_module_load(GTypeModule *module)
{
    /* GTypeModule is all the backend's registration uses of a GIOModule */
    g_io_openssl_load((GIOModule *)module);
    return TRUE;
}

static void wpe_static_module_unload(GTypeModule *module)
{
    (void)module;
}

static void wpe_static_module_class_init(WpeStaticModuleClass *klass)
{
    klass->load = wpe_static_module_load;
    klass->unload = wpe_static_module_unload;
}

static void wpe_static_module_init(WpeStaticModule *module)
{
    (void)module;
}

void wpe_register_tls_backend(void)
{
    static gsize registered;
    if (g_once_init_enter(&registered)) {
        /* GIO registers its extension points when it first looks for a backend: the TLS one
         * must be there before the backend implements it (registering again is harmless) */
        GIOExtensionPoint *point = g_io_extension_point_register(G_TLS_BACKEND_EXTENSION_POINT_NAME);
        g_io_extension_point_set_required_type(point, G_TYPE_TLS_BACKEND);
        GTypeModule *module = g_object_new(wpe_static_module_get_type(), NULL);
        g_type_module_set_name(module, "gioopenssl (linked in)");
        g_type_module_use(module); /* never unused: the types stay */
        g_once_init_leave(&registered, 1);
    }
}

/* ── certificate authorities trusted besides the guest's (web proxy ca) ────────────────────────
 * For a proxy that reads HTTPS (Burp, mitmproxy, a company's), whose certificates its own CA
 * signs. The TLS backend checks a chain against an OpenSSL store made at its start from the
 * guest's bundle (SSL_CERT_FILE) and the folders of SSL_CERT_DIR, where OpenSSL looks for an
 * issuer it does not know yet by the hash of its name (HASH.0, HASH.1...), each time: a
 * certificate put there while the browser runs is trusted from its next connection on. */

/* the browser's own folder first, then what OpenSSL would have looked in (set before WebKit's
 * network process starts: it inherits it) */
void wpe_tls_trust_dir(const char *dir)
{
    const char *before = g_getenv("SSL_CERT_DIR");
    char *dirs = g_strdup_printf("%s:%s", dir, before && *before ? before : X509_get_default_cert_dir());
    g_setenv("SSL_CERT_DIR", dirs, TRUE);
    g_free(dirs);
}

static gboolean same_certificate(const char *path, X509 *x)
{
    BIO *bio = BIO_new_file(path, "r");
    if (!bio)
        return FALSE;
    X509 *there = PEM_read_bio_X509(bio, NULL, NULL, NULL);
    BIO_free(bio);
    gboolean same = there && !X509_cmp(there, x);
    X509_free(there);
    ERR_clear_error();
    return same;
}

/* Trusts the certificates of FILE (PEM, one or more, or DER: Burp exports that) from now on:
 * each written into DIR by its name's hash. *subjects: a line each (its subject, and whether it
 * can sign others). FALSE with *error. */
gboolean wpe_tls_trust_file(const char *dir, const char *file, char **subjects, char **error)
{
    gchar *data = NULL;
    gsize size = 0;
    GError *e = NULL;
    if (!g_file_get_contents(file, &data, &size, &e)) {
        *error = g_strdup(e->message);
        g_error_free(e);
        return FALSE;
    }
    GPtrArray *certificates = g_ptr_array_new_with_free_func((GDestroyNotify)X509_free);
    if (g_strstr_len(data, size, "-----BEGIN CERTIFICATE-----")) {
        BIO *bio = BIO_new_mem_buf(data, (int)size);
        X509 *x;
        while ((x = PEM_read_bio_X509(bio, NULL, NULL, NULL)))
            g_ptr_array_add(certificates, x);
        BIO_free(bio);
    } else {
        const unsigned char *p = (const unsigned char *)data;
        X509 *x = d2i_X509(NULL, &p, (long)size);
        if (x)
            g_ptr_array_add(certificates, x);
    }
    ERR_clear_error();
    g_free(data);
    if (!certificates->len) {
        *error = g_strdup_printf("no certificate in %s (a PEM or DER file of a certificate authority)", file);
        g_ptr_array_free(certificates, TRUE);
        return FALSE;
    }
    GString *lines = g_string_new(NULL);
    gboolean ok = TRUE;
    for (guint i = 0; i < certificates->len && ok; i++) {
        X509 *x = g_ptr_array_index(certificates, i);
        int hashed = 0;
        unsigned long hash = X509_NAME_hash_ex(X509_get_subject_name(x), NULL, NULL, &hashed);
        if (!hashed) {
            *error = g_strdup_printf("%s: a certificate whose name OpenSSL cannot hash", file);
            ok = FALSE;
            break;
        }
        /* the first free name for the hash (a certificate there already: left as it is) */
        for (int k = 0; ok; k++) {
            char *path = g_strdup_printf("%s/%08lx.%d", dir, hash, k);
            if (g_file_test(path, G_FILE_TEST_EXISTS)) {
                gboolean same = same_certificate(path, x);
                g_free(path);
                if (same)
                    break;
                continue;
            }
            BIO *out = BIO_new_file(path, "w");
            if (!out || !PEM_write_bio_X509(out, x)) {
                *error = g_strdup_printf("cannot write %s", path);
                ok = FALSE;
            }
            if (out)
                BIO_free(out);
            g_free(path);
            break;
        }
        if (!ok)
            break;
        BIO *name = BIO_new(BIO_s_mem());
        X509_NAME_print_ex(name, X509_get_subject_name(x), 0, XN_FLAG_RFC2253);
        char *text = NULL;
        long length = BIO_get_mem_data(name, &text);
        g_string_append_printf(lines, "%.*s%s\n", (int)length, text,
                               X509_check_ca(x) > 0 ? "" : " (not a certificate authority: trusted as itself only)");
        BIO_free(name);
    }
    ERR_clear_error();
    g_ptr_array_free(certificates, TRUE);
    if (!ok) {
        g_string_free(lines, TRUE);
        return FALSE;
    }
    *subjects = g_string_free(lines, FALSE);
    return TRUE;
}

/* takes them out of DIR (a store that used one already keeps it until the browser restarts) */
void wpe_tls_untrust_all(const char *dir)
{
    GDir *folder = g_dir_open(dir, 0, NULL);
    if (!folder)
        return;
    const char *entry;
    while ((entry = g_dir_read_name(folder))) {
        char *path = g_build_filename(dir, entry, NULL);
        g_unlink(path);
        g_free(path);
    }
    g_dir_close(folder);
}
