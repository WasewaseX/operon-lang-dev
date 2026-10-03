# LSP versioning contract (W62, ROADMAP-100)

Normative for `operon-ls`. Conflicts resolve toward SPEC.md (§15), then this
file.

## The version pair

`operon-ls --version` prints one machine-readable line:

```
operon <crate-version> / lsp <contract-version>
```

(e.g. `operon 2.2.0 / lsp 1`). `--version` exits before touching stdio, so it
is safe to call from installers and editor plugins. Unknown arguments exit 2
with a message; the server proper takes no arguments and reads LSP frames on
stdio.

- **operon**, the crate version, moves with milestones (D-009). Informational.
- **lsp**, the CONTRACT version editors pin against. Bumped on ANY breaking
  change to the handshake shape, the advertised capability set, or method
  semantics. Additive bug-fixes do not bump it. Current: **1**.

## Handshake advertisement

The `initialize` result carries the contract next to the standard fields:

```json
{
  "operonLsp": { "version": 1, "features": ["diagnostics", "hover",
      "definition", "references", "semanticTokens", "rename", "documentSymbol",
      "completion", "formatting", "signatureHelp"] },
  "capabilities": { ... standard LSP capability map ... },
  "serverInfo": { "name": "operon-ls", "version": "..." }
}
```

`operonLsp.features` and `capabilities` must stay in lockstep; the smoke test
(`tests/lsp_smoke.py`) asserts the version field, the feature list, and the
`--version` output shape, drift fails the build.

## Editor-extension pinning table

| client | lsp version | notes |
|---|---|---|
| Neovim ≥ 0.8 (`vim.lsp.start`) | 1 | zero config beyond `cmd` + `filetypes`; sees diagnostics, hover, definition, symbols, completion, formatting |
| VS Code (generic LSP client extension) | 1 | same surface via `operon-ls` command; no official extension published yet, this table updates with the extension |
| any LSP 3.17-capable client | 1 | stdio framing (Content-Length), full-text document sync (ranged edits ignored by contract, the server advertises sync=1) |

The honest scope line: lsp v1 advertises exactly the ten features above
(W45 added `references` + `semanticTokens` and W45-v2 added `rename`, all
additively, no bump, per rule 1). Repair provenance (W46) rides the
EXISTING `diagnostics` and `hover` features: publishDiagnostics may carry
`relatedInformation` and hover may append the repair note, both additive
fields on advertised features, covered by the same rule. `--explain FILE`
is a CLI door (not an LSP method), outside the contract. Inlay-hints remain
unclaimed.

### W45-v2 rename semantics (part of the lsp 1 contract)

`textDocument/prepareRename` answers `{range, placeholder}` for a plain
code identifier; it refuses (null) on keywords, literals, synonyms, mark
names, builtins, and positions inside strings or comments. `textDocument/rename`
returns a same-file WorkspaceEdit over every code occurrence of the
identifier, declaration included; interpolated `{..}` expressions count
(they are evaluated), string/comment/`@mark` mentions never do. The engine
is the SAME scanner references uses (zero drift between the two features),
grep-class and identifier-precise, NOT scope-aware, run `operon check`
after a rename; that honesty mirrors W67's `operon rna` rename discipline.
All-or-nothing: an invalid, reserved, builtin-target, same-name, or
already-taken new name refuses the WHOLE rename as a JSON-RPC error
(code -32001) so the editor can surface the reason, a silent null or a
partial edit set are both contract violations.

## Skew, minimum handshake, and client detection (W62)

**Where skew can and cannot happen.** `operon-ls` is a second binary target of
the same crate as the `operon` CLI (`src/bin/operon-ls.rs`), compiled from the
same tree: one install always carries matching versions, and "operon-ls newer
than the operon binary it wraps" cannot occur inside an install, because
nothing is wrapped (the analysis and fmt engines are linked in, not shelled
out to). The skew that CAN occur is between the server binary an editor
launches and the `operon` a user runs in a terminal (two installs on PATH), or
an editor holding a stale cached server path after an upgrade. The policy is
detection, not prevention: the initialize result reports both numbers
(`operonLsp.version` for the contract, `serverInfo.version` for the crate), a
client that cares displays both, and the repo rule stays in-lockstep
shipping: operon-ls rides the same release train as operon, never separately.

**Minimum handshake.** initialize request -> initialize result
(`operonLsp` + `capabilities` + `serverInfo`) -> `initialized` notification ->
`textDocument/didOpen` with full text -> `publishDiagnostics` server-to-client.
`shutdown` request -> null result -> `exit` notification ends the process. A
malformed frame or stream end closes the session. Full-text sync only
(`textDocumentSync = 1`); ranged edits are ignored, not mis-applied. lsp 1
claims no workspace/configuration capabilities.

**Client feature detection.** `operonLsp.features` is the authoritative gate;
`capabilities` mirrors it in standard LSP vocabulary and the smoke test holds
the two in lockstep. Clients gate each UI affordance on the named feature
string and MUST ignore unrecognized strings (additive growth, change rule 1,
never breaks an older client). Versions are integers, currently 1: a client
facing a HIGHER version treats it as "plain LSP 3.17 defaults only" (unknown
shape = contract break, change rule 2); a client facing a LOWER version than
it requires gates everything off `operonLsp.features` and degrades cleanly.

## Change rules

1. Adding a NEW feature to `features`/`capabilities` (and nothing else
   changes): no version bump; smoke updated in the same commit.
2. Changing the shape of an existing response (fields renamed/moved/removed):
   **bump lsp**, note in this file, keep the previous behavior one release if
   the deprecation lifecycle (COMPATIBILITY.md, W64) applies.
3. Bumping lsp requires a DECISIONS entry and an editor-table refresh here.
