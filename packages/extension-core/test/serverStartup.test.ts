import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { verifyLanguageServerLaunch, LanguageServerLaunchError } from "../src/serverStartup";
import { readProcessProbe, ProbeOutputLimitError } from "../src/processProbe";
import { settleAndStopLanguageClient } from "../src/languageClientShutdown";

const program = (code: string) => ({ command: process.execPath, args: ["-e", code, "--"] });

test("startup handshake accepts sage-ls and rejects other executables or nonzero exits", async () => {
  assert.equal(await verifyLanguageServerLaunch(program("console.log('sage-ls 0.2.0')")), "0.2.0");
  for (const code of ["console.log('Python 3.13')", "console.log('sage-ls 0.2.0');process.exit(1)"]) {
    await assert.rejects(verifyLanguageServerLaunch(program(code)), LanguageServerLaunchError);
  }
  await assert.rejects(verifyLanguageServerLaunch({ command: path.join(os.tmpdir(), "no-such-sage-ls-executable"), args: [] }), /not found/);
});

test("startup timeout is actionable and cancellation ends a pending probe", async () => {
  const never = program("setInterval(()=>{}, 1000)");
  await assert.rejects(verifyLanguageServerLaunch(never, { timeoutMs: 50 }), /startup check/);
  const abort = new AbortController();
  const pending = verifyLanguageServerLaunch(never, { signal: abort.signal });
  abort.abort();
  await assert.rejects(pending, { name: "AbortError" });
});

test("process probes bound output and preserve Unicode across buffer boundaries", async () => {
  const tooMuch = program("process.stdout.write('x'.repeat(8192))");
  await assert.rejects(readProcessProbe(tooMuch.command, tooMuch.args, { timeoutMs: 1000, maxBytes: 16 }), ProbeOutputLimitError);
  const unicode = program("const b=Buffer.from('中文😀');process.stdout.write(b.subarray(0,1));setTimeout(()=>process.stdout.end(b.subarray(1)),20)");
  assert.equal((await readProcessProbe(unicode.command, unicode.args, { timeoutMs: 1000, maxBytes: 256 })).stdout, "中文😀");
});

test("aborting a probe also stops its descendant", { skip: process.platform === "win32" }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "sage-probe-tree-"));
  const ready = path.join(root, "ready");
  const marker = path.join(root, "orphan");
  const descendant = `setTimeout(()=>require('node:fs').writeFileSync(${JSON.stringify(marker)},'alive'),800)`;
  const launch = program(`require('node:child_process').spawn(process.execPath,['-e',${JSON.stringify(descendant)}],{stdio:'ignore'});require('node:fs').writeFileSync(${JSON.stringify(ready)},'ready');setInterval(()=>{},1000)`);
  const abort = new AbortController();
  const pending = readProcessProbe(launch.command, launch.args, { signal: abort.signal, timeoutMs: 3000, maxBytes: 256 });
  const rejection = assert.rejects(pending, { name: "AbortError" });
  try {
    const deadline = Date.now() + 2000;
    while (await fs.access(ready).then(() => false, () => true)) {
      assert.ok(Date.now() < deadline, "probe should spawn its child");
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    abort.abort();
    await rejection;
    await new Promise(resolve => setTimeout(resolve, 900));
    assert.equal(await fs.access(marker).then(() => true, () => false), false);
  } finally { abort.abort(); await rejection; await fs.rm(root, { recursive: true, force: true }); }
});

test("startup settlement and stop share a single shutdown deadline", async () => {
  const never = new Promise<void>(() => undefined);
  let stopped = false;
  let finished = false;
  const started = Date.now();
  await settleAndStopLanguageClient({
    operation: never, timeoutMs: 100,
    takeClient: () => ({ start: () => never, stop: () => { stopped = true; return never; } }),
    afterStop: () => { finished = true; },
  });
  assert.equal(stopped, true);
  assert.equal(finished, true);
  assert.ok(Date.now() - started < 185, "stop must not consume a second full timeout");
});
