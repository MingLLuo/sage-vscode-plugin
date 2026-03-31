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

  const bootstrapUri = vscode.Uri.joinPath(workspaceFolder.uri, "src", "01_hover_and_definition.sage");
  const bootstrapDocument = await vscode.workspace.openTextDocument(bootstrapUri);
  await vscode.window.showTextDocument(bootstrapDocument);

  await waitForCommand("sage.__test.getLifecycleSnapshot");
  const initialSnapshot = await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");
  assert.ok(initialSnapshot.launchCount >= 1, "expected the language client to launch during activation");
  assert.equal(initialSnapshot.unexpectedCloseCount, 0, "expected no unexpected client shutdowns before restart");

  await assertEventually(() => verifyWorkspaceHoverDefinitionCompletion(workspaceFolder.uri));
  await assertEventually(() => verifyWorkspaceReferencesRenameAndSymbols(workspaceFolder.uri));
  await assertEventually(() => verifyNativeCythonNavigation(workspaceFolder.uri));

  if (process.env.SAGE_TEST_NATIVE_SOURCE_ROOT) {
    await assertEventually(() =>
      verifyNativeSageLibraryNavigation(
        workspaceFolder.uri,
        config,
        process.env.SAGE_TEST_NATIVE_SOURCE_ROOT ?? "",
        process.env.SAGE_TEST_NATIVE_SAGE_EXECUTABLE,
      ),
    );
  }

  const restartBaseline = await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");

  const afterFirstRestart = await lifecycleSnapshot("sage.__test.restartLanguageServerAndWait");
  assert.equal(
    afterFirstRestart.launchCount,
    restartBaseline.launchCount + 1,
    "expected the first managed restart to create exactly one additional client launch",
  );
  assert.ok(
    afterFirstRestart.managedCloseCount >= restartBaseline.managedCloseCount + 1,
    "expected the first managed restart to record a managed close",
  );
  assert.equal(
    afterFirstRestart.unexpectedCloseCount,
    restartBaseline.unexpectedCloseCount,
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

  const uri = vscode.Uri.joinPath(workspaceFolder.uri, "src", "01_hover_and_definition.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);
  const hoverPosition = positionOfNth(document, "make_demo_matrix", 2);
  const hovers = (await vscode.commands.executeCommand<vscode.Hover[]>("vscode.executeHoverProvider", uri, hoverPosition)) ?? [];
  assert.ok(
    hovers.some((hover) => renderHoverContents(hover).includes("looks like a matrix")),
    "expected hover results to remain available after managed restarts",
  );
}

async function verifyWorkspaceHoverDefinitionCompletion(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "01_hover_and_definition.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const hoverPosition = positionOfNth(document, "make_demo_matrix", 2);
  const hovers = (await vscode.commands.executeCommand<vscode.Hover[]>("vscode.executeHoverProvider", uri, hoverPosition)) ?? [];
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
}

async function verifyWorkspaceReferencesRenameAndSymbols(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "01_hover_and_definition.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const helperPosition = positionOfNth(document, "make_demo_matrix", 2);
  const references =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeReferenceProvider",
      uri,
      helperPosition,
    )) ?? [];
  const referenceUris = new Set(references.map((reference) => definitionUri(reference).fsPath));
  assert.ok(referenceUris.size >= 2, "expected references for make_demo_matrix across definition and usage sites");
  assert.ok(
    [...referenceUris].some((entry) => entry.endsWith("src/local_docs.py")),
    "expected references to include the function definition module",
  );

  const renameEdit =
    await vscode.commands.executeCommand<vscode.WorkspaceEdit>(
      "vscode.executeDocumentRenameProvider",
      uri,
      helperPosition,
      "make_demo_matrix_renamed",
    );
  assert.ok(renameEdit, "expected a rename edit for make_demo_matrix");
  const renameEntries = renameEdit.entries().map(([targetUri]) => targetUri.fsPath);
  assert.ok(renameEntries.some((entry) => entry.endsWith("src/local_docs.py")));
  assert.ok(renameEntries.some((entry) => entry.endsWith("src/01_hover_and_definition.sage")));

  const symbolsUri = vscode.Uri.joinPath(workspaceUri, "src", "05_symbols_and_locals.sage");
  const symbols =
    (await vscode.commands.executeCommand<Array<vscode.DocumentSymbol | vscode.SymbolInformation>>(
      "vscode.executeDocumentSymbolProvider",
      symbolsUri,
    )) ?? [];
  const symbolNames = new Set(flattenSymbolNames(symbols));
  for (const expected of ["LocalContainer", "local_builder", "GAMMA", "R", "z"]) {
    assert.ok(symbolNames.has(expected), `expected document symbols to include ${expected}`);
  }

  const workspaceSymbols =
    (await vscode.commands.executeCommand<vscode.SymbolInformation[]>(
      "vscode.executeWorkspaceSymbolProvider",
      "PolynomialNotebook",
    )) ?? [];
  assert.ok(
    workspaceSymbols.some((entry) => entry.name === "PolynomialNotebook" && entry.location.uri.fsPath.endsWith("src/local_docs.py")),
    "expected workspace symbols to include PolynomialNotebook from the local fixture module",
  );
}

async function verifyNativeCythonNavigation(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "cythonish_bridge.pyx");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const nativeAccumulatorPosition = positionOfNth(document, "NativeAccumulator", 1);
  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      nativeAccumulatorPosition,
    )) ?? [];
  assert.ok(
    definitions.some((definition) => definitionUri(definition).fsPath.endsWith("src/native_support.pxd")),
    "expected NativeAccumulator to resolve into native_support.pxd",
  );

  const symbols =
    (await vscode.commands.executeCommand<Array<vscode.DocumentSymbol | vscode.SymbolInformation>>(
      "vscode.executeDocumentSymbolProvider",
      uri,
    )) ?? [];
  const symbolNames = new Set(flattenSymbolNames(symbols));
  for (const expected of ["fast_square", "StepCounter", "stepped_square"]) {
    assert.ok(symbolNames.has(expected), `expected Cython symbols to include ${expected}`);
  }
}

async function verifyNativeSageLibraryNavigation(
  workspaceUri: vscode.Uri,
  config: vscode.WorkspaceConfiguration,
  nativeSourceRoot: string,
  nativeSageExecutable: string | undefined,
): Promise<void> {
  await config.update(
    "analysis.sourceRoots",
    [vscode.Uri.joinPath(workspaceUri, "src").fsPath, nativeSourceRoot],
    vscode.ConfigurationTarget.Workspace,
  );
  await config.update(
    "analysis.enableRuntimeIntrospection",
    Boolean(nativeSageExecutable),
    vscode.ConfigurationTarget.Workspace,
  );
  if (nativeSageExecutable) {
    await config.update("interpreter.path", nativeSageExecutable, vscode.ConfigurationTarget.Workspace);
  }

  await lifecycleSnapshot("sage.__test.restartLanguageServerAndWait");

  const uri = vscode.Uri.joinPath(workspaceUri, "src", "06_runtime_graphs_and_number_theory.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const graphPosition = positionOfNth(document, "graphs.PetersenGraph", 1, "graphs.".length);
  const hovers = (await vscode.commands.executeCommand<vscode.Hover[]>("vscode.executeHoverProvider", uri, graphPosition)) ?? [];
  assert.ok(
    hovers.some((hover) => renderHoverContents(hover).includes("Petersen Graph")),
    "expected native Sage hover docs for graphs.PetersenGraph",
  );

  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      graphPosition,
    )) ?? [];
  assert.ok(
    definitions.some((definition) => definitionUri(definition).fsPath.endsWith("sage/graphs/generators/smallgraphs.py")),
    "expected graphs.PetersenGraph to resolve into the Sage graph generator sources",
  );

  const workspaceSymbols =
    (await vscode.commands.executeCommand<vscode.SymbolInformation[]>(
      "vscode.executeWorkspaceSymbolProvider",
      "PetersenGraph",
    )) ?? [];
  assert.ok(
    workspaceSymbols.some((entry) => entry.location.uri.fsPath.endsWith("sage/graphs/generators/smallgraphs.py")),
    "expected workspace symbols to include the native PetersenGraph implementation",
  );

  const polynomialRingPosition = positionOfNth(document, "PolynomialRing", 1);
  const polynomialRingHovers =
    (await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      polynomialRingPosition,
    )) ?? [];
  assert.ok(
    polynomialRingHovers.some((hover) =>
      normalizeWhitespace(renderHoverContents(hover)).includes(
        "globally unique univariate or multivariate polynomial ring",
      ),
    ),
    "expected native Sage hover docs for PolynomialRing",
  );

  const polynomialRingDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      polynomialRingPosition,
    )) ?? [];
  assert.ok(
    polynomialRingDefinitions.some((definition) => definitionUri(definition).fsPath.endsWith("sage/rings/polynomial/polynomial_ring_constructor.py")),
    "expected PolynomialRing to resolve into the Sage polynomial ring constructor sources",
  );

  const ellipticCurvePosition = positionOfNth(document, "EllipticCurve([", 1);
  const ellipticCurveHovers =
    (await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      ellipticCurvePosition,
    )) ?? [];
  assert.ok(
    ellipticCurveHovers.some((hover) =>
      normalizeWhitespace(renderHoverContents(hover)).includes("Construct an elliptic curve."),
    ),
    "expected native Sage hover docs for EllipticCurve",
  );

  const ellipticCurveDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      ellipticCurvePosition,
    )) ?? [];
  assert.ok(
    ellipticCurveDefinitions.some((definition) =>
      definitionUri(definition).fsPath.endsWith("sage/schemes/elliptic_curves/constructor.py"),
    ),
    "expected EllipticCurve to resolve into the Sage elliptic curve constructor sources",
  );

  if (!nativeSageExecutable) {
    return;
  }

  const signaturePosition = positionOfNth(document, "EllipticCurve([", 1, "EllipticCurve(".length);
  const signatureHelp = await vscode.commands.executeCommand<vscode.SignatureHelp>(
    "vscode.executeSignatureHelpProvider",
    uri,
    signaturePosition,
    "(",
  );
  if (signatureHelp?.signatures?.length) {
    assert.ok(
      signatureHelp.signatures.some((signature) => signature.label.includes("EllipticCurve")),
      "expected runtime signature help for EllipticCurve when the runtime supplies signatures",
    );
  }
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

async function assertEventually(
  check: () => Promise<void>,
  timeoutMs = 15_000,
  intervalMs = 200,
): Promise<void> {
  const start = Date.now();
  let lastError: unknown;

  while (Date.now() - start < timeoutMs) {
    try {
      await check();
      return;
    } catch (error) {
      lastError = error;
      await delay(intervalMs);
    }
  }

  throw lastError instanceof Error ? lastError : new Error("timed out waiting for the expected editor state");
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

function normalizeWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function definitionUri(location: vscode.Location | vscode.LocationLink): vscode.Uri {
  return "targetUri" in location ? location.targetUri : location.uri;
}

function flattenSymbolNames(
  symbols: Array<vscode.DocumentSymbol | vscode.SymbolInformation>,
): string[] {
  const names: string[] = [];
  for (const symbol of symbols) {
    names.push(symbol.name);
    if ("children" in symbol) {
      names.push(...flattenSymbolNames(symbol.children));
    }
  }
  return names;
}

async function delay(milliseconds: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}
