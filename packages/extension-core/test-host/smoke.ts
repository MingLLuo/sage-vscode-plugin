import assert from "node:assert/strict";

import * as vscode from "vscode";

interface LifecycleSnapshot {
  launchCount: number;
  managedCloseCount: number;
  unexpectedCloseCount: number;
  managedShutdownActive: boolean;
  restartQueued: boolean;
  hasClient: boolean;
}

export async function run(): Promise<void> {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(workspaceFolder, "expected the smoke workspace to be open in the extension host");

  const config = vscode.workspace.getConfiguration("sage", workspaceFolder.uri);
  await config.update(
    "languageServer.pythonPath",
    process.env.SAGE_TEST_LSP_PYTHON ?? "python",
    vscode.ConfigurationTarget.Workspace,
  );
  await config.update("analysis.enableRuntimeIntrospection", false, vscode.ConfigurationTarget.Workspace);
  await config.update("docs.showOnHover", true, vscode.ConfigurationTarget.Workspace);

  const uri = vscode.Uri.joinPath(workspaceFolder.uri, "src", "01_hover_and_definition.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  await waitForCommand("sage.__test.getLifecycleSnapshot");
  const initialSnapshot = await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");
  assert.ok(initialSnapshot.launchCount >= 1, "expected the language client to launch during activation");
  assert.equal(initialSnapshot.unexpectedCloseCount, 0, "expected no unexpected client shutdowns before restart");

  const hoverPosition = positionOfNth(document, "make_demo_matrix", 2);
  const hovers =
    (await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      hoverPosition,
    )) ?? [];
  assert.ok(
    hovers.some((hover) => renderHoverContents(hover).includes("looks like a matrix")),
    "expected hover text for make_demo_matrix from the workspace fixture",
  );

  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      hoverPosition,
    )) ?? [];
  assert.ok(
    definitions.some((definition) => definitionUri(definition).fsPath.endsWith("src/local_docs.py")),
    "expected definition for make_demo_matrix to resolve into local_docs.py",
  );

  const completionPosition = positionOfNth(document, "summarize_coefficients", 2, 4);
  const completionResult =
    (await vscode.commands.executeCommand<vscode.CompletionList | vscode.CompletionItem[]>(
      "vscode.executeCompletionItemProvider",
      uri,
      completionPosition,
    )) ?? [];
  const completionItems = Array.isArray(completionResult) ? completionResult : completionResult.items;
  const completionLabels = new Set(completionItems.map((item) => item.label.toString()));
  assert.ok(
    completionLabels.has("summarize_coefficients"),
    "expected completion items to include summarize_coefficients from the workspace fixture",
  );

  const afterFirstRestart = await lifecycleSnapshot("sage.__test.restartLanguageServerAndWait");
  assert.equal(
    afterFirstRestart.launchCount,
    initialSnapshot.launchCount + 1,
    "expected the first managed restart to create exactly one additional client launch",
  );
  assert.ok(
    afterFirstRestart.managedCloseCount >= initialSnapshot.managedCloseCount + 1,
    "expected the first managed restart to record a managed close",
  );
  assert.equal(
    afterFirstRestart.unexpectedCloseCount,
    initialSnapshot.unexpectedCloseCount,
    "expected no unexpected closes during the first managed restart",
  );

  const afterSecondRestart = await lifecycleSnapshot("sage.__test.restartLanguageServerAndWait");
  assert.equal(
    afterSecondRestart.launchCount,
    afterFirstRestart.launchCount + 1,
    "expected the second managed restart to create exactly one additional client launch",
  );
  assert.ok(
    afterSecondRestart.managedCloseCount >= afterFirstRestart.managedCloseCount + 1,
    "expected the second managed restart to record another managed close",
  );
  assert.equal(
    afterSecondRestart.unexpectedCloseCount,
    afterFirstRestart.unexpectedCloseCount,
    "expected no unexpected closes during the second managed restart",
  );

  const postRestartHovers =
    (await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      hoverPosition,
    )) ?? [];
  assert.ok(
    postRestartHovers.some((hover) => renderHoverContents(hover).includes("looks like a matrix")),
    "expected hover results to remain available after managed restarts",
  );
}

async function waitForCommand(command: string, timeoutMs = 15_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const commands = await vscode.commands.getCommands(true);
    if (commands.includes(command)) {
      return;
    }
    await delay(100);
  }
  throw new Error(`timed out waiting for command registration: ${command}`);
}

async function lifecycleSnapshot(command: string): Promise<LifecycleSnapshot> {
  const result = await vscode.commands.executeCommand<LifecycleSnapshot>(command);
  assert.ok(result, `expected ${command} to return a lifecycle snapshot`);
  return result;
}

function positionOfNth(
  document: vscode.TextDocument,
  needle: string,
  occurrence: number,
  prefixLength = 0,
): vscode.Position {
  let startIndex = 0;
  let foundIndex = -1;

  for (let index = 0; index < occurrence; index += 1) {
    foundIndex = document.getText().indexOf(needle, startIndex);
    if (foundIndex === -1) {
      throw new Error(`could not find occurrence ${occurrence} of '${needle}'`);
    }
    startIndex = foundIndex + needle.length;
  }

  return document.positionAt(foundIndex + prefixLength);
}

function renderHoverContents(hover: vscode.Hover): string {
  return hover.contents
    .map((entry) => {
      if (typeof entry === "string") {
        return entry;
      }
      if ("value" in entry) {
        return String(entry.value);
      }
      return String(entry);
    })
    .join("\n");
}

function definitionUri(location: vscode.Location | vscode.LocationLink): vscode.Uri {
  return "targetUri" in location ? location.targetUri : location.uri;
}

async function delay(milliseconds: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}
