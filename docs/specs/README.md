# Specs & policies index (W-track, ROADMAP-100)

Design contracts that live next to the code they govern. Every file here is normative for its
area; conflicts resolve toward SPEC.md, then these files, then code comments.

| document | audit item | status |
|---|---|---|
| [TOTAL-GRAMMAR-CONTRACT.md](TOTAL-GRAMMAR-CONTRACT.md) | W37 | landed (phantom-word warning runtime change tracked separately, oracle-mirrored) |
| [BIO-LAYER-POLICY.md](BIO-LAYER-POLICY.md) | W36 + W81 | landed (syntax freeze + kernel boundary) |
| [COMPATIBILITY.md](COMPATIBILITY.md) | W63 + W64 | landed (2.x contract + deprecation lifecycle; DECISIONS ratification pending) |
| [CELL-SCHEMA.md](CELL-SCHEMA.md) | W66 | landed (schema + validator: `operon lint --cell`) |
| SERIALIZATION.md | W34 | pending (contract drafted in ROADMAP-100; waits on W04 traits for full form) |
| MACROS design | W35 | pending (docs/design/MACROS.md, ratified before any parser keyword) |
| LSP-VERSIONING.md | W62 | pending (handshake carries version once W45/W46 land) |
| MODULE-RESOLUTION.md | W69 | sz lane |
| THREAT-MODEL.md | W100 | sz lane |
| EMBEDDING.md | W76 | sz lane |

Regeneration + truth guards: `scripts/gen_doc_stats.py` → `docs/STATS.md`, `docs/KEYWORDS.md`,
`docs/stats.json`; `scripts/check_docs_sync.py` fails CI on stale doc numbers.
