import fs from "node:fs";
import path from "node:path";
import semver from "semver";

export function supportedNodeVersionRange(repositoryRoot) {
  const packageJsonPath = path.join(repositoryRoot, "package.json");
  const packageJson = readPackageJson(packageJsonPath);
  return declaredMinimumRange(
    packageJson.engines?.node,
    `${packageJsonPath}#engines.node`,
  ).range;
}

export function assertSupportedNodeVersion(repositoryRoot, actual = process.versions.node) {
  const packageJsonPath = path.join(repositoryRoot, "package.json");
  const packageJson = readPackageJson(packageJsonPath);
  const supported = declaredMinimumRange(
    packageJson.engines?.node,
    `${packageJsonPath}#engines.node`,
  );
  return assertAtLeast(
    "Node",
    actual,
    supported,
    "Install a supported Node release before building a VSIX artifact.",
  );
}

export function supportedNpmVersionRange(repositoryRoot) {
  return supportedNpmVersion(repositoryRoot).range;
}

export function assertSupportedNpmVersion(repositoryRoot, actual) {
  return assertAtLeast(
    "npm",
    actual,
    supportedNpmVersion(repositoryRoot),
    "Install a supported npm release before building a VSIX artifact.",
  );
}

export function assertNpmSupportsNodeVersion(
  nodeVersion,
  npmVersion,
  npmNodeRange,
  source = "installed npm package#engines.node",
) {
  const node = semanticVersion(nodeVersion, "Node runtime");
  const npm = semanticVersion(npmVersion, "npm runtime");
  const range = typeof npmNodeRange === "string" ? npmNodeRange.trim() : "";
  if (semver.validRange(range) === null) {
    throw new Error(
      `Expected a valid Node range from ${source}, got ${JSON.stringify(npmNodeRange)}.`,
    );
  }
  if (!semver.satisfies(node.normalized, range)) {
    throw new Error(
      `npm ${npm.normalized} requires Node ${range} from ${source}; `
      + `current Node runtime is ${node.normalized}. Select a compatible Node/npm pair.`,
    );
  }
  return {
    nodeVersion: node.normalized,
    npmVersion: npm.normalized,
    npmNodeRange: range,
  };
}

function supportedNpmVersion(repositoryRoot) {
  const packageJsonPath = path.join(repositoryRoot, "package.json");
  const packageJson = readPackageJson(packageJsonPath);
  return declaredMinimumRange(
    packageJson.engines?.npm,
    `${packageJsonPath}#engines.npm`,
  );
}

function readPackageJson(packageJsonPath) {
  return JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
}

function declaredMinimumRange(value, source) {
  const range = typeof value === "string" ? value.trim() : "";
  const match = /^>=\s*(\d+)(?:\.(\d+))?(?:\.(\d+))?$/.exec(range);
  if (!match) {
    throw new Error(
      `Expected ${source} to be a minimum-version range such as ">=22", `
      + `got ${JSON.stringify(value)}.`,
    );
  }
  const minimum = semanticVersion(
    [match[1], match[2] ?? "0", match[3] ?? "0"].join("."),
    source,
  );
  return { range, minimum, source };
}

function assertAtLeast(toolName, actual, supported, remedy) {
  const current = semanticVersion(actual, `${toolName} runtime`);
  if (compareSemanticVersions(current, supported.minimum) < 0) {
    throw new Error(
      `VSIX packaging requires ${toolName} ${supported.range} from ${supported.source}; `
      + `current runtime is ${current.normalized}. ${remedy}`,
    );
  }
  return current.normalized;
}

function semanticVersion(value, source) {
  const text = typeof value === "string" ? value.trim() : "";
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(text);
  if (!match) {
    throw new Error(
      `Expected a stable semantic version such as "22.0.0" for ${source}, `
      + `got ${JSON.stringify(value)}.`,
    );
  }
  const parts = match.slice(1).map(Number);
  return {
    parts,
    normalized: parts.join("."),
  };
}

function compareSemanticVersions(left, right) {
  for (let index = 0; index < left.parts.length; index += 1) {
    const difference = left.parts[index] - right.parts[index];
    if (difference !== 0) {
      return difference;
    }
  }
  return 0;
}
