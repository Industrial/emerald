// Emerald VSCode extension client (plan 17's leaf-vscode-extension).
// Spawns `emerald-lsp` (path configurable via `emerald.serverPath`,
// defaulting to `emerald-lsp` on PATH — see package.json) and starts a
// LanguageClient for `.em` documents. The grammar (syntax coloring)
// needs no client code at all — VSCode consumes
// `syntaxes/emerald.tmLanguage.json` declaratively via package.json's
// `contributes.grammars`.
//
// `vscode-languageclient` is a real npm dependency (see package.json)
// this sandbox cannot install (no npm/yarn/pnpm on PATH — see plan
// 17's Decision log) or type-check ahead of time; this file is
// authored against its documented API and syntax-checked with `node
// --check`, not executed here.

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
	const config = vscode.workspace.getConfiguration("emerald");
	const serverCommand = config.get("serverPath", "emerald-lsp");

	const serverOptions = {
		run: { command: serverCommand, transport: TransportKind.stdio },
		debug: { command: serverCommand, transport: TransportKind.stdio },
	};

	const clientOptions = {
		documentSelector: [{ scheme: "file", language: "emerald" }],
	};

	client = new LanguageClient(
		"emerald",
		"Emerald Language Server",
		serverOptions,
		clientOptions,
	);

	context.subscriptions.push(client.start());
}

function deactivate() {
	if (!client) {
		return undefined;
	}
	return client.stop();
}

module.exports = { activate, deactivate };
