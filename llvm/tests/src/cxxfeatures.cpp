// libc++ with exceptions in a program of the guest's: throw and catch (through frames with
// destructors to run), rethrow, exception_ptr across threads, RTTI, static constructors, threads,
// containers, std::filesystem, std::format.
#include <algorithm>
#include <atomic>
#include <exception>
#include <filesystem>
#include <format>
#include <fstream>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <typeinfo>
#include <variant>
#include <vector>

struct Init {
  Init() { std::cout << "static constructor" << std::endl; }
} init;

struct Guard {
  const char *name;
  ~Guard() { std::cout << "unwound " << name << std::endl; }
};

struct MyError : std::runtime_error {
  int code;
  MyError(int c) : std::runtime_error("my error"), code(c) {}
};

[[gnu::noinline]] static void deep(int n) {
  Guard g{n == 0 ? "frame 0" : "frame 1"};
  if (n == 0) throw MyError(7);
  deep(n - 1);
}

struct Base { virtual ~Base() = default; };
struct Derived : Base { int x = 5; };

int main() {
  try {
    deep(1);
  } catch (const MyError &e) {
    std::cout << "caught " << e.what() << " code " << e.code << std::endl;
  }
  try {
    try {
      (void)std::vector<int>().at(3);
    } catch (...) {
      std::cout << "rethrowing" << std::endl;
      throw;
    }
  } catch (const std::out_of_range &e) {
    std::cout << "out_of_range" << std::endl;
  }

  std::exception_ptr ep;
  std::thread t([&] {
    try {
      throw std::invalid_argument("from a thread");
    } catch (...) {
      ep = std::current_exception();
    }
  });
  t.join();
  try {
    std::rethrow_exception(ep);
  } catch (const std::exception &e) {
    std::cout << "exception_ptr: " << e.what() << std::endl;
  }

  std::unique_ptr<Base> b = std::make_unique<Derived>();
  if (auto *d = dynamic_cast<Derived *>(b.get())) std::cout << "dynamic_cast: " << d->x << std::endl;
  Base &r = *b;
  std::cout << "typeid: " << (typeid(r) == typeid(Derived)) << std::endl;

  std::atomic<int> sum{0};
  std::mutex m;
  std::vector<std::thread> ts;
  std::vector<int> order;
  for (int i = 1; i <= 4; i++)
    ts.emplace_back([&, i] {
      sum += i;
      std::lock_guard<std::mutex> l(m);
      order.push_back(i);
    });
  for (auto &th : ts) th.join();
  std::sort(order.begin(), order.end());
  std::cout << "threads: " << sum << " " << order.size() << std::endl;

  std::map<std::string, int> words;
  std::istringstream in("b a c a b a");
  for (std::string w; in >> w;) words[w]++;
  for (auto &[w, n] : words) std::cout << w << "=" << n << " ";
  std::cout << std::endl;

  std::variant<int, std::string> v = std::string("variant");
  std::optional<int> o = 3;
  std::cout << std::get<std::string>(v) << " " << *o << std::endl;

  namespace fs = std::filesystem;
  fs::create_directories("fsdir/sub");
  { std::ofstream("fsdir/sub/f.txt") << "12345"; }
  std::cout << "filesystem: " << fs::file_size("fsdir/sub/f.txt") << " " << fs::exists("fsdir/sub") << std::endl;
  fs::remove_all("fsdir");

  std::cout << std::format("format: {} {:>5} {:.3f}", 42, "x", 3.14159) << std::endl;
  return 0;
}
