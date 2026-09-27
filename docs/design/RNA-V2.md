# RNA-V2, AST-based `.rna` edit model (W067, design note; W068 safety mode is LIVE)

Status: **stage 2 SHIPPED** (node-addressed engine live behind the same CLI,
`src/rna2.rs`, 14 pinned tests in `tests/rna_v2.rs`). The v1 engine remains
the default for header-less patches (byte-compatible). **Stage 3 STARTED,
step 1 (Info) shipped**: header-less (v1 span) patches now carry a deprecation
marker per the W63 policy (stderr note + `"engine":"v1","deprecated":true` in
`--json`); text semantics stay byte-compatible and unchanged. Steps 2–3
(Warning severity, then removal one minor later) follow the migration path
below. The current `.rna` engine is span/text-based with the `operon
rna --check` safety mode (W068, PR #18) already shipped: dry-run default,
per-rule fate report (target found/not, replacement count, affected gene),
`--json`, exit 1 on a missed target. V2 is about WHAT the editor targets, not
about safety, that part exists.

## Problem

Text spans are fragile under reformatting: `edit gene foo` matching by line
offsets or byte spans breaks when fmt reprints the file. Two edits that are
individually valid can compose into a corrupted span.

## V2 model: address nodes, not bytes

1. **Parse, don't slice.** The patch applies to the AST produced from the
   CURRENT source at apply time, no cached spans, no offsets.
2. **Node addressing**, a rule's target is a path, not a span:
   - `gene <name>` (+ optional ordinal for same-name shadowing:
     `gene process#2` = the second declaration named `process`)
   - `splice <root>` / `variant <root>.<name>`
   - `phenotype <name>` / `method <name>.<gene>`
   - `fate <name>`, `regulate <ordinal>`
3. **Edit verbs (v2 set, deliberately small):**
   - `rename <old> -> <new>` (rewrites the decl name AND intra-file call sites,
     call-site rewrite is a separate rule in v1; in v2 it is one node operation)
   - `delete <target>` (gene → removed; splice root → variants detach honestly)
   - `body <gene> { ... }` (whole-body replacement; the replacement source is
     parsed FIRST, a parse error in the patch refuses the whole apply)
4. **Reprint, don't splice.** After the AST edit the file is reprinted with the
   formatter (fmt canonical mode, W047/W47, stable today: `fmt∘fmt = fmt` is
   enforced corpus-wide). The output of an apply is fmt-stable by construction,
   which also makes `apply∘apply` associative in the happy path.
5. **Failure semantics inherit W068:** every rule reports its fate; a missed
   target fails the apply (exit 1); `--json` reports per-rule detail. No
   partial application, v2 is all-or-nothing (a dry-run parse builds the full
   edited AST before anything is written).

## Migration path

- **Stage 1 (W068, done):** checked text engine, the safety contract exists
  and is versioned (`operon rna --check`).
- **Stage 2 (v2 core):** node-addressed engine behind the SAME CLI (`operon rna
  file.op patch.rna`); span rules keep working, node rules are new; a `.rna`
  patch self-describes (`syntax: v2` header line), absent header = v1 text
  semantics, byte-compatible.
- **Stage 3:** span rules deprecated (W63 deprecation policy: warn as Info,
  then Warning, then removal one minor later). `edit`/`replace` grammar stays
  in the language regardless, the SYNTAX SURFACE never changes (bio-layer
  freeze, D-011/R9), only the file editor's patch format evolves.

## Why this sequencing

Node addressing needs a stable canonical printer (W047 fmt, shipped) and a
stable AST (W01/W02 grammar waves, landed). Both prerequisites are now met;
implementation is a candidate for a sz lane session AFTER the contract docs
settle (this file is that design note).

## Stage 2, as-built (this commit)

* **Patch self-description:** first non-blank/non-`#` line of the patch must be
  exactly `syntax: v2`; anything else (including an empty patch) routes to the
  v1 checked engine, byte-compatible.
* **Patch grammar (file format, NOT language syntax):** `rename <path> -> <new>`
  · `delete <path>` · `body gene <name>[#<ord>] { ...verbatim lines... }`, one
  rule per line, `#` comments and blank lines skipped. Paths: `gene NAME[#N]`,
  `splice ROOT`, `variant ROOT.NAME`, `phenotype NAME`, `method PHENO.GENE`,
  `fate NAME`, `regulate #N` (delete-only, 1-indexed).
* **Verbs:** rename rewrites the declaration name AND reference nodes
  (gene/splice/fate → Ident nodes, identifier-precise, NOT scope-aware, the
  strictly-safer-than-v1-substring caveat; phenotype → `new X()` constructor
  nodes + type annotations; method → `Method` name strings, name-global).
  delete removes the declaration (references fail honestly at runtime: phantom
  call / unknown gene). body replaces the whole body; the replacement text is
  parsed FIRST and any rung-4 parse note refuses the entire apply.
* **All-or-nothing:** rules resolve sequentially against the (already edited)
  AST; ANY miss → nothing written, every fate reported, exit 1. Bare-name
  ambiguity refuses with an ordinal guide (`foo#1, foo#2, ...`).
* **Reprint:** `tools::format_program`, apply output is a fmt fixpoint by
  construction (fmt∘apply == apply; pinned by `reprint_is_fmt_stable`).
* **Comment preflight:** the reprint drops plain `#` comments (only decl-hugging
  `##` docs roundtrip, W074). A source containing plain comments is refused
  with the offending line numbers unless `--allow-comment-drop`. Multiline
  `"""..."""` strings and string-embedded `#` are tracked so the guard errs
  toward refusal only on real comments.
* **CLI:** same subcommand, new flag: `operon rna f.op patch.rna [--write]
  [--json] [--allow-comment-drop]`; JSON gains `"engine":"v2"` +
  per-rule verb/target/detail rows; a refused apply reports
  `"refused":true` and never writes.
* **Scope notes:** decl addressing walks top-level statements + TAD/Block
  bodies (the same scope class v1's gene_span targeted); Frame and gene-body
  statements are not decl scope. `.cell`/CLI keys (variant selection,
  methylation targets) are configuration, a patch edits source only.
