import * as vscode from "vscode";

import { sageCellMarkers } from "./sageCells";

export interface SageCellCodeLensProviderOptions {
  isEnabled(document: vscode.TextDocument): boolean;
}

export class SageCellCodeLensProvider implements vscode.CodeLensProvider {
  private readonly changeEmitter = new vscode.EventEmitter<void>();

  readonly onDidChangeCodeLenses = this.changeEmitter.event;

  constructor(private readonly options: SageCellCodeLensProviderOptions) {}

  refresh(): void {
    this.changeEmitter.fire();
  }

  dispose(): void {
    this.changeEmitter.dispose();
  }

  provideCodeLenses(document: vscode.TextDocument): vscode.CodeLens[] {
    if (!this.options.isEnabled(document)) {
      return [];
    }

    return sageCellMarkers(document.getText()).map((marker) => {
      const title = marker.kind === "region" ? "Run Region" : "Run Cell";
      const range = new vscode.Range(marker.line, 0, marker.line, 0);
      return new vscode.CodeLens(range, {
        title,
        command: "sage.runCurrentCell",
        tooltip: marker.label,
        arguments: [{ uri: document.uri, line: marker.line }],
      });
    });
  }
}
