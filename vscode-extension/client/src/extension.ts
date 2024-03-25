/* --------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See License.txt in the project root for license information.
 * ------------------------------------------------------------------------------------------ */

import { ExtensionContext } from "vscode";
import * as vscode from "vscode";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient;

export function activate(_: ExtensionContext) {
  const config = vscode.workspace.getConfiguration("gitlabLs");

  const serverOptions: ServerOptions = {
    run: { command: config.get("executablePath") },
    debug: {
      command: config.get("executablePath"),
    },
  };

  // Options to control the language client
  const clientOptions: LanguageClientOptions = {
    // Register the server for plain text documents
    documentSelector: [
      { scheme: "file", language: "yaml", pattern: "**/.gitlab*" },
    ],
    initializationOptions: {
      cache: config.get("cachePath"),
      log_path: config.get("logPath"),
      package_map: config.get("packageMap"),
    },
  };

  // Create the language client and start the client.
  client = new LanguageClient(
    "gitlabLs",
    "Gitlab LS",
    serverOptions,
    clientOptions,
  );

  // Start the client. This will also launch the server
  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
