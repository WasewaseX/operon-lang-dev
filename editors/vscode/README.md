# Operon Debugger (VS Code extension)

VS Code debug support for the Operon language, backed by `operon dap` (the
Debug Adapter Protocol adapter shipped with the Operon binary).

## What it gives you

- line breakpoints (the gutter: click a `.op` file's line numbers)
- continue, step-over (`next`), step-into (`stepIn`), step-out (`stepOut`)
- the call stack, locals per frame, and hover/watch evaluation
- program output in the debug console

## Prerequisites

The `operon` binary (>= 2.6.0) on your PATH, or any path you set as
`operonPath` in the launch configuration.

## Install

From the repository:

```
cd editors/vscode && npx @vscode/vsce package
code --install-extension operon-debug-2.6.0.vsix
```

Or symlink the folder into `~/.vscode/extensions/`.

## Launch configuration

```json
{
  "type": "operon",
  "request": "launch",
  "name": "Debug Operon file",
  "program": "${file}",
  "operonPath": "operon",
  "args": [],
  "cell": ""
}
```

- `program` — the `.op` file to debug (`${file}` = the active editor)
- `operonPath` — the binary (defaults to `operon` on PATH)
- `args` — program arguments
- `cell` — optional operator cell file granting capabilities (file IO etc.)
- `stopOnEntry` — accepted but not supported yet (warned honestly)

## Notes

- Breakpoints stop after the statement on that line completes (the debugger
  is statement-level; the language has no expression-level hooks).
- The debugger runs the interpreter lane; the VM lane keeps running your
  program at full speed when you `operon run` it normally.
- See `docs/DEBUGGER.md` in the repository for the full debugger guide,
  the NDJSON machine protocol, and the DAP surface.
