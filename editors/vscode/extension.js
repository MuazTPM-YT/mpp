// Muaz++ for VS Code: starts `mpp lsp` and adds run/test commands
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function mppPath() {
  return vscode.workspace.getConfiguration("mpp").get("path") || "mpp";
}

// run `mpp <args> <current file>` in a terminal
function runInTerminal(args) {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== "mpp") {
    vscode.window.showWarningMessage("Open a .mpp file first.");
    return;
  }
  editor.document.save().then(() => {
    const term = vscode.window.terminals.find((t) => t.name === "Muaz++") || vscode.window.createTerminal("Muaz++");
    term.show(true);
    const file = JSON.stringify(editor.document.uri.fsPath);
    term.sendText(`${JSON.stringify(mppPath())} ${args} ${file}`);
  });
}

function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand("mpp.run", () => runInTerminal("run")),
    vscode.commands.registerCommand("mpp.test", () => runInTerminal("test")),
    vscode.commands.registerCommand("mpp.testReport", () => runInTerminal("test --report html:mpp_report.html"))
  );
  if (!vscode.workspace.getConfiguration("mpp").get("lsp.enable")) {
    return;
  }
  const server = { command: mppPath(), args: ["lsp"] };
  client = new LanguageClient("mpp", "Muaz++", { run: server, debug: server }, { documentSelector: [{ language: "mpp" }] });
  client.start().catch((e) => {
    vscode.window.showErrorMessage(`Muaz++: cannot start "${mppPath()} lsp" (${e.message}). Set mpp.path in settings.`);
  });
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
