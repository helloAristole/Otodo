import { invoke } from "@tauri-apps/api/core";
import type { Todo, RepeatVal } from "./types";

// 运行期检测：在 Tauri 内走本地 SQLite（invoke），否则用浏览器 localStorage 兜底（仅开发预览）。
// 注意：服务端密钥已移至 Rust 侧（lib.rs），不再出现在前端 bundle 中（修复密钥泄露）。
export const IS_TAURI =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

const LS_KEY = "otodo.todos";

export type TodoOrder = { id: string; sort_order: number };
export type TodoDateUpdate = {
  id: string;
  due_at: string | null;
  due_date: string | null;
  all_day: boolean;
};

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

async function restoreTodoTauri(id: string): Promise<void> {
  await invoke("restore_todo", { id });
}

async function reorderTodosTauri(orders: TodoOrder[]): Promise<void> {
  await invoke("reorder_todos", { orders });
}

async function moveTodoTauri(update: TodoDateUpdate, orders: TodoOrder[]): Promise<void> {
  await invoke("move_todo", {
    id: update.id,
    dueAt: update.due_at,
    dueDate: update.due_date,
    allDay: update.all_day,
    orders,
  });
}

async function rollOverTodosTauri(moves: TodoDateUpdate[]): Promise<void> {
  await invoke("roll_over_todos", { moves });
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
  if (!t) throw new Error(`待办不存在：${id}`);
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
  if (!t) throw new Error(`待办不存在：${id}`);
  validateLocalPatch(patch);
  Object.assign(t, patch);
  saveLocal(list);
}
async function deleteTodoLocal(id: string): Promise<void> {
  const list = loadLocal();
  const t = list.find((x) => x.id === id);
  if (!t) throw new Error(`待办不存在：${id}`);
  t.deleted = true;
  saveLocal(list);
}
async function restoreTodoLocal(id: string): Promise<void> {
  const list = loadLocal();
  const todo = list.find((item) => item.id === id);
  if (!todo) throw new Error(`待办不存在：${id}`);
  if (!todo.deleted) throw new Error(`待办 ${id} 未处于删除状态`);
  todo.deleted = false;
  saveLocal(list);
}
async function reorderTodosLocal(orders: TodoOrder[]): Promise<void> {
  const list = loadLocal();
  ensureLocalTodosExist(list, orders.map((order) => order.id));
  const map = new Map(orders.map((o) => [o.id, o.sort_order]));
  for (const t of list) if (map.has(t.id)) t.sort_order = map.get(t.id)!;
  saveLocal(list);
}

async function moveTodoLocal(update: TodoDateUpdate, orders: TodoOrder[]): Promise<void> {
  const list = loadLocal();
  ensureLocalTodosExist(list, [update.id, ...orders.map((order) => order.id)]);
  const moved = list.find((todo) => todo.id === update.id)!;
  moved.due_at = update.due_at;
  moved.due_date = update.due_date;
  moved.all_day = update.all_day;
  const orderById = new Map(orders.map((order) => [order.id, order.sort_order]));
  for (const todo of list) {
    const sortOrder = orderById.get(todo.id);
    if (sortOrder !== undefined) todo.sort_order = sortOrder;
  }
  saveLocal(list);
}

async function rollOverTodosLocal(moves: TodoDateUpdate[]): Promise<void> {
  const list = loadLocal();
  ensureLocalTodosExist(list, moves.map((move) => move.id));
  const moveById = new Map(moves.map((move) => [move.id, move]));
  for (const todo of list) {
    const move = moveById.get(todo.id);
    if (!move) continue;
    todo.due_at = move.due_at;
    todo.due_date = move.due_date;
    todo.all_day = move.all_day;
  }
  saveLocal(list);
}

function ensureLocalTodosExist(list: Todo[], ids: string[]): void {
  const existingIds = new Set(list.map((todo) => todo.id));
  const missingId = ids.find((id) => !existingIds.has(id));
  if (missingId) throw new Error(`待办不存在：${missingId}`);
}

function validateLocalPatch(patch: Record<string, unknown>): void {
  const nullableTextFields = new Set(["note", "due_at", "due_date", "repeat_until", "skip_dates"]);
  const textFields = new Set(["title", "repeat", "done_dates"]);
  const booleanFields = new Set(["all_day", "done"]);

  for (const [key, value] of Object.entries(patch)) {
    if (nullableTextFields.has(key)) {
      if (value !== null && typeof value !== "string") {
        throw new Error(`字段 ${key} 必须是字符串或 null`);
      }
      continue;
    }
    if (textFields.has(key)) {
      if (typeof value !== "string") throw new Error(`字段 ${key} 必须是字符串且不能为 null`);
      continue;
    }
    if (booleanFields.has(key)) {
      if (typeof value !== "boolean") throw new Error(`字段 ${key} 必须是布尔值且不能为 null`);
      continue;
    }
    if (key === "sort_order") {
      if (!Number.isInteger(value)) throw new Error("字段 sort_order 必须是整数且不能为 null");
      continue;
    }
    throw new Error(`不支持的更新字段：${key}`);
  }
}

// ===== 导出：按运行环境分派 =====
export const initDb = (): Promise<void> => {
  if (IS_TAURI) return invoke("init_db");
  seedIfEmpty();
  return Promise.resolve();
};

/** 只读返回本地数据目录，供启动失败时定位数据库与备份。 */
export const getDataDir = (): Promise<string> => {
  if (IS_TAURI) return invoke("get_data_dir");
  return Promise.resolve("浏览器本地存储（localStorage）");
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
    note?: string | null;
    done?: boolean;
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

export const restoreTodo = (id: string): Promise<void> => {
  if (IS_TAURI) return restoreTodoTauri(id);
  return restoreTodoLocal(id);
};

export const reorderTodos = (
  orders: TodoOrder[]
): Promise<void> => {
  if (IS_TAURI) return reorderTodosTauri(orders);
  return reorderTodosLocal(orders);
};

/** 跨天移动与目标列排序在一次写入中完成。 */
export const moveTodo = (
  update: TodoDateUpdate,
  orders: TodoOrder[]
): Promise<void> => {
  if (IS_TAURI) return moveTodoTauri(update, orders);
  return moveTodoLocal(update, orders);
};

/** 批量顺延在一次写入中完成，任一待办失败则全部保持原状。 */
export const rollOverTodos = (moves: TodoDateUpdate[]): Promise<void> => {
  if (IS_TAURI) return rollOverTodosTauri(moves);
  return rollOverTodosLocal(moves);
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
  revision: number;
};

export const IDLE_POMO: PomoSnap = {
  phase: "idle",
  running: false,
  remaining_ms: 25 * 60 * 1000,
  rounds_today: 0,
  focus_min: 25,
  break_min: 5,
  revision: 0,
};

export const sendDueNotification = (
  title: string,
  body: string,
): Promise<void> => {
  if (IS_TAURI) return invoke("send_due_notification", { title, body });
  return Promise.resolve();
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
