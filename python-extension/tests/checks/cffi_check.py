"""cffi and its backend on libffi: types and their layout, conversions and their errors, the C
library called in ABI mode (ffi.dlopen(None)) with every kind of argument and result — integers of
every width, float, double, long double, structs by value large and small, variadic calls —
callbacks of the same kinds (qsort, bsearch, callbacks calling C calling callbacks, many of one
type and of many types, called from C and from Python, in threads), handles, gc, buffers, errno,
out-of-line ABI mode (a module written by ffi.compile). Fixed-size types only: long and pointers
are not the guest's size here. tests/reference.py runs it with cffi's own wheel.
"""
import os
import sys
import tempfile
import threading

import cffi
from cffi import FFI

print("version", cffi.__version__, cffi.__version_info__)

ffi = FFI()
ffi.cdef("""
typedef struct { int8_t a; int32_t b; int16_t c; double d; } mix_t;
typedef union { uint32_t u; float f; uint8_t b[4]; } pun_t;
typedef struct { uint32_t x : 3, y : 9, z : 20; } bits_t;
enum color { RED, GREEN = 5, BLUE };
typedef struct node { int32_t value; struct node *next; } node_t;
typedef struct { double re, im; } cplx_t;
typedef struct { float f; } onefloat_t;
typedef struct { int64_t big; uint8_t tag; } tagged_t;
typedef struct { char name[12]; int32_t n; } named_t;
typedef struct { int quot, rem; } div_t;
typedef struct { long long quot, rem; } lldiv_t;

size_t strlen(const char *);
int strcmp(const char *, const char *);
char *strchr(const char *, int);
void *memchr(const void *, int, size_t);
double strtod(const char *, char **);
long long strtoll(const char *, char **, int);
unsigned long long strtoull(const char *, char **, int);
long double strtold(const char *, char **);
float strtof(const char *, char **);
int snprintf(char *, size_t, const char *, ...);
int sscanf(const char *, const char *, ...);
int abs(int);
long long llabs(long long);
double ldexp(double, int);
double frexp(double, int *);
double modf(double, double *);
float sqrtf(float);
double hypot(double, double);
int toupper(int);
int atoi(const char *);
div_t div(int, int);
lldiv_t lldiv(long long, long long);
void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
void *bsearch(const void *, const void *, size_t, size_t, int (*)(const void *, const void *));
""")
C = ffi.dlopen(None)

# ── types ───────────────────────────────────────────────────────────────────────────────────────
for t in ["mix_t", "pun_t", "bits_t", "enum color", "cplx_t", "onefloat_t", "tagged_t", "named_t",
          "int8_t", "int16_t", "int32_t", "int64_t", "float", "double", "long double", "_Bool", "char"]:
    print("type", t, ffi.sizeof(t), ffi.alignof(t), ffi.typeof(t).kind, ffi.typeof(t).cname)
print("offsets", [ffi.offsetof("mix_t", f) for f in "abcd"], ffi.offsetof("named_t", "n"), ffi.offsetof("tagged_t", "tag"))
print("fields", [(n, f.type.cname, f.offset, f.bitshift, f.bitsize) for n, f in ffi.typeof("bits_t").fields])
print("enum", ffi.typeof("enum color").elements, ffi.typeof("enum color").relements)
print("list_types", ffi.list_types())

p = ffi.new("pun_t *", {"f": 1.5})
print("union", hex(p.u), list(p.b))
b = ffi.new("bits_t *", {"x": 5, "y": 300, "z": 999999})
print("bits", b.x, b.y, b.z, bytes(ffi.buffer(b)).hex())
m = ffi.new("mix_t *", [-3, 70000, -2, 2.5])
print("mix", m.a, m.b, m.c, m.d, bytes(ffi.buffer(m)).hex())
n2 = ffi.new("node_t *", {"value": 2})
n1 = ffi.new("node_t *", {"value": 1, "next": n2})
print("list", n1.value, n1.next.value, n1.next.next == ffi.NULL)

# ── conversions and their errors ────────────────────────────────────────────────────────────────
print("cast", int(ffi.cast("int8_t", 200)), int(ffi.cast("uint16_t", -1)), int(ffi.cast("int64_t", 2**63 + 5)),
      float(ffi.cast("float", 0.1)), int(ffi.cast("_Bool", 7)), ffi.cast("char", 65), int(ffi.cast("enum color", 6)))
for typ, value in [("int8_t *", 128), ("uint32_t *", -1), ("int64_t *", 2**63), ("_Bool *", 2), ("char *", b"ab"),
                   ("float *", "x")]:
    try:
        ffi.new(typ, value)
        print("conv", typ, "ok")
    except (OverflowError, TypeError, ValueError) as e:
        print("conv", typ, type(e).__name__, e)
arr = ffi.new("int32_t[]", [10, 20, 30])
print("array", len(arr), list(arr), arr[2], ffi.unpack(arr, 3), ffi.typeof(arr).cname)
try:
    arr[3]
except IndexError as e:
    print("index", e)
s = ffi.new("char[]", b"hello\0world")
print("string", ffi.string(s), ffi.string(s, 3), len(s), bytes(ffi.buffer(s)))
print("char16/32", ffi.string(ffi.new("char16_t[]", "가나다")), ffi.string(ffi.new("char32_t[]", "😀 ok")))
named = ffi.new("named_t *", {"name": b"cffi", "n": 7})
print("named", ffi.string(named.name), named.n)
ba = bytearray(b"abcdef")
fb = ffi.from_buffer("uint8_t[]", ba)
fb[0] = ord("X")
ffi.memmove(fb + 1, b"YZ", 2)
print("from_buffer", ba, len(fb))
with ffi.new("int32_t[4]") as tmp:
    tmp[3] = 9
    print("with", list(tmp))

# ── the C library, ABI mode ─────────────────────────────────────────────────────────────────────
print("strlen", C.strlen(b"hello, world"), C.strcmp(b"abc", b"abd") < 0, C.atoi(b"-1234"), C.toupper(ord("q")))
hay = ffi.new("char[]", b"find the needle")
print("pointers", C.strchr(hay, ord("n")) - hay, ffi.cast("char *", C.memchr(hay, ord("e"), 15)) - hay,
      C.strchr(hay, ord("#")) == ffi.NULL)
end = ffi.new("char **")
print("strtod", C.strtod(b"6.02214076e23 rest", end), ffi.string(end[0]), C.strtof(b"0.1", ffi.NULL))
print("strtoll", C.strtoll(b"-9223372036854775807", ffi.NULL, 10), C.strtoull(b"ffffffffffffffff", ffi.NULL, 16))
ffi.errno = 0
print("errno", C.strtoll(b"99999999999999999999", ffi.NULL, 10), ffi.errno)
print("strtold", float(C.strtold(b"1.000000000000000000000000000001", ffi.NULL)), ffi.typeof(C.strtold).result.cname)
print("math", C.abs(-7), C.llabs(-(2**60)), C.ldexp(0.75, 12), C.sqrtf(2.0), C.hypot(3.0, 4.0))
e = ffi.new("int *")
ip = ffi.new("double *")
print("out params", C.frexp(48.0, e), e[0], C.modf(-3.25, ip), ip[0])
d = C.div(-17, 5)
ld = C.lldiv(10**15 + 7, 1000)
print("struct results", d.quot, d.rem, ld.quot, ld.rem)
buf = ffi.new("char[128]")
n = C.snprintf(buf, 128, b"%d|%s|%.4f|%lld|%c|%u|%5.1e|%Lf|%x", ffi.cast("int", -42), ffi.new("char[]", b"str"),
               ffi.cast("double", 3.14159265), ffi.cast("long long", -(1 << 40)), ffi.cast("int", 65),
               ffi.cast("unsigned int", 4000000000), ffi.cast("double", 12345.678), ffi.cast("long double", 2.5),
               ffi.cast("int", 255))
print("snprintf", n, ffi.string(buf))
x, y, word = ffi.new("int *"), ffi.new("double *"), ffi.new("char[16]")
print("sscanf", C.sscanf(b"12 3.5 word", b"%d %lf %15s", x, y, word), x[0], y[0], ffi.string(word))

# ── callbacks ───────────────────────────────────────────────────────────────────────────────────
calls = []


@ffi.callback("int(const void *, const void *)")
def compare(a, b):
    x, y = ffi.cast("int32_t *", a)[0], ffi.cast("int32_t *", b)[0]
    calls.append(1)
    return (x > y) - (x < y)


data = ffi.new("int32_t[]", [42, -7, 19, 0, 1000, 3, -500, 77])
C.qsort(data, len(data), 4, compare)
print("qsort", list(data), len(calls) > 0)
key = ffi.new("int32_t *", 19)
found = C.bsearch(key, data, len(data), 4, compare)
print("bsearch", (ffi.cast("int32_t *", found) - data), ffi.cast("int32_t *", found)[0])


@ffi.callback("int(const void *, const void *)")
def by_length(a, b):
    # a callback calling C (strlen) while C (qsort) calls it
    la, lb = C.strlen(ffi.cast("char **", a)[0]), C.strlen(ffi.cast("char **", b)[0])
    return (la > lb) - (la < lb)


words = [ffi.new("char[]", w) for w in [b"ccc", b"a", b"eeeee", b"bb", b"dddd"]]
ptrs = ffi.new("char *[]", words)
C.qsort(ptrs, len(words), ffi.sizeof("char *"), by_length)
print("qsort strings", [ffi.string(p) for p in ptrs])

kinds = [
    ("int8_t(int8_t, uint8_t)", lambda a, b: a - b, (-100, 20)),
    ("uint16_t(int16_t, uint16_t)", lambda a, b: (a + b) & 0xffff, (-1, 65535)),
    ("int64_t(int64_t, int32_t)", lambda a, b: a * b, (2**40, -3)),
    ("float(float, double)", lambda a, b: a * b, (1.5, 0.25)),
    ("double(double, int, float, long long)", lambda a, b, c, d: a + b + c + d, (0.5, 2, 0.25, 10**12)),
    ("long double(long double, long double)", lambda a, b: float(a) / float(b), (1.0, 3.0)),
    ("cplx_t(cplx_t, cplx_t)", lambda a, b: {"re": a.re * b.re - a.im * b.im, "im": a.re * b.im + a.im * b.re},
     ({"re": 1.0, "im": 2.0}, {"re": 3.0, "im": -1.0})),
    ("onefloat_t(onefloat_t, int)", lambda a, n: {"f": a.f * n}, ({"f": 1.25}, 4)),
    ("tagged_t(tagged_t)", lambda t: {"big": -t.big, "tag": t.tag + 1}, ({"big": 2**50, "tag": 7},)),
    ("void(int32_t *, int32_t)", lambda p, v: p.__setitem__(0, v * 2), None),
    ("_Bool(_Bool)", lambda v: not v, (True,)),
    ("int(int, ...)", None, None),
]
for sig, fn, args in kinds:
    if fn is None:
        try:
            ffi.callback(sig, lambda *a: 0)
            print("callback", sig, "made")
        except NotImplementedError as e:
            print("callback", sig, type(e).__name__)
        continue
    cb = ffi.callback(sig, fn)
    if args is None:
        out = ffi.new("int32_t *")
        cb(out, 21)
        result = out[0]
    else:
        result = cb(*args)
    if isinstance(result, ffi.CData) and ffi.typeof(result).kind == "struct":
        result = {f: getattr(result, f) for f, _ in ffi.typeof(result).fields}
    elif ffi.typeof(cb).result.cname == "long double":
        result = round(float(result), 12)
    print("callback", sig, result)

# many callbacks of one type, and of several
many = [ffi.callback("int32_t(int32_t)", lambda v, k=k: v * k) for k in range(70)]
print("many", sum(f(3) for f in many), many[69](2))
mixed = [ffi.callback(f"double({', '.join(['int32_t'] * k + ['double'])})", lambda *a: sum(a)) for k in range(12)]
print("mixed", [f(*range(1, k + 1), 0.5) for k, f in enumerate(mixed)])
wide = ffi.callback("double(" + ", ".join(["int8_t", "int64_t", "float", "double"] * 6) + ")", lambda *a: sum(a))
print("wide", wide(*([-1, 2**33, 0.5, 0.25] * 6)))
del many, mixed

# errors in callbacks
seen = []


def onerror(exc, val, tb):
    seen.append(exc.__name__)
    return -1


bad = ffi.callback("int32_t(int32_t)", lambda v: 1 // v, error=-99, onerror=onerror)
print("onerror", bad(0), bad(5), seen)

# handles and gc
obj = {"payload": [1, 2, 3]}
h = ffi.new_handle(obj)
print("handle", ffi.from_handle(h) is obj, ffi.from_handle(ffi.cast("void *", h))["payload"])
freed = []
g = ffi.gc(ffi.new("int32_t *", 5), lambda p: freed.append(p[0]))
del g
import gc
gc.collect()
print("gc", freed)
alloc = ffi.new_allocator(should_clear_after_alloc=True)
print("allocator", list(alloc("int32_t[3]")))

# ── threads ─────────────────────────────────────────────────────────────────────────────────────
results = {}


def work(i):
    @ffi.callback("int32_t(int32_t, int32_t)")
    def add(a, b):
        return a + b + i

    f = ffi.cast("int32_t(*)(int32_t, int32_t)", add)
    local = ffi.new("int32_t[]", list(range(100, 0, -1)))
    C.qsort(local, 100, 4, compare)
    results[i] = (sum(f(j, j) for j in range(50)), local[0], local[99], C.strlen(b"x" * i))


threads = [threading.Thread(target=work, args=(i,)) for i in range(6)]
for t in threads:
    t.start()
for t in threads:
    t.join()
print("threads", sorted(results.items()))

# ── out-of-line ABI mode: a module ffi.compile writes (no C compiler needed) ───────────────────────
d = tempfile.mkdtemp()
builder = FFI()
builder.cdef("int abs(int); size_t strlen(const char *); typedef struct { int32_t x, y; } pt_t;")
builder.set_source("_cffi_check_abi", None)
builder.compile(tmpdir=d)
sys.path.insert(0, d)
from _cffi_check_abi import ffi as abi_ffi  # noqa: E402
lib = abi_ffi.dlopen(None)
print("out-of-line", sorted(f for f in os.listdir(d) if f.endswith(".py")), lib.abs(-5), lib.strlen(b"four"), abi_ffi.sizeof("pt_t"))
print("done")
