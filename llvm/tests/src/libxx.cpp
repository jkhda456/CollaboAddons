// A C++ shared library (c++ -shared): libc++ in it (statically, its own copy), a static
// constructor (__wasm_call_ctors, which the loader runs), exceptions thrown and caught inside it.
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

static std::vector<std::string> made = {"made", "by", "a", "constructor"};

static int risky(int n) {
  if (n > 2) throw std::range_error("too big: " + std::to_string(n));
  return n * 10;
}

extern "C" int libxx_entry(int n, char *out, int size) {
  std::string s;
  for (auto &w : made) s += w + " ";
  try {
    s += std::to_string(risky(n));
  } catch (const std::exception &e) {
    s += std::string("caught ") + e.what();
  }
  snprintf(out, size, "%s", s.c_str());
  return (int)s.size();
}
