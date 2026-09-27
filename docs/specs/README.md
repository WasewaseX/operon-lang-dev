# Specs & policies index (W-track, ROADMAP-100)

Design contracts that live next to the code they govern. Every file here is normative for its
area; conflicts resolve toward SPEC.md, then these files, then code comments.

| document | audit item | status |
|---|---|---|
| [TOTAL-GRAMMAR-CONTRACT.md](TOTAL-GRAMMAR-CONTRACT.md) | W37 | landed (phantom-word warning runtime change tracked separately, oracle-mirrored) |
| [BIO-LAYER-POLICY.md](BIO-LAYER-POLICY.md) | W36 + W81 | landed (syntax freeze + kernel boundary) |
| [COMPATIBILITY.md](COMPATIBILITY.md) | W63 + W64 | landed (2.x contract + deprecation lifecycle; DECISIONS ratification pending) |
| [CELL-SCHEMA.md](CELL-SCHEMA.md) | W66 | landed (schema + validator: `operon lint --cell`) |
| [FMT-CONFIG.md](FMT-CONFIG.md) | W47 | landed (indent/quotes + config file; byte-stability law corpus-wide; `--width` honestly deferred) |
| SERIALIZATION.md | W34 | landed (stage 1) + stage 2 landed via PR #28, the W04 trait hook stays spec'd as the future OVERRIDE |
| [GENERICS.md](GENERICS.md) | W03 | landed stage 1 (callable-generic std, zero duplication); stages 2–3 specified, deliberately unscheduled |
| [ASYNC.md](ASYNC.md) | W16 | spec-only this cycle per roadmap, green threads over the VM loop, frame-field reservation carried in vm-design.md §6 from A2 |
| [MACROS design](../design/MACROS.md) | W35 | draft complete, models priced, Model A (declarative, rules-as-data) recommended, staged migration of the 7 hardcoded bio arms, open questions filed for sz ratification |
| [LSP-VERSIONING.md](LSP-VERSIONING.md) | W62 | landed (operon-ls --version pin line, operonLsp handshake block, editor pinning table; smoke-enforced) |
| MODULE-RESOLUTION.md | W69 | sz lane |
| THREAT-MODEL.md | W100 | sz lane |
| EMBEDDING.md | W76 | sz lane |

Regeneration + truth guards: `scripts/gen_doc_stats.py` → `docs/STATS.md`, `docs/KEYWORDS.md`,
`docs/stats.json`; `scripts/check_docs_sync.py` fails CI on stale doc numbers.
