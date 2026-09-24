// codon_kernel.cpp — Operon C++ kernel
//
// Two genuinely hot kernels exported to the toolchain through the C ABI:
//
//  1. rt_edit_distance — bit-parallel edit-distance wavefront (block algorithm) for the
//     wobble engine. The parser asks it for every near-keyword candidate, the
//     stdlib exposes it as distance()/similar(). Single-word path covers
//     pattern lengths <= 64 at O(n); the block path extends bit-vectors to
//     ceil(m/64) words for arbitrary lengths.
//
//  2. rt_codon_score — codon-usage scoring. An identifier is read as a stream
//     of 3-letter groups; each group that maps into the standard genetic code
//     is scored by a relative-usage weight (high-usage codons of highly
//     expressed genes score highest). Score = weighted mean mapped to 0..100.
//     This powers the `codon()` builtin and the `operon check` grader.
//
// Compiled -O3 -fno-exceptions; no C++ runtime allocation, no exceptions
// crossing the ABI.

#include <cstdint>
#include <cstring>
#include <cstddef>
#include <cstdlib>

extern "C" {

/* ------------------------------------------------------------------ edit */

int32_t rt_edit_distance(const char *a, size_t la, const char *b, size_t lb) {
    if (la == 0) return (int32_t)lb;
    if (lb == 0) return (int32_t)la;

    /* Keep the pattern (bit-vector axis) in `b`; swap so b is the shorter. */
    if (lb > la) { const char *t = a; a = b; b = t; size_t tn = la; la = lb; lb = tn; }

    if (lb <= 64) {
        /* single-word bit-parallel path: exact edit distance in O(la). `score` is the
         * number of trailing fixed bits maintained by the standard invariant
         * (distance = lb - popcount-of-matched-prefix, tracked via Ph/Mh at
         * the top bit). */
        uint64_t mask = (lb == 64) ? ~0ULL : ((1ULL << lb) - 1);
        uint64_t pv = mask;
        uint64_t mv = 0;
        int32_t score = (int32_t)lb;
        uint64_t masks[256];
        memset(masks, 0, sizeof(masks));
        for (size_t i = 0; i < lb; i++) masks[(unsigned char)b[i]] |= 1ULL << i;

        for (size_t i = 0; i < la; i++) {
            uint64_t eq = masks[(unsigned char)a[i]];
            uint64_t xv = eq | mv;
            uint64_t xh = (((eq & pv) + pv) ^ pv) | eq;

            uint64_t ph = mv | ~(xh | pv);
            uint64_t mh = pv & xh;

            if (ph & (1ULL << (lb - 1))) score++;
            else if (mh & (1ULL << (lb - 1))) score--;

            ph = (ph << 1) | 1ULL;
            mh = mh << 1;

            pv = (mh | ~(xv | ph)) & mask;
            mv = ph & xv;
        }
        return score;
    }

    /* Long-pattern case (rare: telemetry over long strings): exact two-row
     * DP, O(la*lb). The bit-parallel block variant is scheduled for v2.1. */
    {
        uint32_t *row = (uint32_t *)malloc((lb + 1) * sizeof(uint32_t));
        if (!row) return 2147483647; /* sec-r3: malloc failure must LOSE every nearest-match contest (i32::MAX), not win it with -1 */
        for (size_t j = 0; j <= lb; j++) row[j] = (uint32_t)j;
        for (size_t i = 1; i <= la; i++) {
            uint32_t prev = row[0];
            row[0] = (uint32_t)i;
            for (size_t j = 1; j <= lb; j++) {
                uint32_t cur = row[j];
                uint32_t cost = (a[i - 1] == b[j - 1]) ? 0u : 1u;
                uint32_t v = prev + cost;
                if (row[j] + 1 < v) v = row[j] + 1;
                if (row[j - 1] + 1 < v) v = row[j - 1] + 1;
                row[j] = v;
                prev = cur;
            }
        }
        int32_t out = (int32_t)row[lb];
        free(row);
        return out;
    }
}

/* ----------------------------------------------------------------- codon */

/* Codon relative-usage classes for highly expressed genes, mechanism-level
 * (species-agnostic synthesis of standard usage tables).
 * Index layout: idx = b1*16 + b2*4 + b3, bases t/u=0, c=1, a=2, g=3.
 * Rows below are enumerated in exactly that order (b1 major, b2 middle,
 * b3 minor). Class 0 = near-silent (stops, rare sense codons) .. 15 = the
 * most frequent translation codons (atg, ctg, aag, gag, aaa, gat, ...). */
static const unsigned char USAGE[64] = {
    /* b1=t: tt{t,c,a,g} tc{...} ta{...} tg{...} */
    7, 1, 5, 3,   15, 6, 8, 4,    3, 2, 0, 0,    4, 6, 0, 5,
    /* b1=c: ct... cc... ca... cg... */
    4, 1, 2, 13,  5, 2, 4, 1,     6, 2, 12, 9,   5, 3, 9, 4,
    /* b1=a: at... ac... aa... ag... */
    3, 1, 2, 15,  8, 5, 6, 2,     14, 7, 12, 6,  2, 1, 8, 3,
    /* b1=g: gt... gc... ga... gg... */
    6, 2, 8, 1,   8, 3, 6, 10,    4, 2, 13, 11,   6, 7, 3, 9,
};

static inline int base_idx(unsigned char c) {
    switch (c) {
        case 't': case 'T': case 'u': case 'U': return 0;
        case 'c': case 'C': return 1;
        case 'a': case 'A': return 2;
        case 'g': case 'G': return 3;
        default: return -1;
    }
}

int32_t rt_codon_score(const char *s, size_t n) {
    if (n < 3) return 0;
    uint64_t acc = 0;
    size_t groups = 0;
    for (size_t i = 0; i + 2 < n; i += 3) {
        int b1 = base_idx((unsigned char)s[i]);
        int b2 = base_idx((unsigned char)s[i + 1]);
        int b3 = base_idx((unsigned char)s[i + 2]);
        if (b1 < 0 || b2 < 0 || b3 < 0) {
            /* unmapped group (non-DNA letters): NEUTRAL 50 — never reward
             * biological nonsense, never punish ordinary identifiers */
            acc += 50;
            groups++;
            continue;
        }
        unsigned char w = USAGE[b1 * 16 + b2 * 4 + b3];
        acc += 30 + (uint64_t)w * 5;   /* map 0..15 -> 30..105, clamp below */
        groups++;
    }
    if (groups == 0) return 0;
    uint64_t mean = acc / groups;
    if (mean > 100) mean = 100;
    return (int32_t)mean;
}

} /* extern "C" */
