/* operon_rt.h — Operon C runtime kernel
 * Role: the load-bearing native substrate of the Operon toolchain.
 *  - identifier interning (all AST names live here, like classic compiler runtimes)
 *  - run-scoped bump arena backing the intern storage
 *  - FNV-1a hashing, monotonic clock, allocation counters
 *  - bridges to the C++ kernel (edit distance, codon scoring)
 */
#ifndef OPERON_RT_H
#define OPERON_RT_H

#include <stddef.h>
#include <stdint.h>

/* ---- identifier interning ------------------------------------------ */
/* Interns a byte string, returning a stable id. Equal bytes => equal id. */
uint32_t rt_intern(const char *s, size_t n);

/* Returns the bytes for an interned id (never NULL for valid ids). */
const char *rt_intern_get(uint32_t id, size_t *n_out);

/* Number of distinct interned strings this process has created. */
uint32_t rt_intern_count(void);

/* ---- hashing / clock / counters ------------------------------------ */
uint64_t rt_hash64(const void *p, size_t n);   /* FNV-1a 64 */
double   rt_now_ns(void);                      /* monotonic, nanoseconds */
void     rt_reset(void);                       /* run-scoped reset (arena + counters) */
size_t   rt_arena_used(void);                  /* bytes allocated from the arena */
uint64_t rt_alloc_count(void);                 /* arena allocation ops */

/* ---- C++ kernel exports (codon_kernel.cpp) -------------------------- */
/* Myers bit-parallel Levenshtein distance; DP fallback for long strings. */
int32_t rt_edit_distance(const char *a, size_t la, const char *b, size_t lb);

/* Codon-usage-style style score of an identifier, 0..100. */
int32_t rt_codon_score(const char *s, size_t n);

#endif /* OPERON_RT_H */
