import type { Todo } from "./types";
import { matchesTodoOnDay, todoDoneOnDay } from "./todoLogic.ts";

export const NOTIFICATION_HISTORY_KEY = "otodo.notificationHistory";
export const NOTIFICATION_LOOKAHEAD_MS = 30 * 60 * 1000;
export const NOTIFICATION_GRACE_MS = 10 * 60 * 1000;

export type NotificationCandidate = {
  todo: Todo;
  dueAt: Date;
  instanceDay: string;
  dedupeKey: string;
};

type KeyValueStorage = Pick<Storage, "getItem" | "setItem">;
export type NotificationHistoryUpdate = {
  sentKeys: Set<string>;
  persisted: boolean;
};

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function localDayKey(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function getNotificationHistoryRetentionDay(now: Date): string {
  return localDayKey(new Date(now.getTime() - NOTIFICATION_GRACE_MS));
}

function startOfLocalDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function addLocalDays(date: Date, count: number): Date {
  const result = new Date(date);
  result.setDate(result.getDate() + count);
  return result;
}

function instanceDueAt(todo: Todo, day: Date): Date | null {
  if (!todo.due_at) return null;
  const originalTime = new Date(todo.due_at);
  if (Number.isNaN(originalTime.getTime())) return null;
  return new Date(
    day.getFullYear(),
    day.getMonth(),
    day.getDate(),
    originalTime.getHours(),
    originalTime.getMinutes(),
    originalTime.getSeconds(),
    originalTime.getMilliseconds(),
  );
}

export function collectNotificationCandidates(
  todos: Todo[],
  now: Date,
  sentKeys: ReadonlySet<string>,
  includeGracePeriod: boolean,
): NotificationCandidate[] {
  const earliest = includeGracePeriod
    ? new Date(now.getTime() - NOTIFICATION_GRACE_MS)
    : now;
  const latest = new Date(now.getTime() + NOTIFICATION_LOOKAHEAD_MS);
  const firstDay = startOfLocalDay(earliest);
  const lastDayKey = localDayKey(latest);
  const candidates: NotificationCandidate[] = [];

  for (
    let instanceDay = firstDay;
    localDayKey(instanceDay) <= lastDayKey;
    instanceDay = addLocalDays(instanceDay, 1)
  ) {
    const dayKey = localDayKey(instanceDay);
    for (const todo of todos) {
      if (!todo.due_at || !matchesTodoOnDay(todo, dayKey)) continue;
      if (todoDoneOnDay(todo, dayKey)) continue;

      const dueAt = instanceDueAt(todo, instanceDay);
      if (!dueAt || dueAt < earliest || dueAt > latest) continue;

      const dedupeKey = `${todo.id}:${dayKey}`;
      if (sentKeys.has(dedupeKey)) continue;
      candidates.push({ todo, dueAt, instanceDay: dayKey, dedupeKey });
    }
  }

  return candidates.sort((left, right) => left.dueAt.getTime() - right.dueAt.getTime());
}

export function markNotificationSent(
  sentKeys: ReadonlySet<string>,
  candidate: NotificationCandidate,
): Set<string> {
  const next = new Set(sentKeys);
  next.add(candidate.dedupeKey);
  return next;
}

export function pruneNotificationHistory(
  sentKeys: ReadonlySet<string>,
  todayKey: string,
): Set<string> {
  return new Set(
    [...sentKeys].filter((key) => {
      const separator = key.lastIndexOf(":");
      if (separator < 0) return false;
      return key.slice(separator + 1) >= todayKey;
    }),
  );
}

export function loadNotificationHistory(
  storage: KeyValueStorage,
  todayKey: string,
): Set<string> {
  try {
    const parsed = JSON.parse(storage.getItem(NOTIFICATION_HISTORY_KEY) || "[]");
    if (!Array.isArray(parsed)) return new Set();
    return pruneNotificationHistory(
      new Set(parsed.filter((value): value is string => typeof value === "string")),
      todayKey,
    );
  } catch {
    return new Set();
  }
}

export function saveNotificationHistory(
  storage: KeyValueStorage,
  sentKeys: ReadonlySet<string>,
): boolean {
  try {
    storage.setItem(NOTIFICATION_HISTORY_KEY, JSON.stringify([...sentKeys]));
    return true;
  } catch {
    return false;
  }
}

export function recordSuccessfulNotification(
  sentKeys: ReadonlySet<string>,
  candidate: NotificationCandidate,
  storage: KeyValueStorage,
): NotificationHistoryUpdate {
  const next = markNotificationSent(sentKeys, candidate);
  return {
    sentKeys: next,
    persisted: saveNotificationHistory(storage, next),
  };
}

export function pruneAndPersistNotificationHistory(
  sentKeys: ReadonlySet<string>,
  todayKey: string,
  storage: KeyValueStorage,
): NotificationHistoryUpdate {
  const pruned = pruneNotificationHistory(sentKeys, todayKey);
  return {
    sentKeys: pruned,
    persisted: saveNotificationHistory(storage, pruned),
  };
}
