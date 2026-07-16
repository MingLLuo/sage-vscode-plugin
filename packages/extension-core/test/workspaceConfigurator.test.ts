import test from "node:test";
import assert from "node:assert/strict";

import {
  buildWorkspaceConfigurationUpdates,
  recommendedWorkspaceProfile,
  WORKSPACE_CONFIGURATION_PROFILES,
} from "../src/workspaceConfigurator";

test("recommendedWorkspaceProfile follows the active editor language", () => {
  assert.equal(recommendedWorkspaceProfile("python").id, "python");
  assert.equal(recommendedWorkspaceProfile("sagemath-cython").id, "native");
  assert.equal(recommendedWorkspaceProfile("sagemath").id, "standard");
});

test("buildWorkspaceConfigurationUpdates writes a conservative Sage-heavy Python profile", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "python");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: [
      "/workspace/project",
      "/workspace/project/src",
      "/opt/sage/src",
    ],
    profile,
  });
  const bySection = new Map(updates.map((update) => [update.section, update.value]));

  assert.equal(bySection.get("languageServer.rustPath"), "auto");
  assert.equal(bySection.get("analysis.mode"), "full");
  assert.equal(bySection.get("analysis.enablePythonFiles"), true);
  assert.equal(bySection.get("analysis.enablePyxParsing"), true);
  assert.deepEqual(bySection.get("analysis.sourceRoots"), [".", "src", "/opt/sage/src"]);
  assert.deepEqual(
    updates.find((update) => !update.namespace && update.section === "analysis.extraPaths")?.value,
    [".", "src", "/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.extraPaths")?.value,
    [".", "src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.diagnosticSeverityOverrides")?.value,
    {
      reportMissingImports: "none",
      reportMissingModuleSource: "none",
    },
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.exclude")?.value,
    ["/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.ignore")?.value,
    ["/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "ruff" && update.section === "exclude")?.value,
    ["/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "ruff" && update.section === "configuration")?.value,
    {
      exclude: ["/opt/sage/src"],
      "force-exclude": true,
    },
  );
  assert.deepEqual(
    bySection.get("indexing.exclude"),
    [
      "**/.git/**",
      "**/__pycache__/**",
      "**/.venv/**",
      "**/.ruff_cache/**",
      "**/.quarto/**",
      "**/.quarto-cache/**",
      "**/.quarto-deno/**",
      "**/.quarto-home/**",
      "**/build/**",
      "**/tmp/**",
    ],
  );
});

test("buildWorkspaceConfigurationUpdates preserves existing extra paths", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: [
      "/workspace/project/src",
      "/opt/sage/src",
    ],
    configuredExtraPaths: [
      "vendor",
      "/workspace/project/src",
      "/opt/custom/sage-stubs",
    ],
    profile,
  });
  const bySection = new Map(updates.map((update) => [update.section, update.value]));

  assert.deepEqual(bySection.get("analysis.sourceRoots"), ["src", "/opt/sage/src"]);
  assert.deepEqual(
    updates.find((update) => !update.namespace && update.section === "analysis.extraPaths")?.value,
    ["src", "/opt/sage/src", "vendor", "/opt/custom/sage-stubs"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.extraPaths")?.value,
    ["src", "vendor", "/opt/custom/sage-stubs"],
  );
});

test("buildWorkspaceConfigurationUpdates merges existing Python and Ruff settings", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: ["/workspace/project/src", "/opt/sage/src"],
    configuredPythonExtraPaths: ["python-stubs"],
    configuredPythonDiagnosticSeverityOverrides: {
      reportUnusedImport: "warning",
      reportMissingImports: "error",
    },
    configuredPythonExclude: ["generated"],
    configuredPythonIgnore: ["legacy"],
    configuredRuffExclude: ["dist"],
    profile,
  });

  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.extraPaths")?.value,
    ["src", "python-stubs"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.diagnosticSeverityOverrides")?.value,
    {
      reportUnusedImport: "warning",
      reportMissingImports: "none",
      reportMissingModuleSource: "none",
    },
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.exclude")?.value,
    ["generated", "/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.ignore")?.value,
    ["legacy", "/opt/sage/src"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "ruff" && update.section === "exclude")?.value,
    ["dist", "/opt/sage/src"],
  );
});

test("buildWorkspaceConfigurationUpdates keeps external Sage roots out of Pylance paths", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: [
      "/workspace/project/src",
      "/opt/sage/src",
    ],
    configuredExtraPaths: [
      "/opt/sage/src",
      "/opt/custom/sage-stubs",
    ],
    profile,
  });

  assert.deepEqual(
    updates.find((update) => update.section === "analysis.extraPaths")?.value,
    ["src", "/opt/sage/src", "/opt/custom/sage-stubs"],
  );
  assert.deepEqual(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.extraPaths")?.value,
    ["src", "/opt/custom/sage-stubs"],
  );
});

test("buildWorkspaceConfigurationUpdates does not exclude Sage roots inside the workspace", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/sage/src"],
    discoveredSourceRoots: ["/workspace/sage/src"],
    profile,
  });

  assert.equal(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.exclude"),
    undefined,
  );
  assert.equal(
    updates.find((update) => update.namespace === "python" && update.section === "analysis.ignore"),
    undefined,
  );
  assert.equal(
    updates.find((update) => update.namespace === "ruff" && update.section === "exclude"),
    undefined,
  );
  assert.equal(
    updates.find((update) => update.namespace === "ruff" && update.section === "configuration"),
    undefined,
  );
});

test("buildWorkspaceConfigurationUpdates merges inline Ruff configuration for external Sage roots", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: ["/opt/sage/src"],
    configuredRuffConfiguration: {
      lineLength: 100,
      exclude: ["build"],
    },
    profile,
  });

  assert.deepEqual(
    updates.find((update) => update.namespace === "ruff" && update.section === "configuration")?.value,
    {
      lineLength: 100,
      exclude: ["build", "/opt/sage/src"],
      "force-exclude": true,
    },
  );
});

test("buildWorkspaceConfigurationUpdates preserves file-based Ruff configuration", () => {
  const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === "research");
  assert.ok(profile);

  const updates = buildWorkspaceConfigurationUpdates({
    workspaceFolders: ["/workspace/project"],
    discoveredSourceRoots: ["/opt/sage/src"],
    configuredRuffConfiguration: "/workspace/project/ruff.toml",
    profile,
  });

  assert.equal(
    updates.find((update) => update.namespace === "ruff" && update.section === "configuration"),
    undefined,
  );
});
