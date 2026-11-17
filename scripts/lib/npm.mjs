import { spawnSync } from "node:child_process";

export const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";

/**
 * Runs npm synchronously. On Windows npm is a `.cmd` shim, which Node only
 * spawns through a shell, so arguments must not need shell quoting.
 */
export function spawnNpm(args, options = {}) {
  return spawnSync(npmCommand, args, {
    encoding: "utf8",
    shell: process.platform === "win32",
    ...options,
  });
}
