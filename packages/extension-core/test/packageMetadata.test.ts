import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const packageRoot = path.resolve(__dirname, "..", "..");
const repositoryRoot = path.resolve(packageRoot, "..", "..");

interface ExtensionManifest {
  name?: string;
  publisher?: string;
  version?: string;
  license?: string;
  private?: boolean;
  categories?: string[];
  keywords?: string[];
  icon?: string;
  galleryBanner?: {
    color?: string;
    theme?: string;
  };
  preview?: boolean;
  qna?: boolean | string;
  extensionKind?: string[];
  engines?: {
    vscode?: string;
  };
  devDependencies?: Record<string, string>;
  capabilities?: {
    untrustedWorkspaces?: {
      supported?: boolean | "limited";
      description?: string;
      restrictedConfigurations?: string[];
    };
    virtualWorkspaces?: boolean | {
      supported?: boolean | "limited";
      description?: string;
    };
  };
  activationEvents?: string[];
  contributes?: {
    languages?: Array<{
      id: string;
      configuration?: string;
    }>;
    grammars?: Array<{
      language: string;
      path: string;
    }>;
    snippets?: Array<{
      language: string;
      path: string;
    }>;
    commands?: Array<{
      command: string;
      title: string;
      category?: string;
      icon?: string;
      enablement?: string;
    }>;
    menus?: Record<string, Array<{
      command?: string;
      when?: string;
      group?: string;
    }>>;
    configuration?: {
      properties?: Record<string, unknown>;
    };
    walkthroughs?: Array<{
      id: string;
      title: string;
      steps: Array<{
        id: string;
        title: string;
        media?: {
          markdown?: string;
        };
        completionEvents?: string[];
      }>;
    }>;
  };
}

interface RootPackage {
  packageManager?: string;
  scripts?: Record<string, string>;
}

function readManifest(): ExtensionManifest {
  return JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf-8")) as ExtensionManifest;
}

function readRootPackage(): RootPackage {
  return JSON.parse(fs.readFileSync(path.join(repositoryRoot, "package.json"), "utf-8")) as RootPackage;
}

function readCiWorkflow(): string {
  return fs.readFileSync(path.join(repositoryRoot, ".github", "workflows", "ci.yml"), "utf-8");
}

function readRepositoryFile(relativePath: string): string {
  return fs.readFileSync(path.join(repositoryRoot, relativePath), "utf-8");
}

function readPngDimensions(filePath: string): { width: number; height: number } {
  const buffer = fs.readFileSync(filePath);
  assert.equal(buffer.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  return {
    width: buffer.readUInt32BE(16),
    height: buffer.readUInt32BE(20),
  };
}

test("extension package name is a valid VS Code extension identifier", () => {
  const manifest = readManifest();

  assert.equal(manifest.name, "sage-vscode-extension");
  assert.equal(manifest.publisher, "sage-vscode");
  assert.match(manifest.name ?? "", /^[a-z0-9][a-z0-9-]*$/);
  assert.doesNotMatch(manifest.name ?? "", /^@/);
  assert.deepEqual(manifest.categories, ["Programming Languages", "Linters"]);
});

test("extension package metadata and ignore rules are release-oriented", () => {
  const manifest = readManifest();
  const keywords = new Set(manifest.keywords ?? []);
  for (const expected of ["sage", "sagemath", "language-server", "lsp", "cython"]) {
    assert.ok(keywords.has(expected), `expected package keyword ${expected}`);
  }
  assert.equal(manifest.icon, "resources/branding/icon.png");
  assert.deepEqual(manifest.galleryBanner, { color: "#0b3438", theme: "dark" });
  assert.deepEqual(readPngDimensions(path.join(packageRoot, manifest.icon)), { width: 256, height: 256 });
  assert.equal(manifest.preview, true);
  assert.equal(manifest.qna, false);
  assert.deepEqual(manifest.extensionKind, ["workspace"]);
  assert.equal(manifest.license, "MIT");
  assert.notEqual(manifest.private, true);
  assert.equal(manifest.engines?.vscode, "^1.97.0");
  assert.equal(
    manifest.devDependencies?.["@types/vscode"],
    manifest.engines?.vscode?.slice(1),
    "VS Code API types must stay pinned to the declared minimum runtime version",
  );

  const packagedLicense = fs.readFileSync(path.join(packageRoot, "LICENSE"), "utf-8");
  assert.match(packagedLicense, /MIT License/);
  assert.match(packagedLicense, /Permission is hereby granted/);

  const packagedChangelog = fs.readFileSync(path.join(packageRoot, "CHANGELOG.md"), "utf-8");
  assert.match(packagedChangelog, new RegExp(`## ${manifest.version}`));
  assert.match(packagedChangelog, /preview/i);

  const vscodeIgnorePath = path.join(packageRoot, ".vscodeignore");
  const ignored = fs.readFileSync(vscodeIgnorePath, "utf-8");
  for (const expected of ["src/**", "test/**", "test-host/**", "out/test/**", "**/*.map"]) {
    assert.match(ignored, new RegExp(expected.replaceAll("*", "\\*")));
  }
  assert.ok(
    fs.existsSync(path.join(packageRoot, "resources", "bin", "README.md")),
    "expected packaged binary layout documentation",
  );
});

test("packaged README is user-facing and support-oriented", () => {
  const packagedReadme = fs.readFileSync(path.join(packageRoot, "README.md"), "utf-8");

  for (const expected of [
    "Sage VS Code Extension",
    "First Run",
    "Important Settings",
    "Troubleshooting",
    "Preview Limitations",
    "Support and Security",
    "Sage: Open Getting Started",
    "Sage: Select Interpreter",
    "Sage: Configure Workspace",
    "Sage: Show Environment Details",
    "Sage: Show Index Status",
    "Sage: Show Docs Status",
    "Sage: Find References",
    "Sage: Run UX Self Check",
    "Sage: Copy Support Bundle",
    "sage.analysis.enablePythonFiles",
    "sage.docs.preferredSource",
    "SUPPORT.md",
    "SECURITY.md",
  ]) {
    assert.match(packagedReadme, new RegExp(expected.replaceAll(".", "\\.")));
  }

  assert.doesNotMatch(packagedReadme, /Current Scope/);
  assert.doesNotMatch(packagedReadme, /\/Users\/(?!example\/|\.\.\.\/|<[^>]+>\/)[^/\s]+\//);
});

test("release scripts cover packaged Rust binaries and real Sage smoke gates", () => {
  const rootPackage = readRootPackage();
  assert.match(rootPackage.scripts?.["generate:icon"] ?? "", /scripts\/generate-extension-icon\.mjs/);
  assert.match(rootPackage.scripts?.["test:generated-assets"] ?? "", /sync-syntax-assets\.mjs --check/);
  assert.match(rootPackage.scripts?.["test:generated-assets"] ?? "", /generate-extension-icon\.mjs --check/);

  const packageRustBinary = rootPackage.scripts?.["package:rust-binary"] ?? "";
  assert.match(packageRustBinary, /npm run check:package-toolchain/);
  assert.match(packageRustBinary, /scripts\/build-packaged-rust-binary\.mjs/);

  const packagedRustBuilder = readRepositoryFile("scripts/build-packaged-rust-binary.mjs");
  assert.match(packagedRustBuilder, /"build", "--locked", "--release", "-p", "sage-ls"/);
  assert.match(packagedRustBuilder, /--remap-path-prefix/);
  assert.match(packagedRustBuilder, /CARGO_ENCODED_RUSTFLAGS/);
  assert.match(packagedRustBuilder, /path\.join\(__dirname, "package-rust-binary\.mjs"\)/);

  const packageVsix = rootPackage.scripts?.["package:vsix"] ?? "";
  assert.match(packageVsix, /npm run check:package-toolchain/);
  assert.match(packageVsix, /npm run build/);
  assert.match(packageVsix, /npm run test:generated-assets/);
  assert.match(packageVsix, /npm run package:rust-binary/);
  assert.match(packageVsix, /npm run test:vsix-contents/);
  assert.match(packageVsix, /scripts\/package-vsix\.mjs/);

  const releaseGate = rootPackage.scripts?.["test:release"] ?? "";
  for (const expected of [
    "cargo clippy --locked --all-targets --all-features -- -D warnings",
    "npm run test:lsp-protocol",
    "npm run test:generated-assets",
    "npm run package:rust-binary",
    "npm run test:vsix-contents",
    "npm run test:vsix-package",
    "npm run test:vsix-install",
    "npm run test:cache-maintenance",
    "npm run test:repo-hygiene",
    "npm run test:product-readiness",
    "npm run test:performance -- --skip-workbench",
    "npm run test:lsp-latency",
    "npm run test:real-file-smoke",
    "git diff --check",
  ]) {
    assert.match(releaseGate, new RegExp(expected.replaceAll("/", "\\/").replaceAll("+", "\\+")));
  }

  const fullGate = rootPackage.scripts?.["test:full"] ?? "";
  assert.match(fullGate, /npm run test:release/);
  assert.match(fullGate, /npm run test:extension-host/);
});

test("VSIX packaging enforces a reproducible toolchain, modes, and private-path checks", () => {
  const rootPackage = readRootPackage();
  assert.equal(rootPackage.scripts?.["check:package-toolchain"], "node scripts/check-package-toolchain.mjs");

  const toolchainCheck = readRepositoryFile("scripts/package-toolchain.mjs");
  assert.match(toolchainCheck, /process\.versions\.node/);
  assert.match(toolchainCheck, /actual !== expected/);
  assert.match(toolchainCheck, /packageManager/);
  assert.match(toolchainCheck, /requires npm/);

  const toolchainCommand = readRepositoryFile("scripts/check-package-toolchain.mjs");
  assert.match(toolchainCommand, /npmCommand/);
  assert.match(toolchainCommand, /assertPinnedNpmVersion/);

  const packager = readRepositoryFile("scripts/package-vsix.mjs");
  assert.match(packager, /assertPinnedNodeVersion\(repositoryRoot\)/);
  assert.match(packager, /assertNoBuildMachinePaths/);
  assert.match(packager, /0o100755/);
  assert.match(packager, /0o100644/);

  const packageSmoke = readRepositoryFile("scripts/vsix-package-smoke.mjs");
  assert.match(packageSmoke, /buildExtension\(0o022\)/);
  assert.match(packageSmoke, /buildExtension\(0o077\)/);
  assert.match(packageSmoke, /findBuildMachinePaths/);
  assert.match(packageSmoke, /excludes build-machine home and repository paths/);

  const iconGenerator = readRepositoryFile("scripts/generate-extension-icon.mjs");
  assert.match(iconGenerator, /import pako from "pako"/);
  assert.doesNotMatch(iconGenerator, /node:zlib/);
});

test("macOS CI gate is public-repository safe", () => {
  const rootPackage = readRootPackage();
  assert.equal(rootPackage.packageManager, "npm@11.17.0");
  assert.equal(readRepositoryFile(".node-version").trim(), "22.23.1");
  assert.match(readRepositoryFile("rust-toolchain.toml"), /channel\s*=\s*"1\.92\.0"/);
  const ciGate = rootPackage.scripts?.["test:ci"] ?? "";
  for (const expected of [
    "cargo test --locked",
    "cargo clippy --locked --all-targets --all-features -- -D warnings",
    "npm run lint",
    "npm run build",
    "npm run test",
    "npm run test:lsp-protocol",
    "npm run test:generated-assets",
    "npm run package:rust-binary",
    "npm run test:vsix-contents",
    "npm run test:vsix-package",
    "npm run test:cache-maintenance",
    "npm run test:repo-hygiene",
    "npm run test:performance -- --skip-workbench",
    "git diff --check",
  ]) {
    assert.match(ciGate, new RegExp(expected.replaceAll("/", "\\/").replaceAll("+", "\\+")));
  }

  for (const localOnly of ["test:lsp-latency", "test:real-file-smoke", "test:native-smoke", "test:extension-host"]) {
    assert.doesNotMatch(ciGate, new RegExp(localOnly.replaceAll(":", "\\:")));
  }

  const workflow = readCiWorkflow();
  assert.match(workflow, /runs-on:\s*macos-latest/);
  assert.match(workflow, /npm ci/);
  assert.match(workflow, /npm install --global npm@11\.17\.0/);
  assert.match(workflow, /node-version-file:\s*"\.node-version"/);
  assert.match(workflow, /cache:\s*["']?npm["']?/);
  assert.match(workflow, /cache-dependency-path:\s*package-lock\.json/);
  assert.match(workflow, /dtolnay\/rust-toolchain@stable/);
  assert.match(workflow, /toolchain:\s*"1\.92\.0"/);
  assert.match(workflow, /Swatinem\/rust-cache@v2/);
  assert.match(workflow, /cache-on-failure:\s*true/);
  assert.match(workflow, /cargo fetch --locked/);
  assert.match(workflow, /python -m pip install -e \.\/packages\/sage-lsp\[dev\]/);
  assert.match(workflow, /npm run test:ci/);
  assert.doesNotMatch(workflow, /npm run test:release/);
});

test("extension declares modern workspace capability limits", () => {
  const manifest = readManifest();
  assert.equal(manifest.capabilities?.untrustedWorkspaces?.supported, "limited");
  assert.match(manifest.capabilities?.untrustedWorkspaces?.description ?? "", /execute workspace code/);
  assert.equal(
    typeof manifest.capabilities?.virtualWorkspaces === "object"
      ? manifest.capabilities.virtualWorkspaces.supported
      : manifest.capabilities?.virtualWorkspaces,
    "limited",
  );
  assert.match(
    typeof manifest.capabilities?.virtualWorkspaces === "object"
      ? manifest.capabilities.virtualWorkspaces.description ?? ""
      : "",
    /local file workspace/,
  );

  const restrictedConfigurations = new Set(manifest.capabilities?.untrustedWorkspaces?.restrictedConfigurations ?? []);
  for (const expected of [
    "sage.interpreter.path",
    "sage.languageServer.rustPath",
    "sage.analysis.sourceRoots",
    "sage.analysis.enableRuntimeIntrospection",
    "sage.analysis.enablePythonFiles",
    "sage.run.target",
  ]) {
    assert.ok(restrictedConfigurations.has(expected), `missing restricted configuration ${expected}`);
  }
});

test("user commands are grouped, iconed, and exposed through contextual menus", () => {
  const manifest = readManifest();
  const commands = new Map((manifest.contributes?.commands ?? []).map((command) => [command.command, command]));
  for (const command of manifest.contributes?.commands ?? []) {
    assert.equal(command.category, "Sage", `${command.command} should use the Sage command category`);
    assert.match(command.icon ?? "", /^\$\([a-z0-9-]+\)$/i, `${command.command} should use a product icon`);
  }

  for (const command of [
    "sage.runCurrentFile",
    "sage.runSelection",
    "sage.runCurrentCell",
    "sage.showDocumentation",
    "sage.findReferences",
    "sage.runUxSelfCheck",
  ]) {
    assert.match(commands.get(command)?.enablement ?? "", /sage\.workspaceRuntimeAvailable/, `${command} should be runtime-gated`);
  }
  assert.equal(
    commands.get("sage.showDocumentation")?.enablement,
    "sage.workspaceRuntimeAvailable",
    "documentation command should stay available when focus is in output/status panels",
  );
  assert.equal(
    commands.get("sage.findReferences")?.enablement,
    "sage.workspaceRuntimeAvailable",
    "reference command should stay available when focus is in docs/output panels",
  );
  assert.equal(
    commands.get("sage.runUxSelfCheck")?.enablement,
    "sage.workspaceRuntimeAvailable",
    "UX self check should stay available when focus is in docs/output panels",
  );

  const menus = manifest.contributes?.menus ?? {};
  assert.ok(
    menus["editor/title/run"]?.some((item) => item.command === "sage.runCurrentFile" && item.when?.includes("resourceLangId == sagemath")),
    "missing Sage run button in the editor title run menu",
  );
  assert.ok(
    menus["editor/context"]?.some((item) => item.when?.includes("resourceLangId == python && config.sage.analysis.enablePythonFiles")),
    "missing Sage-aware Python context menu gating",
  );
  assert.ok(
    menus["editor/context"]?.some((item) => item.command === "sage.runSelection"),
    "missing Sage selection action in the editor context menu",
  );
  assert.ok(
    menus["editor/context"]?.some((item) => item.command === "sage.runCurrentCell"),
    "missing Sage cell action in the editor context menu",
  );
  assert.ok(
    menus["editor/context"]?.some((item) => item.command === "sage.findReferences"),
    "missing Sage references action in the editor context menu",
  );
  assert.ok(
    menus.commandPalette?.some((item) => item.command === "sage.showDocumentation" && item.when === "sage.workspaceRuntimeAvailable"),
    "documentation command should remain discoverable whenever the Sage runtime is available",
  );
  assert.ok(
    menus.commandPalette?.some((item) => item.command === "sage.findReferences" && item.when === "sage.workspaceRuntimeAvailable"),
    "reference command should remain discoverable whenever the Sage runtime is available",
  );
  assert.ok(
    menus.commandPalette?.some((item) => item.command === "sage.runUxSelfCheck" && item.when === "sage.workspaceRuntimeAvailable"),
    "UX self check should remain discoverable whenever the Sage runtime is available",
  );
});

test("extension scopes direct navigation bridges to read-only external Sage sources", () => {
  const extensionSource = fs.readFileSync(path.join(packageRoot, "src", "extension.ts"), "utf8");
  const navigationSource = fs.readFileSync(path.join(packageRoot, "src", "externalSourceNavigation.ts"), "utf8");
  const languageFeaturesSource = fs.readFileSync(
    path.join(packageRoot, "src", "externalSourceLanguageFeatures.ts"),
    "utf8",
  );
  const clientSource = fs.readFileSync(path.join(packageRoot, "src", "languageClient.ts"), "utf8");

  assert.match(
    navigationSource,
    /registerDefinitionProvider\(selector/,
    "read-only sage-source documents need a file-URI bridge for follow-up definitions",
  );
  assert.doesNotMatch(
    navigationSource,
    /registerDefinitionProvider\([^)]*scheme: "file"/s,
    "normal file definitions should stay owned by the Rust LSP client",
  );
  assert.match(clientSource, /provideDefinition/);
  assert.match(clientSource, /provideImplementation/);
  assert.match(clientSource, /provideTypeDefinition/);
  assert.match(clientSource, /provideReferences/);
  assert.match(clientSource, /rewriteExternalDefinitionUris/);
  assert.match(clientSource, /buildSageSourceUri/);
  assert.match(
    clientSource,
    /\*\.\{sage,py,pyx,pxd,pxi,spyx\}/,
    "the LSP watcher should cover every Sage and Sage-Cython file with one glob",
  );
  assert.match(
    navigationSource,
    /registerReferenceProvider/,
    "external Sage source files need a VS Code reference bridge after definition jumps",
  );
  assert.doesNotMatch(
    navigationSource,
    /scheme: "file", language: "python"/,
    "the external bridge must not duplicate the LanguageClient Python reference provider",
  );
  assert.match(navigationSource, /isExternalSageSourceDocument/);
  assert.doesNotMatch(
    navigationSource,
    /openTextDocument\(sourceUri\)/,
    "the bridge must not create a stale hidden file document for external source navigation",
  );
  assert.match(
    navigationSource,
    /externalSourceTextIsCurrent/,
    "the bridge should reject stale visible positions before requesting follow-up navigation",
  );
  assert.match(
    navigationSource,
    /textDocument: \{ uri: sourceUri\.toString\(\) \}/,
    "the LSP request must use the backing file URI rather than the read-only view URI",
  );
  assert.match(navigationSource, /rewriteExternalDefinitionUris/);
  for (const provider of [
    "registerHoverProvider",
    "registerSignatureHelpProvider",
    "registerDocumentLinkProvider",
    "registerCallHierarchyProvider",
  ]) {
    assert.match(
      languageFeaturesSource,
      new RegExp(`${provider}\\(\\s*selector`),
      `${provider} should bridge the read-only external source view`,
    );
  }
  assert.match(
    languageFeaturesSource,
    /const selector: vscode\.DocumentSelector = \[\{ scheme: SAGE_SOURCE_SCHEME \}\]/,
    "follow-up language providers must stay scoped to sage-source documents",
  );
  assert.doesNotMatch(
    languageFeaturesSource,
    /openTextDocument/,
    "follow-up language requests must not open a hidden backing file document",
  );
  assert.match(languageFeaturesSource, /HoverRequest\.method/);
  assert.match(languageFeaturesSource, /SignatureHelpRequest\.method/);
  assert.match(languageFeaturesSource, /DocumentLinkRequest\.method/);
  assert.match(languageFeaturesSource, /CallHierarchyPrepareRequest\.method/);
  assert.match(languageFeaturesSource, /CallHierarchyIncomingCallsRequest\.method/);
  assert.match(languageFeaturesSource, /CallHierarchyOutgoingCallsRequest\.method/);
  assert.match(
    languageFeaturesSource,
    /textIsCurrent\(document, sourceUri\)/,
    "external follow-up requests should reject positions from stale visible text",
  );
  assert.match(
    languageFeaturesSource,
    /protocolItemWithUri\(converted, backingUri\.toString\(\)\)/,
    "call hierarchy expansion must restore sage-source items to backing file URIs",
  );
  assert.match(
    languageFeaturesSource,
    /rewriteCallHierarchyItems/,
    "call hierarchy results should map external file items back to sage-source",
  );
  assert.match(
    languageFeaturesSource,
    /link\.target = rewriteExternalSourceUri/,
    "external document-link targets should keep the read-only URI identity",
  );
  assert.match(extensionSource, /effectiveSourceRootPaths/);
  assert.match(extensionSource, /source_root_fingerprints/);
  assert.match(
    clientSource,
    /fileEvents: lifecycle\.fileSystemWatcher/,
    "language-client restarts should reuse the extension-lifetime file watcher",
  );
  assert.doesNotMatch(
    clientSource,
    /createFileSystemWatcher/,
    "individual language clients must not allocate undisposed file watchers",
  );
  assert.match(
    extensionSource,
    /continuing automatic refresh/,
    "cold indexes should keep refreshing status until the server reports idle",
  );
  assert.doesNotMatch(extensionSource, /pausing automatic refresh/);
  assert.match(
    extensionSource,
    /extensionMode !== vscode\.ExtensionMode\.Production/,
    "internal test commands must not be registered in production",
  );
});

test("extension activation stays a compact orchestrator with focused feature modules", () => {
  const extensionSource = fs.readFileSync(path.join(packageRoot, "src", "extension.ts"), "utf8");
  assert.ok(
    extensionSource.split(/\r?\n/).length <= 1300,
    "extension.ts should delegate feature implementations instead of growing another monolith",
  );
  assert.match(
    extensionSource,
    /client === nextClient[\s\S]*!languageClientManagedShutdown[\s\S]*!extensionDeactivating/,
    "only the installed language client may auto-restart outside managed shutdown",
  );
  for (const moduleName of [
    "executionCommands.ts",
    "externalSourceNavigation.ts",
    "navigationCommands.ts",
    "sourceRootPaths.ts",
    "statusCommands.ts",
    "statusRefreshController.ts",
    "workspaceScope.ts",
  ]) {
    assert.equal(
      fs.existsSync(path.join(packageRoot, "src", moduleName)),
      true,
      `missing focused extension module ${moduleName}`,
    );
  }
});

test("extension contributions are activation-covered and backed by generated assets", () => {
  const manifest = readManifest();
  const activationEvents = new Set(manifest.activationEvents ?? []);
  const languages = manifest.contributes?.languages ?? [];
  const grammars = new Set((manifest.contributes?.grammars ?? []).map((grammar) => grammar.language));
  const snippets = new Set((manifest.contributes?.snippets ?? []).map((snippet) => snippet.language));

  assert.ok(
    !activationEvents.has("onStartupFinished"),
    "the extension should not activate every VS Code window at startup",
  );
  assert.ok(
    activationEvents.has("workspaceContains:**/*.sage.py"),
    "missing activation event for Sage-heavy Python workspaces",
  );
  assert.ok(
    activationEvents.has("workspaceContains:**/*sage*.py"),
    "missing activation event for Sage-named Python research files",
  );
  assert.ok(
    activationEvents.has("workspaceContains:**/*.spyx"),
    "missing activation event for Sage Cython .spyx workspaces",
  );
  assert.ok(
    activationEvents.has("onLanguage:python"),
    "missing opt-in activation path for ordinary Python Sage workspaces",
  );

  for (const language of languages) {
    assert.ok(activationEvents.has(`onLanguage:${language.id}`), `missing activation event for ${language.id}`);
    assert.ok(grammars.has(language.id), `missing grammar contribution for ${language.id}`);
    assert.ok(snippets.has(language.id), `missing snippet contribution for ${language.id}`);
    if (language.configuration) {
      assert.ok(fs.existsSync(path.resolve(packageRoot, language.configuration)), `missing language configuration ${language.configuration}`);
    }
  }

  for (const grammar of manifest.contributes?.grammars ?? []) {
    assert.ok(fs.existsSync(path.resolve(packageRoot, grammar.path)), `missing grammar file ${grammar.path}`);
  }
  for (const snippet of manifest.contributes?.snippets ?? []) {
    assert.ok(fs.existsSync(path.resolve(packageRoot, snippet.path)), `missing snippet file ${snippet.path}`);
  }
  for (const command of manifest.contributes?.commands ?? []) {
    assert.ok(activationEvents.has(`onCommand:${command.command}`), `missing activation event for ${command.command}`);
  }
});

test("getting started walkthrough covers first-run Sage setup", () => {
  const manifest = readManifest();
  const walkthrough = manifest.contributes?.walkthroughs?.find((item) => item.id === "gettingStarted");
  assert.ok(walkthrough, "missing Sage getting started walkthrough");
  assert.match(walkthrough.title, /Sage/);

  const stepIds = new Set(walkthrough.steps.map((step) => step.id));
  for (const expected of ["selectInterpreter", "configureWorkspace", "inspectIndex", "validateEditLoop"]) {
    assert.ok(stepIds.has(expected), `missing walkthrough step ${expected}`);
  }

  const completionEvents = new Set(walkthrough.steps.flatMap((step) => step.completionEvents ?? []));
  for (const expected of [
    "onCommand:sage.selectInterpreter",
    "onCommand:sage.configureWorkspace",
    "onCommand:sage.showEnvironmentDetails",
    "onCommand:sage.showIndexStatus",
    "onCommand:sage.showDocsStatus",
    "onCommand:sage.copySupportBundle",
    "onCommand:sage.runUxSelfCheck",
  ]) {
    assert.ok(completionEvents.has(expected), `missing walkthrough completion event ${expected}`);
  }

  for (const step of walkthrough.steps) {
    assert.ok(step.media?.markdown, `walkthrough step ${step.id} should use markdown media`);
    assert.ok(
      fs.existsSync(path.resolve(packageRoot, step.media.markdown)),
      `missing walkthrough media ${step.media.markdown}`,
    );
  }
});

test("user docs cover every command and setting", () => {
  const manifest = readManifest();
  const userDocs = [
    fs.readFileSync(path.join(repositoryRoot, "README.md"), "utf-8"),
    fs.readFileSync(path.join(repositoryRoot, "docs", "install-and-configure.md"), "utf-8"),
    fs.readFileSync(path.join(repositoryRoot, "docs", "plugin-completeness.md"), "utf-8"),
  ].join("\n");

  for (const command of manifest.contributes?.commands ?? []) {
    assert.match(userDocs, new RegExp(command.command.replaceAll(".", "\\.")), `user docs missing ${command.command}`);
  }

  for (const setting of Object.keys(manifest.contributes?.configuration?.properties ?? {})) {
    assert.match(userDocs, new RegExp(setting.replaceAll(".", "\\.")), `user docs missing ${setting}`);
  }
});
