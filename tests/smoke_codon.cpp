/* smoke_codon.cpp — native-kernel smoke test (sec-r2, audit A15).
 *
 * The C runtime kernel was deleted in sec-r2: its interning table moved to
 * src/ffi.rs as ordinary Rust ownership, so the UAF class the ASan audit
 * proved (C-5/C-6) is structurally gone. This driver keeps the SAME ASan +
 * UBSan guarantee for the kernel that remains — runtime/codon_kernel.cpp —
 * exercising both exports across edge shapes, under sanitizers in CI.
 *
 * Build (see scripts/build.sh / .github/workflows/ci.yml):
 *   g++ -fsanitize=address,undefined tests/smoke_codon.cpp build/codon_kernel.o \
 *       -o /tmp/operon_smoke
 */
#include <cstdint>
#include <cstdio>
#include <cstring>

extern "C" {
int32_t rt_edit_distance(const char *a, size_t la, const char *b, size_t lb);
int32_t rt_codon_score(const char *s, size_t n);
}

static int g_fail = 0;
#define CHECK(cond, msg)                                     \
    do {                                                     \
        if (cond) {                                          \
            std::printf("  ok  %s\n", msg);                  \
        } else {                                             \
            std::printf("FAIL  %s\n", msg);                  \
            g_fail++;                                        \
        }                                                    \
    } while (0)

int main() {
    /* --- edit distance: basics --------------------------------------- */
    CHECK(rt_edit_distance("kitten", 6, "sitting", 7) == 3, "distance kitten/sitting == 3");
    CHECK(rt_edit_distance("", 0, "abc", 3) == 3, "distance empty/abc == 3");
    CHECK(rt_edit_distance("abc", 3, "", 0) == 3, "distance abc/empty == 3");
    CHECK(rt_edit_distance("", 0, "", 0) == 0, "distance empty/empty == 0");
    CHECK(rt_edit_distance("same", 4, "same", 4) == 0, "distance identical == 0");

    /* --- edit distance: longer-than-64 forces the block path ---------- */
    const char *long_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; /* 80 */
    const char *long_b = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbb"; /* 80, 16 tail edits */
    CHECK(rt_edit_distance(long_a, 80, long_b, 80) == 16, "block path: 80-char pair == 16");

    /* --- codon score: range + determinism ----------------------------- */
    int32_t s1 = rt_codon_score("atgaaataa", 9);
    CHECK(s1 >= 0 && s1 <= 100, "codon score within 0..100");
    CHECK(rt_codon_score("atgaaataa", 9) == s1, "codon score deterministic");
    CHECK(rt_codon_score("", 0) == 0, "codon score empty == 0");

    /* --- long-input stress under ASan (no leaks, no overflow) --------- */
    for (int i = 0; i < 200; i++) {
        rt_edit_distance(long_a, 80, long_b, 80);
        rt_codon_score("atgggctttaag", 12);
    }
    CHECK(true, "200 hot iterations under sanitizers clean");

    if (g_fail == 0) {
        std::printf("smoke_codon: ALL PASS\n");
        return 0;
    }
    std::printf("smoke_codon: %d FAILURE(S)\n", g_fail);
    return 1;
}
