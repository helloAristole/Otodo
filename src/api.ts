import { invoke } from "@tauri-apps/api/core";
import type { Todo, RepeatVal } from "./types";

// 运行期检测：在 Tauri 内走本地 SQLite（invoke），否则用浏览器 localStorage 兜底（仅开发预览）。
// 注意：服务端密钥已移至 Rust 侧（lib.rs），不再出现在前端 bundle 中（修复密钥泄露）。
export const IS_TAURI =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

const LS_KEY = "otodo.todos";

function normalize(t: any): Todo {
  return {
    id: t.id,
    title: t.title ?? "",
    note: t.note ?? null,
    due_at: t.due_at ?? null,
    all_day: !!t.all_day,
    due_date: t.due_date ?? null,
    done: !!t.done,
    sort_order: t.sort_order ?? 0,
    deleted: !!t.deleted,
    repeat: (t.repeat as RepeatVal) ?? "none",
    done_dates: t.done_dates ?? "",
    repeat_until: t.repeat_until ?? null,
    skip_dates: t.skip_dates ?? null,
    created_at: t.created_at ?? null,
    updated_at: t.updated_at ?? null,
  };
}

function loadLocal(): Todo[] {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) || "[]");
  } catch {
    return [];
  }
}
function saveLocal(list: Todo[]) {
  localStorage.setItem(LS_KEY, JSON.stringify(list));
}

function seedIfEmpty() {
  const list = loadLocal();
  if (list.length > 0) return;
  const now = new Date();
  const at = (h: number, m = 0) => {
    const d = new Date(now);
    d.setHours(h, m, 0, 0);
    return d.toISOString();
  };
  const samples: Todo[] = [
    { id: crypto.randomUUID(), title: "晨会同步", note: null, due_at: at(9, 30), all_day: false, due_date: null, done: true, sort_order: 0, deleted: false, repeat: "none", done_dates: "", repeat_until: null, skip_dates: null },
    { id: crypto.randomUUID(), title: "写 PRD 评审", note: null, due_at: at(14, 0), all_day: false, due_date: null, done: false, sort_order: 1, deleted: false, repeat: "none", done_dates: "", repeat_until: null, skip_dates: null },
    { id: crypto.randomUUID(), title: "健身", note: null, due_at: at(19, 0), all_day: false, due_date: null, done: false, sort_order: 2, deleted: false, repeat: "none", done_dates: "", repeat_until: null, skip_dates: null },
    { id: crypto.randomUUID(), title: "交水电费", note: null, due_at: null, all_day: true, due_date: now.toISOString().slice(0, 10), done: false, sort_order: 3, deleted: false, repeat: "none", done_dates: "", repeat_until: null, skip_dates: null },
  ];
  saveLocal(samples);
}

// ===== Tauri：本地 SQLite 为真实数据源，后台由 Rust 同步到服务端 =====
async function getTodosTauri(): Promise<Todo[]> {
  const list = (await invoke("get_todos")) as any[];
  return list.map(normalize);
}

async function addTodoTauri(payload: {
  title: string;
  due_at: string | null;
  due_date?: string | null;
  all_day?: boolean;
  note?: string;
  repeat?: RepeatVal;
  repeat_until?: string | null;
  skip_dates?: string | null;
}): Promise<string> {
  // Tauri v2 约定：前端 invoke 参数使用 camelCase，IPC 层自动转为 Rust 端的 snake_case
  return (await invoke("add_todo", {
    title: payload.title,
    dueAt: payload.due_at ?? null,
    dueDate: payload.due_date ?? null,
    allDay: payload.all_day ?? false,
    note: payload.note ?? null,
    repeat: payload.repeat ?? "none",
    repeatUntil: payload.repeat_until ?? null,
    skipDates: payload.skip_dates ?? null,
  })) as string;
}

async function toggleTodoTauri(id: string, done: boolean, day?: string): Promise<void> {
  const args: any = { id, done };
  if (day) args.day = day;
  await invoke("toggle_todo", args);
}

async function updateTodoTauri(
  id: string,
  patch: Record<string, unknown>
): Promise<void> {
  await invoke("update_todo", { id, fields: patch });
}

async function deleteTodoTauri(id: string): Promise<void> {
  await invoke("delete_todo", { id });
}

async function reorderTodosTauri(orders: { id: string; sort_order: number }[]): Promise<void> {
  await invoke("reorder_todos", { orders });
}

// ===== 浏览器 localStorage 兜底（仅开发预览，不联网）=====
async function getTodosLocal(): Promise<Todo[]> {
  return loadLocal();
}
async function addTodoLocal(payload: any): Promise<string> {
  const list = loadLocal();
  const id = crypto.randomUUID();
  list.push({
    id,
    title: payload.title,
    note: payload.note ?? null,
    due_at: payload.due_at ?? null,
    all_day: payload.all_day ?? false,
    due_date: payload.due_date ?? null,
    done: false,
    sort_order: list.length,
    deleted: false,
    repeat: payload.repeat ?? "none",
    done_dates: "",
    repeat_until: payload.repeat_until ?? null,
    skip_dates: payload.skip_dates ?? null,
  });
  saveLocal(list);
  return id;
}
async function toggleTodoLocal(id: string, done: boolean, day?: string): Promise<void> {
  const list = loadLocal();
  const t = list.find((x) => x.id === id);
  if (!t) return;
  if (t.repeat && t.repeat !== "none" && day) {
    const set = t.done_dates ? t.done_dates.split(" ").filter(Boolean) : [];
    if (done) {
      if (!set.includes(day)) set.push(day);
    } else {
      const i = set.indexOf(day);
      if (i >= 0) set.splice(i, 1);
    }
    t.done_dates = set.join(" ");
  } else {
    t.done = done;
  }
  saveLocal(list);
}
async function updateTodoLocal(id: string, patch: Record<string, unknown>): Promise<void> {
  const list = loadLocal();
  const t = list.find((x) => x.id === id);
  if (!t) return;
  Object.assign(t, patch);
  saveLocal(list);
}
async function deleteTodoLocal(id: string): Promise<void> {
  const list = loadLocal();
  const t = list.find((x) => x.id === id);
  if (t) t.deleted = true;
  saveLocal(list);
}
async function reorderTodosLocal(orders: { id: string; sort_order: number }[]): Promise<void> {
  const list = loadLocal();
  const map = new Map(orders.map((o) => [o.id, o.sort_order]));
  for (const t of list) if (map.has(t.id)) t.sort_order = map.get(t.id)!;
  saveLocal(list);
}

// ===== 导出：按运行环境分派 =====
export const initDb = (): Promise<void> => {
  if (IS_TAURI) return invoke("init_db");
  seedIfEmpty();
  return Promise.resolve();
};

export const getTodos = (): Promise<Todo[]> => {
  if (IS_TAURI) return getTodosTauri();
  return getTodosLocal();
};

export const addTodo = (payload: {
  title: string;
  due_at: string | null;
  due_date?: string | null;
  all_day?: boolean;
  note?: string;
  repeat?: RepeatVal;
  repeat_until?: string | null;
  skip_dates?: string | null;
}): Promise<string> => {
  if (IS_TAURI) return addTodoTauri(payload);
  return addTodoLocal(payload);
};

export const toggleTodo = (id: string, done: boolean, day?: string): Promise<void> => {
  if (IS_TAURI) return toggleTodoTauri(id, done, day);
  return toggleTodoLocal(id, done, day);
};

export const updateTodo = (
  id: string,
  patch: {
    title?: string;
    due_at?: string | null;
    due_date?: string | null;
    all_day?: boolean;
    note?: string;
    repeat?: RepeatVal;
    sort_order?: number;
    done_dates?: string;
    repeat_until?: string | null;
    skip_dates?: string | null;
  }
): Promise<void> => {
  if (IS_TAURI) return updateTodoTauri(id, patch as Record<string, unknown>);
  return updateTodoLocal(id, patch as Record<string, unknown>);
};

export const deleteTodo = (id: string): Promise<void> => {
  if (IS_TAURI) return deleteTodoTauri(id);
  return deleteTodoLocal(id);
};

export const reorderTodos = (
  orders: { id: string; sort_order: number }[]
): Promise<void> => {
  if (IS_TAURI) return reorderTodosTauri(orders);
  return reorderTodosLocal(orders);
};

// 关闭窗口到托盘（不退出，真正退出仅由托盘「退出」完成）
export const hideWindow = (): Promise<void> => {
  if (IS_TAURI) return invoke("hide_window");
  return Promise.resolve();
};

/** 首帧绘制后再显示窗口，避免透明窗启动白闪 */
export const showWindow = (): Promise<void> => {
  if (IS_TAURI) return invoke("show_window");
  return Promise.resolve();
};

export type PomoSnap = {
  phase: "idle" | "focus" | "break";
  running: boolean;
  remaining_ms: number;
  rounds_today: number;
  focus_min?: number;
  break_min?: number;
};

export const IDLE_POMO: PomoSnap = {
  phase: "idle",
  running: false,
  remaining_ms: 25 * 60 * 1000,
  rounds_today: 0,
  focus_min: 25,
  break_min: 5,
};

export const getPomodoro = (): Promise<PomoSnap> => {
  if (IS_TAURI) return invoke("get_pomodoro");
  return Promise.resolve({ ...IDLE_POMO });
};

export const togglePomodoro = (): Promise<PomoSnap> => {
  if (IS_TAURI) return invoke("pomodoro_toggle");
  return Promise.resolve({ ...IDLE_POMO });
};

export const resetPomodoro = (): Promise<PomoSnap> => {
  if (IS_TAURI) return invoke("pomodoro_reset");
  return Promise.resolve({ ...IDLE_POMO });
};

export const setPomodoroDurations = (focusMin: number, breakMin: number): Promise<PomoSnap> => {
  const focus = Math.min(180, Math.max(1, Math.round(focusMin) || 25));
  const brk = Math.min(60, Math.max(1, Math.round(breakMin) || 5));
  if (IS_TAURI) return invoke("pomodoro_set_durations", { focusMin: focus, breakMin: brk });
  IDLE_POMO.remaining_ms = focus * 60 * 1000;
  IDLE_POMO.focus_min = focus;
  IDLE_POMO.break_min = brk;
  return Promise.resolve({ ...IDLE_POMO, remaining_ms: focus * 60 * 1000, focus_min: focus, break_min: brk });
};
