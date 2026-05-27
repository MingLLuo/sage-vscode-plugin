import test from "node:test";
import assert from "node:assert/strict";
import type * as vscode from "vscode";

import { createOutputLogger, formatLogLine } from "../src/extensionLogger";

test("formatLogLine uses structured level component and key-value fields", () => {
  assert.equal(
    formatLogLine("info", "extension", "started", { launchCount: 2, path: "/tmp/sage env" }),
    '[info] [extension] started launchCount=2 path="/tmp/sage env"',
  );
});

test("OutputLogger filters messages below configured verbosity", () => {
  const lines: string[] = [];
  const outputChannel = {
    appendLine(value: string) {
      lines.push(value);
    },
  } as vscode.OutputChannel;

  const logger = createOutputLogger(outputChannel, "warn");
  logger.info("extension", "hidden");
  logger.warn("extension", "shown");
  logger.setLevel("debug");
  logger.debug("extension", "debug-visible", { request: "hover" });

  assert.deepEqual(lines, [
    "[warn] [extension] shown",
    "[debug] [extension] debug-visible request=hover",
  ]);
});
