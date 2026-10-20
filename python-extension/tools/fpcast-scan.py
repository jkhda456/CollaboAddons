#!/usr/bin/env python3
"""Finds the functions the guest's CPython will call as a type they do not have: WebAssembly
checks the type of every indirect call ("indirect call type mismatch"), where native code gets
away with calling a function with more arguments than it takes. A C extension module's classic
cases: a method registered METH_NOARGS or METH_VARARGS but written with one parameter,
`f(PyObject *self)`, which CPython calls as `f(self, args)` (Pillow's ExportArrowSchemaPyCapsule);
one registered METH_VARARGS|METH_KEYWORDS but written without `kwargs` (numpy's
_get_castingimpl); a getter without its closure (numpy's array_converter_get_scalar_input).
Pyodide's CPython calls methods through a JavaScript trampoline that forgives these; the guest's
CPython cannot.

Two analyses of each source, compiled as its compile record says:

  * the method tables (PyMethodDef), type slot tables (PyType_Slot) and module slot tables
    (PyModuleDef_Slot) of the preprocessed source: each entry's function, by its declaration,
    against the type CPython calls it as — by the entry's METH_ flags, or the slot's type (slot
    numbers from CPython's typeslots.h; Py_mod_exec and Py_mod_create). Pillow's _imagingtk
    registered an `int f(void)` as Py_mod_exec, which CPython calls with the module;
  * clang's -Wcast-function-type-strict, which names every cast between function types: the casts
    to one of CPython's slot types (getter, setter, destructor, ...), whose values CPython calls
    as that type, where the wasm signatures differ (a different number of parameters, an i64 or a
    double where an i32 was, a result where there was none).

    fpcast-scan.py [--all] SOURCE...
        each SOURCE a directory of compile records (wasm-pyext-cc writes one per compile when
        PYEXT_COMPDB names a directory) or a build directory with compile_commands.json (meson)
        or a ninja build (CMake). Prints FILE:LINE: a <signature> function called as <signature>
        (<how>); exit status 1 if any. --all: also every other cast whose signatures differ, for
        review (most are a library's own, stored as one type and cast back before the call).
"""
import json
import os
import re
import shlex
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

I64 = {"long long", "unsigned long long", "int64_t", "uint64_t", "npy_int64", "npy_uint64", "npy_longlong",
       "npy_ulonglong", "off_t", "off64_t", "time_t", "PY_LONG_LONG", "unsigned PY_LONG_LONG", "__int64"}
F64 = {"double", "npy_double", "long double", "npy_longdouble"}
F32 = {"float", "npy_float"}
WARN = re.compile(r"^(.*?):(\d+):\d+: warning: cast from '([^']*)'(?: \(aka '([^']*)'\))? to '([^']*)'(?: \(aka '([^']*)'\))? "
                  r"converts to incompatible function type")
# CPython's function types whose values it calls as they are: a cast of a function to one of these
# is how the function will be called (Include/object.h, descrobject.h, methodobject.h)
SLOT_TYPES = {"getter", "setter", "destructor", "reprfunc", "hashfunc", "richcmpfunc", "getiterfunc", "iternextfunc",
              "initproc", "newfunc", "allocfunc", "freefunc", "traverseproc", "inquiry", "lenfunc", "unaryfunc",
              "binaryfunc", "ternaryfunc", "ssizeargfunc", "ssizeobjargproc", "objobjproc", "objobjargproc",
              "getattrfunc", "getattrofunc", "setattrfunc", "setattrofunc", "descrgetfunc", "descrsetfunc", "visitproc",
              "vectorcallfunc", "getbufferproc", "releasebufferproc", "printfunc", "PyCapsule_Destructor"}
# (Not PyCFunction and its family — PyCFunctionWithKeywords, ...: what a method table's function is
# called as depends on its flags, which the table analysis reads; elsewhere casts to those are how
# a stored PyCFunction is cast back to be called.)


def split_top(s):
    out, depth, cur = [], 0, ""
    for c in s:
        if c in "(<[":
            depth += 1
        elif c in ")>]":
            depth -= 1
        if c == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += c
    if cur.strip():
        out.append(cur.strip())
    return out


def wasm_type(t):
    t = re.sub(r"\b(const|volatile|struct|enum|union|restrict|__restrict|signed)\b", "", t).strip()
    t = re.sub(r"\s+", " ", t)
    if "*" in t or "&" in t or "[" in t or "(" in t:
        return "i"
    if t in I64:
        return "j"
    if t in F64:
        return "d"
    if t in F32:
        return "f"
    if t == "void":
        return "v"
    return "i"


def signature(fptr):
    """'int (*)(void *, long long)' -> 'i_ij'; None when not a plain function pointer type."""
    m = re.match(r"^(.*?)\s*\(\*\)\s*\((.*)\)\s*$", fptr.strip()) or re.match(r"^(.*?)\s*\((.*)\)\s*$", fptr.strip())
    if not m or m.group(2).strip().endswith("..."):
        return None
    ret, params = m.group(1), m.group(2)
    ps = [] if params.strip() in ("", "void") else [wasm_type(p) for p in split_top(params)]
    return wasm_type(ret) + "_" + "".join(ps)


# what CPython calls each PyType_Slot's function as (Include/typeslots.h names, Include/object.h types)
SLOT_SIGNATURES = {
    "Py_bf_getbuffer": "i_iii", "Py_bf_releasebuffer": "v_ii", "Py_mp_ass_subscript": "i_iii", "Py_mp_length": "i_i",
    "Py_mp_subscript": "i_ii", "Py_nb_bool": "i_i", "Py_nb_power": "i_iii", "Py_nb_inplace_power": "i_iii",
    "Py_sq_ass_item": "i_iii", "Py_sq_concat": "i_ii", "Py_sq_contains": "i_ii", "Py_sq_inplace_concat": "i_ii",
    "Py_sq_inplace_repeat": "i_ii", "Py_sq_item": "i_ii", "Py_sq_length": "i_i", "Py_sq_repeat": "i_ii",
    "Py_tp_alloc": "i_ii", "Py_tp_call": "i_iii", "Py_tp_clear": "i_i", "Py_tp_dealloc": "v_i", "Py_tp_del": "v_i",
    "Py_tp_descr_get": "i_iii", "Py_tp_descr_set": "i_iii", "Py_tp_getattr": "i_ii", "Py_tp_getattro": "i_ii",
    "Py_tp_hash": "i_i", "Py_tp_init": "i_iii", "Py_tp_is_gc": "i_i", "Py_tp_iter": "i_i", "Py_tp_iternext": "i_i",
    "Py_tp_new": "i_iii", "Py_tp_repr": "i_i", "Py_tp_richcompare": "i_iii", "Py_tp_setattr": "i_iii",
    "Py_tp_setattro": "i_iii", "Py_tp_str": "i_i", "Py_tp_traverse": "i_iii", "Py_tp_free": "v_i",
    "Py_tp_finalize": "v_i", "Py_am_await": "i_i", "Py_am_aiter": "i_i", "Py_am_anext": "i_i", "Py_am_send": "i_iii",
    "Py_tp_vectorcall": "i_iiii",
}
for _unary in ("absolute", "float", "index", "int", "invert", "negative", "positive"):
    SLOT_SIGNATURES[f"Py_nb_{_unary}"] = "i_i"
for _binary in ("add", "and", "divmod", "floor_divide", "lshift", "multiply", "or", "remainder", "rshift", "subtract",
                "true_divide", "xor", "matrix_multiply"):
    SLOT_SIGNATURES[f"Py_nb_{_binary}"] = "i_ii"
    if _binary != "divmod":
        SLOT_SIGNATURES[f"Py_nb_inplace_{_binary}"] = "i_ii"
TYPESLOTS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../python/src/Python-3.13.14/Include/typeslots.h")
SLOT_NUMBERS = {}
if os.path.exists(TYPESLOTS):
    for _m in re.finditer(r"#define\s+(Py_\w+)\s+(\d+)", open(TYPESLOTS).read()):
        SLOT_NUMBERS[int(_m.group(2))] = _m.group(1)
# PyModuleDef_Slot: Py_mod_create (1), PyObject *(*)(PyObject *spec, PyModuleDef *); Py_mod_exec
# (2), int (*)(PyObject *module) (Include/moduleobject.h; the other slots hold values)
MODULE_SLOT_SIGNATURES = {1: ("i_ii", "Py_mod_create"), 2: ("i_i", "Py_mod_exec")}
METH = {"VARARGS": 0x1, "KEYWORDS": 0x2, "NOARGS": 0x4, "O": 0x8, "FASTCALL": 0x80, "METHOD": 0x200}


def meth_signature(flags):
    if flags & METH["METHOD"]:
        return "i_iiiii"
    if flags & METH["FASTCALL"]:
        return "i_iiii" if flags & METH["KEYWORDS"] else "i_iii"
    return "i_iii" if flags & METH["KEYWORDS"] else "i_ii"


def braces(text, start):
    """The end of the brace group that starts at text[start] == '{'."""
    depth, i, quote = 0, start, None
    while i < len(text):
        c = text[i]
        if quote:
            if c == "\\":
                i += 1
            elif c == quote:
                quote = None
        elif c in "\"'":
            quote = c
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(text)


def groups(body):
    """The top-level {...} groups of an initializer's body, with their offsets."""
    out, i = [], 0
    while True:
        j = body.find("{", i)
        if j < 0:
            return out
        k = braces(body, j)
        out.append((j, body[j + 1:k]))
        i = k + 1


def target(expr):
    """The function an initializer's field names, through any casts: `(PyCFunction)(void(*)(void))f`."""
    expr = re.sub(r"\(\s*\w[\w\s\*]*\(\s*\*\s*\)\s*\([^()]*\)\s*\)", " ", expr)   # (void (*)(void)) and alike
    expr = re.sub(r"\(\s*[A-Za-z_]\w*(\s*\*)*\s*\)(?=\s*[\w(&])", " ", expr)             # (PyCFunction), (void *)
    names = re.findall(r"[A-Za-z_]\w*", expr)
    return names[-1] if len(names) == 1 else None


def declared(text, name, cache):
    """The wasm signature of function `name` from its first declaration in `text`."""
    if name in cache:
        return cache[name]
    sig = None
    for m in re.finditer(r"(?:^|[;{}\n])\s*((?:[A-Za-z_]\w*[\s\*]+)+)" + re.escape(name) + r"\s*\(", text):
        ret = re.sub(r"\b(static|inline|extern|__inline__|__inline|NPY_INLINE|NPY_NO_EXPORT|NPY_FINLINE|CYTHON_UNUSED)\b", "", m.group(1))
        if re.search(r"\b(return|typedef|if|while|sizeof|else)\b", ret) or not ret.strip():
            continue
        start = m.end()
        depth, i = 1, start
        while i < len(text) and depth:
            depth += {"(": 1, ")": -1}.get(text[i], 0)
            i += 1
        if not re.match(r"\s*(__attribute__\s*\(\(.*?\)\)\s*)*[;{]", text[i:i + 200], re.S):
            continue
        sig = signature(re.sub(r"\s+", " ", ret.strip() + " (" + text[start:i - 1] + ")"))
        break
    cache[name] = sig
    return sig


def strip_attributes(text):
    """The preprocessed text without __attribute__((...)) (visibility, unused, ...), which would hide
    a declaration's return type from declared()."""
    out, i = [], 0
    for m in re.finditer(r"\b__attribute__\s*\(", text):
        if m.start() < i:
            continue
        depth, j = 1, m.end()
        while j < len(text) and depth:
            depth += {"(": 1, ")": -1}.get(text[j], 0)
            j += 1
        out.append(text[i:m.start()])
        i = j
    out.append(text[i:])
    return "".join(out)


def source_line(text, offset, markers):
    """(file, line) of an offset of preprocessed text, from its line markers."""
    line_no = text.count("\n", 0, offset)
    best = None
    for at, (path, line) in markers:
        if at > line_no:
            break
        best = (path, line + (line_no - at - 1))
    return best or ("?", line_no)


def tables(entry, keep):
    """The method and slot tables of a source, preprocessed: (path, line, signature, called as, how)."""
    out = []
    src = os.path.join(entry["directory"], entry["file"])
    try:
        raw = open(src, errors="replace").read()
    except OSError:
        return out
    if "METH_" not in raw and "PyType_Slot" not in raw and "Py_tp_" not in raw and "Py_mod_" not in raw:
        return out
    pre = [a for a in keep if a not in ("-fsyntax-only",)] + ["-E", "-Wno-everything"]
    text = strip_attributes(subprocess.run(pre, cwd=entry["directory"], capture_output=True, text=True, errors="replace").stdout)
    markers = [(text.count("\n", 0, m.start()), (m.group(2), int(m.group(1))))
               for m in re.finditer(r'^# (\d+) "([^"]+)"', text, re.M)]
    cache = {}
    for m in re.finditer(r"\b(PyMethodDef|PyType_Slot|PyModuleDef_Slot)\s+[A-Za-z_]\w*\s*(\[\s*\w*\s*\])?\s*=\s*\{", text):
        kind, start = m.group(1), m.end() - 1
        body = text[start + 1:braces(text, start)]
        items = groups(body) if m.group(2) else [(0, body)]
        for off, item in items:
            fields = split_top(item)
            if kind == "PyMethodDef" and len(fields) >= 3:
                fn = target(fields[1])
                try:
                    flags = eval(re.sub(r"\(\s*int\s*\)", "", fields[2]), {"__builtins__": {}})
                except Exception:
                    continue
                want, how = meth_signature(flags), f"method flags {flags:#x}"
            elif kind == "PyType_Slot" and len(fields) >= 2:
                # a CPython slot only where the source names one (numpy keeps its DType slots in
                # PyType_Slot arrays too, with numbers of its own)
                path, line = source_line(text, start + 1 + off, markers)
                try:
                    named = re.search(r"\bPy_(?:tp|nb|sq|mp|am|bf)_\w+", open(path, errors="replace").read().splitlines()[line - 1])
                except (OSError, IndexError):
                    named = None
                if not named:
                    continue
                fn, slot = target(fields[1]), named.group(0)
                want, how = SLOT_SIGNATURES.get(slot), slot
            elif kind == "PyModuleDef_Slot" and len(fields) >= 2:
                try:
                    number = int(eval(fields[0], {"__builtins__": {}}))
                except Exception:
                    continue
                if number not in MODULE_SLOT_SIGNATURES:
                    continue
                fn = target(fields[1])
                want, how = MODULE_SLOT_SIGNATURES[number]
            else:
                continue
            if not fn or not want or fn in ("NULL", "0"):
                continue
            have = declared(text, fn, cache)
            STATS["entries"] += 1
            if not have:
                STATS["unknown"].add(fn)
            if have and have != want:
                path, line = source_line(text, start + 1 + off, markers)
                path = path if os.path.isabs(path) else os.path.normpath(os.path.join(entry["directory"], path))
                out.append((path, line, have, want, f"{how}: {fn}"))
    return out


def entries(source):
    if os.path.exists(os.path.join(source, "compile_commands.json")):
        return json.load(open(os.path.join(source, "compile_commands.json")))
    if os.path.exists(os.path.join(source, "build.ninja")):
        out = subprocess.run([os.environ.get("NINJA", "ninja"), "-C", source, "-t", "compdb"], errors="replace",
                             capture_output=True, text=True, check=True).stdout
        return json.loads(out)
    return [json.load(open(os.path.join(source, f))) for f in sorted(os.listdir(source)) if f.endswith(".json")]


def scan(entry):
    args = entry.get("arguments") or shlex.split(entry["command"])
    keep, skip = [], False
    for a in args:
        if skip:
            skip = False
            continue
        if a in ("-o", "-MF", "-MQ", "-MT"):
            skip = True
            continue
        if a.startswith("-W") or a in ("-c", "-MD", "-MMD"):
            continue
        keep.append(a)
    if (not keep or keep[0].startswith("-") or "&&" in keep or not os.path.isdir(entry["directory"])
            or not os.path.exists(os.path.join(entry["directory"], entry["file"]))):
        return entry, "", []
    found = tables(entry, keep)
    keep += ["-fsyntax-only", "-Wno-everything", "-Wcast-function-type-strict", "-fno-color-diagnostics"]
    return entry, subprocess.run(keep, cwd=entry["directory"], capture_output=True, text=True, errors="replace").stderr, found


ALL = False
STATS = {"entries": 0, "unknown": set()}


def main():
    global ALL
    args = sys.argv[1:]
    if args and args[0] == "--all":
        ALL, args = True, args[1:]
    todo = [e for s in args for e in entries(s) if e["file"].endswith((".c", ".cpp", ".cc", ".cxx"))]
    found = set()
    with ThreadPoolExecutor(max_workers=int(os.environ.get("JOBS", "6"))) as pool:
        for entry, text, table in pool.map(scan, todo):
            found.update(table)
            pending = None
            for line in text.splitlines():
                m = WARN.match(line)
                if not m:
                    continue
                path, lineno, src, src_aka, dst, dst_aka = m.groups()
                path = path if os.path.isabs(path) else os.path.normpath(os.path.join(entry["directory"], path))
                src_t, dst_t = src_aka or src, dst_aka or dst
                if dst_t.replace(" ", "") == "void(*)(void)":
                    pending = (path, lineno, src_t)   # the first half of (T)(void (*)(void)) f
                    continue
                if src_t.replace(" ", "") == "void(*)(void)" and pending and pending[:2] == (path, lineno):
                    src_t = pending[2]
                pending = None
                a, b = signature(src_t), signature(dst_t)
                if a is None or b is None or a == b:
                    continue
                name = dst.strip()
                if name in SLOT_TYPES or ALL:
                    found.add((path, int(lineno), a, b, name))
    for path, lineno, a, b, what in sorted(found):
        print(f"{path}:{lineno}: a {a} function called as {b} ({what})")
    print(f"{len(found)} functions called as a type they do not have ({len(todo)} files, {STATS['entries']} table "
          f"entries, {len(STATS['unknown'])} functions of them not found declared)", file=sys.stderr)
    if os.environ.get("FPCAST_UNKNOWN"):
        print("not found declared: " + " ".join(sorted(STATS["unknown"])), file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
