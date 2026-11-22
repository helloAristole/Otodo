import assert from "node:assert/strict";
import test from "node:test";
import { getStartupScreen } from "../src/startupState.ts";

test("loading state hides every writable view", () => {
  assert.equal(getStartupScreen(true, null), "loading");
  assert.equal(getStartupScreen(true, "旧错误"), "loading");
});

test("failed initialization shows recovery instead of normal views", () => {
  assert.equal(getStartupScreen(false, "数据库损坏"), "recovery");
});

test("normal views are available only after successful initialization", () => {
  assert.equal(getStartupScreen(false, null), "ready");
});

test("initialization state priority is loading then error then ready", () => {
  const cases = [
    { loading: true, error: "数据库错误", expected: "loading" },
    { loading: false, error: "数据库错误", expected: "recovery" },
    { loading: false, error: null, expected: "ready" },
  ] as const;

  for (const item of cases) {
    assert.equal(getStartupScreen(item.loading, item.error), item.expected);
  }
});
