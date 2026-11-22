import assert from "node:assert/strict";
import test from "node:test";
import {
  appendSkipDate,
  countRemainingTodosForDay,
  matchesTodoOnDay,
} from "../src/todoLogic.ts";
import type { Todo } from "../src/types.ts";

function todo(overrides: Partial<Todo>): Todo {
  return {
    id: "todo-1",
    title: "测试待办",
    note: null,
    due_at: null,
    all_day: true,
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

test("appendSkipDate uses the supplied card day key", () => {
  assert.equal(appendSkipDate(null, "2026-11-18"), "2026-11-18");
  assert.equal(
    appendSkipDate("2026-11-18", "2026-11-25"),
    "2026-11-18 2026-11-25",
  );
});

test("appendSkipDate is idempotent for the same instance", () => {
  assert.equal(
    appendSkipDate("2026-11-18 2026-11-25", "2026-11-25"),
    "2026-11-18 2026-11-25",
  );
});

test("matchesTodoOnDay excludes only the skipped repeat instance", () => {
  const repeated = todo({
    repeat: "daily",
    due_date: "2026-11-01",
    skip_dates: "2026-11-18",
  });

  assert.equal(matchesTodoOnDay(repeated, "2026-11-18"), false);
  assert.equal(matchesTodoOnDay(repeated, "2026-11-19"), true);
});

test("today count uses repeat matching and per-day completion", () => {
  const todos = [
    todo({ id: "daily-open", repeat: "daily", due_date: "2026-11-01" }),
    todo({
      id: "daily-done",
      repeat: "daily",
      due_date: "2026-11-01",
      done_dates: "2026-11-21",
    }),
    todo({
      id: "weekly-other-day",
      repeat: "weekly",
      due_date: "2026-11-20",
    }),
    todo({ id: "today-normal", due_date: "2026-11-21" }),
    todo({ id: "future-normal", due_date: "2026-11-22" }),
    todo({ id: "deleted", due_date: "2026-11-21", deleted: true }),
  ];

  assert.equal(countRemainingTodosForDay(todos, "2026-11-21"), 2);
});
