import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

// Licenses that may be statically linked into the distributed `sage-ls`
// binary. A crate is accepted when its SPDX expression can be satisfied using
// only these identifiers; anything else fails generation so a new copyleft or
// unknown license is a deliberate decision, not a silent addition.
export const ACCEPTED_LICENSES = new Set([
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "BSL-1.0",
  "CC0-1.0",
  "ISC",
  "MIT",
  "MIT-0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "Unlicense",
  "Zlib",
]);
export const ACCEPTED_LICENSE_EXCEPTIONS = new Set(["LLVM-exception"]);

const LICENSE_FILE_PATTERN = /^(?:licen[cs]e|copying|copyright|notice|unlicense)(?:[._-].*)?$/i;
const RULE = "-".repeat(80);

/**
 * Evaluates an SPDX license expression (including the legacy `A/B` form)
 * against the accepted set.
 */
export function evaluateLicenseExpression(expression) {
  if (typeof expression !== "string" || !expression.trim()) {
    return { accepted: false, unaccepted: ["(no license declared)"] };
  }
  const tokens = expression
    .replaceAll("/", " OR ")
    .replaceAll("(", " ( ")
    .replaceAll(")", " ) ")
    .split(/\s+/)
    .filter(Boolean);
  const unaccepted = new Set();
  let position = 0;

  const parseExpression = () => {
    let accepted = parseTerm();
    while (tokens[position]?.toUpperCase() === "OR") {
      position += 1;
      accepted = parseTerm() || accepted;
    }
    return accepted;
  };
  const parseTerm = () => {
    let accepted = parseFactor();
    while (tokens[position]?.toUpperCase() === "AND") {
      position += 1;
      accepted = parseFactor() && accepted;
    }
    return accepted;
  };
  const parseFactor = () => {
    const token = tokens[position];
    if (token === undefined) {
      throw new Error(`unexpected end of license expression: ${expression}`);
    }
    position += 1;
    if (token === "(") {
      const accepted = parseExpression();
      if (tokens[position] !== ")") {
        throw new Error(`unbalanced parentheses in license expression: ${expression}`);
      }
      position += 1;
      return accepted;
    }
    const identifier = token.replace(/\+$/, "");
    let accepted = ACCEPTED_LICENSES.has(identifier);
    let label = identifier;
    if (tokens[position]?.toUpperCase() === "WITH") {
      const exception = tokens[position + 1];
      position += 2;
      label = `${identifier} WITH ${exception}`;
      accepted = accepted && ACCEPTED_LICENSE_EXCEPTIONS.has(exception);
    }
    if (!accepted) {
      unaccepted.add(label);
    }
    return accepted;
  };

  const accepted = parseExpression();
  if (position !== tokens.length) {
    throw new Error(`could not parse license expression: ${expression}`);
  }
  return { accepted, unaccepted: accepted ? [] : [...unaccepted].sort() };
}

/**
 * Returns the third-party crates linked into `rootName` according to
 * `cargo metadata` output: the closure over normal dependencies, without dev-
 * and build-only dependencies and without workspace members.
 */
export function bundledCrates(metadata, rootName = "sage-ls") {
  const packages = new Map(metadata.packages.map((entry) => [entry.id, entry]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const workspaceMembers = new Set(metadata.workspace_members);
  const root = metadata.packages.find((entry) => entry.name === rootName && workspaceMembers.has(entry.id));
  if (!root) {
    throw new Error(`workspace package ${rootName} is missing from cargo metadata`);
  }
  const seen = new Set([root.id]);
  const queue = [root.id];
  for (let index = 0; index < queue.length; index += 1) {
    const node = nodes.get(queue[index]);
    for (const dependency of node?.deps ?? []) {
      const linked = (dependency.dep_kinds ?? [{ kind: null }]).some((kind) => kind.kind === null);
      if (linked && !seen.has(dependency.pkg)) {
        seen.add(dependency.pkg);
        queue.push(dependency.pkg);
      }
    }
  }
  return [...seen]
    .filter((id) => !workspaceMembers.has(id))
    .map((id) => packages.get(id))
    .sort((left, right) => compareText(left.name, right.name) || compareText(left.version, right.version));
}

/** Reads the license, notice, and copyright files shipped at the root of a crate package. */
export function readLicenseFiles(crate) {
  const directory = path.dirname(crate.manifest_path);
  const names = new Set();
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (entry.isFile() && LICENSE_FILE_PATTERN.test(entry.name)) {
      names.add(entry.name);
    }
  }
  if (crate.license_file && fs.existsSync(path.join(directory, crate.license_file))) {
    names.add(crate.license_file.split(path.sep).join("/"));
  }
  return [...names].sort(compareText).map((name) => ({
    name,
    text: normalizeText(fs.readFileSync(path.join(directory, name), "utf8")),
  })).filter((file) => file.text.length > 0);
}

/**
 * Renders the notices document. `crates` entries are
 * `{ name, version, license, repository, licenseFiles: [{ name, text }] }`.
 * Output depends only on its inputs, so the same lock file yields the same bytes.
 */
export function renderNotices({ rustTarget, crates }) {
  const lines = [
    "THIRD-PARTY SOFTWARE NOTICES",
    "",
    "This extension bundles the sage-ls language server, a native executable that",
    `statically links the third-party Rust crates listed below (Rust target: ${rustTarget}).`,
    "Each crate is distributed under the license it declares. The license, notice,",
    "and copyright files shipped with those crates are reproduced after the list.",
    "JavaScript dependencies under node_modules/ carry their own license files.",
    "",
    "Generated by scripts/generate-third-party-notices.mjs from Cargo.lock. Do not edit.",
    "",
    `CRATES (${crates.length})`,
    RULE,
  ];
  for (const crate of crates) {
    lines.push(
      `${crate.name} ${crate.version} | ${crate.license ?? "see license file"}`
      + (crate.repository ? ` | ${crate.repository}` : ""),
    );
  }

  const withoutText = crates.filter((crate) => crate.licenseFiles.length === 0);
  if (withoutText.length > 0) {
    lines.push(
      "",
      "CRATES THAT SHIP NO LICENSE FILE",
      RULE,
      "These crates declare the license above but do not include its text in their",
      "published package. The standard text for each SPDX identifier is available at",
      "https://spdx.org/licenses/.",
      "",
    );
    for (const crate of withoutText) {
      lines.push(`${crate.name} ${crate.version} | ${crate.license ?? "unknown"}`);
    }
  }

  lines.push("", "LICENSE TEXTS", RULE);
  for (const group of groupLicenseTexts(crates)) {
    lines.push("", RULE, "Used by:");
    for (const user of group.users) {
      lines.push(`  - ${user}`);
    }
    lines.push(RULE, "", group.text);
  }
  return `${lines.join("\n")}\n`;
}

/** Identical license texts are printed once and list every crate that ships them. */
export function groupLicenseTexts(crates) {
  const groups = new Map();
  for (const crate of crates) {
    for (const file of crate.licenseFiles) {
      const key = crypto.createHash("sha256").update(file.text).digest("hex");
      const group = groups.get(key) ?? { text: file.text, users: [] };
      group.users.push(`${crate.name} ${crate.version} (${file.name})`);
      groups.set(key, group);
    }
  }
  return [...groups.values()];
}

/**
 * Runs `cargo metadata` for one Rust target and builds the notices document.
 * Throws when a bundled crate's license is outside the accepted set.
 */
export function generateThirdPartyNotices({ repositoryRoot, rustTarget, rootName = "sage-ls" }) {
  const result = spawnSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", "--filter-platform", rustTarget],
    { cwd: repositoryRoot, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  if (result.error || result.status !== 0) {
    throw new Error(
      `cargo metadata failed: ${result.error?.message || (result.stderr ?? "").trim() || `exit ${result.status}`}`,
    );
  }
  const crates = bundledCrates(JSON.parse(result.stdout), rootName).map((crate) => ({
    name: crate.name,
    version: crate.version,
    license: crate.license ?? null,
    repository: crate.repository ?? null,
    licenseFiles: readLicenseFiles(crate),
  }));

  const rejected = crates
    .map((crate) => ({ crate, evaluation: evaluateLicenseExpression(crate.license) }))
    .filter(({ evaluation }) => !evaluation.accepted)
    .map(({ crate, evaluation }) => (
      `${crate.name} ${crate.version}: ${crate.license ?? "no license declared"}`
      + ` (not accepted: ${evaluation.unaccepted.join(", ")})`
    ));
  if (rejected.length > 0) {
    throw new Error(
      "Bundled crates use licenses outside the accepted set in scripts/lib/third-party-notices.mjs:\n"
      + rejected.map((line) => `  - ${line}`).join("\n"),
    );
  }

  return {
    text: renderNotices({ rustTarget, crates }),
    crates,
    licenseTexts: groupLicenseTexts(crates).length,
    cratesWithoutLicenseText: crates
      .filter((crate) => crate.licenseFiles.length === 0)
      .map((crate) => `${crate.name} ${crate.version}`),
  };
}

function normalizeText(text) {
  return text.replace(/^﻿/, "").replace(/\r\n?/g, "\n").replace(/[ \t]+$/gm, "").trim();
}

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}
