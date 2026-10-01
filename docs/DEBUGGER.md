# The Operon Debugger (W08r)

`operon debug` is the interactive debugger for Operon programs. It runs on
the interpreter lane (the VM lane is the performance lane; the debugger
forces the tree-walker) and stops AFTER each statement completes, reporting
the statement's own source line.

Status: **implemented** (W08r, 2026-10-01). Phases: the human REPL, the
NDJSON machine protocol, the Debug Adapter Protocol adapter, and the VS Code
extension.

## The REPL (human surface)

```
operon debug f.op --break 12        # start a session, break at line 12
operon debug f.op --break 3 --break 7   # multiple initial breakpoints
```

At a stop the prompt names the line you stopped at:

```
(dbg) line 12 > 
```

Commands:

| command | meaning |
|---|---|
| `c` / `continue` | resume to the next stop |
| `s` / `step` | step-INTO: break after the next statement at any depth |
| `n` / `next` | step-OVER: break after the next statement at this depth (calls run to completion without trapping) |
| `fin` / `finish` | step-OUT: run until the current frame returns |
| `until N` | one-shot continue-to-line: stop when line N's statement completes |
| `b N` | add a breakpoint at line N (fires immediately, even mid-session) |
| `b del N` | delete the breakpoint at line N |
| `b list` | list active breakpoints |
| `bt` | the call chain, innermost last, each frame with its current source line (`at work (line 6)`) — W008-P1: outer frames carry their live call-site lines (captured while a session is armed; a frame with no line — the entry frame before any call — prints bare) |
| `vars` | dump the frame chain's variables (innermost first, 8 frames) |
| `p EXPR` | evaluate EXPR in the current frame |
| `q` / `quit` | end the session, exit code 0 |

EOF on stdin resumes to completion: piped sessions never wedge. This is a
load-bearing contract, enforced by `scripts/debug_e2e.sh` on every gate run.

### Line accuracy

The trap matches the statement's OWN line (extracted from its line-bearing
expression nodes; the parser wraps line-silent statements — pure
literals/idents — in a transparent position marker). A breakpoint inside a
called gene never re-fires on the caller's call statement the moment the
call returns, and a stop after `let a = work(10)` reports line 8, not the
last line inside `work`.

## The machine protocol (NDJSON)

`operon debug f.op --protocol=json` replaces the human REPL with a
line-delimited JSON protocol — one JSON object per line on stdout, one
request per line on stdin. Program `print` output is captured and drained to
**stderr** (`[out] ...` lines) so stdout stays pure protocol.

Events (stdout): `{"event":"stopped","reason":"breakpoint"|"step"|"until","line":N,"depth":D}`

Requests (stdin), replies `{"id":N,"ok":true,...}` / `{"id":N,"ok":false,"error":"..."}`:

```
{"id":1,"cmd":"continue"}                              # resume
{"id":2,"cmd":"next"} {"id":2,"cmd":"stepIn"} {"id":2,"cmd":"stepOut"}
{"id":3,"cmd":"until","args":{"line":9}}
{"id":4,"cmd":"breakpoints","args":{"add":[9],"remove":[3]}}
{"id":5,"cmd":"stack"}      # frames, innermost first; each frame carries its current source line (W008-P1: outer frames report their live call-site line, null only when no line is known)
{"id":6,"cmd":"vars"}       # scopes with rendered variable maps
{"id":7,"cmd":"eval","args":{"expr":"z * 100"}}
{"id":8,"cmd":"status"}     # line + depth
{"id":9,"cmd":"quit"}
```

`scripts/debug_protocol_e2e.py` is a complete reference client.

## DAP (editors and IDEs)

`operon dap f.op` speaks the **Debug Adapter Protocol** on stdio
(Content-Length base protocol) — the same protocol VS Code, Visual Studio,
Neovim, emacs and JetBrains debug clients speak natively:

- lifecycle: `initialize` → `initialized` event → `launch` →
  `setBreakpoints` → `configurationDone` → run → `stopped` events →
  `stackTrace`/`scopes`/`variables`/`evaluate` → `terminated`/`exited`
- stepping verbs: `continue`, `next` (step-over), `stepIn`, `stepOut` —
  mapped onto the same depth-aware machinery as the REPL
- program output arrives as `output` events (category `stdout`); the
  adapter's stdout is protocol-only
- `evaluate` runs in the stopped frame, same semantics as the REPL's `p`

`scripts/dap_e2e.py` is a complete reference client.

## VS Code integration

The repository ships `editors/vscode/` — a VS Code extension contributing
the `operon` debug type. It launches `operon dap <program>` as an embedded
Debug Adapter (no extra server process). Install: copy/symlink the folder
into your extensions directory (or package it with `vsce`), open a `.op`
file, add a launch configuration:

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

`operonPath` defaults to `operon` on PATH; `cell` grants capabilities for
programs that read/write files.

## Gates

| gate | what it pins |
|---|---|
| `scripts/debug_e2e.sh` | the REPL contract (breaks fire, vars/p/s/c, next/finish/until, bp management, EOF resumes) |
| `scripts/debug_protocol_e2e.py` | the NDJSON contract (purity, stop reasons, all verbs, print rerouting) |
| `scripts/dap_e2e.py` | the DAP contract (lifecycle, stopped events, stacks/scopes/vars/eval, output events) |
