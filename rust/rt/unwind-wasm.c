/* libunwind.a for wasm32-unknown-linux-musl: what Rust's `unwind` crate (its libunwind bindings,
 * as on every unix target) asks for, over WebAssembly exception handling, after LLVM's
 * libunwind/src/Unwind-wasm.c (Apache-2.0 WITH LLVM-exception).
 *
 * A panic is thrown with the `throw` instruction and the C++ exception tag (`__cpp_exception`,
 * which compiler-rt defines, in every program and in every side module); the engine unwinds,
 * running the landing pads (cleanups and `catch_unwind`'s catch-all) on the way. There are no
 * unwind tables, so a backtrace has no frames (_Unwind_Backtrace stops at once).
 *
 * All hidden: each program and each side module has its own copy. */
#include <stdint.h>

#define HIDDEN __attribute__((visibility("hidden")))

typedef int _Unwind_Reason_Code;
enum {
	_URC_NO_REASON = 0,
	_URC_FOREIGN_EXCEPTION_CAUGHT = 1,
	_URC_END_OF_STACK = 5,
	_URC_CONTINUE_UNWIND = 8,
};
typedef int _Unwind_Action;
#define _UA_SEARCH_PHASE 1

struct _Unwind_Exception {
	uint64_t exception_class;
	void (*exception_cleanup)(_Unwind_Reason_Code, struct _Unwind_Exception *);
	uintptr_t private_1, private_2;
} __attribute__((__aligned__));

/* What the personality routine reads and writes; LLVM's landing pads fill it in. */
struct _Unwind_LandingPadContext {
	uintptr_t lpad_index;
	uintptr_t lsda;
	uintptr_t selector;
};
_Thread_local struct _Unwind_LandingPadContext __wasm_lpad_context;

/* C++'s personality routine (libc++abi), when C++ code with typed catch clauses is linked in;
 * Rust's catch_unwind catches everything and never asks. */
__attribute__((weak)) _Unwind_Reason_Code __gxx_personality_wasm0(int, _Unwind_Action, uint64_t,
	struct _Unwind_Exception *, void *);

HIDDEN _Unwind_Reason_Code _Unwind_CallPersonality(void *exception_ptr)
{
	struct _Unwind_Exception *e = exception_ptr;
	__wasm_lpad_context.selector = 0;
	if (!__gxx_personality_wasm0)
		return _URC_CONTINUE_UNWIND;
	return __gxx_personality_wasm0(1, _UA_SEARCH_PHASE, e->exception_class, e,
	                               &__wasm_lpad_context);
}

HIDDEN _Unwind_Reason_Code _Unwind_RaiseException(struct _Unwind_Exception *e)
{
	__builtin_wasm_throw(0, e);
}

HIDDEN void _Unwind_Resume(struct _Unwind_Exception *e)
{
	__builtin_wasm_throw(0, e);
}

HIDDEN void _Unwind_DeleteException(struct _Unwind_Exception *e)
{
	if (e->exception_cleanup)
		e->exception_cleanup(_URC_FOREIGN_EXCEPTION_CAUGHT, e);
}

HIDDEN void _Unwind_SetGR(void *context, int index, uintptr_t value)
{
	if (index == 1)
		((struct _Unwind_LandingPadContext *)context)->selector = value;
}

HIDDEN uintptr_t _Unwind_GetGR(void *context, int index)
{
	return index == 1 ? ((struct _Unwind_LandingPadContext *)context)->selector : 0;
}

HIDDEN uintptr_t _Unwind_GetIP(void *context)
{
	return ((struct _Unwind_LandingPadContext *)context)->lpad_index + 2;
}

HIDDEN uintptr_t _Unwind_GetIPInfo(void *context, int *ip_before_insn)
{
	if (ip_before_insn)
		*ip_before_insn = 0;
	return _Unwind_GetIP(context);
}

HIDDEN void _Unwind_SetIP(void *context, uintptr_t value)
{
	(void)context; (void)value;
}

HIDDEN uintptr_t _Unwind_GetLanguageSpecificData(void *context)
{
	return ((struct _Unwind_LandingPadContext *)context)->lsda;
}

HIDDEN uintptr_t _Unwind_GetRegionStart(void *context) { (void)context; return 0; }
HIDDEN uintptr_t _Unwind_GetTextRelBase(void *context) { (void)context; return 0; }
HIDDEN uintptr_t _Unwind_GetDataRelBase(void *context) { (void)context; return 0; }
HIDDEN uintptr_t _Unwind_GetCFA(void *context) { (void)context; return 0; }
HIDDEN void *_Unwind_FindEnclosingFunction(void *pc) { (void)pc; return 0; }

HIDDEN _Unwind_Reason_Code _Unwind_Backtrace(_Unwind_Reason_Code (*fn)(void *, void *), void *arg)
{
	(void)fn; (void)arg;
	return _URC_END_OF_STACK;
}
