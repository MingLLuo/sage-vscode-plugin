import * as vscode from "vscode";
import { renderDocumentationHtml } from "./documentationMarkup";

export class DocumentationPanel {
  private panel: vscode.WebviewPanel | undefined;

  show(title: string, markdown: string): void {
    if (!this.panel) {
      this.panel = vscode.window.createWebviewPanel(
        "sageDocumentation",
        title,
        vscode.ViewColumn.Beside,
        {
          enableFindWidget: true,
          enableScripts: false,
        },
      );
      this.panel.onDidDispose(() => {
        this.panel = undefined;
      });
    }

    this.panel.title = title;
    this.panel.webview.html = renderDocumentationHtml(markdown);
    this.panel.reveal(vscode.ViewColumn.Beside, true);
  }
}
