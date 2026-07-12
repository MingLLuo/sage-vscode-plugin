import assert from "node:assert/strict";
import path from "node:path";

import * as vscode from "vscode";

interface LifecycleSnapshot {
  launchCount: number;
  managedCloseCount: number;
  unexpectedCloseCount: number;
  managedShutdownActive: boolean;
  restartQueued: boolean;
  operationInFlight: boolean;
  hasClient: boolean;
}

interface SageContextSnapshot {
  languageId?: string;
  pythonFilesEnabled: boolean;
  sourceRootCount: number;
  extraPathCount: number;
  isSageEditor: boolean;
  shouldAutoStartLanguageClient: boolean;
  shouldExposeSageExperience: boolean;
}

interface ConfigureWorkspaceProfileResult {
  profileId: string;
  updates: Array<{ setting: string; value: unknown }>;
}

interface IndexStatusSnapshot {
  generation?: number;
  pending_jobs?: number;
  pending_task?: string | null;
}

export async function run(): Promise<void> {
  if (process.env.SAGE_TEST_HOST_MODE === "plain-python") {
    await runPlainPythonQuietSmoke();
    return;
  }

  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(workspaceFolder, "expected the smoke workspace to be open in the extension host");

  const config = vscode.workspace.getConfiguration("sage", workspaceFolder.uri);
  await config.update(
    "languageServer.rustPath",
    process.env.SAGE_TEST_LS_PATH ?? "auto",
    vscode.ConfigurationTarget.Workspace,
  );
  await config.update(
    "analysis.sourceRoots",
    [
      "src",
      ...(process.env.SAGE_TEST_EXTERNAL_SOURCE_ROOT ? [process.env.SAGE_TEST_EXTERNAL_SOURCE_ROOT] : []),
      ...(process.env.SAGE_TEST_NATIVE_SOURCE_ROOT ? [process.env.SAGE_TEST_NATIVE_SOURCE_ROOT] : []),
    ],
    vscode.ConfigurationTarget.Workspace,
  );
  await config.update("analysis.extraPaths", ["vendor"], vscode.ConfigurationTarget.Workspace);
  await config.update("analysis.enablePythonFiles", false, vscode.ConfigurationTarget.Workspace);
  await config.update("analysis.enableRuntimeIntrospection", false, vscode.ConfigurationTarget.Workspace);
  await config.update("docs.showOnHover", true, vscode.ConfigurationTarget.Workspace);

  const bootstrapUri = vscode.Uri.joinPath(workspaceFolder.uri, "src", "01_hover_and_definition.sage");
  const bootstrapDocument = await vscode.workspace.openTextDocument(bootstrapUri);
  await vscode.window.showTextDocument(bootstrapDocument);

  await waitForCommand("sage.__test.getLifecycleSnapshot");
  await waitForCommand("sage.showIndexStatus");
  await waitForCommand("sage.showDocsStatus");
  await waitForCommand("sage.rebuildIndex");
  await waitForCommand("sage.__test.getCurrentSageContext");

  const activationSnapshot = await lifecycleSnapshot("sage.__test.getLifecycleSnapshot");
  assert.equal(typeof activationSnapshot.operationInFlight, "boolean", "expected activation to expose background LSP state");

  const initialSnapshot = await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");
  assert.ok(initialSnapshot.launchCount >= 1, "expected the Rust language client to launch during activation");
  assert.equal(initialSnapshot.unexpectedCloseCount, 0, "expected no unexpected client shutdowns before restart");
  assert.equal(
    vscode.workspace.getConfiguration("sage", workspaceFolder.uri).get("analysis.enablePythonFiles"),
    false,
    "expected the external source bridge check to run with ordinary Python analysis disabled",
  );
  await assertEventually(() => verifyExternalSageSourceFollowUpWithPythonDisabled(), 30_000);

  await waitForCommand("sage.__test.configureWorkspaceProfile");
  await verifyConfigureWorkspaceProfile(workspaceFolder.uri);
  await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");
  await triggerRefreshDuringInitialCacheReconcile(workspaceFolder.uri);

  await assertEventually(() => verifyWorkspaceHoverDefinitionCompletion(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyWorkspaceReferencesRename(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyExternalSageSourceReferenceBridge(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyProjectedDiagnostics(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyDocumentAndWorkspaceSymbols(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyNativeCythonDocumentSymbols(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyNativeCythonNavigation(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifySageAwarePythonWorkspace(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifyCellCodeLens(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifySavedModuleRefresh(workspaceFolder.uri), 30_000);
  await verifyUnsavedUnicodeNavigationRanges(workspaceFolder.uri);

  const baselineIndexStatus = await vscode.commands.executeCommand<IndexStatusSnapshot>("sage.showIndexStatus");
  await vscode.commands.executeCommand("sage.showDocsStatus");
  const rebuiltIndexStatus = await vscode.commands.executeCommand<IndexStatusSnapshot>("sage.rebuildIndex");
  assert.ok(baselineIndexStatus, "expected index status before the explicit rebuild");
  assert.ok(rebuiltIndexStatus, "expected the rebuild command to return its completed status");
  assert.equal(rebuiltIndexStatus.pending_jobs, 0, "expected rebuild command to wait until indexing is idle");
  assert.ok(
    typeof rebuiltIndexStatus.generation === "number"
      && typeof baselineIndexStatus.generation === "number"
      && rebuiltIndexStatus.generation > baselineIndexStatus.generation,
    "expected rebuild command to wait for a newer index generation",
  );
  await assertEventually(() => verifyWorkspaceHoverDefinitionCompletion(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifySageAwarePythonWorkspace(workspaceFolder.uri), 30_000);

  const restartBaseline = await lifecycleSnapshot("sage.__test.awaitLanguageClientStable");
  const afterRestart = await lifecycleSnapshot("sage.__test.restartLanguageServerAndWait");
  assert.equal(
    afterRestart.launchCount,
    restartBaseline.launchCount + 1,
    "expected a managed restart to create exactly one additional client launch",
  );
  assert.ok(
    afterRestart.managedCloseCount >= restartBaseline.managedCloseCount + 1,
    "expected a managed restart to record a managed close",
  );
  assert.equal(
    afterRestart.unexpectedCloseCount,
    restartBaseline.unexpectedCloseCount,
    "expected no unexpected closes during managed restart",
  );

  await assertEventually(() => verifyWorkspaceHoverDefinitionCompletion(workspaceFolder.uri), 30_000);
  await assertEventually(() => verifySageAwarePythonWorkspace(workspaceFolder.uri), 30_000);
}

async function verifyExternalSageSourceFollowUpWithPythonDisabled(): Promise<void> {
  const externalSourceRoot = process.env.SAGE_TEST_EXTERNAL_SOURCE_ROOT;
  assert.ok(externalSourceRoot, "expected extension-host smoke to provide an external Sage source root");

  const sourceUri = vscode.Uri.from({
    scheme: "sage-source",
    path: path.join(externalSourceRoot, "sage", "combinat", "combination.py"),
  });
  const sourceDocument = await vscode.workspace.openTextDocument(sourceUri);
  await vscode.window.showTextDocument(sourceDocument);
  assert.equal(
    (await sageContextSnapshot()).isSageEditor,
    true,
    "expected a configured read-only Sage source to remain a Sage editor with Python analysis disabled",
  );
  const sourcePosition = positionOfNth(sourceDocument, "ExternalSmokeCombinations", 1);
  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      sourceUri,
      sourcePosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    definitions,
    "external-sage-src/sage/combinat/combination.py",
    "follow-up external definition while ordinary Python analysis is disabled",
    "sage-source",
  );
}

async function verifyUnsavedUnicodeNavigationRanges(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "__unsaved_unicode_navigation.sage");
  await vscode.workspace.fs.writeFile(
    uri,
    Buffer.from("def live_helper():\n    return 1\n\nvalue = live_helper()\n", "utf-8"),
  );
  const document = await vscode.workspace.openTextDocument(uri);
  const editor = await vscode.window.showTextDocument(document);
  const liveText = [
    "π_value = '🚀'",
    "def live_helper():",
    "    return 1",
    "",
    "prefix = '🚀'; value = live_helper()",
    "",
  ].join("\n");
  const edit = new vscode.WorkspaceEdit();
  edit.replace(uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), liveText);
  assert.equal(await vscode.workspace.applyEdit(edit), true, "expected to apply the unsaved Unicode navigation edit");
  assert.equal(document.isDirty, true, "expected the Unicode navigation fixture to remain unsaved");

  const usagePosition = positionOfNth(document, "live_helper", 2);
  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      usagePosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    definitions,
    "src/__unsaved_unicode_navigation.sage",
    "definition in an unsaved Unicode document",
    "file",
  );
  const definition = definitions[0];
  assert.ok(definition, "expected the unsaved definition result");
  assert.equal(
    document.getText(definitionRange(definition)),
    "live_helper",
    "expected the definition range to use the live UTF-16 document coordinates",
  );

  const references =
    (await vscode.commands.executeCommand<vscode.Location[]>(
      "vscode.executeReferenceProvider",
      uri,
      usagePosition,
    )) ?? [];
  const localReferences = references.filter((location) => location.uri.toString() === uri.toString());
  assert.equal(localReferences.length, 2, "expected stale indexed ranges to be replaced by two live references");
  assert.ok(
    localReferences.every((location) => document.getText(location.range) === "live_helper"),
    "expected every live reference range to select the symbol exactly",
  );

  const renameEdit = await vscode.commands.executeCommand<vscode.WorkspaceEdit>(
    "vscode.executeDocumentRenameProvider",
    uri,
    usagePosition,
    "live_helper_renamed",
  );
  assert.ok(renameEdit, "expected rename edits for the unsaved Unicode document");
  const localRenameEdits = renameEdit.get(uri);
  assert.equal(localRenameEdits.length, 2, "expected rename to avoid stale duplicate edits");
  assert.ok(
    localRenameEdits.every((entry) => document.getText(entry.range) === "live_helper"),
    "expected every rename edit to use the live UTF-16 symbol range",
  );
  editor.selection = new vscode.Selection(usagePosition, usagePosition);
}

async function triggerRefreshDuringInitialCacheReconcile(workspaceUri: vscode.Uri): Promise<void> {
  await assertEventually(async () => {
    const status = await vscode.commands.executeCommand<IndexStatusSnapshot>("sage.showIndexStatus");
    assert.equal(
      status?.pending_task,
      "cache-check",
      `expected the delayed cold index task, got ${status?.pending_task ?? "idle"}`,
    );
  }, 5_000, 50);

  const raceUri = vscode.Uri.joinPath(workspaceUri, "src", "__index_reconcile_refresh_race.sage");
  await vscode.workspace.fs.writeFile(raceUri, Buffer.from("race_marker = 1\n", "utf-8"));
  const raceDocument = await vscode.workspace.openTextDocument(raceUri);
  const edit = new vscode.WorkspaceEdit();
  edit.insert(raceUri, raceDocument.positionAt(raceDocument.getText().length), "race_marker_saved = race_marker\n");
  assert.equal(await vscode.workspace.applyEdit(edit), true, "expected to edit the reconcile race fixture");
  assert.equal(await raceDocument.save(), true, "expected to save while the cold index task is running");
}

async function runPlainPythonQuietSmoke(): Promise<void> {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(workspaceFolder, "expected the plain Python smoke workspace to be open in the extension host");

  const plainUri = vscode.Uri.joinPath(workspaceFolder.uri, "plain.py");
  const document = await vscode.workspace.openTextDocument(plainUri);
  await vscode.window.showTextDocument(document);

  await waitForCommand("sage.__test.getLifecycleSnapshot");
  await waitForCommand("sage.__test.getCurrentSageContext");
  await delay(1_000);

  const context = await sageContextSnapshot();
  assert.equal(context.languageId, "python");
  assert.equal(context.pythonFilesEnabled, false);
  assert.equal(context.sourceRootCount, 0);
  assert.equal(context.extraPathCount, 0);
  assert.equal(context.isSageEditor, false);
  assert.equal(context.shouldAutoStartLanguageClient, false);
  assert.equal(context.shouldExposeSageExperience, false);

  const snapshot = await lifecycleSnapshot("sage.__test.getLifecycleSnapshot");
  assert.equal(snapshot.launchCount, 0, "ordinary Python workspace should not auto-start the Sage LSP");
  assert.equal(snapshot.hasClient, false, "ordinary Python workspace should not hold a Sage LSP client");
  assert.equal(snapshot.operationInFlight, false, "ordinary Python workspace should not have Sage LSP startup in flight");
}

async function verifyWorkspaceHoverDefinitionCompletion(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "01_hover_and_definition.sage");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const hoverPosition = positionOfNth(document, "make_demo_matrix", 2);
  const hovers = (await vscode.commands.executeCommand<vscode.Hover[]>(
    "vscode.executeHoverProvider",
    uri,
    hoverPosition,
  )) ?? [];
  assert.ok(
    hovers.some((hover) => renderHoverContents(hover).includes("looks like a matrix")),
    "expected Rust hover text for make_demo_matrix from the workspace fixture",
  );

  const definitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      hoverPosition,
    )) ?? [];
  const localDefinition = definitions.find((definition) =>
    definitionUri(definition).fsPath.endsWith("src/local_docs.py")
  );
  assert.ok(
    localDefinition,
    "expected Rust definition for make_demo_matrix to resolve into local_docs.py",
  );
  const expectedLocalDefinitionUri = vscode.Uri.joinPath(workspaceUri, "src", "local_docs.py");
  assert.equal(
    definitionUri(localDefinition).toString(),
    expectedLocalDefinitionUri.toString(),
    "expected canonical server paths to map back to the workspace URI identity",
  );
  assert.equal(
    vscode.workspace.getWorkspaceFolder(definitionUri(localDefinition))?.uri.toString(),
    workspaceUri.toString(),
    "expected the definition target to retain workspace scope",
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
    "expected Rust completion items to include summarize_coefficients from the workspace fixture",
  );
}

async function verifyWorkspaceReferencesRename(workspaceUri: vscode.Uri): Promise<void> {
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
  assert.ok(referenceUris.size >= 2, "expected Rust references for make_demo_matrix across definition and usage sites");
  assert.ok(
    [...referenceUris].some((entry) => entry.endsWith("src/local_docs.py")),
    "expected Rust references to include the function definition module",
  );

  const renameEdit =
    await vscode.commands.executeCommand<vscode.WorkspaceEdit>(
      "vscode.executeDocumentRenameProvider",
      uri,
      helperPosition,
      "make_demo_matrix_renamed",
    );
  assert.ok(renameEdit, "expected a Rust rename edit for make_demo_matrix");
  const renameEntries = renameEdit.entries().map(([targetUri]) => targetUri.fsPath);
  assert.ok(renameEntries.some((entry) => entry.endsWith("src/local_docs.py")));
  assert.ok(renameEntries.some((entry) => entry.endsWith("src/01_hover_and_definition.sage")));
}

async function verifyExternalSageSourceReferenceBridge(workspaceUri: vscode.Uri): Promise<void> {
  const externalSourceRoot = process.env.SAGE_TEST_EXTERNAL_SOURCE_ROOT;
  assert.ok(externalSourceRoot, "expected extension-host smoke to provide an external Sage source root");

  const usageUri = vscode.Uri.joinPath(workspaceUri, "src", "__external_navigation_bridge.sage");
  const usageDocument = await vscode.workspace.openTextDocument(usageUri);
  await vscode.window.showTextDocument(usageDocument);

  const usagePosition = positionOfNth(usageDocument, "ExternalSmokeCombinations", 1);
  const sourceDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      usageUri,
      usagePosition,
    )) ?? [];
  const sourceDefinition = sourceDefinitions.find((definition) =>
    normalizePathForAssertion(definitionUri(definition).fsPath)
      .endsWith("external-sage-src/sage/combinat/combination.py")
  );
  assert.ok(
    sourceDefinition,
    `expected ExternalSmokeCombinations to resolve into the external Sage source fixture, got ${sourceDefinitions.map((definition) => definitionUri(definition).toString()).join(", ")}`,
  );
  const sourceUri = definitionUri(sourceDefinition);
  assert.equal(sourceUri.scheme, "sage-source", "expected the real definition jump to use the read-only source view");

  const sourceDocument = await vscode.workspace.openTextDocument(sourceUri);
  const sourceEditor = await vscode.window.showTextDocument(sourceDocument);
  const sourcePosition = definitionRange(sourceDefinition).start;
  sourceEditor.selection = new vscode.Selection(sourcePosition, sourcePosition);

  const followUpDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      sourceUri,
      sourcePosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    followUpDefinitions,
    "external-sage-src/sage/combinat/combination.py",
    "follow-up ExternalSmokeCombinations definition from sage-source",
    "sage-source",
  );

  const references =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeReferenceProvider",
      sourceUri,
      sourcePosition,
    )) ?? [];
  const referenceUris = references.map((reference) => definitionUri(reference));
  assert.ok(
    referenceUris.some((entry) =>
      entry.scheme === "sage-source"
      && normalizePathForAssertion(entry.fsPath).endsWith("external-sage-src/sage/combinat/combination.py")
    ),
    "expected references from the real sage-source jump to include its read-only declaration",
  );
  assert.ok(
    referenceUris.some((entry) =>
      normalizePathForAssertion(entry.fsPath).endsWith("src/__external_navigation_bridge.sage")
    ),
    "expected references from the real sage-source jump to include workspace Sage usages",
  );

  const commandReferences =
    (await vscode.commands.executeCommand<vscode.Location[]>("sage.findReferences")) ?? [];
  assert.ok(
    commandReferences.some((entry) =>
      entry.uri.scheme === "sage-source"
      && normalizePathForAssertion(entry.uri.fsPath).endsWith("external-sage-src/sage/combinat/combination.py")
    ),
    "expected the user-facing Sage references command to preserve the read-only declaration URI",
  );
  assert.ok(
    commandReferences.some((entry) =>
      normalizePathForAssertion(entry.uri.fsPath).endsWith("src/__external_navigation_bridge.sage")
    ),
    "expected the user-facing Sage references command to include the workspace usage",
  );
}

async function verifyProjectedDiagnostics(workspaceUri: vscode.Uri): Promise<void> {
  const syntaxUri = vscode.Uri.joinPath(workspaceUri, "src", "__tmp_projection_check.sage");
  await vscode.workspace.fs.writeFile(syntaxUri, Buffer.from("value = 2^\n", "utf-8"));
  const syntaxDocument = await vscode.workspace.openTextDocument(syntaxUri);
  await vscode.window.showTextDocument(syntaxDocument);

  const syntaxDiagnostics = await waitForDiagnostics(
    syntaxUri,
    (diagnostics) => diagnostics.some((diagnostic) => diagnostic.message.startsWith("Syntax error:")),
  );
  const syntaxDiagnostic = syntaxDiagnostics.find((diagnostic) => diagnostic.message.startsWith("Syntax error:"));
  assert.ok(syntaxDiagnostic, "expected a Rust syntax diagnostic for the projected .sage error");
  assert.equal(String(syntaxDiagnostic.code), "syntax-error");
  assert.equal(syntaxDiagnostic.range.start.line, 0);
  assert.equal(syntaxDiagnostic.range.start.character, 9);
  assert.equal(syntaxDiagnostic.range.end.character, 10);
}

async function verifyDocumentAndWorkspaceSymbols(workspaceUri: vscode.Uri): Promise<void> {
  const symbolsUri = vscode.Uri.joinPath(workspaceUri, "src", "05_symbols_and_locals.sage");
  const symbolsDocument = await vscode.workspace.openTextDocument(symbolsUri);
  await vscode.window.showTextDocument(symbolsDocument);

  const symbols =
    (await vscode.commands.executeCommand<Array<vscode.DocumentSymbol | vscode.SymbolInformation>>(
      "vscode.executeDocumentSymbolProvider",
      symbolsUri,
    )) ?? [];
  const symbolNames = new Set(flattenSymbolNames(symbols));
  for (const expected of ["LocalContainer", "local_builder", "R", "z"]) {
    assert.ok(symbolNames.has(expected), `expected Rust document symbols to include ${expected}`);
  }

  const workspaceSymbols =
    (await vscode.commands.executeCommand<vscode.SymbolInformation[]>(
      "vscode.executeWorkspaceSymbolProvider",
      "PolynomialNotebook",
    )) ?? [];
  assert.ok(
    workspaceSymbols.some((entry) =>
      entry.name === "PolynomialNotebook" && entry.location.uri.fsPath.endsWith("src/local_docs.py"),
    ),
    "expected Rust workspace symbols to include PolynomialNotebook from the local fixture module",
  );
}

async function verifyNativeCythonDocumentSymbols(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "cythonish_bridge.pyx");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const symbols =
    (await vscode.commands.executeCommand<Array<vscode.DocumentSymbol | vscode.SymbolInformation>>(
      "vscode.executeDocumentSymbolProvider",
      uri,
    )) ?? [];
  const symbolNames = new Set(flattenSymbolNames(symbols));
  for (const expected of ["fast_square", "StepCounter", "describe_counter", "stepped_square"]) {
    assert.ok(symbolNames.has(expected), `expected Rust Cython document symbols to include ${expected}`);
  }
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
    "expected Rust NativeAccumulator definition to resolve into native_support.pxd",
  );
}

async function verifySageAwarePythonWorkspace(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "10_sage_heavy_python.py");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const context = await sageContextSnapshot();
  assert.equal(context.languageId, "python");
  assert.equal(context.pythonFilesEnabled, true);
  assert.equal(context.isSageEditor, true);
  assert.equal(context.shouldAutoStartLanguageClient, true);
  assert.equal(context.shouldExposeSageExperience, true);

  const polynomialPosition = positionOfNth(document, "PolynomialRing", 2);
  const polynomialDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      polynomialPosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    polynomialDefinitions,
    "sage/rings/polynomial/polynomial_ring_constructor.py",
    "PolynomialRing",
    "sage-source",
  );
  const polynomialDeclarations =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDeclarationProvider",
      uri,
      polynomialPosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    polynomialDeclarations,
    "sage/rings/polynomial/polynomial_ring_constructor.py",
    "PolynomialRing declaration",
    "sage-source",
  );
  const polynomialImplementations =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeImplementationProvider",
      uri,
      polynomialPosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    polynomialImplementations,
    "sage/rings/polynomial/polynomial_ring_constructor.py",
    "PolynomialRing implementation",
    "sage-source",
  );

  const matrixPosition = positionOfNth(document, "mat = matrix", 1, "mat = ".length);
  const matrixDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      uri,
      matrixPosition,
    )) ?? [];
  assertSingleDefinitionTarget(matrixDefinitions, "sage/matrix/constructor.pyx", "matrix", "sage-source");
  const matrixDefinition = matrixDefinitions[0];
  assert.ok(matrixDefinition, "expected matrix definition target for follow-up navigation");
  const matrixSourceUri = definitionUri(matrixDefinition);
  const matrixSourceDocument = await vscode.workspace.openTextDocument(matrixSourceUri);
  await vscode.window.showTextDocument(matrixSourceDocument);
  assert.equal(matrixSourceUri.scheme, "sage-source");
  assert.equal(matrixSourceDocument.languageId, "sagemath-cython");
  const matrixFollowUpDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeDefinitionProvider",
      matrixSourceUri,
      definitionRange(matrixDefinition).start,
    )) ?? [];
  assertSingleDefinitionTarget(
    matrixFollowUpDefinitions,
    "sage/matrix/constructor.pyx",
    "matrix follow-up definition from sage-source Cython",
    "sage-source",
  );

  const ringUsagePosition = positionOfNth(document, "ring.gens", 1);
  const polynomialTypeDefinitions =
    (await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
      "vscode.executeTypeDefinitionProvider",
      uri,
      ringUsagePosition,
    )) ?? [];
  assertSingleDefinitionTarget(
    polynomialTypeDefinitions,
    "sage/rings/polynomial/polynomial_ring_constructor.py",
    "PolynomialRing type definition",
    "sage-source",
  );

  const rankPosition = positionOfNth(document, ".rank", 1, 1);
  const rankHovers = (await vscode.commands.executeCommand<vscode.Hover[]>(
    "vscode.executeHoverProvider",
    uri,
    rankPosition,
  )) ?? [];
  assert.ok(
    rankHovers.some((hover) => renderHoverContents(hover).includes("Return the rank of this matrix")),
    "expected Sage-aware Python hover docs for Matrix.rank",
  );
}

async function verifyConfigureWorkspaceProfile(workspaceUri: vscode.Uri): Promise<void> {
  const result = await vscode.commands.executeCommand<ConfigureWorkspaceProfileResult>(
    "sage.__test.configureWorkspaceProfile",
    "research",
  );
  assert.ok(result, "expected test workspace configuration command to return applied updates");
  assert.equal(result.profileId, "research");

  const updatedSections = new Set(result.updates.map((entry) => entry.setting));
  for (const expected of [
    "sage.languageServer.rustPath",
    "sage.analysis.mode",
    "sage.analysis.sourceRoots",
    "sage.analysis.extraPaths",
    "sage.analysis.enablePythonFiles",
    "sage.analysis.enablePyxParsing",
    "sage.docs.showOnHover",
    "python.analysis.extraPaths",
    "python.analysis.diagnosticSeverityOverrides",
  ]) {
    assert.ok(updatedSections.has(expected), `expected Configure Workspace to update ${expected}`);
  }

  const config = vscode.workspace.getConfiguration("sage", workspaceUri);
  assert.equal(config.get("analysis.mode"), "full");
  assert.equal(config.get("analysis.enablePythonFiles"), true);
  assert.equal(config.get("analysis.enablePyxParsing"), true);
  assert.equal(config.get("languageServer.rustPath"), "auto");
  const sourceRoots = config.get<string[]>("analysis.sourceRoots") ?? [];
  assert.ok(sourceRoots.includes("src"), "expected Configure Workspace to include the workspace source root");
  const extraPaths = config.get<string[]>("analysis.extraPaths") ?? [];
  assert.ok(extraPaths.includes("src"), "expected Configure Workspace to include the workspace source root in extra paths");
  assert.ok(extraPaths.includes("vendor"), "expected Configure Workspace to preserve existing extra paths");
  const pythonConfig = vscode.workspace.getConfiguration("python", workspaceUri);
  const pythonExtraPaths = pythonConfig.get<string[]>("analysis.extraPaths") ?? [];
  assert.ok(pythonExtraPaths.includes("src"), "expected Configure Workspace to teach Pylance about local workspace roots");
  assert.ok(pythonExtraPaths.includes("vendor"), "expected Configure Workspace to preserve Pylance-visible helper paths");
  assert.ok(
    !pythonExtraPaths.some((entry) => entry.endsWith("sage/src") || entry.endsWith("sage\\src")),
    "expected Configure Workspace to keep external Sage internals out of Pylance extra paths",
  );
  const severityOverrides = pythonConfig.get<Record<string, string>>("analysis.diagnosticSeverityOverrides") ?? {};
  assert.equal(severityOverrides.reportMissingImports, "none");
  assert.equal(severityOverrides.reportMissingModuleSource, "none");
}

async function verifyCellCodeLens(workspaceUri: vscode.Uri): Promise<void> {
  const uri = vscode.Uri.joinPath(workspaceUri, "src", "__tmp_cell_codelens.sage");
  await vscode.workspace.fs.writeFile(
    uri,
    Buffer.from([
      "# %% setup",
      "R = PolynomialRing(QQ, 'x')",
      "# region solve block",
      "I = R.ideal(x^2 + 1)",
      "# endregion",
      "I.variety()",
      "",
    ].join("\n"), "utf-8"),
  );
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  const config = vscode.workspace.getConfiguration("sage", workspaceUri);
  await config.update("run.showCellCodeLens", true, vscode.ConfigurationTarget.Workspace);
  try {
    const lenses =
      (await vscode.commands.executeCommand<vscode.CodeLens[]>(
        "vscode.executeCodeLensProvider",
        uri,
      )) ?? [];
    assert.ok(lenses.some((lens) => lens.command?.title === "Run Cell"), "expected Sage Run Cell CodeLens");
    assert.ok(lenses.some((lens) => lens.command?.title === "Run Region"), "expected Sage Run Region CodeLens");

    const cellLens = lenses.find((lens) => lens.command?.title === "Run Cell");
    const regionLens = lenses.find((lens) => lens.command?.title === "Run Region");
    assert.equal(cellLens?.command?.command, "sage.runCurrentCell");
    assert.equal(regionLens?.command?.command, "sage.runCurrentCell");
    assert.equal(codeLensTarget(cellLens).line, 0);
    assert.equal(codeLensTarget(regionLens).line, 2);
    assert.equal(codeLensTarget(cellLens).uri?.toString(), uri.toString());
    assert.equal(codeLensTarget(regionLens).uri?.toString(), uri.toString());

    await config.update("run.showCellCodeLens", false, vscode.ConfigurationTarget.Workspace);
    const hiddenLenses =
      (await vscode.commands.executeCommand<vscode.CodeLens[]>(
        "vscode.executeCodeLensProvider",
        uri,
      )) ?? [];
    assert.equal(hiddenLenses.length, 0, "expected Sage cell CodeLens to respect sage.run.showCellCodeLens=false");
  } finally {
    await config.update("run.showCellCodeLens", true, vscode.ConfigurationTarget.Workspace);
  }
}

async function verifySavedModuleRefresh(workspaceUri: vscode.Uri): Promise<void> {
  const helperUri = vscode.Uri.joinPath(workspaceUri, "src", "local_docs.py");
  const helperDocument = await vscode.workspace.openTextDocument(helperUri);
  const helperEditor = await vscode.window.showTextDocument(helperDocument);
  const originalSource = helperDocument.getText();
  const originalSummary = "Return a comma-separated summary for documentation and hover tests.";
  const updatedSummary = "Return an updated comma-separated summary after a Rust save.";
  const updatedSource = originalSource.includes(updatedSummary)
    ? originalSource
    : originalSource.replace(originalSummary, updatedSummary);
  assert.ok(updatedSource.includes(updatedSummary), "expected the smoke fixture docstring to be replaceable");

  if (updatedSource !== originalSource) {
    await helperEditor.edit((editBuilder) => {
      const finalLine = helperDocument.lineAt(helperDocument.lineCount - 1);
      editBuilder.replace(
        new vscode.Range(0, 0, helperDocument.lineCount - 1, finalLine.text.length),
        updatedSource,
      );
    });
    await helperDocument.save();
  }

  const sageUri = vscode.Uri.joinPath(workspaceUri, "src", "01_hover_and_definition.sage");
  const sageDocument = await vscode.workspace.openTextDocument(sageUri);
  await vscode.window.showTextDocument(sageDocument);

  const hoverPosition = positionOfNth(sageDocument, "summarize_coefficients", 2);
  const hovers = (await vscode.commands.executeCommand<vscode.Hover[]>(
    "vscode.executeHoverProvider",
    sageUri,
    hoverPosition,
  )) ?? [];
  assert.ok(
    hovers.some((hover) =>
      normalizeWhitespace(renderHoverContents(hover)).includes(
        "updated comma-separated summary after a Rust save",
      ),
    ),
    "expected Rust hover docs to refresh after saving the imported Python helper module",
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

async function waitForDiagnostics(
  uri: vscode.Uri,
  predicate: (diagnostics: readonly vscode.Diagnostic[]) => boolean,
  timeoutMs = 15_000,
): Promise<readonly vscode.Diagnostic[]> {
  const start = Date.now();
  let lastDiagnostics: readonly vscode.Diagnostic[] = [];

  while (Date.now() - start < timeoutMs) {
    lastDiagnostics = vscode.languages.getDiagnostics(uri);
    if (predicate(lastDiagnostics)) {
      return lastDiagnostics;
    }
    await delay(100);
  }

  throw new Error(`timed out waiting for diagnostics on ${uri.fsPath}: ${lastDiagnostics.map((diagnostic) => diagnostic.message).join(" | ")}`);
}

async function lifecycleSnapshot(command: string): Promise<LifecycleSnapshot> {
  const result = await vscode.commands.executeCommand<LifecycleSnapshot>(command);
  assert.ok(result, `expected ${command} to return a lifecycle snapshot`);
  return result;
}

async function sageContextSnapshot(): Promise<SageContextSnapshot> {
  const result = await vscode.commands.executeCommand<SageContextSnapshot>("sage.__test.getCurrentSageContext");
  assert.ok(result, "expected sage.__test.getCurrentSageContext to return a context snapshot");
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

function normalizePathForAssertion(value: string): string {
  return value.replace(/\\/g, "/").replace(/^\/private\/var\//, "/var/");
}

function definitionUri(location: vscode.Location | vscode.LocationLink): vscode.Uri {
  return "targetUri" in location ? location.targetUri : location.uri;
}

function definitionRange(location: vscode.Location | vscode.LocationLink): vscode.Range {
  if ("targetUri" in location) {
    return location.targetSelectionRange ?? location.targetRange;
  }
  return location.range;
}

function definitionKey(location: vscode.Location | vscode.LocationLink): string {
  const uri = definitionUri(location);
  const range = definitionRange(location);
  return [
    uri.toString(),
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  ].join("|");
}

function assertSingleDefinitionTarget(
  definitions: Array<vscode.Location | vscode.LocationLink>,
  expectedPathSuffix: string,
  label: string,
  expectedScheme?: string,
): void {
  const keys = new Set(definitions.map(definitionKey));
  assert.equal(
    keys.size,
    definitions.length,
    `expected ${label} definition targets to be deduplicated`,
  );
  assert.equal(
    definitions.length,
    1,
    `expected ${label} to have exactly one VS Code definition target, got ${definitions.map((definition) => definitionUri(definition).toString()).join(", ")}`,
  );
  const onlyDefinition = definitions[0];
  assert.ok(onlyDefinition, `expected ${label} definition to exist`);
  const actualUri = definitionUri(onlyDefinition);
  assert.ok(
    normalizePathForAssertion(actualUri.fsPath).endsWith(expectedPathSuffix),
    `expected ${label} definition to resolve into ${expectedPathSuffix}, got ${actualUri.toString()} (${actualUri.fsPath})`,
  );
  if (expectedScheme) {
    assert.equal(
      definitionUri(onlyDefinition).scheme,
      expectedScheme,
      `expected ${label} definition to use ${expectedScheme} source view`,
    );
  }
}

function codeLensTarget(lens: vscode.CodeLens | undefined): { uri?: vscode.Uri; line?: number } {
  assert.ok(lens?.command?.arguments?.[0], "expected Sage CodeLens command arguments");
  return lens.command.arguments[0] as { uri?: vscode.Uri; line?: number };
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
