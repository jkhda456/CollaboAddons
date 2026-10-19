//! The system calls std does not wrap or wraps in ways the guest's wasm32 std does not support:
//! duplicating a descriptor (the guest's std `try_clone` is cfg'd out), a pty pair, terminal size,
//! `poll`, non-blocking reads, and spawning a program on the slave side of a pty.

use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use crate::{err, Result};

/// The guest has no `fork`; std names it only in fallbacks this program never takes. This stub
/// completes the link and fails loudly if one ever is.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
extern "C" fn fork() -> libc::pid_t {
    // SAFETY: errno is this thread's own.
    unsafe { *libc::__errno_location() = libc::ENOSYS };
    -1
}

/// Duplicates a descriptor with `F_DUPFD_CLOEXEC` (the guest's std cannot `dup`).
pub fn dup(fd: RawFd) -> Result<OwnedFd> {
    // SAFETY: fcntl on an open descriptor; the result is a fresh owned fd.
    let new = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if new < 0 {
        return Err(err!("cannot duplicate a descriptor: {}", io::Error::last_os_error()));
    }
    // SAFETY: new is a valid, owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(new) })
}

/// A descriptor as a byte stream: both [Read] and [Write] over the same fd (a socket or a pty
/// master). Several may share one connection, each a dup of the fd.
pub struct FdStream {
    fd: OwnedFd,
}

impl FdStream {
    pub fn new(fd: OwnedFd) -> FdStream {
        FdStream { fd }
    }

    /// Two streams over the same connection (one a dup): a reader and a writer.
    pub fn split(fd: OwnedFd) -> Result<(FdStream, FdStream)> {
        let other = dup(fd.as_raw_fd())?;
        Ok((FdStream { fd }, FdStream { fd: other }))
    }


    pub fn set_nonblocking(&self, on: bool) -> Result<()> {
        set_nonblocking(self.fd.as_raw_fd(), on)
    }
}

impl AsRawFd for FdStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Read for FdStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: buf is writable for its length; read reports how many it wrote.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }
}

impl Write for FdStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: buf is readable for its length.
        let n = unsafe { libc::write(self.fd.as_raw_fd(), buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn set_nonblocking(fd: RawFd, on: bool) -> Result<()> {
    // SAFETY: fcntl on an open descriptor.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(err!("F_GETFL: {}", io::Error::last_os_error()));
        }
        let new = if on { flags | libc::O_NONBLOCK } else { flags & !libc::O_NONBLOCK };
        if libc::fcntl(fd, libc::F_SETFL, new) < 0 {
            return Err(err!("F_SETFL: {}", io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// Waits until `fd` is readable, or `timeout_ms` passes (negative: forever). True if readable.
pub fn wait_readable(fd: RawFd, timeout_ms: i32) -> Result<bool> {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    loop {
        // SAFETY: a single valid pollfd.
        let n = unsafe { libc::poll(&mut p, 1, timeout_ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err!("poll: {e}"));
        }
        return Ok(n > 0 && p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0);
    }
}

/// A pseudo-terminal: the master we read/write, and the slave path a child opens as its console.
pub struct Pty {
    pub master: OwnedFd,
    pub slave_path: CString,
}

impl Pty {
    pub fn open() -> Result<Pty> {
        // SAFETY: the standard posix_openpt / grantpt / unlockpt / ptsname_r sequence.
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if master < 0 {
                return Err(err!("posix_openpt: {}", io::Error::last_os_error()));
            }
            let master = OwnedFd::from_raw_fd(master);
            if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
                return Err(err!("grantpt/unlockpt: {}", io::Error::last_os_error()));
            }
            let mut buf = [0 as libc::c_char; 256];
            if libc::ptsname_r(master.as_raw_fd(), buf.as_mut_ptr(), buf.len()) != 0 {
                return Err(err!("ptsname_r: {}", io::Error::last_os_error()));
            }
            let slave_path = std::ffi::CStr::from_ptr(buf.as_ptr()).to_owned();
            Ok(Pty { master, slave_path })
        }
    }

    pub fn set_size(&self, cols: u16, rows: u16) {
        set_winsize(self.master.as_raw_fd(), cols, rows);
    }
}

pub fn set_winsize(fd: RawFd, cols: u16, rows: u16) {
    let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: ws is a valid winsize for the duration of the call.
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
}

/// Spawns `argv` with the pty slave as its stdin/stdout/stderr and controlling terminal, in its
/// own session. `env` is added to the current environment. Returns the child pid.
pub fn spawn_on_pty(pty: &Pty, argv: &[String], env: &[(String, String)]) -> Result<libc::pid_t> {
    if argv.is_empty() {
        return Err(err!("nothing to run"));
    }
    let program = CString::new(argv[0].clone()).map_err(|_| err!("a NUL in the command"))?;
    let c_args: Vec<CString> = argv.iter().map(|a| CString::new(a.as_str()).unwrap()).collect();
    let mut argv_ptrs: Vec<*mut libc::c_char> = c_args.iter().map(|a| a.as_ptr() as *mut _).collect();
    argv_ptrs.push(std::ptr::null_mut());

    // Build the child's environment: the current one plus the extras (TERM etc.).
    let mut env_strings: Vec<CString> = Vec::new();
    for (k, v) in std::env::vars() {
        if !env.iter().any(|(ek, _)| *ek == k) {
            env_strings.push(CString::new(format!("{k}={v}")).unwrap());
        }
    }
    for (k, v) in env {
        env_strings.push(CString::new(format!("{k}={v}")).unwrap());
    }
    let mut env_ptrs: Vec<*mut libc::c_char> = env_strings.iter().map(|e| e.as_ptr() as *mut _).collect();
    env_ptrs.push(std::ptr::null_mut());

    // SAFETY: posix_spawn with file actions that put the pty slave on 0/1/2 and SETSID so the
    // child leads a new session with the pty as its controlling terminal.
    unsafe {
        let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
        libc::posix_spawn_file_actions_init(&mut actions);
        libc::posix_spawn_file_actions_addopen(&mut actions, 0, pty.slave_path.as_ptr(), libc::O_RDWR, 0);
        libc::posix_spawn_file_actions_adddup2(&mut actions, 0, 1);
        libc::posix_spawn_file_actions_adddup2(&mut actions, 0, 2);
        let mut attr: libc::posix_spawnattr_t = std::mem::zeroed();
        libc::posix_spawnattr_init(&mut attr);
        libc::posix_spawnattr_setflags(&mut attr, libc::POSIX_SPAWN_SETSID as libc::c_short);
        let mut pid: libc::pid_t = 0;
        let rc = libc::posix_spawnp(&mut pid, program.as_ptr(), &actions, &attr, argv_ptrs.as_ptr(), env_ptrs.as_ptr());
        libc::posix_spawn_file_actions_destroy(&mut actions);
        libc::posix_spawnattr_destroy(&mut attr);
        if rc != 0 {
            return Err(err!("cannot run {}: {}", argv[0], io::Error::from_raw_os_error(rc)));
        }
        Ok(pid)
    }
}

/// Reaps `pid` if it has exited; returns its exit status (128 + signal for a signalled death).
pub fn try_wait(pid: libc::pid_t) -> Option<i32> {
    let mut status = 0;
    // SAFETY: waitpid with WNOHANG on our own child.
    let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if r == pid {
        if libc::WIFEXITED(status) {
            Some(libc::WEXITSTATUS(status))
        } else if libc::WIFSIGNALED(status) {
            Some(128 + libc::WTERMSIG(status))
        } else {
            None
        }
    } else {
        None
    }
}

/// Connects a TCP socket to `host:port`, resolving the name, with a connect timeout.
pub fn tcp_connect(host: &str, port: u16, timeout: std::time::Duration) -> Result<std::net::TcpStream> {
    use std::net::ToSocketAddrs;
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|e| err!("cannot resolve {host}: {e}"))?.collect();
    if addrs.is_empty() {
        return Err(err!("{host} resolved to no addresses"));
    }
    let mut last = err!("no address for {host}");
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = err!("cannot connect to {host}:{port}: {e}"),
        }
    }
    Err(last)
}

/// Connects a vsock stream socket to `cid:port` (the host is CID 2).
pub fn vsock_connect(cid: u32, port: u32) -> Result<OwnedFd> {
    // SAFETY: a fresh socket, connected with a filled sockaddr_vm, owned on success.
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(err!("{}", io::Error::last_os_error()));
        }
        let owned = OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = cid;
        addr.svm_port = port;
        if libc::connect(fd, &addr as *const libc::sockaddr_vm as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t) != 0 {
            return Err(err!("{}", io::Error::last_os_error()));
        }
        Ok(owned)
    }
}

/// The owned fd out of a TcpStream (so it can be wrapped in [FdStream]s).
pub fn tcp_into_fd(stream: std::net::TcpStream) -> OwnedFd {
    // SAFETY: into_raw_fd yields ownership of a valid descriptor.
    unsafe { OwnedFd::from_raw_fd(stream.into_raw_fd()) }
}
