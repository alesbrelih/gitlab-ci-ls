/* --------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See License.txt in the project root for license information.
 * ------------------------------------------------------------------------------------------ */

import { ExtensionContext } from "vscode";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient;

export function activate(_: ExtensionContext) {
  const serverOptions: ServerOptions = {
    run: { command: "gitlab-lsp" },
    debug: {
      command: "gitlab-lsp",
    },
  };

  // Options to control the language client
  const clientOptions: LanguageClientOptions = {
    // Register the server for plain text documents
    documentSelector: [
      { scheme: "file", language: "yaml", pattern: "**/.gitlab*" },
    ],
    initializationOptions: {
      cache: "~/.gitlab-ls/cache/",
      log_path: "/tmp/gitlab-lsp.log",
      package_map: {
        somepackage: "git@host",
      },
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
