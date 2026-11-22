import assert from "node:assert/strict";
import test from "node:test";
import {
  beginDeleteUndo,
  advanceStartupWindowState,
  expireDeleteUndo,
  getPomodoroSnapshotUpdate,
  getTodayNavigationState,
  getTodoCheckboxLabel,
  getUndoRescheduleDelay,
  takeDeleteUndo,
} from "../src/desktopState.ts";

test("all views navigate to today and synchronize monthYear", () => {
  const today = new Date(2026, 10, 21, 16, 30);

  for (const view of ["day", "week", "month"] as const) {
    assert.deepEqual(getTodayNavigationState(view, today), {
      view,
      cursor: new Date(2026, 10, 21),
      monthYear: { y: 2026, m: 11 },
    });
  }
});

test("consecutive deletes expose only the latest item for undo", () => {
  const first = beginDeleteUndo(null, { id: "first", title: "第一项" }, 1_000);
  const second = beginDeleteUndo(first, { id: "second", title: "第二项" }, 2_000);

  assert.equal(expireDeleteUndo(second, "first", 6_000), second);
  assert.deepEqual(takeDeleteUndo(second), {
    todo: { id: "second", title: "第二项" },
    nextState: null,
  });
});

test("only the matching current timer can expire undo state", () => {
  const state = beginDeleteUndo(null, { id: "latest", title: "最新项" }, 2_000);

  assert.equal(expireDeleteUndo(state, "latest", 6_999), state);
  assert.equal(expireDeleteUndo(state, "latest", 7_000), null);
});

test("early undo timer is rescheduled for the exact remaining duration", () => {
  const state = beginDeleteUndo(null, { id: "latest", title: "最新项" }, 2_000);

  assert.equal(getUndoRescheduleDelay(state, "latest", 6_200), 800);
  assert.equal(getUndoRescheduleDelay(state, "latest", 7_000), null);
  assert.equal(getUndoRescheduleDelay(state, "stale", 6_200), null);
});

test("collapsed recovery expansion survives loading until retry succeeds", () => {
  let state = { temporaryRecoveryExpanded: false };

  let transition = advanceStartupWindowState(state, "recovery", true);
  assert.equal(transition.action, "expand-temporary");
  state = transition.state;

  transition = advanceStartupWindowState(state, "loading", true);
  assert.equal(transition.action, null);
  assert.equal(transition.state.temporaryRecoveryExpanded, true);
  state = transition.state;

  transition = advanceStartupWindowState(state, "ready", true);
  assert.equal(transition.action, "collapse-persistent");
  assert.equal(transition.state.temporaryRecoveryExpanded, false);
});

test("failed retry keeps collapsed recovery temporarily expanded", () => {
  const expanded = { temporaryRecoveryExpanded: true };

  const loading = advanceStartupWindowState(expanded, "loading", true);
  const failedAgain = advanceStartupWindowState(loading.state, "recovery", true);

  assert.equal(loading.action, null);
  assert.equal(failedAgain.action, null);
  assert.equal(failedAgain.state.temporaryRecoveryExpanded, true);
});

test("normal startup never synchronizes window size", () => {
  const initial = { temporaryRecoveryExpanded: false };

  assert.equal(advanceStartupWindowState(initial, "loading", true).action, null);
  assert.equal(advanceStartupWindowState(initial, "ready", true).action, null);
  assert.equal(advanceStartupWindowState(initial, "recovery", false).action, null);
});

test("pomodoro state accepts only snapshots with a nondecreasing revision", () => {
  const paused = {
    phase: "focus" as const,
    running: false,
    remaining_ms: 50_000,
    rounds_today: 0,
    revision: 4,
  };
  const staleRunning = {
    ...paused,
    running: true,
    remaining_ms: 49_000,
    revision: 3,
  };

  assert.deepEqual(getPomodoroSnapshotUpdate(undefined, paused), paused);
  assert.equal(getPomodoroSnapshotUpdate(paused, staleRunning), paused);
  assert.deepEqual(
    getPomodoroSnapshotUpdate(paused, { ...paused, remaining_ms: 48_000, revision: 4 }),
    { ...paused, remaining_ms: 48_000, revision: 4 },
  );
});

test("todo checkbox label contains title date and target action", () => {
  assert.equal(
    getTodoCheckboxLabel("晨会", "2026-11-21", false),
    "标记“晨会”在 2026-11-21 为完成",
  );
  assert.equal(
    getTodoCheckboxLabel("晨会", "2026-11-21", true),
    "标记“晨会”在 2026-11-21 为未完成",
  );
});
