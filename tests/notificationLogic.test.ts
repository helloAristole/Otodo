import assert from "node:assert/strict";
import test from "node:test";
import {
  collectNotificationCandidates,
  getNotificationHistoryRetentionDay,
  markNotificationSent,
  pruneAndPersistNotificationHistory,
  pruneNotificationHistory,
  recordSuccessfulNotification,
} from "../src/notificationLogic.ts";
import type { Todo } from "../src/types.ts";

function todo(overrides: Partial<Todo> = {}): Todo {
  return {
    id: "todo-1",
    title: "测试提醒",
    note: null,
    due_at: "2026-11-21T08:20:00.000Z",
    all_day: false,
    due_date: "2026-11-21",
    done: false,
    sort_order: 0,
    deleted: false,
    repeat: "none",
    done_dates: "",
    repeat_until: null,
    skip_dates: null,
    ...overrides,
  };
}

const now = new Date("2026-11-21T08:00:00.000Z");

test("selects an unfinished timed todo in the next 30 minutes", () => {
  const candidates = collectNotificationCandidates([todo()], now, new Set(), false);

  assert.equal(candidates.length, 1);
  assert.equal(candidates[0].instanceDay, "2026-11-21");
  assert.equal(candidates[0].dedupeKey, "todo-1:2026-11-21");
});

test("uses the current repeat instance date with the original local time", () => {
  const repeated = todo({
    id: "daily",
    due_at: "2026-11-01T08:20:00.000Z",
    due_date: "2026-11-01",
    repeat: "daily",
  });

  const candidates = collectNotificationCandidates([repeated], now, new Set(), false);

  assert.equal(candidates.length, 1);
  assert.equal(candidates[0].instanceDay, "2026-11-21");
  assert.equal(candidates[0].dueAt.toISOString(), "2026-11-21T08:20:00.000Z");
});

test("excludes completed and skipped repeat instances", () => {
  const completed = todo({
    id: "completed",
    repeat: "daily",
    due_date: "2026-11-01",
    done_dates: "2026-11-21",
  });
  const skipped = todo({
    id: "skipped",
    repeat: "daily",
    due_date: "2026-11-01",
    skip_dates: "2026-11-21",
  });

  assert.deepEqual(
    collectNotificationCandidates([completed, skipped], now, new Set(), false),
    [],
  );
});

test("only visibility recovery includes instances overdue by at most 10 minutes", () => {
  const overdue = todo({ due_at: "2026-11-21T07:55:00.000Z" });
  const tooOld = todo({ id: "old", due_at: "2026-11-21T07:49:59.000Z" });

  assert.deepEqual(collectNotificationCandidates([overdue], now, new Set(), false), []);
  assert.deepEqual(
    collectNotificationCandidates([overdue, tooOld], now, new Set(), true).map(
      (candidate) => candidate.todo.id,
    ),
    ["todo-1"],
  );
});

test("dedupe history changes only when explicitly marked after a successful send", () => {
  const history = new Set<string>();
  const candidate = collectNotificationCandidates([todo()], now, history, false)[0];

  assert.equal(history.size, 0);
  const next = markNotificationSent(history, candidate);
  assert.deepEqual([...next], ["todo-1:2026-11-21"]);
  assert.equal(collectNotificationCandidates([todo()], now, next, false).length, 0);
});

test("prunes persisted dedupe keys outside the retained date range", () => {
  const history = new Set([
    "old:2026-11-18",
    "yesterday:2026-11-20",
    "today:2026-11-21",
    "future:2026-11-22",
    "invalid",
  ]);

  assert.deepEqual(
    [...pruneNotificationHistory(history, "2026-11-20")].sort(),
    ["future:2026-11-22", "today:2026-11-21", "yesterday:2026-11-20"],
  );
});

test("midnight grace cleanup retains yesterday dedupe keys", () => {
  const justAfterMidnight = new Date(2026, 10, 21, 0, 5);
  const cleanupDay = new Date(
    justAfterMidnight.getTime() - 10 * 60 * 1000,
  );

  const result = pruneNotificationHistory(
    new Set(["daily:2026-11-20", "old:2026-11-19"]),
    [
      cleanupDay.getFullYear(),
      String(cleanupDay.getMonth() + 1).padStart(2, "0"),
      String(cleanupDay.getDate()).padStart(2, "0"),
    ].join("-"),
  );

  assert.deepEqual([...result], ["daily:2026-11-20"]);
});

test("initial notification history load uses the grace-period retention day", () => {
  assert.equal(
    getNotificationHistoryRetentionDay(new Date(2026, 10, 21, 0, 5)),
    "2026-11-20",
  );
  assert.equal(
    getNotificationHistoryRetentionDay(new Date(2026, 10, 21, 0, 15)),
    "2026-11-21",
  );
});

test("successful send stays deduped in memory when persistence fails", () => {
  const candidate = collectNotificationCandidates([todo()], now, new Set(), false)[0];
  const failingStorage = {
    getItem: () => null,
    setItem: () => {
      throw new Error("quota exceeded");
    },
  };

  const result = recordSuccessfulNotification(new Set(), candidate, failingStorage);

  assert.equal(result.persisted, false);
  assert.equal(result.sentKeys.has(candidate.dedupeKey), true);
  assert.equal(
    collectNotificationCandidates([todo()], now, result.sentKeys, false).length,
    0,
  );
});

test("history cleanup persistence failure still returns usable pruned history", () => {
  const failingStorage = {
    getItem: () => null,
    setItem: () => {
      throw new Error("storage unavailable");
    },
  };

  const result = pruneAndPersistNotificationHistory(
    new Set(["old:2026-11-20", "today:2026-11-21"]),
    "2026-11-21",
    failingStorage,
  );

  assert.equal(result.persisted, false);
  assert.deepEqual([...result.sentKeys], ["today:2026-11-21"]);
});
