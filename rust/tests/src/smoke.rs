// The guest's std, end to end: unwinding panics (caught, and their destructors run), threads,
// thread-locals, processes (spawned with pre_exec/env/cwd: the vfork path), files, time.
use std::cell::Cell;
use std::os::unix::process::CommandExt;
use std::process::Command;

struct Noisy(&'static str);
impl Drop for Noisy {
    fn drop(&mut self) {
        println!("drop {}", self.0);
    }
}

thread_local!(static TL: Cell<u32> = Cell::new(1));

fn deep(n: u32) -> u32 {
    let _g = Noisy(if n == 0 { "deep 0" } else { "deep n" });
    if n == 0 {
        panic!("boom at the bottom");
    }
    deep(n - 1) + 1
}

fn main() {
    println!("hello from rust {}", std::env::args().count());
    let r = std::panic::catch_unwind(|| deep(2));
    println!("caught: {}", r.is_err());
    let r = std::panic::catch_unwind(|| -> i32 { std::panic::resume_unwind(Box::new(7i32)) });
    println!("payload: {:?}", r.unwrap_err().downcast_ref::<i32>());

    let h = std::thread::spawn(|| {
        TL.with(|t| t.set(5));
        std::panic::catch_unwind(|| panic!("in a thread")).is_err() as u32 + TL.with(|t| t.get())
    });
    println!("thread: {} main tl {}", h.join().unwrap(), TL.with(|t| t.get()));
    let p = std::thread::spawn(|| panic!("uncaught in thread")).join();
    println!("join of panicked thread: {}", p.is_err());

    // posix_spawn path
    let out = Command::new("/bin/echo").arg("spawned").output().unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
    // the vfork path: a pre_exec closure, a changed PATH, a cwd and env
    let out = unsafe {
        Command::new("sh")
            .args(["-c", "echo vfork $FOO $(pwd)"])
            .env("FOO", "bar")
            .env("PATH", "/bin:/usr/bin")
            .current_dir("/tmp")
            .pre_exec(|| Ok(()))
            .output()
            .unwrap()
    };
    print!("{}", String::from_utf8_lossy(&out.stdout));
    let e = unsafe { Command::new("/nonexistent").pre_exec(|| Ok(())).spawn() };
    println!("missing program: {:?}", e.err().map(|e| e.kind()));
    println!("env after: FOO={:?}", std::env::var("FOO").ok());

    std::fs::write("/tmp/smoke.txt", b"12345").unwrap();
    println!("file: {}", std::fs::read_to_string("/tmp/smoke.txt").unwrap());
    let t = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(20));
    println!("slept: {}", t.elapsed().as_millis() >= 20);
    if std::env::args().nth(1).as_deref() == Some("panic") {
        let _g = Noisy("main");
        panic!("uncaught in main");
    }
}
