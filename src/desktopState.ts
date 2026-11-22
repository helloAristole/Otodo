export type CalendarView = "day" | "week" | "month";

export type TodayNavigationState = {
  view: CalendarView;
  cursor: Date;
  monthYear: { y: number; m: number };
};

export type UndoTodo = {
  id: string;
  title: string;
};

export type DeleteUndoState = {
  todo: UndoTodo;
  expiresAt: number;
};

type StartupScreen = "loading" | "recovery" | "ready";
export type StartupWindowState = {
  temporaryRecoveryExpanded: boolean;
};
export type StartupWindowSizeAction =
  | "expand-temporary"
  | "collapse-persistent"
  | null;

type RevisionedSnapshot = {
  revision: number;
};

const DELETE_UNDO_DURATION_MS = 5_000;

export function getTodayNavigationState(
  view: CalendarView,
  now: Date,
): TodayNavigationState {
  const cursor = new Date(now);
  cursor.setHours(0, 0, 0, 0);
  return {
    view,
    cursor,
    monthYear: { y: cursor.getFullYear(), m: cursor.getMonth() + 1 },
  };
}

export function beginDeleteUndo(
  _current: DeleteUndoState | null,
  todo: UndoTodo,
  now: number,
): DeleteUndoState {
  return {
    todo,
    expiresAt: now + DELETE_UNDO_DURATION_MS,
  };
}

export function expireDeleteUndo(
  state: DeleteUndoState | null,
  timerTodoId: string,
  now: number,
): DeleteUndoState | null {
  if (!state) return null;
  if (state.todo.id !== timerTodoId) return state;
  if (now < state.expiresAt) return state;
  return null;
}

export function getUndoRescheduleDelay(
  state: DeleteUndoState | null,
  timerTodoId: string,
  now: number,
): number | null {
  if (!state || state.todo.id !== timerTodoId || now >= state.expiresAt) {
    return null;
  }
  return state.expiresAt - now;
}

export function advanceStartupWindowState(
  current: StartupWindowState,
  screen: StartupScreen,
  collapsedPreference: boolean,
): { state: StartupWindowState; action: StartupWindowSizeAction } {
  if (!collapsedPreference) {
    return { state: current, action: null };
  }
  if (screen === "recovery") {
    if (current.temporaryRecoveryExpanded) {
      return { state: current, action: null };
    }
    return {
      state: { temporaryRecoveryExpanded: true },
      action: "expand-temporary",
    };
  }
  if (screen === "ready" && current.temporaryRecoveryExpanded) {
    return {
      state: { temporaryRecoveryExpanded: false },
      action: "collapse-persistent",
    };
  }
  return { state: current, action: null };
}

export function getPomodoroSnapshotUpdate<T extends RevisionedSnapshot>(
  current: T | undefined,
  incoming: T,
): T {
  if (current && incoming.revision < current.revision) {
    return current;
  }
  return incoming;
}

export function takeDeleteUndo(
  state: DeleteUndoState | null,
): { todo: UndoTodo; nextState: null } | null {
  if (!state) return null;
  return { todo: state.todo, nextState: null };
}

export function getTodoCheckboxLabel(
  title: string,
  dayKey: string,
  doneForDay: boolean,
): string {
  const targetState = doneForDay ? "未完成" : "完成";
  return `标记“${title}”在 ${dayKey} 为${targetState}`;
}
