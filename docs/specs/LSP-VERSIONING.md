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

- **operon** — the crate version, moves with milestones (D-009). Informational.
- **lsp** — the CONTRACT version editors pin against. Bumped on ANY breaking
  change to the handshake shape, the advertised capability set, or method
  semantics. Additive bug-fixes do not bump it. Current: **1**.

## Handshake advertisement

The `initialize` result carries the contract next to the standard fields:

```json
{
  "operonLsp": { "version": 1, "features": ["diagnostics", "hover",
      "definition", "documentSymbol", "completion", "formatting",
      "signatureHelp"] },
  "capabilities": { ... standard LSP capability map ... },
  "serverInfo": { "name": "operon-ls", "version": "..." }
}
```

`operonLsp.features` and `capabilities` must stay in lockstep; the smoke test
(`tests/lsp_smoke.py`) asserts the version field, the feature list, and the
`--version` output shape — drift fails the build.

## Editor-extension pinning table

| client | lsp version | notes |
|---|---|---|
| Neovim ≥ 0.8 (`vim.lsp.start`) | 1 | zero config beyond `cmd` + `filetypes`; sees diagnostics, hover, definition, symbols, completion, formatting |
| VS Code (generic LSP client extension) | 1 | same surface via `operon-ls` command; no official extension published yet — this table updates with the extension |
| any LSP 3.17-capable client | 1 | stdio framing (Content-Length), full-text document sync (ranged edits ignored by contract — the server advertises sync=1) |

The honest scope line: lsp v1 advertises exactly the seven features above.
Nothing else (rename, references, semantic tokens) is claimed — those are
W45/W46 items and will land as additive features (still lsp 1) or with a
contract bump if any existing shape changes.

## Change rules

1. Adding a NEW feature to `features`/`capabilities` (and nothing else
   changes): no version bump; smoke updated in the same commit.
2. Changing the shape of an existing response (fields renamed/moved/removed):
   **bump lsp**, note in this file, keep the previous behavior one release if
   the deprecation lifecycle (COMPATIBILITY.md, W64) applies.
3. Bumping lsp requires a DECISIONS entry and an editor-table refresh here.
