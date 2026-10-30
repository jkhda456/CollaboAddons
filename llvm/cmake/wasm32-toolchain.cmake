# CMake toolchain for the cross stage of build.sh: programs for the guest (wasm32 Linux, musl),
# compiled by the native stage's clang. build.sh passes the compile and link flags itself (the ones
# userspace/bin/wasm-cc uses); this file names the tools and keeps CMake's checks and searches
# away from the build machine.
#
#   -DWASM_SYSROOT=userspace/sysroot -DWASM_BIN=build/native/bin

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR wasm32)

# try_compile projects read this file again: they get the two variables too
list(APPEND CMAKE_TRY_COMPILE_PLATFORM_VARIABLES WASM_SYSROOT WASM_BIN)

set(CMAKE_C_COMPILER   "${WASM_BIN}/clang")
set(CMAKE_CXX_COMPILER "${WASM_BIN}/clang++")
set(CMAKE_ASM_COMPILER "${WASM_BIN}/clang")
set(CMAKE_AR     "${WASM_BIN}/llvm-ar")
set(CMAKE_RANLIB "${WASM_BIN}/llvm-ranlib")

# Checks build static libraries, so they compile but never link: a link check cannot fail.
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# Programs (tblgen, python) come from the build machine; headers and libraries from the sysroot.
set(CMAKE_FIND_ROOT_PATH "${WASM_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
