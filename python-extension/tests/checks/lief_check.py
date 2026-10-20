"""LIEF, its compiled part lief._lief: ELF, PE, COFF and Mach-O parsing of the binaries in
tests/samples (tests/make-samples.sh), changes written back and parsed again, the PE authentihash
(mbedtls), JSON, demangling, and C++ exceptions turned into Python ones by nanobind (an index out of
range, iteration's end, a bad argument). Run from tests/samples. tests/reference.py runs it with
LIEF's own wheel.
"""
import hashlib
import json
import os
import tempfile

import lief

lief.logging.disable()
print("version", lief.__version__.split("-")[0], lief.__is_tagged__, lief.__extended__)
tmp = tempfile.mkdtemp()


def names(it):
    return [x.name for x in it]


# the abstract layer, every format
for f in sorted(os.listdir(".")):
    if f.endswith((".c", ".def", ".lib", ".sh")):
        continue
    b = lief.parse(f)
    head = b.abstract.header if b is not None and hasattr(b, "abstract") else None
    print("parse", f, type(b).__module__ if b is not None else None, getattr(b, "format", None), head.architecture if head else None,
          head.is_64 if head else None, hex(b.entrypoint) if hasattr(b, "entrypoint") else None)
print("is", lief.is_elf("elf-x86_64-static"), lief.is_pe("pe-x86_64.exe"), lief.is_macho("macho-arm64.o"),
      lief.is_elf("pe-x86_64.exe"))
# (not lief.parse of a bare "\x7fELF": LIEF then prints a header it never initialized)
print("garbage", lief.parse(b"\0" * 64), lief.parse(b"MZ" + b"\0" * 62))

# ELF: an executable and a shared library
e = lief.ELF.parse("elf-x86_64-static")
print("elf header", e.header.identity_class, e.header.machine_type, e.header.file_type, hex(e.header.entrypoint))
print("elf sections", [(s.name, s.type, s.size, s.flags_list) for s in e.sections])
print("elf segments", [(s.type, hex(s.virtual_address), s.virtual_size, s.flags) for s in e.segments])
print("elf symbols", sorted((s.name, hex(s.value), s.type, s.binding) for s in e.symtab_symbols if s.name))
print("elf strings", sorted(e.strings)[:5], e.get_section(".rodata").content.tobytes()[:24])
print("elf vaddr", e.get_content_from_virtual_address(e.get_symbol("message").value, 18).tobytes())
so = lief.ELF.parse("elf-aarch64.so")
print("so dynamic", [(d.tag, d.value if not isinstance(d, lief.ELF.DynamicEntryLibrary) else d.name) for d in so.dynamic_entries
                     if d.tag != lief.ELF.DynamicEntry.TAG.NULL][:12])
print("so soname", so[lief.ELF.DynamicEntry.TAG.SONAME].name, sorted(names(so.exported_functions)))
print("so dynsym", sorted((s.name, s.type, s.visibility) for s in so.dynamic_symbols if s.name))
print("so relocations", len(so.relocations), sorted({r.type for r in so.relocations}, key=str))
print("so gnu hash", so.use_gnu_hash, so.gnu_hash.nb_buckets if so.use_gnu_hash else None)

# ELF changes: a section, a needed library, a patched value, written and parsed again
sec = lief.ELF.Section(".collabo")
sec.type = lief.ELF.Section.TYPE.PROGBITS
sec.content = list(b"collabo core!")
e.add(sec, loaded=False)
e.patch_address(e.get_symbol("counter").value, [7, 0, 0, 0])
e.write(os.path.join(tmp, "out.elf"))
again = lief.ELF.parse(os.path.join(tmp, "out.elf"))
print("elf written", again.get_section(".collabo").content.tobytes(),
      again.get_content_from_virtual_address(again.get_symbol("counter").value, 4).tobytes())
so.add_library("libcollabo-extra.so")
so.add_exported_function(so.get_symbol("square").value, "square_alias") if so.has_symbol("square") else None
so.write(os.path.join(tmp, "out.so"))
so2 = lief.ELF.parse(os.path.join(tmp, "out.so"))
print("so written", sorted(lib for lib in so2.libraries), "square_alias" in names(so2.exported_functions))

# PE
pe = lief.PE.parse("pe-x86_64.exe")
print("pe header", pe.header.machine, pe.header.characteristics_list, pe.optional_header.magic,
      hex(pe.optional_header.imagebase), hex(pe.optional_header.addressof_entrypoint), pe.optional_header.subsystem)
print("pe sections", [(s.name, s.virtual_size, s.characteristics_lists) for s in pe.sections])
print("pe imports", [(i.name, names(i.entries)) for i in pe.imports])
print("pe exports", pe.get_export().name if pe.has_exports else None, names(pe.get_export().entries) if pe.has_exports else None)
print("pe dirs", [(d.type, d.size > 0) for d in pe.data_directories][:6])
print("pe hash", pe.authentihash(lief.PE.ALGORITHMS.SHA_256).hex(), pe.authentihash_md5.hex(), pe.compute_checksum())
print("pe signed", pe.has_signatures, pe.verify_signature())
ns = lief.PE.Section(".collab")
ns.content = list(b"\xcc" * 64)
ns.characteristics = lief.PE.Section.CHARACTERISTICS.MEM_READ | lief.PE.Section.CHARACTERISTICS.CNT_INITIALIZED_DATA
pe.add_section(ns)
pe.write(os.path.join(tmp, "out.exe"))
pe2 = lief.PE.parse(os.path.join(tmp, "out.exe"))
print("pe written", names(pe2.sections), pe2.get_section(".collab").content.tobytes()[:4])

# COFF
c = lief.COFF.parse("coff-x86_64.obj")
print("coff", c.header.machine, names(c.sections), sorted(s.name for s in c.symbols if s.name)[:8])

# Mach-O
fat = lief.MachO.parse("macho-arm64.o")
m = fat.at(0)
print("macho object", m.header.cpu_type, m.header.file_type, [cmd.command for cmd in m.commands], names(m.sections))
print("macho symbols", sorted(s.name for s in m.symbols if s.name))
dy = lief.MachO.parse("macho-arm64.dylib").at(0)
print("macho dylib", dy.header.file_type, sorted(names(dy.exported_functions)), [cmd.command for cmd in dy.commands][:8])

# JSON, demangling
doc = json.loads(lief.to_json(e))
print("json", sorted(doc)[:6], len(doc.get("sections", [])))
print("demangle", lief.demangle("_ZN7collabo4core3addEii"), lief.demangle("?add@@YAHHH@Z"))

# C++ exceptions through nanobind
try:
    e.sections[1000]
except IndexError as ex:
    print("IndexError", type(ex).__name__)
print("iteration", len(list(e.sections)), len([s for s in e.segments]))
try:
    lief.ELF.Section(".x").content = "not bytes"
except TypeError:
    print("TypeError")
print("digest", hashlib.sha256(bytes(lief.ELF.parse("elf-aarch64.so").get_section(".text").content)).hexdigest()[:16])
print("done")
