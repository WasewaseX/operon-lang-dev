// extension.js — W08r stage 4: the VS Code extension entry for the Operon
// debugger. The extension contributes the `operon` debug type and launches
// `operon dap <program>` as an embedded Debug Adapter (the adapter speaks
// the Debug Adapter Protocol over stdio with Content-Length framing, so VS
// Code's built-in DAP client drives it directly — no extra processes).
//
// stopOnEntry: `operon dap` has no stop-at-first-statement flag yet, so the
// extension maps stopOnEntry to a breakpoint at the entry gene's first body
// line is NOT possible without parsing — it is accepted by the schema and
// refused honestly (a configuration warning) until W08r stage 5.

const vscode = require('vscode');

function activate(context) {
    const provider = {
        createDebugAdapterDescriptor(session) {
            const config = session.configuration;
            if (config.stopOnEntry) {
                // honest v1 refusal: the adapter has no stop-at-first-
                // statement flag yet (W08r stage 5 candidate)
                vscode.window.showWarningMessage(
                    'Operon: stopOnEntry is not supported yet; breakpoints work normally.'
                );
            }
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
