import assert from "node:assert/strict";
import test from "node:test";
import {
  advanceUpdateDownload,
  createUpdateDownload,
  getUpdateProgressPercent,
} from "../src/updateLogic.ts";

test("update download progress accumulates chunks and finishes at 100 percent", () => {
  let state = createUpdateDownload();

  state = advanceUpdateDownload(state, {
    event: "Started",
    data: { contentLength: 1_000 },
  });
  state = advanceUpdateDownload(state, {
    event: "Progress",
    data: { chunkLength: 250 },
  });

  assert.deepEqual(state, {
    downloadedBytes: 250,
    totalBytes: 1_000,
    finished: false,
  });
  assert.equal(getUpdateProgressPercent(state), 25);

  state = advanceUpdateDownload(state, { event: "Finished" });
  assert.equal(state.finished, true);
  assert.equal(getUpdateProgressPercent(state), 100);
});

test("unknown download size stays indeterminate until finished", () => {
  let state = createUpdateDownload();
  state = advanceUpdateDownload(state, {
    event: "Started",
    data: {},
  });
  state = advanceUpdateDownload(state, {
    event: "Progress",
    data: { chunkLength: 128 },
  });

  assert.equal(getUpdateProgressPercent(state), null);

  state = advanceUpdateDownload(state, { event: "Finished" });
  assert.equal(getUpdateProgressPercent(state), 100);
});
