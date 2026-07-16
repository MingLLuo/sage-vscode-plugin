import assert from "node:assert/strict";
import test from "node:test";

import {
  prepareRunFileDocument,
  type SaveableRunFileDocument,
} from "../src/runFilePreparation";

interface FakeDocument extends SaveableRunFileDocument {
  isDirty: boolean;
  saveCalls: number;
}

test("prepareRunFileDocument leaves an already saved document untouched", async () => {
  const document = fakeDocument(false, async () => {
    assert.fail("a clean document must not be saved again");
  });

  assert.deepEqual(await prepareRunFileDocument(document), {
    ready: true,
    saved: false,
  });
  assert.equal(document.saveCalls, 0);
});

test("prepareRunFileDocument saves dirty content before allowing execution", async () => {
  const document = fakeDocument(true, async () => {
    document.isDirty = false;
    return true;
  });

  assert.deepEqual(await prepareRunFileDocument(document), {
    ready: true,
    saved: true,
  });
  assert.equal(document.saveCalls, 1);
});

test("prepareRunFileDocument blocks execution when saving is cancelled", async () => {
  const document = fakeDocument(true, async () => false);

  assert.deepEqual(await prepareRunFileDocument(document), {
    ready: false,
    reason: "save-not-completed",
  });
  assert.equal(document.saveCalls, 1);
});

test("prepareRunFileDocument blocks execution when changes remain dirty after save", async () => {
  const document = fakeDocument(true, async () => true);

  assert.deepEqual(await prepareRunFileDocument(document), {
    ready: false,
    reason: "save-not-completed",
  });
  assert.equal(document.saveCalls, 1);
});

test("prepareRunFileDocument reports save failures without throwing", async () => {
  const failure = new Error("disk is read-only");
  const document = fakeDocument(true, async () => { throw failure; });

  assert.deepEqual(await prepareRunFileDocument(document), {
    ready: false,
    reason: "save-failed",
    error: failure,
  });
  assert.equal(document.saveCalls, 1);
});

function fakeDocument(
  isDirty: boolean,
  save: () => PromiseLike<boolean>,
): FakeDocument {
  return {
    isDirty,
    saveCalls: 0,
    save() {
      this.saveCalls += 1;
      return save();
    },
  };
}
