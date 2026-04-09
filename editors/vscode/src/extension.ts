import { workspace, ExtensionContext, window } from "vscode";
import {
  Executable,
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext) {
  const config = workspace.getConfiguration("mago");
  let magoPath: string = config.get("path", "mago");
  // If using the default "mago" from PATH, try the local debug build first.
  if (magoPath === "mago") {
    const localBuild = "C:/Users/ms/mago/target/debug/mago.exe";
    try {
      const fs = require("fs");
      if (fs.existsSync(localBuild)) {
        magoPath = localBuild;
      }
    } catch {
      // ignore
    }
  }

  const executable: Executable = {
    command: magoPath,
    args: ["lsp"],
  };

  const serverOptions: ServerOptions = {
    run: executable,
    debug: executable,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "php" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.php"),
    },
    outputChannelName: "Mago Language Server",
  };

  client = new LanguageClient(
    "mago",
    "Mago Language Server",
    serverOptions,
    clientOptions,
  );

  client.start().catch((err) => {
    window.showErrorMessage(
      `Failed to start Mago language server: ${err.message}. ` +
        `Make sure the 'mago' binary is installed and available in your PATH, ` +
        `or set the 'mago.path' setting to the correct path.`,
    );
  });
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
