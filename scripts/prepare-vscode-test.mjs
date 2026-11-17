#!/usr/bin/env node
import fs from "node:fs";
import { downloadAndUnzipVSCode, resolveCliPathFromVSCodeExecutablePath } from "@vscode/test-electron";

// CI uses an isolated editor download rather than depending on runner-installed applications.
const executable = await downloadAndUnzipVSCode();
const cli = resolveCliPathFromVSCodeExecutablePath(executable);
const settings = { SAGE_TEST_VSCODE_EXECUTABLE: executable, SAGE_CODE_CLI: cli, SAGE_REQUIRE_CODE_CLI: "1" };
if (process.env.GITHUB_ENV) {
  fs.appendFileSync(process.env.GITHUB_ENV, Object.entries(settings).map(([key, value]) => `${key}=${value}\n`).join(""));
}
console.log(JSON.stringify(settings, null, 2));
