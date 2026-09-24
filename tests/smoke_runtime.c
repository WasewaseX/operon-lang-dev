/* smoke_runtime.c — verifies the C/C++ kernels return known-correct values. */
#include "../runtime/operon_rt.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static int failures = 0;
#define CHECK(cond, msg) do { if (!(cond)) { printf("FAIL: %s\n", msg); failures++; } \
                              else printf("ok:   %s\n", msg); } while (0)

int main(void) {
    /* interning: equal bytes -> equal id, distinct -> distinct, stable */
    uint32_t a = rt_intern("promote", 7);
    uint32_t b = rt_intern("promote", 7);
    uint32_t c = rt_intern("gene", 4);
    CHECK(a == b && a != c, "intern identity");
    size_t n = 0;
    const char *s = rt_intern_get(a, &n);
    CHECK(n == 7 && strncmp(s, "promote", 7) == 0, "intern roundtrip");
    CHECK(rt_intern_count() >= 2, "intern count");

    /* FNV-1a known vector: hash of "a" with FNV basis = 0xaf63dc4c8601ec8c */
    uint64_t h = rt_hash64("a", 1);
    CHECK(h == 0xaf63dc4c8601ec8cULL, "fnv1a-64 known vector");

    /* edit distance: known edit-distance values (powers the wobble engine) */
    CHECK(rt_edit_distance("kitten", 6, "sitting", 7) == 3, "lev kitten/sitting=3");
    CHECK(rt_edit_distance("gene", 4, "gen", 3) == 1, "lev gene/gen=1");
    CHECK(rt_edit_distance("else", 4, "elf", 3) == 2, "lev else/elf=2");
    CHECK(rt_edit_distance("", 0, "abc", 3) == 3, "lev empty=3");
    CHECK(rt_edit_distance("abc", 3, "abc", 3) == 0, "lev equal=0");
    CHECK(rt_edit_distance("flaw", 4, "lawn", 4) == 2, "lev flaw/lawn=2");

    /* codon score: numeric identifier-free junk vs atg-rich strings */
    int32_t cs = rt_codon_score("atgatgaag", 9);   /* atg=15, atg, aag=11 */
    CHECK(cs > 60 && cs <= 100, "codon score high-usage range");
    CHECK(rt_codon_score("xy", 2) == 0, "codon score too short");

    /* clock + arena counters move */
    double t0 = rt_now_ns();
    rt_reset();
    CHECK(rt_now_ns() >= t0, "monotonic clock");
    CHECK(rt_arena_used() == 0 && rt_alloc_count() == 0, "arena reset");

    if (failures) { printf("SMOKE: %d failure(s)\n", failures); return 1; }
    printf("SMOKE: all runtime kernel checks passed\n");
    return 0;
}
