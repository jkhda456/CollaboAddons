/* The memory-mapping calls the guest's C library leaves out (WebAssembly has no MMU, and musl's
 * wasm32 port has no mmap, munmap, mremap, mprotect, madvise, msync, mlock, mincore), for the Rust
 * programs of wasm32-unknown-linux-musl: the crates that map files (memmap2: gix, ripgrep, ...)
 * link and work. build.sh adds this to the target's self-contained libc.a.
 *
 * A mapping is a copy: memory from malloc, page-aligned, filled from the file with pread (zeros
 * for anonymous memory and past the end of the file). What a shared, writable mapping holds goes
 * back to the file on msync and munmap. Unlike a real mapping, a copy does not see later writes to
 * the file by others, and others see the mapping's writes only once they are written back. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define PAGE 65536

/* the Linux values (the C library's header leaves them out on wasm; Rust's libc crate has them) */
#ifndef MAP_SHARED
#define MAP_SHARED 0x01
#define MAP_PRIVATE 0x02
#define MAP_FIXED 0x10
#define MAP_ANONYMOUS 0x20
#endif
#ifndef PROT_WRITE
#define PROT_READ 1
#define PROT_WRITE 2
#endif
#ifndef MREMAP_MAYMOVE
#define MREMAP_MAYMOVE 1
#endif

struct map {
	struct map *next;
	char *addr;
	size_t len;
	int fd;        /* a duplicate, for writing back (shared and writable); else -1 */
	off_t off;
};

static struct map *maps;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;

static struct map *find(void *addr, struct map ***link)
{
	struct map **l = &maps;
	for (; *l; l = &(*l)->next) {
		char *a = (*l)->addr;
		if ((char *)addr >= a && (char *)addr < a + (*l)->len) {
			if (link) *link = l;
			return *l;
		}
	}
	return 0;
}

static int fill(char *p, size_t len, int fd, off_t off)
{
	size_t done = 0;
	while (done < len) {
		ssize_t n = pread(fd, p + done, len - done, off + done);
		if (n < 0) {
			if (errno == EINTR) continue;
			return -1;
		}
		if (n == 0) break;
		done += n;
	}
	memset(p + done, 0, len - done);
	return 0;
}

static int write_back(struct map *m, char *from, size_t len)
{
	if (m->fd < 0) return 0;
	off_t off = m->off + (from - m->addr);
	size_t done = 0;
	/* not past the end of the file: a mapping never grows it */
	off_t end = lseek(m->fd, 0, SEEK_END);
	if (end >= 0 && off + (off_t)len > end) len = end > off ? (size_t)(end - off) : 0;
	while (done < len) {
		ssize_t n = pwrite(m->fd, from + done, len - done, off + done);
		if (n < 0) {
			if (errno == EINTR) continue;
			return -1;
		}
		done += n;
	}
	return 0;
}

void *mmap(void *start, size_t len, int prot, int flags, int fd, off_t off)
{
	if (!len || off < 0) {
		errno = EINVAL;
		return MAP_FAILED;
	}
	if (len >= PTRDIFF_MAX) {
		errno = ENOMEM;
		return MAP_FAILED;
	}
	int anon = flags & MAP_ANONYMOUS;
	if (!anon && fcntl(fd, F_GETFL) < 0) {
		errno = EBADF;
		return MAP_FAILED;
	}
	pthread_mutex_lock(&lock);
	if (flags & MAP_FIXED) {
		/* only over memory this already mapped: refill it */
		struct map *m = find(start, 0);
		if (!m || (char *)start + len > m->addr + m->len) {
			pthread_mutex_unlock(&lock);
			errno = ENOMEM;
			return MAP_FAILED;
		}
		int r = anon ? (memset(start, 0, len), 0) : fill(start, len, fd, off);
		pthread_mutex_unlock(&lock);
		return r ? MAP_FAILED : start;
	}
	struct map *m = malloc(sizeof *m);
	char *p = 0;
	if (!m || posix_memalign((void **)&p, PAGE, len)) {
		free(m);
		pthread_mutex_unlock(&lock);
		errno = ENOMEM;
		return MAP_FAILED;
	}
	if (anon ? (memset(p, 0, len), 0) : fill(p, len, fd, off)) {
		int e = errno;
		free(p); free(m);
		pthread_mutex_unlock(&lock);
		errno = e;
		return MAP_FAILED;
	}
	m->addr = p;
	m->len = len;
	m->off = off;
	m->fd = -1;
	if (!anon && (flags & MAP_SHARED) && (prot & PROT_WRITE))
		m->fd = fcntl(fd, F_DUPFD_CLOEXEC, 0);
	m->next = maps;
	maps = m;
	pthread_mutex_unlock(&lock);
	return p;
}

int munmap(void *start, size_t len)
{
	struct map **link, *m;
	pthread_mutex_lock(&lock);
	m = find(start, &link);
	if (m) {
		size_t end = (char *)start - m->addr + len;
		if (end > m->len) end = m->len;
		write_back(m, start, m->addr + end - (char *)start);
		if ((char *)start == m->addr && end == m->len) {
			/* all of it */
			*link = m->next;
			if (m->fd >= 0) close(m->fd);
			free(m->addr);
			free(m);
		} else if ((char *)start == m->addr) {
			/* its beginning: keep the rest (the memory stays allocated until all of it goes) */
			m->off += end; m->len -= end; m->addr += end;
		} else if (end == m->len) {
			m->len = (char *)start - m->addr;
		}
		/* a hole in the middle: left as it is */
	}
	pthread_mutex_unlock(&lock);
	return 0;
}

int msync(void *start, size_t len, int flags)
{
	(void)flags;
	pthread_mutex_lock(&lock);
	struct map *m = find(start, 0);
	int r = 0;
	if (m) {
		size_t end = (char *)start - m->addr + len;
		if (end > m->len) end = m->len;
		r = write_back(m, start, m->addr + end - (char *)start);
	}
	pthread_mutex_unlock(&lock);
	return r;
}

void *mremap(void *old, size_t old_len, size_t new_len, int flags, ...)
{
	pthread_mutex_lock(&lock);
	struct map *m = find(old, 0);
	if (!m || (char *)old != m->addr || !new_len) {
		pthread_mutex_unlock(&lock);
		errno = EINVAL;
		return MAP_FAILED;
	}
	(void)old_len;
	if (new_len <= m->len) {
		m->len = new_len;
		pthread_mutex_unlock(&lock);
		return old;
	}
	if (!(flags & MREMAP_MAYMOVE)) {
		pthread_mutex_unlock(&lock);
		errno = ENOMEM;
		return MAP_FAILED;
	}
	char *p = 0;
	if (posix_memalign((void **)&p, PAGE, new_len)) {
		pthread_mutex_unlock(&lock);
		errno = ENOMEM;
		return MAP_FAILED;
	}
	memcpy(p, m->addr, m->len);
	memset(p + m->len, 0, new_len - m->len);
	free(m->addr);
	m->addr = p;
	m->len = new_len;
	pthread_mutex_unlock(&lock);
	return p;
}

int mprotect(void *addr, size_t len, int prot) { (void)addr; (void)len; (void)prot; return 0; }
int madvise(void *addr, size_t len, int advice) { (void)addr; (void)len; (void)advice; return 0; }
int posix_madvise(void *addr, size_t len, int advice) { (void)addr; (void)len; (void)advice; return 0; }
int mlock(const void *addr, size_t len) { (void)addr; (void)len; return 0; }
int munlock(const void *addr, size_t len) { (void)addr; (void)len; return 0; }
int mlockall(int flags) { (void)flags; return 0; }
int munlockall(void) { return 0; }

int mincore(void *addr, size_t len, unsigned char *vec)
{
	(void)addr;
	memset(vec, 1, (len + PAGE - 1) / PAGE);
	return 0;
}

/* the 64-bit-offset names (musl's are aliases) */
void *mmap64(void *, size_t, int, int, int, off_t) __attribute__((alias("mmap")));
