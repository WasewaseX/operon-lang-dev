/* operon_rt.c — Operon C runtime kernel implementation.
 *
 * Design notes:
 *  - The intern table is the workhorse: every identifier, keyword and mark in
 *    every parsed file is stored here exactly once. The Rust AST refers to
 *    strings through the table; the table's storage is a run-scoped bump
 *    arena, so a full toolchain run allocates monotonically and resets in O(1).
 *  - Open-addressed hash map (power-of-two capacity, linear probing) maps
 *    byte strings -> intern ids. FNV-1a hashes both the table and (via the
 *    exported rt_hash64) the interpreter's dynamic map keys.
 *  - All mutating entry points are mutex-guarded so worker threads produced by
 *    `spawn()` can safely intern too.
 */
#define _POSIX_C_SOURCE 199309L
#include "operon_rt.h"

#include <stdlib.h>
#include <string.h>
#include <time.h>

/* ---------------------------------------------------------------- portability
 * t3c: the kernel must compile on MSVC (release matrix) where pthread.h and
 * CLOCK_MONOTONIC do not exist. Win32 maps the mutex onto an SRWLOCK and the
 * monotonic clock onto QueryPerformanceCounter; POSIX keeps pthread/clock_gettime.
 */
#ifdef _WIN32
  #include <windows.h>
  typedef SRWLOCK rt_mutex_t;
  #define RT_MUTEX_INIT SRWLOCK_INIT
  static void rt_mutex_lock(rt_mutex_t *m)   { AcquireSRWLockExclusive(m); }
  static void rt_mutex_unlock(rt_mutex_t *m) { ReleaseSRWLockExclusive(m); }
#else
  #include <pthread.h>
  typedef pthread_mutex_t rt_mutex_t;
  #define RT_MUTEX_INIT PTHREAD_MUTEX_INITIALIZER
  #define rt_mutex_lock(m)   pthread_mutex_lock(m)
  #define rt_mutex_unlock(m) pthread_mutex_unlock(m)
#endif

/* ---------------------------------------------------------------- arena */
#define ARENA_CAP ((size_t)64u * 1024u * 1024u) /* 64 MiB run scope */

static unsigned char *g_arena      = NULL;
static size_t         g_arena_used = 0;
static uint64_t       g_allocs     = 0;

static void arena_init(void) {
    if (g_arena == NULL) {
        g_arena = (unsigned char *)malloc(ARENA_CAP);
        if (g_arena == NULL) abort(); /* cannot continue without memory */
    }
}

static void *arena_take(size_t n, size_t align) {
    arena_init();
    size_t pad = ((g_arena_used + (align - 1)) & ~(align - 1)) - g_arena_used;
    if (g_arena_used + pad + n > ARENA_CAP) return NULL; /* run scope exhausted */
    void *p = g_arena + g_arena_used + pad;
    g_arena_used += pad + n;
    g_allocs++;
    return p;
}

/* ------------------------------------------------------------ intern map */
typedef struct {
    uint32_t    id;
    uint32_t    len;
    const char *bytes; /* arena-owned */
} Slot;

static Slot   *g_slots     = NULL;
static size_t  g_cap       = 0;   /* power of two */
static size_t  g_count     = 0;   /* occupied slots */
static uint32_t g_next_id  = 1;   /* id 0 reserved */

/* dense index: g_dense[id] = slot INDEX + 1 (0 = absent). sec-r1 (audit
 * C-5/C-6): the old design stored &Slot pointers — table_init's growth
 * free()d the slot array without refreshing the index, so rt_intern_get
 * dereferenced freed memory (ASan-proven heap-use-after-free), and realloc
 * left unassigned entries pointing at garbage (SEGV on id 0). Indices
 * survive slot-array moves only if rebuilt on growth; calloc guarantees
 * every unassigned entry is the defined 0 = "absent". */
static size_t *g_dense     = NULL;
static size_t  g_dense_cap = 0;

/* rebuild the dense index from scratch — called only on table growth and
 * after rt_reset's table clear (amortized O(n) per doubling) */
static void dense_rebuild(void) {
    if (g_slots == NULL) return;
    size_t need = (size_t)g_next_id + 1;
    if (need > g_dense_cap) {
        size_t  ncap = need * 2;
        size_t *nd   = (size_t *)calloc(ncap, sizeof(size_t));
        if (nd == NULL) return; /* degraded: reads fail closed, writes fine */
        free(g_dense);
        g_dense     = nd;
        g_dense_cap = ncap;
    } else {
        memset(g_dense, 0, g_dense_cap * sizeof(size_t));
    }
    for (size_t i = 0; i < g_cap; i++) {
        if (g_slots[i].bytes == NULL) continue;
        if ((size_t)g_slots[i].id < g_dense_cap) {
            g_dense[g_slots[i].id] = i + 1;
        }
    }
}

static rt_mutex_t g_lock = RT_MUTEX_INIT;

static void table_init(size_t need) {
    if (g_cap == 0) {
        g_cap   = 1024;
        g_slots = (Slot *)calloc(g_cap, sizeof(Slot));
        if (!g_slots) abort();
    }
    while ((size_t)(g_count * 10) >= g_cap * 7) { /* load factor 0.7 */
        size_t   ncap  = g_cap * 2;
        Slot    *nslot = (Slot *)calloc(ncap, sizeof(Slot));
        if (!nslot) abort();
        for (size_t i = 0; i < g_cap; i++) {
            if (g_slots[i].bytes == NULL) continue;
            size_t j = (size_t)(rt_hash64(g_slots[i].bytes, g_slots[i].len) & (ncap - 1));
            while (nslot[j].bytes != NULL) j = (j + 1) & (ncap - 1);
            nslot[j] = g_slots[i];
        }
        free(g_slots);
        g_slots = nslot;
        g_cap   = ncap;
        /* slots moved (rehash): stale slot indices must be rebuilt */
        dense_rebuild();
    }
    (void)need;
}

uint64_t rt_hash64(const void *p, size_t n) {
    const unsigned char *b = (const unsigned char *)p;
    uint64_t h = 14695981039346656037ULL; /* FNV-1a 64-bit offset basis */
    for (size_t i = 0; i < n; i++) {
        h ^= (uint64_t)b[i];
        h *= 1099511628211ULL;
    }
    return h;
}

uint32_t rt_intern(const char *s, size_t n) {
    rt_mutex_lock(&g_lock);
    table_init(n);

    size_t i = (size_t)(rt_hash64(s, n) & (g_cap - 1));
    while (g_slots[i].bytes != NULL) {
        if (g_slots[i].len == n && memcmp(g_slots[i].bytes, s, n) == 0) {
            rt_mutex_unlock(&g_lock);
            return g_slots[i].id;
        }
        i = (i + 1) & (g_cap - 1);
    }

    char *copy = (char *)arena_take(n + 1, 16);
    if (copy == NULL) { rt_mutex_unlock(&g_lock); return 0; }
    memcpy(copy, s, n);
    copy[n] = 0;

    g_slots[i].id    = g_next_id++;
    g_slots[i].len   = (uint32_t)n;
    g_slots[i].bytes = copy;
    g_count++;

    /* maintain dense index: grow preserving entries (slots did not move
     * here — only the index array reallocates, and calloc zero-fills the
     * new region so unassigned ids stay 0 = absent) */
    if ((size_t)g_slots[i].id >= g_dense_cap) {
        size_t ncap = ((size_t)g_slots[i].id + 1) * 2;
        size_t *nd = (size_t *)calloc(ncap, sizeof(size_t));
        if (nd) {
            if (g_dense) {
                memcpy(nd, g_dense, g_dense_cap * sizeof(size_t));
                free(g_dense);
            }
            g_dense     = nd;
            g_dense_cap = ncap;
        }
    }
    if (g_dense && (size_t)g_slots[i].id < g_dense_cap)
        g_dense[g_slots[i].id] = i + 1;

    rt_mutex_unlock(&g_lock);
    return g_slots[i].id;
}

const char *rt_intern_get(uint32_t id, size_t *n_out) {
    rt_mutex_lock(&g_lock);
    const char *out = NULL;
    size_t len = 0;
    if (id > 0 && id < g_dense_cap && g_dense && g_dense[id] != 0) {
        const Slot *s = &g_slots[g_dense[id] - 1];
        if (s->bytes != NULL) {
            out = s->bytes;
            len = s->len;
        }
    }
    rt_mutex_unlock(&g_lock);
    if (n_out) *n_out = len;
    return out;
}

uint32_t rt_intern_count(void) {
    rt_mutex_lock(&g_lock);
    uint32_t c = g_next_id - 1;
    rt_mutex_unlock(&g_lock);
    return c;
}

/* ------------------------------------------------------------ clock/misc */
double rt_now_ns(void) {
#ifdef _WIN32
    LARGE_INTEGER freq, count;
    QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&count);
    return (double)count.QuadPart * 1e9 / (double)freq.QuadPart;
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
#endif
}

void rt_reset(void) {
    rt_mutex_lock(&g_lock);
    g_arena_used = 0;
    g_allocs     = 0;
    /* the intern table lives inside the arena: a reset invalidates every
     * string it points to, so the table MUST be cleared with it — leaving
     * stale entries would alias surviving ids into reused memory */
    if (g_slots) {
        memset(g_slots, 0, (size_t)g_cap * sizeof(Slot));
    }
    g_count   = 0;
    g_next_id = 1;
    /* the dense index refers to the table we just cleared: free it so the
     * next intern starts from a provably zero state (stale indices would
     * resurrect dangling slot positions) */
    if (g_dense) {
        free(g_dense);
        g_dense     = NULL;
        g_dense_cap = 0;
    }
    rt_mutex_unlock(&g_lock);
}

size_t rt_arena_used(void) {
    rt_mutex_lock(&g_lock);
    size_t u = g_arena_used;
    rt_mutex_unlock(&g_lock);
    return u;
}

uint64_t rt_alloc_count(void) {
    rt_mutex_lock(&g_lock);
    uint64_t a = g_allocs;
    rt_mutex_unlock(&g_lock);
    return a;
}
