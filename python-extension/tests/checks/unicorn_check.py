"""unicorn, the CPU emulator, through its binding (ctypes): every architecture it has running code
that loops, multiplies and divides, and (x86, ARM64) SIMD and floating point, which TCG does in
helpers; x86 in 16, 32 and 64-bit modes with the x87 FPU and SSE; hooks — code, blocks, memory
reads and writes, unmapped memory mapped by the hook so that the run goes on, interrupts, IN/OUT,
SYSCALL, CPUID, invalid instructions — each a Python callback; memory regions, protections and
their errors, MMIO; contexts saved and restored; stopping after a count and at exits; the
controls. Nothing printed depends on the host: tests/reference.py runs it with unicorn's own
wheel, which translates to x86-64 code, and the guest's must print the same with its
interpreter. The code bytes were assembled with LLVM (M68K's by hand), each listed with its source.

The same for the earlier versions the add-on carries as wheels (tests/test_guest.py installs each
with pip): what a version has not is left out — Unicorn 1 has no PPC, RISC-V, S390X and TriCore
(PPC is named, not supported), no CPUID hook, no MMIO and no controls (uc_ctl, exits).
"""
from unicorn import *
from unicorn.arm64_const import *
from unicorn.arm_const import *
from unicorn.m68k_const import *
from unicorn.mips_const import *
from unicorn.sparc_const import *
from unicorn.x86_const import *
try:
    from unicorn.ppc_const import *
    from unicorn.riscv_const import *
    from unicorn.s390x_const import *
    from unicorn.tricore_const import *
except ImportError:   # Unicorn 1
    pass

BASE = 0x10000
V2 = uc_version()[0] >= 2


def run(arch, mode, code, regs, setup=None, base=BASE):
    uc = Uc(arch, mode)
    uc.mem_map(base, 0x10000)
    uc.mem_write(base, code)
    if setup:
        setup(uc)
    uc.emu_start(base, base + len(code))
    return [hex(uc.reg_read(r)) for r in regs]


print("version", uc_version()[:2], version_bind()[:2])
arches = {"arm": UC_ARCH_ARM, "arm64": UC_ARCH_ARM64, "mips": UC_ARCH_MIPS, "x86": UC_ARCH_X86, "ppc": UC_ARCH_PPC,
          "sparc": UC_ARCH_SPARC, "m68k": UC_ARCH_M68K, "riscv": globals().get("UC_ARCH_RISCV"),
          "s390x": globals().get("UC_ARCH_S390X"), "tricore": globals().get("UC_ARCH_TRICORE")}
supported = [name for name, a in arches.items() if a is not None and uc_arch_supported(a)]
print("supported", supported)

# ── x86 ─────────────────────────────────────────────────────────────────────────────────────────
# x86-64: fib(90) in a loop; div, imul, bsr; SSE paddq/pshufd/pxor; cvtsi2sd, sqrtsd, mulsd,
# cvttsd2si; push/pop; rol
X64 = bytes.fromhex(
    "48c7c15a00000031c0bb01000000488d14184889d84889d348ffc975f14989c049c7c143420f0031d249f7f14c69d2393000004d0fbdd8"
    "66490f6ec066490f6eca660fd4c1660f70d04e660fefc266490f7ec4f2490f2ad9f20f51dbf20f59dbf24c0f2ceb4154415e49c7c734"
    "12000049c1c70d90")
print("x86-64", run(UC_ARCH_X86, UC_MODE_64, X64,
                    [UC_X86_REG_R8, UC_X86_REG_RDX, UC_X86_REG_R10, UC_X86_REG_R11, UC_X86_REG_R12, UC_X86_REG_R13,
                     UC_X86_REG_R14, UC_X86_REG_R15, UC_X86_REG_EFLAGS],
                    lambda uc: uc.reg_write(UC_X86_REG_RSP, BASE + 0x8000)))
# x86-32: a bitwise CRC-32 over 200 rounds (loop); x87 fld1/fldpi/fmulp/fsqrt/fistp
X86 = bytes.fromhex("b8ffffffffb9c800000089c2d1e883e201f7da81e22083b8ed31d0345ae2ebd9e8d9ebdec9d9fadb1c248b1c2490")
print("x86-32", run(UC_ARCH_X86, UC_MODE_32, X86, [UC_X86_REG_EAX, UC_X86_REG_EBX, UC_X86_REG_ECX],
                    lambda uc: uc.reg_write(UC_X86_REG_ESP, BASE + 0x8000)))
# x86-16: mov ax, 0x1234; add ax, 0x1111; xchg ah, al; inc bx
print("x86-16", run(UC_ARCH_X86, UC_MODE_16, bytes.fromhex("b83412050111" "86e0" "43"),
                    [UC_X86_REG_AX, UC_X86_REG_BX], base=0x1000))
# sample_x86's: inc ecx; dec edx; pxor xmm0, xmm1
uc = Uc(UC_ARCH_X86, UC_MODE_32)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, b"\x41\x4a\x66\x0f\xef\xc1")
uc.reg_write(UC_X86_REG_ECX, 0x1234)
uc.reg_write(UC_X86_REG_EDX, 0x7890)
uc.reg_write(UC_X86_REG_XMM0, 0x000102030405060708090A0B0C0D0E0F)
uc.reg_write(UC_X86_REG_XMM1, 0x00102030405060708090A0B0C0D0E0F0)
uc.emu_start(BASE, BASE + 6)
print("sample_x86", hex(uc.reg_read(UC_X86_REG_ECX)), hex(uc.reg_read(UC_X86_REG_EDX)), hex(uc.reg_read(UC_X86_REG_XMM0)))

# ── the other architectures ─────────────────────────────────────────────────────────────────────
# ARM: sum 100..1; mul, mla, eor with a rotated register, clz
ARM = bytes.fromhex("0000a0e36410a0e3010080e0011051e2fcffff1a2520a0e3900203e0930224e0e35324e0156f6fe100f020e3")
print("arm", run(UC_ARCH_ARM, UC_MODE_ARM, ARM, [UC_ARM_REG_R0, UC_ARM_REG_R3, UC_ARM_REG_R4, UC_ARM_REG_R5, UC_ARM_REG_R6]))
# Thumb-2: sum 50..1; lsls, rev (started at an odd address: Thumb)
THUMB = bytes.fromhex("0020322140180139fcd1c20013ba00bf")
uc = Uc(UC_ARCH_ARM, UC_MODE_THUMB)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, THUMB)
uc.emu_start(BASE | 1, BASE + len(THUMB))
print("thumb", [hex(uc.reg_read(r)) for r in (UC_ARM_REG_R0, UC_ARM_REG_R2, UC_ARM_REG_R3)])
# ARM64: fib(80); udiv, msub, mul, rbit; scvtf, fsqrt, fmul, fcvtzs; dup and add of a vector, umov
ARM64 = bytes.fromhex(
    "000080d2210080d2020a80d20300018be00301aae10303aa420400f181ffff54247e80d20508c49aa680049bc77c049be800c0da8000629e"
    "01c0611e2208611e4900789e830c044e6484a34e8a3c140e1f2003d5")
# (Unicorn 1 starts ARM64 with FP and SIMD disabled, CPACR_EL1.FPEN 0: an instruction of them
# would be an exception; Unicorn 2 starts with them enabled)
print("arm64", run(UC_ARCH_ARM64, UC_MODE_ARM, ARM64,
                   [UC_ARM64_REG_X0, UC_ARM64_REG_X5, UC_ARM64_REG_X6, UC_ARM64_REG_X7, UC_ARM64_REG_X8, UC_ARM64_REG_X9,
                    UC_ARM64_REG_X10], None if V2 else lambda uc: uc.reg_write(UC_ARM64_REG_CPACR_EL1, 3 << 20)))
# MIPS32 little-endian: sum 100..1 (branch delay slots); mult/mflo, divu/mfhi/mflo, sll
MIPSEL = bytes.fromhex(
    "000002246400032421104300ffff6324fdff601400000000070004241800440012280000020080141b00a4000d000700122800001030000012"
    "380000c040070000000000")
print("mipsel", run(UC_ARCH_MIPS, UC_MODE_MIPS32 + UC_MODE_LITTLE_ENDIAN, MIPSEL,
                    [UC_MIPS_REG_2, UC_MIPS_REG_5, UC_MIPS_REG_6, UC_MIPS_REG_7, UC_MIPS_REG_8]))
# sample_mips's: ori $at, $at, 0x3456 (big- and little-endian)
for mode, code in ((UC_MODE_BIG_ENDIAN, b"\x34\x21\x34\x56"), (UC_MODE_LITTLE_ENDIAN, b"\x56\x34\x21\x34")):
    print("sample_mips", run(UC_ARCH_MIPS, UC_MODE_MIPS32 + mode, code, [UC_MIPS_REG_1],
                             lambda uc: uc.reg_write(UC_MIPS_REG_1, 0x6789)))
# PPC32 (big-endian): sum 100..1 with ctr; mullw, divw, rlwinm
PPC = bytes.fromhex("38600000388000647c8903a67c6322143884ffff4200fff838a0000d7cc329d67ce623d654c8283e60000000")
if "ppc" in supported:
    print("ppc", run(UC_ARCH_PPC, UC_MODE_PPC32 + UC_MODE_BIG_ENDIAN, PPC,
                     [UC_PPC_REG_3, UC_PPC_REG_6, UC_PPC_REG_7, UC_PPC_REG_8]))
# RISC-V 64 (compressed instructions): sum 100..1; mul, divu, remu, slli
RISCV = bytes.fromhex("0145930540062e95fd15f5fd13061006b306c50233d7b602b3f7c602139846010100")
if "riscv" in supported:
    print("riscv64", run(UC_ARCH_RISCV, UC_MODE_RISCV64, RISCV,
                         [UC_RISCV_REG_A0, UC_RISCV_REG_A3, UC_RISCV_REG_A4, UC_RISCV_REG_A5, UC_RISCV_REG_A6]))
# SPARC (big-endian): sum 100..1 (delay slot); sll, xor
SPARC = bytes.fromhex("82102000841020648200400284a0a00112bffffe01000000872860048818c00101000000")
print("sparc", run(UC_ARCH_SPARC, UC_MODE_SPARC32 + UC_MODE_BIG_ENDIAN, SPARC,
                   [UC_SPARC_REG_G1, UC_SPARC_REG_G3, UC_SPARC_REG_G4]))
# S390X: sum 100..1 with brctg; msgfi, sllg
S390X = bytes.fromhex("a7290000a7390064b9080023a737fffeb9040042c24000000021eb54000c000d0707")
if "s390x" in supported:
    print("s390x", run(UC_ARCH_S390X, UC_MODE_BIG_ENDIAN, S390X, [UC_S390X_REG_R2, UC_S390X_REG_R4, UC_S390X_REG_R5]))
# M68K: moveq #0,d0; moveq #100,d1; 1: add.l d1,d0; subq.l #1,d1; bne.s 1b; move.l d0,d2; lsl.l #3,d2; nop
M68K = bytes.fromhex("70007264d081538166fa2400e78a4e71")
print("m68k", run(UC_ARCH_M68K, UC_MODE_BIG_ENDIAN, M68K, [UC_M68K_REG_D0, UC_M68K_REG_D2]))
# sample_tricore's: mov d1, #0x1; mov.u d0, #0x8000
if "tricore" in supported:
    print("tricore", run(UC_ARCH_TRICORE, UC_MODE_LITTLE_ENDIAN, bytes.fromhex("8211bb000008"),
                         [UC_TRICORE_REG_D0, UC_TRICORE_REG_D1]))

# ── hooks ───────────────────────────────────────────────────────────────────────────────────────
uc = Uc(UC_ARCH_X86, UC_MODE_64)
uc.mem_map(0x400000, 0x10000)
# mov rax, 0x1122334455667788; mov [0x401000], rax; mov rbx, [0x401000]; add rbx, 1; jmp +0; nop
code = (bytes.fromhex("48b88877665544332211") + bytes.fromhex("48890425") + (0x401000).to_bytes(4, "little")
        + bytes.fromhex("488b1c25") + (0x401000).to_bytes(4, "little") + bytes.fromhex("4883c301") + b"\xeb\x00\x90")
uc.mem_write(0x400000, code)
seen = []
uc.hook_add(UC_HOOK_CODE, lambda uc, addr, size, ud: seen.append(("code", hex(addr), size)))
uc.hook_add(UC_HOOK_BLOCK, lambda uc, addr, size, ud: seen.append(("block", hex(addr), size)))
uc.hook_add(UC_HOOK_MEM_READ | UC_HOOK_MEM_WRITE,
            lambda uc, access, addr, size, value, ud: seen.append(("mem", access, hex(addr), size, hex(value & 0xffffffffffffffff))))
uc.emu_start(0x400000, 0x400000 + len(code))
print("hooks", hex(uc.reg_read(UC_X86_REG_RBX)))
for s in seen:
    print("  ", s)

# unmapped memory: the hook maps it and the run goes on; a write to a read-only page is an error
uc = Uc(UC_ARCH_X86, UC_MODE_32)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, bytes.fromhex("a100002000" "40" "a300002000"))   # mov eax, [0x200000]; inc eax; mov [0x200000], eax
faults = []

def unmapped(uc, access, addr, size, value, ud):
    faults.append((access, hex(addr), size))
    uc.mem_map(addr & ~0xfff, 0x1000)
    uc.mem_write(addr, (41).to_bytes(4, "little"))
    return True

uc.hook_add(UC_HOOK_MEM_UNMAPPED, unmapped)
uc.emu_start(BASE, BASE + 11)
print("unmapped", faults, uc.reg_read(UC_X86_REG_EAX), int.from_bytes(uc.mem_read(0x200000, 4), "little"))
uc.mem_protect(0x200000, 0x1000, UC_PROT_READ)
try:
    uc.emu_start(BASE, BASE + 11)
except UcError as e:
    print("protected", e.errno, e)
print("regions", [(hex(b), hex(e), p) for b, e, p in uc.mem_regions()])
uc.mem_unmap(0x200000, 0x1000)
try:
    uc.mem_read(0x200000, 4)
except UcError as e:
    print("unmapped read", e)

# interrupts, IN/OUT, SYSCALL, CPUID, invalid instructions
uc = Uc(UC_ARCH_X86, UC_MODE_32)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, bytes.fromhex("cd80" "e510" "e612"))   # int 0x80; in eax, 0x10; out 0x12, al
events = []
uc.hook_add(UC_HOOK_INTR, lambda uc, intno, ud: events.append(("intr", intno)))
uc.hook_add(UC_HOOK_INSN, lambda uc, port, size, ud: events.append(("in", port, size)) or 0x55aa, None, 1, 0, UC_X86_INS_IN)
uc.hook_add(UC_HOOK_INSN, lambda uc, port, size, value, ud: events.append(("out", port, size, hex(value))), None, 1, 0,
            UC_X86_INS_OUT)
uc.emu_start(BASE, BASE + 6)
print("x86 events", events, hex(uc.reg_read(UC_X86_REG_EAX)))
uc = Uc(UC_ARCH_X86, UC_MODE_64)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, bytes.fromhex("0f05" "0fa2"))   # syscall; cpuid
events = []
uc.hook_add(UC_HOOK_INSN, lambda uc, ud: events.append(("syscall", uc.reg_read(UC_X86_REG_RAX))), None, 1, 0, UC_X86_INS_SYSCALL)
if V2:
    uc.hook_add(UC_HOOK_INSN, lambda uc, ud: events.append(("cpuid", uc.reg_read(UC_X86_REG_EAX))) or 1, None, 1, 0,
                UC_X86_INS_CPUID)
uc.reg_write(UC_X86_REG_RAX, 60)
uc.emu_start(BASE, BASE + 4)
print("x86-64 events", events)
uc = Uc(UC_ARCH_X86, UC_MODE_32)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, b"\x0f\x0b")   # ud2
uc.hook_add(UC_HOOK_INSN_INVALID, lambda uc, ud: events.append(("invalid", hex(uc.reg_read(UC_X86_REG_EIP)))) or False)
try:
    uc.emu_start(BASE, BASE + 2)
except UcError as e:
    print("invalid", events[-1], e)

# MMIO
if V2:
    uc = Uc(UC_ARCH_ARM64, UC_MODE_ARM)
    uc.mem_map(BASE, 0x1000)
    uc.mmio_map(0x20000, 0x1000, lambda uc, off, size, ud: 0x100 + off, None,
                lambda uc, off, size, value, ud: events.append(("mmio write", hex(off), size, hex(value))), None)
    # ldr w1, [x0, #8]; add w1, w1, #1; str w1, [x0, #12]
    uc.mem_write(BASE, bytes.fromhex("010840b9" "21040011" "010c00b9"))
    uc.reg_write(UC_ARM64_REG_X0, 0x20000)
    uc.emu_start(BASE, BASE + 12)
    print("mmio", hex(uc.reg_read(UC_ARM64_REG_X1)), events[-1])

# ── contexts, counts, exits, controls ───────────────────────────────────────────────────────────
uc = Uc(UC_ARCH_ARM, UC_MODE_ARM)
uc.mem_map(BASE, 0x1000)
uc.mem_write(BASE, ARM)
saved = uc.context_save()
uc.emu_start(BASE, BASE + len(ARM), count=4)
print("count", hex(uc.reg_read(UC_ARM_REG_PC)), uc.reg_read(UC_ARM_REG_R0), uc.reg_read(UC_ARM_REG_R1))
uc.context_restore(saved)
print("restored", hex(uc.reg_read(UC_ARM_REG_PC)), uc.reg_read(UC_ARM_REG_R0), uc.reg_read(UC_ARM_REG_R1))
if V2:
    uc.ctl_exits_enabled(True)
    uc.ctl_set_exits([BASE + 0x14])
    uc.emu_start(BASE, 0)
    print("exit", hex(uc.reg_read(UC_ARM_REG_PC)), uc.reg_read(UC_ARM_REG_R0), uc.ctl_get_exits())
    print("ctl", uc.ctl_get_mode(), uc.ctl_get_arch(), uc.ctl_get_page_size(), uc.query(UC_QUERY_MODE))
    uc.ctl_flush_tb()
    uc.ctl_remove_cache(BASE, BASE + 0x1000)
else:
    uc.emu_start(BASE, BASE + 0x14)
    print("until", hex(uc.reg_read(UC_ARM_REG_PC)), uc.reg_read(UC_ARM_REG_R0), uc.query(UC_QUERY_MODE),
          uc.query(UC_QUERY_PAGE_SIZE))
print("errors", [str(UcError(e)) for e in (UC_ERR_OK, UC_ERR_NOMEM, UC_ERR_READ_UNMAPPED, UC_ERR_FETCH_PROT, UC_ERR_INSN_INVALID)])
print("done")
