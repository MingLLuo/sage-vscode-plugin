import test from "node:test";
import assert from "node:assert/strict";

import {
  buildRunTaskDefinition,
  RunTaskLifecycle,
  type TerminableTaskExecution,
} from "../src/runTaskLifecycle";

interface FakeExecution extends TerminableTaskExecution {
  terminations: number;
}

function execution(): FakeExecution {
  return {
    terminations: 0,
    terminate() {
      this.terminations += 1;
    },
  };
}

function lifecycleHarness() {
  const cleaned: Array<string | undefined> = [];
  let idleNotifications = 0;
  const lifecycle = new RunTaskLifecycle<FakeExecution>(
    (cleanupPath) => cleaned.push(cleanupPath),
    () => {
      idleNotifications += 1;
    },
  );
  return { lifecycle, cleaned, idleNotifications: () => idleNotifications };
}

test("buildRunTaskDefinition stays unique across launches and activations", () => {
  const first = buildRunTaskDefinition("/tmp/example.sage", "activation-a", 1);
  const nextLaunch = buildRunTaskDefinition("/tmp/example.sage", "activation-a", 2);
  const nextActivation = buildRunTaskDefinition("/tmp/example.sage", "activation-b", 1);

  assert.equal(first.type, "sage.runFile");
  assert.notEqual(first.invocation, nextLaunch.invocation);
  assert.notEqual(first.invocation, nextActivation.invocation);
});

test("RunTaskLifecycle cleans a completed task exactly once", () => {
  const harness = lifecycleHarness();
  const task = execution();
  harness.lifecycle.beginLaunch();
  harness.lifecycle.completeLaunch(task, "/tmp/example.sage.py");
  harness.lifecycle.end(task);
  harness.lifecycle.end(task);

  assert.deepEqual(harness.cleaned, ["/tmp/example.sage.py"]);
  assert.equal(task.terminations, 0);
  assert.deepEqual(harness.lifecycle.snapshot(), {
    activeTasks: 0,
    pendingLaunches: 0,
    disposed: false,
  });
});

test("RunTaskLifecycle handles a task-end event that races launch completion", () => {
  const harness = lifecycleHarness();
  const task = execution();
  harness.lifecycle.beginLaunch();
  harness.lifecycle.end(task);
  harness.lifecycle.completeLaunch(task, "/tmp/raced.sage.py");

  assert.deepEqual(harness.cleaned, ["/tmp/raced.sage.py"]);
  assert.equal(harness.lifecycle.snapshot().activeTasks, 0);
});

test("RunTaskLifecycle keeps an end tombstone when a stable execution id is reused", () => {
  const harness = lifecycleHarness();
  const task = execution();
  harness.lifecycle.beginLaunch();
  harness.lifecycle.completeLaunch(task, "/tmp/first.sage.py");

  harness.lifecycle.beginLaunch();
  harness.lifecycle.end(task);
  harness.lifecycle.completeLaunch(task, "/tmp/second.sage.py");

  assert.deepEqual(harness.cleaned, ["/tmp/first.sage.py", "/tmp/second.sage.py"]);
  assert.equal(harness.lifecycle.snapshot().activeTasks, 0);
});

test("RunTaskLifecycle terminates an in-flight launch after disposal and waits for task end", () => {
  const harness = lifecycleHarness();
  const task = execution();
  harness.lifecycle.beginLaunch();
  harness.lifecycle.dispose();
  assert.equal(harness.idleNotifications(), 0);

  harness.lifecycle.completeLaunch(task, "/tmp/disposed.sage.py");
  assert.equal(task.terminations, 1);
  assert.equal(harness.idleNotifications(), 0);

  harness.lifecycle.end(task);
  assert.deepEqual(harness.cleaned, ["/tmp/disposed.sage.py"]);
  assert.equal(harness.idleNotifications(), 1);
});

test("RunTaskLifecycle releases its listener when a pending launch fails after disposal", () => {
  const harness = lifecycleHarness();
  harness.lifecycle.beginLaunch();
  harness.lifecycle.dispose();
  harness.lifecycle.failLaunch();

  assert.equal(harness.idleNotifications(), 1);
  assert.deepEqual(harness.lifecycle.snapshot(), {
    activeTasks: 0,
    pendingLaunches: 0,
    disposed: true,
  });
});

test("RunTaskLifecycle rejects new launches after disposal", () => {
  const harness = lifecycleHarness();
  harness.lifecycle.dispose();

  assert.throws(() => harness.lifecycle.beginLaunch(), /has been disposed/);
  assert.equal(harness.idleNotifications(), 1);
});
