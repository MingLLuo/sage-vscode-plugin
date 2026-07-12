import fs from "node:fs";
import path from "node:path";

export function pinnedNodeVersion(repositoryRoot) {
  const versionFile = path.join(repositoryRoot, ".node-version");
  const version = fs.readFileSync(versionFile, "utf8").trim();
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Expected an exact Node version in ${versionFile}, got ${JSON.stringify(version)}`);
  }
  return version;
}

export function assertPinnedNodeVersion(repositoryRoot, actual = process.versions.node) {
  const expected = pinnedNodeVersion(repositoryRoot);
  if (actual !== expected) {
    throw new Error(
      `VSIX packaging requires Node ${expected} from .node-version; current runtime is ${actual}. `
      + "Switch Node versions before building a release artifact.",
    );
  }
  return expected;
}
