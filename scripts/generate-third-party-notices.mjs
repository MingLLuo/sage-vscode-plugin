#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { selectedPackageTarget, stagedPaths } from "./lib/package-targets.mjs";
import { generateThirdPartyNotices } from "./lib/third-party-notices.mjs";

// Writes the third-party notices for the Rust crates linked into `sage-ls`
// next to the staged binary. `package-vsix.mjs` ships that file as
// `extension/THIRD_PARTY_NOTICES.txt`.
//
// Usage: node scripts/generate-third-party-notices.mjs [--target <vscode-target>] [--out <file>] [--stdout]
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const argv = process.argv.slice(2);

let target;
let notices;
try {
  target = selectedPackageTarget(argv);
  notices = generateThirdPartyNotices({ repositoryRoot, rustTarget: target.rustTarget });
} catch (error) {
  console.error(error.message);
  process.exit(1);
}

if (argv.includes("--stdout")) {
  process.stdout.write(notices.text);
} else {
  const output = path.resolve(argumentValue("--out") ?? stagedPaths(repositoryRoot, target).notices);
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, notices.text, "utf8");
  console.log(JSON.stringify({
    status: "generated",
    target: target.name,
    rustTarget: target.rustTarget,
    output: path.relative(repositoryRoot, output).split(path.sep).join("/"),
    crates: notices.crates.length,
    licenseTexts: notices.licenseTexts,
    cratesWithoutLicenseText: notices.cratesWithoutLicenseText,
  }, null, 2));
}

function argumentValue(name) {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : undefined;
}
