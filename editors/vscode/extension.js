// extension.js — W08r stage 4: the VS Code extension entry for the Operon
// debugger. The extension contributes the `operon` debug type and launches
// `operon dap <program>` as an embedded Debug Adapter (the adapter speaks
// the Debug Adapter Protocol over stdio with Content-Length framing, so VS
// Code's built-in DAP client drives it directly — no extra processes).
//
// W008 polish: stopOnEntry is handled natively by the adapter (launch
// argument → one-shot stopped event with reason "entry" at the program's
// first statement); conditional breakpoints flow through setBreakpoints'
// `condition` fields, and setVariable edits live frame state. The
// extension is a thin pass-through — no configuration massaging needed.

const vscode = require('vscode');

function activate(context) {
    const provider = {
        createDebugAdapterDescriptor(session) {
            const config = session.configuration;
            const operonPath = config.operonPath || 'operon';
            const args = ['dap', config.program];
            if (config.cell) {
                args.push('--cell', config.cell);
            }
            for (const a of config.args || []) {
                args.push(a);
            }
            return new vscode.DebugAdapterExecutable(operonPath, args);
        },
    };

    context.subscriptions.push(
        vscode.debug.registerDebugAdapterDescriptorFactory('operon', provider)
    );
}

function deactivate() {}

module.exports = { activate, deactivate };
