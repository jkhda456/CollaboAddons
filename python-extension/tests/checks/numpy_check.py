"""numpy, its compiled parts: array core, ufuncs, casting, sorting (C++), strings (C++ ufuncs and
StringDType), datetimes, linalg (lapack_lite), fft (pocketfft, C++), random (every bit generator,
Generator and the legacy RandomState, whose mtrand carries its own build of the distributions).

Prints what it computes, platform-independent: explicit dtypes (the default integer is 32-bit on
wasm32), floats rounded, no long double. tests/reference.py runs it on this machine with numpy's
own wheel; the guest's output must be the same.
"""
import io
import pickle

import numpy as np

np.set_printoptions(precision=8, suppress=True, floatmode="maxprec", linewidth=200)


def show(label, value):
    a = np.asarray(value)
    if a.dtype.kind in "fc":
        a = np.round(a, 8) + 0  # no -0
    print(f"{label}: {a.tolist() if a.ndim else a.item()}")


print("version", np.__version__)

# core: creation, broadcasting, reductions, casting
a = np.arange(24, dtype=np.int64).reshape(2, 3, 4)
show("sum axes", [a.sum(), *a.sum(axis=(0, 2))])
show("broadcast", (a[:, :, :1] * np.array([1, -1, 2, -2], dtype=np.int64))[1])
show("cumprod", np.cumprod(np.arange(1, 11, dtype=np.int64)))
show("mean std", [a.mean(), a.std(), np.median(a), np.percentile(a, 37.5)])
show("cast", np.array([1.7, -1.7, 2.5, 300.0]).astype(np.int64))
show("uint8 wrap", np.array([250, 5], dtype=np.uint8) + np.uint8(10))
show("float32", (np.float32(1) / np.float32(3)).astype(np.float64))
show("where", np.where(a[0] % 3 == 0, a[0], -1))
show("fancy", a.reshape(-1)[[0, 5, 23, -1]])
show("einsum", np.einsum("ijk,ik->j", a, a[:, 0, :]))
show("tensordot", np.tensordot(np.eye(3), np.arange(9.0).reshape(3, 3), axes=1))
show("nan funcs", [np.nanmax([1.0, np.nan, 3.0]), np.nansum([np.nan, 2.0]), np.isnan(np.nan)])
show("math", np.array([np.sin(1), np.exp(2), np.log1p(1e-10), np.arctan2(1, 2), np.hypot(3, 4), np.cbrt(27.0)]))
show("complex", np.array([1 + 2j, 3 - 1j]) ** 2)
show("histogram", np.histogram(np.arange(20, dtype=np.int64), bins=4)[0].astype(np.int64))
show("interp", np.interp([0.5, 1.5, 2.5], [0, 1, 2, 3], [0, 10, 20, 30]))
show("convolve", np.convolve([1.0, 2, 3], [0.0, 1, 0.5]))
show("gradient", np.gradient(np.array([1.0, 4, 9, 16, 25])))
show("polyfit", np.polyfit([0.0, 1, 2, 3], [1.0, 3, 7, 13], 2))
show("roots", np.sort(np.roots([1.0, -6, 11, -6])))

# sorting (npysort, C++), searching, sets
rng = np.random.default_rng(7)
f = rng.random(50)
i = rng.integers(-1000, 1000, 50, dtype=np.int64)
for kind in ("quicksort", "mergesort", "heapsort", "stable"):
    show(f"sort {kind}", [*np.sort(f, kind=kind)[:3], *np.sort(i, kind=kind)[-3:].astype(np.float64)])
show("argsort", np.argsort(i, kind="stable")[:8].astype(np.int64))
show("partition", np.partition(i, 5)[5])
show("unique", np.unique(np.array([3, 1, 3, 2, 1], dtype=np.int64), return_counts=True)[1].astype(np.int64))
show("searchsorted", np.searchsorted(np.sort(f), [0.25, 0.5, 0.75]).astype(np.int64))
show("lexsort", np.lexsort((np.array([1, 0, 1], dtype=np.int64), np.array([2, 2, 1], dtype=np.int64))).astype(np.int64))

# strings: the np.strings ufuncs (C++) and StringDType
s = np.array(["hello", "World", "numpy", "wasm32"])
print("strings", np.strings.upper(s).tolist(), np.strings.find(s, "o").tolist(), np.strings.replace(s, "l", "L").tolist())
print("strings2", np.strings.zfill(np.array(["7", "-3"]), 4).tolist(), np.strings.str_len(s).tolist(), np.char.center(s, 9, "*").tolist())
t = np.array(["가나다", "αβγ", "plain"], dtype=np.dtypes.StringDType())
print("stringdtype", (t + "!").tolist(), np.strings.str_len(t).tolist(), np.sort(t).tolist(), t.dtype)

# datetimes
d = np.arange("2026-01-30", "2026-02-03", dtype="datetime64[D]")
print("datetime", d.tolist(), (d[-1] - d[0]).astype(np.int64), np.busday_count("2026-10-01", "2026-10-31"))
print("datetime2", np.datetime64("2026-10-14T12:30") + np.timedelta64(90, "m"), np.datetime_as_string(np.datetime64("2026-03-08T02:30", "s")))

# structured, records, buffers, pickles, npy files
st = np.array([(1, 2.5, b"ab"), (2, -1.0, b"cd")], dtype=[("id", "<i8"), ("v", "<f8"), ("tag", "S2")])
print("structured", st["v"].tolist(), st[1].tolist(), st.dtype.descr)
print("frombuffer", np.frombuffer(b"\x01\x00\x00\x00\x02\x00\x00\x00", dtype="<i4").tolist(), memoryview(np.arange(4, dtype=np.int16)).tolist())
buf = io.BytesIO()
np.save(buf, a)
buf.seek(0)
print("npy", np.array_equal(np.load(buf), a), pickle.loads(pickle.dumps(st)).tolist() == st.tolist())

# linalg (lapack_lite and numpy's own matrix products)
m = np.array([[4.0, 1, 2], [1, 3, 0], [2, 0, 5]])
show("solve", np.linalg.solve(m, [1.0, 2, 3]))
show("inv", np.linalg.inv(m))
show("det", np.linalg.det(m))
show("eigvalsh", np.linalg.eigvalsh(m))
w, v = np.linalg.eigh(m)
show("eigh check", np.abs(m @ v - v * w).max() < 1e-12)
show("eig", np.sort(np.linalg.eigvals(np.array([[0.0, 1], [-2, -3]]))))
show("svd", np.linalg.svd(np.arange(12.0).reshape(3, 4), compute_uv=False))
show("qr r", np.abs(np.diag(np.linalg.qr(m)[1])))
show("cholesky", np.linalg.cholesky(m))
show("lstsq", np.linalg.lstsq(np.vander([0.0, 1, 2, 3], 2), [1.0, 3, 5, 7], rcond=None)[0])
show("pinv", np.linalg.pinv(np.array([[1.0, 2], [2, 4]])))
show("norms", [np.linalg.norm(m), np.linalg.norm(m, 2), np.linalg.cond(m), np.linalg.matrix_rank(m)])
show("matmul big", (np.arange(64.0).reshape(8, 8) @ np.arange(64.0).reshape(8, 8).T)[3])
show("complex solve", np.linalg.solve(np.array([[1 + 1j, 2], [3, 4 - 1j]]), np.array([1, 1j])))
show("float32 inv", np.linalg.inv(m.astype(np.float32)).astype(np.float64).round(4))
show("matrix_power", np.linalg.matrix_power(np.array([[1, 1], [1, 0]], dtype=np.int64), 30))

# fft (pocketfft, C++)
x = np.cos(np.arange(16) * 2 * np.pi / 16 * 3) + 0.5 * np.sin(np.arange(16) * 2 * np.pi / 16 * 5)
show("fft", np.abs(np.fft.fft(x)))
show("rfft", np.fft.rfft(x)[:6])
show("irfft", np.fft.irfft(np.fft.rfft(x), 16) - x)
show("fft2", np.fft.fft2(np.arange(12.0).reshape(3, 4)))
show("fftfreq", np.fft.fftfreq(8, d=0.5))

# random: bit generators, Generator, legacy RandomState
for bitgen in (np.random.PCG64, np.random.PCG64DXSM, np.random.MT19937, np.random.Philox, np.random.SFC64):
    g = np.random.Generator(bitgen(12345))
    print(bitgen.__name__, g.integers(0, 2**40, 3, dtype=np.int64).tolist(), np.round(g.random(2), 12).tolist())
g = np.random.default_rng(2026)
show("normal", g.normal(1.0, 2.0, 4))
show("exponential gamma beta", [g.exponential(), g.gamma(2.5), g.beta(2, 5), g.standard_t(3), g.chisquare(4)])
show("discrete", [g.poisson(4.5, 6), g.binomial(20, 0.3, 6), g.geometric(0.2, 6), g.hypergeometric(10, 5, 7, 6),
                  g.negative_binomial(3, 0.4, 6), g.zipf(2.5, 6), g.logseries(0.6, 6)])
show("choice", g.choice(100, 5, replace=False).astype(np.int64))
show("permutation", g.permutation(10).astype(np.int64))
show("multivariate", g.multivariate_normal([0.0, 0.0], [[1.0, 0.5], [0.5, 2.0]], 2))
show("dirichlet", g.dirichlet([1.0, 2.0, 3.0]))
rs = np.random.RandomState(42)
show("legacy rand", rs.rand(3))
show("legacy randn", rs.randn(3))
show("legacy randint", rs.randint(0, 1000, 5).astype(np.int64))
show("legacy poisson", rs.poisson(5.0, 6).astype(np.int64))
show("legacy geometric", rs.geometric(0.3, 6).astype(np.int64))
show("legacy binomial", rs.binomial(30, 0.4, 6).astype(np.int64))
show("legacy hypergeometric", rs.hypergeometric(20, 10, 8, 6).astype(np.int64))
show("legacy other", [rs.standard_gamma(1.5), rs.beta(0.5, 0.5), rs.vonmises(0.0, 2.0), rs.wald(1.0, 2.0), rs.zipf(3.0)])
show("legacy choice", rs.choice(50, 6, replace=False).astype(np.int64))
np.random.seed(1)
show("global", [np.random.random(), np.random.normal(), float(np.random.randint(10))])

# polynomial and testing helpers (pure Python over the compiled core)
P = np.polynomial.Polynomial([1.0, -3, 2])
show("polynomial", [*P.roots(), *P.deriv().coef, P(2.0)])
np.testing.assert_allclose(np.linalg.inv(m) @ m, np.eye(3), atol=1e-12)
print("done")
