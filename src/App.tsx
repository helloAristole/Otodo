import { useEffect, useLayoutEffect, useMemo, useState, useCallback, useRef } from "react";
import { initDb, getTodos, addTodo, updateTodo, toggleTodo, deleteTodo, hideWindow, showWindow, reorderTodos, IS_TAURI, getPomodoro, togglePomodoro, resetPomodoro, setPomodoroDurations, IDLE_POMO, type PomoSnap } from "./api";
import solarLunar from "solarlunar";
import { zh as chrono } from "chrono-node";
import type { Todo, RepeatVal } from "./types";

// ---- 常量 ----
type View = "day" | "week" | "month";
const WEEK_LABELS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
const MONTH_HEAD = ["日", "一", "二", "三", "四", "五", "六"];
const SETTINGS_KEY = "otodo.settings";
const EXPANDED_W = 1000;
const EXPANDED_H = 720;
const COLLAPSED_H = 40;
const MIN_WINDOW_W = 360;

type AppSettings = {
  alwaysOnTop?: boolean;
  rollOver?: boolean;
  collapsed?: boolean;
  locked?: boolean;
  view?: View;
  expandedW?: number;
  expandedH?: number;
  pomoFocusMin?: number;
  pomoBreakMin?: number;
};

function clampPomoMin(n: unknown, min: number, max: number, fallback: number): number {
  const v = typeof n === "number" ? n : Number(n);
  if (!Number.isFinite(v)) return fallback;
  return Math.min(max, Math.max(min, Math.round(v)));
}

function loadSettings(): AppSettings {
  try {
    return JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}") as AppSettings;
  } catch {
    return {};
  }
}

function persistSettings(partial: AppSettings) {
  const next: AppSettings = { ...loadSettings(), ...partial };
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(next));
  } catch { /* ignore */ }
  if (IS_TAURI) {
    void import("@tauri-apps/api/core")
      .then(({ invoke }) =>
        invoke("save_ui_settings", {
          settings: {
            alwaysOnTop: !!next.alwaysOnTop,
            rollOver: typeof next.rollOver === "boolean" ? next.rollOver : true,
            collapsed: !!next.collapsed,
            locked: !!next.locked,
            view: next.view ?? "week",
            expandedW: next.expandedW ?? EXPANDED_W,
            expandedH: next.expandedH ?? EXPANDED_H,
            pomoFocusMin: clampPomoMin(next.pomoFocusMin, 1, 180, 25),
            pomoBreakMin: clampPomoMin(next.pomoBreakMin, 1, 60, 5),
          },
        })
      )
      .catch(() => {});
  }
  return next;
}

function settingsFromRust(raw: Record<string, unknown>): AppSettings {
  return {
    alwaysOnTop: !!raw.always_on_top || !!raw.alwaysOnTop,
    rollOver:
      typeof raw.roll_over === "boolean"
        ? (raw.roll_over as boolean)
        : typeof raw.rollOver === "boolean"
          ? (raw.rollOver as boolean)
          : true,
    collapsed: !!raw.collapsed,
    locked: !!raw.locked,
    view: (raw.view as View) || undefined,
    expandedW:
      typeof raw.expanded_w === "number"
        ? (raw.expanded_w as number)
        : typeof raw.expandedW === "number"
          ? (raw.expandedW as number)
          : undefined,
    expandedH:
      typeof raw.expanded_h === "number"
        ? (raw.expanded_h as number)
        : typeof raw.expandedH === "number"
          ? (raw.expandedH as number)
          : undefined,
    pomoFocusMin: clampPomoMin(raw.pomo_focus_min ?? raw.pomoFocusMin, 1, 180, 25),
    pomoBreakMin: clampPomoMin(raw.pomo_break_min ?? raw.pomoBreakMin, 1, 60, 5),
  };
}

/** 线型图标：currentColor，细描边。锁定时锁扣闭合，未锁定时锁扣打开 */
function IconLock({ locked = false }: { locked?: boolean }) {
  return (
    <svg className="ui-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect
        x="5"
        y="11"
        width="14"
        height="10"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.75"
      />
      <path
        d={locked ? "M8 11V7a4 4 0 0 1 8 0v4" : "M8 11V7a4 4 0 0 1 7.5-1.5"}
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function IconSettings() {
  return (
    <svg className="ui-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="12" cy="12" r="3" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}

function IconClose() {
  return (
    <svg className="ui-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M6 6l12 12M18 6L6 18"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
      />
    </svg>
  );
}

function IconSearch() {
  return (
    <svg className="ui-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="11" cy="11" r="6.5" stroke="currentColor" strokeWidth="1.75" />
      <path d="M16.5 16.5 21 21" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" />
    </svg>
  );
}

// ---- 工具函数 ----
function pad(n: number) { return String(n).padStart(2, "0"); }
function ymd(d: Date): string { return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`; }
function addDays(d: Date, n: number): Date { const x = new Date(d); x.setDate(x.getDate() + n); return x; }
function startOfWeekMonday(d: Date): Date { const x = new Date(d); x.setHours(0, 0, 0, 0); const offset = (x.getDay() + 6) % 7; return addDays(x, -offset); }
function isAllDay(t: Todo): boolean { return t.all_day || !t.due_at; }
function todoDayKey(t: Todo): string {
  if (t.due_at) return ymd(new Date(t.due_at));
  if (t.due_date) return t.due_date;
  // 无日期的历史待办：落在创建日，禁止回退成「今天」否则会每天跟到当日列
  if (t.created_at) return ymd(new Date(t.created_at));
  return ymd(new Date());
}
function isLeap(y: number): boolean { return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0; }
function daysInMonth(dayKey: string): number {
  const [y, m] = dayKey.split("-").map(Number);
  return new Date(y, m, 0).getDate();
}
function todoDoneOnDay(t: Todo, dayKey: string): boolean {
  if (t.repeat && t.repeat !== "none") return t.done_dates.split(" ").filter(Boolean).includes(dayKey);
  return t.done;
}
function matchesRepeat(t: Todo, dayKey: string): boolean {
  // 跳过单天（Bug 10：可单独跳过某次重复）
  if (t.skip_dates && t.skip_dates.split(" ").filter(Boolean).includes(dayKey)) return false;
  if (!t.repeat || t.repeat === "none") {
    if (t.repeat_until && dayKey > t.repeat_until) return false;
    return todoDayKey(t) === dayKey;
  }
  const base = t.due_date ? new Date(t.due_date + "T00:00:00") : t.due_at ? new Date(t.due_at) : new Date();
  if (dayKey < ymd(base)) return false; // 不显示创建日之前
  if (t.repeat_until && dayKey > t.repeat_until) return false; // 重复结束日期（Bug 10）
  const d = new Date(dayKey + "T00:00:00");
  switch (t.repeat) {
    case "daily": return true;
    case "weekly": return d.getDay() === base.getDay();
    case "monthly": {
      const target = Math.min(base.getDate(), daysInMonth(dayKey)); // 月末不足时顺延到最后一天（Bug 6）
      return d.getDate() === target;
    }
    case "yearly": {
      const baseDay = base.getDate();
      const baseMonth = base.getMonth();
      let targetDay = baseDay;
      if (baseMonth === 1 && baseDay === 29 && !isLeap(d.getFullYear())) targetDay = 28; // 2/29 平年显示 2/28（Bug 6）
      return d.getMonth() === baseMonth && d.getDate() === targetDay;
    }
    default: return todoDayKey(t) === dayKey;
  }
}

// ---- 拖拽排序 helper ----
function sortByOrder(list: Todo[]): Todo[] {
  return [...list].sort((a, b) => a.sort_order - b.sort_order);
}
/** 未完成在上、已完成沉底（同组内保持 sort_order） */
function sortActiveFirst(list: Todo[], dayKey: string): Todo[] {
  return [...list].sort((a, b) => {
    const da = todoDoneOnDay(a, dayKey) ? 1 : 0;
    const db = todoDoneOnDay(b, dayKey) ? 1 : 0;
    if (da !== db) return da - db;
    return a.sort_order - b.sort_order;
  });
}
function parseKey(dayKey: string): Date {
  return new Date(dayKey + "T00:00:00");
}
function computeMoveDate(
  t: Todo,
  newDayKey: string
): { due_at: string | null; due_date: string | null; all_day: boolean } {
  if (t.due_at) {
    const d = new Date(t.due_at);
    const [y, m, dd] = newDayKey.split("-").map(Number);
    return {
      due_at: new Date(y, m - 1, dd, d.getHours(), d.getMinutes()).toISOString(),
      due_date: newDayKey,
      all_day: false,
    };
  }
  return { due_at: null, due_date: newDayKey, all_day: true };
}

function lunarInfo(y: number, m: number, d: number) { const r = solarLunar.solar2lunar(y, m, d); return { term: (r.term as string) || "", festival: (r.festival as string) || "", lFestival: (r.lFestival as string) || "", day: r.dayCn as string }; }
function isFestival(y: number, m: number, d: number): boolean { const r = solarLunar.solar2lunar(y, m, d); return !!(r.festival || r.lFestival); }

function formatPomo(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${pad(Math.floor(s / 60))}:${pad(s % 60)}`;
}

function usePomodoro() {
  const [snap, setSnap] = useState<PomoSnap>(IDLE_POMO);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    (async () => {
      try {
        const s = await getPomodoro();
        if (active) setSnap(s);
      } catch { /* ignore */ }
      if (!IS_TAURI) return;
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<PomoSnap>("pomodoro-update", (e) => setSnap(e.payload));
      } catch { /* ignore */ }
    })();
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
  const toggle = useCallback(async () => {
    try { setSnap(await togglePomodoro()); } catch { /* ignore */ }
  }, []);
  const reset = useCallback(async () => {
    try { setSnap(await resetPomodoro()); } catch { /* ignore */ }
  }, []);
  const applyDurations = useCallback(async (focusMin: number, breakMin: number) => {
    try { setSnap(await setPomodoroDurations(focusMin, breakMin)); } catch { /* ignore */ }
  }, []);
  return { snap, toggle, reset, applyDurations };
}

function PomoMinInput({
  value,
  min,
  max,
  onCommit,
}: {
  value: number;
  min: number;
  max: number;
  onCommit: (n: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => { setDraft(String(value)); }, [value]);
  const commit = () => {
    const n = parseInt(draft, 10);
    const next = Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : value;
    setDraft(String(next));
    if (next !== value) onCommit(next);
  };
  return (
    <span className="settings-num-wrap">
      <input
        className="settings-num"
        type="number"
        min={min}
        max={max}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            (e.target as HTMLInputElement).blur();
          }
        }}
      />
      <span className="settings-num-unit">分</span>
    </span>
  );
}

function PomoChip({ snap, onToggle, onReset }: { snap: PomoSnap; onToggle: () => void; onReset: () => void }) {
  const done = !snap.running && snap.remaining_ms < 400 && snap.phase !== "idle";
  const title = snap.running
    ? (snap.phase === "break" ? "暂停休息" : "暂停专注")
    : done
      ? (snap.phase === "focus" ? "开始休息" : "开始专注")
      : snap.phase === "idle"
        ? "开始专注"
        : "继续";
  return (
    <button
      type="button"
      className={`pomo-chip ${snap.running ? "running" : ""} ${snap.phase === "break" ? "break" : ""} ${snap.phase === "idle" && !snap.running ? "idle" : ""} ${done ? "done" : ""}`}
      title={`${title}（右键重置）`}
      onClick={(e) => { e.stopPropagation(); onToggle(); }}
      onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onReset(); }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {formatPomo(snap.remaining_ms)}
    </button>
  );
}

/** FR-09 自然语句解析 */
function parseNatural(input: string): { title: string; due_at: string | null; due_date: string | null } | null {
  const now = new Date();
  const results = chrono.casual.parse(input, now, { forwardDate: true });
  if (results.length === 0) return null;
  const r = results[0];
  const parsed = r.start; // ParsedComponents
  const hasTime = parsed.isCertain("hour");
  const date = parsed.date(); // JS Date（已按用户本地时区解析）
  const y = date.getFullYear();
  const m = date.getMonth() + 1;
  const d = date.getDate();
  let title = input.slice(0, r.index) + input.slice(r.index + r.text.length);
  title = title.trim().replace(/\s+/g, " ");
  if (!title) title = input.trim();
  return {
    title,
    due_at: hasTime ? new Date(y, m - 1, d, date.getHours(), date.getMinutes()).toISOString() : null,
    due_date: ymd(new Date(y, m - 1, d)),
  };
}

/** 生成月视图网格 */
function buildMonthGrid(year: number, month: number): Date[][] {
  const firstDay = new Date(year, month - 1, 1);
  const startDow = firstDay.getDay();
  // 从该月第一周的周日开始
  const gridStart = addDays(firstDay, -startDow);
  const weeks: Date[][] = [];
  for (let w = 0; w < 6; w++) {
    const row: Date[] = [];
    for (let d = 0; d < 7; d++) {
      row.push(addDays(gridStart, w * 7 + d));
    }
    weeks.push(row);
  }
  return weeks;
}

export default function App() {
  const saved = useMemo(() => loadSettings(), []);
  const [view, setView] = useState<View>(() =>
    saved.view === "day" || saved.view === "week" || saved.view === "month" ? saved.view : "week"
  );
  const [todos, setTodos] = useState<Todo[]>([]);
  const [loading, setLoading] = useState(true);
  const [collapsed, setCollapsed] = useState(() => !!saved.collapsed);
  const [locked, setLocked] = useState(() => !!saved.locked); // FR-16 锁定位置
  const [alwaysOnTop, setAlwaysOnTop] = useState(() => !!saved.alwaysOnTop); // 窗口置顶
  const [rollOver, setRollOver] = useState<boolean>(() =>
    typeof saved.rollOver === "boolean" ? saved.rollOver : true
  ); // 未完成顺延到次日（默认开启）
  const [pomoFocusMin, setPomoFocusMin] = useState(() => clampPomoMin(saved.pomoFocusMin, 1, 180, 25));
  const [pomoBreakMin, setPomoBreakMin] = useState(() => clampPomoMin(saved.pomoBreakMin, 1, 60, 5));
  const [settingsOpen, setSettingsOpen] = useState(false);
  const rollOverRef = useRef(rollOver); // 始终持有最新顺延开关，供定时器同步读取
  const expandedSizeRef = useRef({
    w: typeof saved.expandedW === "number" && saved.expandedW > 200 ? saved.expandedW : EXPANDED_W,
    h: typeof saved.expandedH === "number" && saved.expandedH > 120 ? saved.expandedH : EXPANDED_H,
  });
  const sizeBootstrapped = useRef(false);
  const settingsReady = useRef(!IS_TAURI);
  const pomo = usePomodoro();

  const [cursor, setCursor] = useState<Date>(() => { const d = new Date(); d.setHours(0, 0, 0, 0); return d; });
  const cursorRef = useRef(cursor);
  useEffect(() => { cursorRef.current = cursor; }, [cursor]);

  // ---- 新建待办：按天跟踪（周视图中每列独立）----
  const [addingDay, setAddingDay] = useState<string | null>(null); // ymd 或 null
  const [newTitle, setNewTitle] = useState("");
  const [parsedPreview, setParsedPreview] = useState<string | null>(null);
  const [newRepeat, setNewRepeat] = useState<RepeatVal>("none");
  const [newRepeatUntil, setNewRepeatUntil] = useState<string | null>(null);
  const [newNote, setNewNote] = useState("");
  const [newHour, setNewHour] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const addInputRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => { const list = await getTodos(); setTodos(list.filter((t) => !t.deleted)); }, []);

  // 顺延：开启后，未完成的「不重复」待办会在跨天（due_date < 今天）时自动移到当天
  const lastRollRef = useRef<string>("");
  const runRollover = useCallback(async () => {
    if (!rollOverRef.current) return;
    const today = ymd(new Date());
    if (lastRollRef.current === today) return; // 当天已顺延过，幂等
    lastRollRef.current = today;
    try {
      const list = await getTodos();
      const moves = list
        .filter((t) => {
          if (t.deleted || t.done) return false;
          if (t.repeat && t.repeat !== "none") return false; // 重复待办按天生成，不自动顺延
          if (todoDayKey(t) >= today) return false; // 今天及以后不动
          return true;
        })
        .map((t) => {
          const mv = computeMoveDate(t, today); // 保留原时间，移到当天
          return mv ? updateTodo(t.id, mv) : null;
        })
        .filter(Boolean) as Promise<void>[];
      if (moves.length) {
        await Promise.all(moves);
        await refresh();
      }
    } catch (e) { console.warn("顺延失败:", e); }
  }, [refresh]);

  useEffect(() => {
    let active = true;
    (async () => {
      try { await initDb(); } catch (e) { console.warn("initDb 失败:", e); }
      if (!active) return;
      // 本地优先：先显示 SQLite 数据，不让网络状态阻塞首屏。
      try { await refresh(); } catch (e) { console.warn("getTodos 失败:", e); }
      if (!active) return;
      setLoading(false);
      try { await runRollover(); } catch (e) { console.warn("顺延失败:", e); }
    })();
    return () => { active = false; };
  }, [refresh, runRollover]);

  // 启动：先从 Rust 恢复锁定/置顶等，再显示窗口
  useEffect(() => {
    if (!IS_TAURI) return;
    let cancelled = false;
    (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const raw = await invoke<Record<string, unknown>>("get_ui_settings");
        if (cancelled) return;
        const s = settingsFromRust(raw);
        if (typeof s.locked === "boolean") setLocked(s.locked);
        if (typeof s.alwaysOnTop === "boolean") setAlwaysOnTop(s.alwaysOnTop);
        if (typeof s.rollOver === "boolean") {
          setRollOver(s.rollOver);
          rollOverRef.current = s.rollOver;
        }
        if (typeof s.collapsed === "boolean") setCollapsed(s.collapsed);
        if (s.view === "day" || s.view === "week" || s.view === "month") setView(s.view);
        if (typeof s.expandedW === "number" && s.expandedW > 200) expandedSizeRef.current.w = s.expandedW;
        if (typeof s.expandedH === "number" && s.expandedH > 120) expandedSizeRef.current.h = s.expandedH;
        if (typeof s.pomoFocusMin === "number") setPomoFocusMin(s.pomoFocusMin);
        if (typeof s.pomoBreakMin === "number") setPomoBreakMin(s.pomoBreakMin);
        if (typeof s.pomoFocusMin === "number" || typeof s.pomoBreakMin === "number") {
          void pomo.applyDurations(
            s.pomoFocusMin ?? 25,
            s.pomoBreakMin ?? 5,
          );
        }
        try {
          localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...loadSettings(), ...s }));
        } catch { /* ignore */ }
        settingsReady.current = true;
        persistSettings(s);
      } catch {
        settingsReady.current = true;
        persistSettings({
          alwaysOnTop,
          rollOver,
          collapsed,
          locked,
          view,
          expandedW: expandedSizeRef.current.w,
          expandedH: expandedSizeRef.current.h,
          pomoFocusMin,
          pomoBreakMin,
        });
      }
      if (cancelled) return;
      await showWindow().catch(() => {});
    })();
    return () => { cancelled = true; };
  }, []);

  // todos 最新值的引用，供通知检查读取
  const todosRef = useRef(todos);
  useEffect(() => { todosRef.current = todos; }, [todos]);

  // 到期提醒（Bug 9）：当天到期/即将到期且未完成 → 桌面通知（当天只提醒一次）
  const notifiedToday = useRef<Set<string>>(new Set());
  const checkDueNotifications = useCallback(async () => {
    if (!IS_TAURI) return;
    try {
      const { sendNotification, isPermissionGranted, requestPermission } = await import("@tauri-apps/plugin-notification");
      const perm = await isPermissionGranted();
      if (!perm) {
        const p = await requestPermission();
        if (p !== "granted") { console.warn("[Otodo] 通知权限被拒绝:", p); return; }
      }
      const now = new Date();
      const soon = new Date(now.getTime() + 30 * 60 * 1000);
      for (const t of todosRef.current) {
        if (t.deleted || t.done || !t.due_at) continue;
        const dt = new Date(t.due_at);
        const key = `${t.id}-${ymd(dt)}`;
        if (dt >= now && dt <= soon && !notifiedToday.current.has(key)) {
          notifiedToday.current.add(key);
          const body = `${t.title} · ${dt.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}`;
          try {
            await sendNotification({ title: "待办即将到期", body });
          } catch (e) {
            alert(`通知发送失败: ${e}`);
          }
        }
      }
    } catch (e) { alert(`[Otodo] 通知检查出错: ${e}`); }
  }, []);

  // 数据变化后检查通知
  useEffect(() => { checkDueNotifications(); }, [todos, checkDueNotifications]);

  // 每 15 秒检查到期提醒
  useEffect(() => {
    const id = setInterval(() => {
      checkDueNotifications();
    }, 15 * 1000);
    return () => clearInterval(id);
  }, [checkDueNotifications]);

  // 跨天：顺延未完成待办，并把仍停在「昨天」的焦点切到今天
  const lastTodayRef = useRef(ymd(new Date()));
  useEffect(() => {
    const tick = () => {
      const today = ymd(new Date());
      if (today !== lastTodayRef.current) {
        const prevToday = lastTodayRef.current;
        lastTodayRef.current = today;
        if (ymd(cursorRef.current) === prevToday) {
          const d = new Date();
          d.setHours(0, 0, 0, 0);
          setCursor(d);
        }
      }
      void runRollover();
    };
    const id = setInterval(tick, 60 * 1000);
    // 从托盘唤起 / 系统休眠醒来时也立刻检查一次
    const onVisible = () => {
      if (document.visibilityState === "visible") tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", tick);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", tick);
    };
  }, [runRollover]);

  // 加载并应用设置（置顶等）；启动时展开态尺寸交给 Rust 从 window.json 恢复
  useEffect(() => {
    (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        await invoke("set_always_on_top", { onTop: alwaysOnTop });
      } catch { /* browser */ }
    })();
  }, [alwaysOnTop]);

  useEffect(() => {
    if (!settingsReady.current) return;
    persistSettings({
      alwaysOnTop,
      rollOver,
      collapsed,
      locked,
      view,
      expandedW: expandedSizeRef.current.w,
      expandedH: expandedSizeRef.current.h,
    });
  }, [alwaysOnTop, rollOver, collapsed, locked, view]);

  // 收起只压高度、宽度跟展开窗口一致；启动首次展开不强制改尺寸
  useEffect(() => {
    (async () => {
      if (!sizeBootstrapped.current) {
        sizeBootstrapped.current = true;
        if (!collapsed) return;
      }
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const w = Math.max(MIN_WINDOW_W, expandedSizeRef.current.w);
        if (collapsed) {
          await invoke("set_window_size", { width: w, height: COLLAPSED_H });
        } else {
          await invoke("set_window_size", {
            width: w,
            height: expandedSizeRef.current.h,
          });
        }
      } catch { /* browser */ }
    })();
  }, [collapsed]);

  const persistExpandedSize = useCallback((w: number, h: number, nextCollapsed: boolean) => {
    expandedSizeRef.current = { w, h };
    persistSettings({
      alwaysOnTop,
      rollOver,
      collapsed: nextCollapsed,
      locked,
      view,
      expandedW: w,
      expandedH: h,
    });
  }, [alwaysOnTop, rollOver, locked, view]);

  const collapseWindow = useCallback(async () => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const size = await invoke<[number, number]>("get_window_size");
      if (size[0] > 80 && size[1] > 80) {
        persistExpandedSize(size[0], size[1], true);
      }
    } catch { /* browser */ }
    setCollapsed(true);
  }, [persistExpandedSize]);

  const expandWindow = useCallback(async () => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const size = await invoke<[number, number]>("get_window_size");
      if (size[0] > 80) {
        persistExpandedSize(size[0], expandedSizeRef.current.h, false);
      }
    } catch { /* browser */ }
    setCollapsed(false);
  }, [persistExpandedSize]);

  // 点击外部关闭设置弹窗
  useEffect(() => {
    if (!settingsOpen) return;
    const close = () => setSettingsOpen(false);
    setTimeout(() => document.addEventListener("mousedown", close), 0);
    return () => document.removeEventListener("mousedown", close);
  }, [settingsOpen]);

  useEffect(() => {
    const unlistens: Array<() => void> = [];
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        // 用 cursorRef 读取最新 cursor，避免闭包锁死在启动那天的坑（Bug 1）
        unlistens.push(
          await listen("tray-new-todo", () => {
            const day = ymd(cursorRef.current);
            setAddingDay(day);
            setNewTitle("");
            setParsedPreview(null);
            setNewRepeat("none");
            setNewRepeatUntil(null);
            setTimeout(() => addInputRef.current?.focus(), 50);
          })
        );
      } catch { /* browser */ }
    })();
    return () => { unlistens.forEach((u) => u()); };
  }, []);

  const onToggle = async (t: Todo, dayKey?: string) => { const done = dayKey ? todoDoneOnDay(t, dayKey) : t.done; await toggleTodo(t.id, !done, dayKey); await refresh(); };
  const onDelete = async (t: Todo) => { await deleteTodo(t.id); await refresh(); };
  const onEdit = async (id: string, patch: Record<string, unknown>) => { await updateTodo(id, patch as any); await refresh(); };
  // 跳过单次重复：把当天加入 skip_dates（Bug 10）
  const onSkip = async (t: Todo) => {
    const day = ymd(new Date());
    const set = (t.skip_dates ? t.skip_dates.split(" ").filter(Boolean) : [] as string[]);
    if (!set.includes(day)) set.push(day);
    await updateTodo(t.id, { skip_dates: set.join(" ") });
    await refresh();
  };

  // ---- 卡片拖拽（列内重排序 / 周视图跨天移动）----
  const [dragTodo, setDragTodo] = useState<{ id: string; dayKey: string } | null>(null);

  const onTodoDragStart = useCallback(
    (e: React.DragEvent, t: Todo, dayKey: string) => {
      if (locked) { e.preventDefault(); return; }
      e.stopPropagation();
      setDragTodo({ id: t.id, dayKey });
      e.dataTransfer.effectAllowed = "move";
      try { e.dataTransfer.setData("text/plain", t.id); } catch { /* ignore */ }
    },
    [locked]
  );

  const onTodoDrop = useCallback(
    async (targetDayKey: string, beforeId: string | null) => {
      if (!dragTodo) return;
      const dragged = todos.find((t) => t.id === dragTodo.id);
      if (!dragged) { setDragTodo(null); return; }
      if (dragged.id === beforeId) { setDragTodo(null); return; }

      const isCrossDay = dragTodo.dayKey !== targetDayKey;
      // 重复待办按天跟踪，禁止跨天移动（仅允许同列重排）
      if (isCrossDay && dragged.repeat && dragged.repeat !== "none") {
        setDragTodo(null);
        return;
      }

      const colAll = todos.filter((t) => matchesRepeat(t, targetDayKey));
      const targetList = sortByOrder(colAll).filter((t) => t.id !== dragged.id);
      const idx = beforeId == null ? targetList.length : targetList.findIndex((t) => t.id === beforeId);
      const insertAt = idx < 0 ? targetList.length : idx;
      targetList.splice(insertAt, 0, dragged);

      if (isCrossDay) {
        const mv = computeMoveDate(dragged, targetDayKey);
        await updateTodo(dragged.id, { due_at: mv.due_at, due_date: mv.due_date, all_day: mv.all_day });
      }
      // 一次调用完成整列重排，避免逐条串行请求（Bug 8）
      await reorderTodos(targetList.map((t, i) => ({ id: t.id, sort_order: i })));
      await refresh();
      setDragTodo(null);
    },
    [dragTodo, todos, refresh]
  );

  // ---- 新建 ----
  const startAdd = useCallback((dayKey: string, hour?: number | null) => {
    setAddingDay(dayKey);
    setNewTitle("");
    setParsedPreview(null);
    setNewRepeat("none");
    setNewRepeatUntil(null);
    setNewNote("");
    setNewHour(typeof hour === "number" ? hour : null);
    setTimeout(() => addInputRef.current?.focus(), 50);
  }, []);

  const handleNewTitleChange = (val: string) => {
    setNewTitle(val);
    if (val.trim().length > 2) {
      const parsed = parseNatural(val);
      if (parsed) {
        const fmt = parsed.due_at ? `${parsed.due_date} ${new Date(parsed.due_at).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}` : parsed.due_date;
        setParsedPreview(`${fmt} · ${parsed.title}`);
      } else { setParsedPreview(null); }
    } else { setParsedPreview(null); }
  };

  const resetAddForm = () => {
    setNewTitle("");
    setParsedPreview(null);
    setAddingDay(null);
    setNewRepeat("none");
    setNewRepeatUntil(null);
    setNewNote("");
    setNewHour(null);
  };

  const commitAdd = async () => {
    const title = newTitle.trim();
    if (!title || !addingDay) return;
    try {
      const parsed = parseNatural(title);
      let dueAt = parsed?.due_at || null;
      let allDay = parsed ? !parsed.due_at : true;
      if (!dueAt && newHour != null) {
        const [yy, mm, dd] = addingDay.split("-").map(Number);
        dueAt = new Date(yy, mm - 1, dd, newHour, 0, 0, 0).toISOString();
        allDay = false;
      }
      await addTodo({
        title: parsed?.title || title,
        due_at: dueAt,
        due_date: parsed?.due_date || addingDay,
        all_day: allDay,
        note: newNote.trim(),
        repeat: newRepeat,
        repeat_until: newRepeatUntil,
      });
    } catch (err) {
      console.error("[Otodo] 新建待办失败:", err);
      alert(`新建失败: ${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    resetAddForm();
    await refresh();
  };

  // ---- 锁定位置 (FR-16) ----
  const toggleLock = () => {
    setLocked((v) => {
      const next = !v;
      persistSettings({
        alwaysOnTop,
        rollOver,
        collapsed,
        locked: next,
        view,
        expandedW: expandedSizeRef.current.w,
        expandedH: expandedSizeRef.current.h,
      });
      return next;
    });
  };

  // ---- 未完成顺延开关 ----
  const toggleRollOver = () => {
    const next = !rollOver;
    rollOverRef.current = next; // 立即同步，便于本次切换立即触发顺延
    setRollOver(next);
    if (next) runRollover();
  };

  // ---- 视图数据 ----
  const todayYmd = ymd(new Date());
  const todosOnDay = (d: Date) => todos.filter((t) => matchesRepeat(t, ymd(d)));

  // 周视图
  const weekStart = useMemo(() => startOfWeekMonday(cursor), [cursor]);
  const weekDays = useMemo(() => Array.from({ length: 7 }, (_, i) => addDays(weekStart, i)), [weekStart]);

  // 月视图
  const [monthYear, setMonthYear] = useState(() => ({ y: cursor.getFullYear(), m: cursor.getMonth() + 1 }));
  const monthGrid = useMemo(() => buildMonthGrid(monthYear.y, monthYear.m), [monthYear]);

  const navLabel = (() => {
    if (view === "month") return `${monthYear.y}年${pad(monthYear.m)}月`;
    return `${cursor.getFullYear()}年${pad(cursor.getMonth() + 1)}月`;
  })();

  const stepMonth = (dir: number) => {
    const m = monthYear.m + dir;
    const y = monthYear.y + (m > 12 ? 1 : m < 1 ? -1 : 0);
    const nm = ((m - 1 + 12) % 12) + 1;
    setMonthYear({ y, m: nm });
    // 尽量保留原「日」，月末不足则夹到当月最后一天
    const maxDay = daysInMonth(`${y}-${pad(nm)}-01`);
    const day = Math.min(cursor.getDate(), maxDay);
    const next = new Date(y, nm - 1, day);
    next.setHours(0, 0, 0, 0);
    setCursor(next);
  };

  const stepWeek = (dir: number) => {
    if (view === "month") {
      stepMonth(dir);
    } else {
      setCursor((c) => addDays(c, dir * (view === "week" ? 7 : 1)));
    }
  };

  // ---- 拖拽窗口（仅顶栏/日期头等 chrome；内容区禁止，否则会吞掉双击添加）----
  const onDragStart = useCallback(async (e: React.MouseEvent) => {
    if (locked) return;
    if ((e.target as HTMLElement).closest("button, input, a, select, textarea, .content-area, .week-columns, .day-view, .month-grid, .todo-card, .add-form, .week-add-pop, .day-add-pop, .month-add-pop, .settings-popover, .search-results, .pomo-chip")) return;
    try { const { invoke } = await import("@tauri-apps/api/core"); await invoke("drag_window"); } catch { /* browser */ }
  }, [locked]);

  // ---- FR-18 收起态 ----
  if (collapsed) {
    return (
      <CollapsedBar
        todos={todos}
        locked={locked}
        pomo={pomo.snap}
        onTogglePomo={pomo.toggle}
        onResetPomo={pomo.reset}
        onToggleLock={toggleLock}
        onExpand={() => { void expandWindow(); }}
        onClose={() => hideWindow()}
        onDragStart={onDragStart}
      />
    );
  }

  return (
    <div className="app">
      {/* === 顶栏 === */}
      <header className="topbar" onMouseDown={onDragStart}>
        <span className="topbar-brand">Otodo</span>
        <span className="topbar-title">{navLabel}</span>
        <div className="topbar-right">
          <PomoChip snap={pomo.snap} onToggle={pomo.toggle} onReset={pomo.reset} />
          <div className={`search-wrap ${searchOpen || search ? "open" : ""}`}>
            <button
              type="button"
              className={`search-btn ${searchOpen || search ? "active" : ""}`}
              title="搜索"
              onClick={(e) => {
                e.stopPropagation();
                setSearchOpen((v) => {
                  const next = !v;
                  if (next) setTimeout(() => searchInputRef.current?.focus(), 30);
                  else setSearch("");
                  return next;
                });
              }}
            >
              <IconSearch />
            </button>
            {(searchOpen || search) && (
              <input
                ref={searchInputRef}
                className="search-input"
                placeholder="搜索待办"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") { setSearch(""); setSearchOpen(false); }
                }}
                onMouseDown={(e) => e.stopPropagation()}
              />
            )}
          </div>
          <button className={`lock-btn ${locked ? "active" : ""}`} title={locked ? "解除锁定：可再次拖动窗口" : "锁定位置：禁止拖动窗口"} onClick={toggleLock}><IconLock locked={locked} /></button>
          <div className="settings-wrap">
            <button className={`settings-btn ${settingsOpen ? "active" : ""}`} title="设置" onClick={(e) => { e.stopPropagation(); setSettingsOpen((v) => !v); }}><IconSettings /></button>
            {settingsOpen && (
              <div className="settings-popover" onMouseDown={(e) => e.stopPropagation()}>
                <div className="settings-row">
                  <span className="settings-label">窗口置顶</span>
                  <button
                    className={`toggle ${alwaysOnTop ? "on" : ""}`}
                    onClick={() => setAlwaysOnTop((v) => !v)}
                  >
                    {alwaysOnTop ? "开" : "关"}
                  </button>
                </div>
                <div className="settings-row">
                  <span className="settings-label" title="未完成的「不重复」待办会在次日自动移到当天">未完成顺延</span>
                  <button
                    className={`toggle ${rollOver ? "on" : ""}`}
                    onClick={toggleRollOver}
                  >
                    {rollOver ? "开" : "关"}
                  </button>
                </div>
                <div className="settings-row">
                  <span className="settings-label">专注时长</span>
                  <PomoMinInput
                    value={pomoFocusMin}
                    min={1}
                    max={180}
                    onCommit={(n) => {
                      setPomoFocusMin(n);
                      persistSettings({ pomoFocusMin: n, pomoBreakMin });
                      void pomo.applyDurations(n, pomoBreakMin);
                    }}
                  />
                </div>
                <div className="settings-row">
                  <span className="settings-label">休息时长</span>
                  <PomoMinInput
                    value={pomoBreakMin}
                    min={1}
                    max={60}
                    onCommit={(n) => {
                      setPomoBreakMin(n);
                      persistSettings({ pomoFocusMin, pomoBreakMin: n });
                      void pomo.applyDurations(pomoFocusMin, n);
                    }}
                  />
                </div>
              </div>
            )}
          </div>
          <div className="view-switch">
            <button className={`view-btn ${view === "day" ? "active" : ""}`} onClick={() => setView("day")}>日</button>
            <button className={`view-btn ${view === "week" ? "active" : ""}`} onClick={() => setView("week")}>周</button>
            <button
              className={`view-btn ${view === "month" ? "active" : ""}`}
              onClick={() => {
                setMonthYear({ y: cursor.getFullYear(), m: cursor.getMonth() + 1 });
                setView("month");
              }}
            >月</button>
          </div>
          <button className="collapse-btn" title="收起" onClick={() => { void collapseWindow(); }}>▴</button>
          <button className="close-btn" title="关闭（最小化到托盘）" onClick={() => hideWindow()}><IconClose /></button>
        </div>
      </header>

      {/* 全局搜索结果面板（Bug 10：缺失的搜索能力） */}
      {search.trim() && (
        <div className="search-results">
          {todos
            .filter((t) => t.title.toLowerCase().includes(search.trim().toLowerCase()))
            .map((t) => (
              <div
                key={t.id}
                className="search-result"
                onClick={() => { const k = todoDayKey(t); setCursor(parseKey(k)); setView("day"); setSearch(""); }}
              >
                <span className="search-result-title">{t.title}</span>
                <span className="search-result-date">{todoDayKey(t)}</span>
              </div>
            ))}
          {todos.filter((t) => t.title.toLowerCase().includes(search.trim().toLowerCase())).length === 0 && (
            <div className="search-empty">无匹配</div>
          )}
        </div>
      )}

      {/* ====== 日视图 / 周视图 ====== */}
      {(view === "day" || view === "week") && (
        <>
          {/* 日期头行 */}
          <div className="date-row" onMouseDown={onDragStart}>
            <button className="page-arrow page-arrow-left" onClick={() => stepWeek(-1)} onMouseDown={(e) => e.stopPropagation()}>‹</button>
            <div className="date-grid" style={{ gridTemplateColumns: `repeat(${view === "week" ? 7 : 1}, 1fr)` }}>
              {(view === "week" ? weekDays : [cursor]).map((day) => {
                const key = ymd(day);
                const isToday = key === todayYmd;
                const isWeekend = day.getDay() === 0 || day.getDay() === 6;
                const info = lunarInfo(day.getFullYear(), day.getMonth() + 1, day.getDate());
                const hasFest = isFestival(day.getFullYear(), day.getMonth() + 1, day.getDate());
                const displayText = info.term || info.festival || info.lFestival || info.day;
                const label = view === "week" ? WEEK_LABELS[(day.getDay() + 6) % 7] : `${day.getMonth() + 1}月${day.getDate()}日`;
                return (
                  <div key={key} className={`date-col ${isWeekend ? "weekend" : ""}`} onClick={() => setCursor(day)}>
                    <span className="weekday-label">{label}</span>
                    <div className={`date-cell ${isToday ? "today" : ""} ${key === ymd(cursor) ? "focused" : ""}`}>
                      <span className="day-num">{day.getDate()}</span>
                      <span className={`lunar-text ${hasFest && !info.term ? "festival" : ""} ${info.term ? "term" : ""}`}>{displayText}</span>
                    </div>
                  </div>
                );
              })}
            </div>
            <button className="page-arrow page-arrow-right" onClick={() => stepWeek(1)} onMouseDown={(e) => e.stopPropagation()}>›</button>
          </div>

          {/* 内容区：阻止冒泡到窗口拖拽，保证双击添加可用 */}
          <div className="content-area" onMouseDown={(e) => e.stopPropagation()}>
            {loading && <div className="empty-state"><div className="empty-hint">加载中…</div></div>}
            {!loading && view === "day" && <DayContent day={cursor} todos={todosOnDay(cursor)} adding={addingDay === ymd(cursor)} newTitle={newTitle} newNote={newNote} newHour={newHour} parsedPreview={parsedPreview} addInputRef={addInputRef} onStartAdd={(hour) => startAdd(ymd(cursor), hour)} onTitleChange={handleNewTitleChange} onNoteChange={setNewNote} onCommit={commitAdd} onCancel={resetAddForm} onToggle={onToggle} onDelete={onDelete} onEdit={onEdit} onSkip={onSkip} repeat={newRepeat} onRepeatChange={setNewRepeat} repeatUntil={newRepeatUntil} onRepeatUntilChange={setNewRepeatUntil} draggingId={dragTodo?.id ?? null} onTodoDragStart={onTodoDragStart} onTodoDrop={onTodoDrop} />}
            {!loading && view === "week" && <WeekContent weekDays={weekDays} todosOnDay={todosOnDay} addingDay={addingDay} newTitle={newTitle} newNote={newNote} parsedPreview={parsedPreview} addInputRef={addInputRef} startAdd={startAdd} onTitleChange={handleNewTitleChange} onNoteChange={setNewNote} onCommit={commitAdd} onCancel={resetAddForm} onToggle={onToggle} onDelete={onDelete} onEdit={onEdit} onSkip={onSkip} repeat={newRepeat} onRepeatChange={setNewRepeat} repeatUntil={newRepeatUntil} onRepeatUntilChange={setNewRepeatUntil} draggingId={dragTodo?.id ?? null} onTodoDragStart={onTodoDragStart} onTodoDrop={onTodoDrop} />}
          </div>
        </>
      )}

      {/* ====== 月视图 ====== */}
      {view === "month" && (
        <div className="month-view">
          <div className="month-nav" onMouseDown={onDragStart}>
            <button
              className="page-arrow"
              title="上一年"
              onClick={() => stepMonth(-12)}
              onMouseDown={(e) => e.stopPropagation()}
            >«</button>
            <button
              className="page-arrow"
              title="上一月"
              onClick={() => stepMonth(-1)}
              onMouseDown={(e) => e.stopPropagation()}
            >‹</button>
            <span className="month-nav-label">{monthYear.y}年{pad(monthYear.m)}月</span>
            <button
              className="page-arrow"
              title="下一月"
              onClick={() => stepMonth(1)}
              onMouseDown={(e) => e.stopPropagation()}
            >›</button>
            <button
              className="page-arrow"
              title="下一年"
              onClick={() => stepMonth(12)}
              onMouseDown={(e) => e.stopPropagation()}
            >»</button>
          </div>
          <div className="month-grid">
            {MONTH_HEAD.map((h, hi) => (
              <div key={h} className={`month-head-cell ${hi === 0 || hi === 6 ? "weekend" : ""}`}>{h}</div>
            ))}
            {monthGrid.flatMap((row, ri) => row.map((day) => {
              const key = ymd(day);
              const isToday = key === todayYmd;
              const isFocused = key === ymd(cursor);
              const isWeekend = day.getDay() === 0 || day.getDay() === 6;
              const inMonth = day.getMonth() + 1 === monthYear.m;
              const info = lunarInfo(day.getFullYear(), day.getMonth() + 1, day.getDate());
              const hasFest = isFestival(day.getFullYear(), day.getMonth() + 1, day.getDate());
              const displayText = info.term || info.festival || info.lFestival || info.day;
              const dayTodos = todosOnDay(day);
              const isAdding = addingDay === key;
              const popAbove = ri >= 3;
              return (
                <div
                  key={key}
                  className={`month-cell ${!inMonth ? "other-month" : ""} ${isToday ? "today" : ""} ${isFocused ? "focused" : ""} ${isWeekend ? "weekend" : ""}`}
                  onDoubleClick={() => { setCursor(day); setView("day"); }}
                  onClick={() => setCursor(day)}
                >
                  <button className="month-cell-add" title="在此日添加待办" onClick={(e) => { e.stopPropagation(); startAdd(key); }}>+</button>
                  <div className="month-cell-header">
                    <span className="month-day-num">{day.getDate()}</span>
                    <span className={`month-lunar ${hasFest && !info.term ? "festival" : ""} ${info.term ? "term" : ""}`}>{displayText}</span>
                  </div>
                  <div className="month-todos">
                    {dayTodos.map((t) => {
                      const done = todoDoneOnDay(t, key);
                      return (
                        <div key={t.id} className={`month-todo-item ${done ? "done" : ""}`} onClick={(e) => { e.stopPropagation(); onToggle(t, key); }}>
                          <span className="month-todo-dot" style={{ background: done ? "#bbb" : "#2f6f6a" }} />
                          <span className="month-todo-title">{t.title}</span>
                        </div>
                      );
                    })}
                  </div>
                  {isAdding && (
                    <div className={`month-add-pop ${popAbove ? "above" : "below"}`} onClick={(e) => e.stopPropagation()}>
                      <AddForm inputRef={addInputRef} value={newTitle} onChange={handleNewTitleChange} note={newNote} onNoteChange={setNewNote} onCommit={commitAdd} onCancel={resetAddForm} preview={parsedPreview} compact repeat={newRepeat} onRepeatChange={setNewRepeat} repeatUntil={newRepeatUntil} onRepeatUntilChange={setNewRepeatUntil} />
                    </div>
                  )}
                </div>
              );
            }))}
          </div>
        </div>
      )}
    </div>
  );
}

/* =====================================================
   日视图内容 ── 全天带 + 0–24h 小时轴
   ===================================================== */
const DAY_HOUR_H = 48;

function DayContent({ day, todos, adding, newTitle, newNote, newHour, parsedPreview, addInputRef, onStartAdd, onTitleChange, onNoteChange, onCommit, onCancel, onToggle, onDelete, onEdit, onSkip, repeat, onRepeatChange, repeatUntil, onRepeatUntilChange, draggingId, onTodoDragStart, onTodoDrop }: {
  day: Date; todos: Todo[]; adding: boolean; newTitle: string; newNote: string; newHour: number | null; parsedPreview: string | null;
  addInputRef: React.RefObject<HTMLInputElement>; onStartAdd: (hour?: number | null) => void; onTitleChange: (v: string) => void; onNoteChange: (v: string) => void; onCommit: () => void; onCancel: () => void;
  onToggle: (t: Todo, dayKey?: string) => void; onDelete: (t: Todo) => void; onEdit: (id: string, p: Record<string, unknown>) => void; onSkip: (t: Todo) => void;
  repeat: RepeatVal; onRepeatChange: (v: RepeatVal) => void; repeatUntil: string | null; onRepeatUntilChange: (v: string | null) => void;
  draggingId: string | null; onTodoDragStart: (e: React.DragEvent, t: Todo, dayKey: string) => void; onTodoDrop: (targetDayKey: string, beforeId: string | null) => void;
}) {
  const dayKey = ymd(day);
  const allDayTodos = sortActiveFirst(todos.filter(isAllDay), dayKey);
  const timedTodos = todos.filter((t) => !isAllDay(t));
  const timelineRef = useRef<HTMLDivElement>(null);
  const isToday = ymd(day) === ymd(new Date());
  const nowTop = (() => {
    if (!isToday) return null;
    const n = new Date();
    return ((n.getHours() * 60 + n.getMinutes()) / 60) * DAY_HOUR_H;
  })();

  useEffect(() => {
    const el = timelineRef.current;
    if (!el) return;
    if (adding && newHour != null) {
      el.scrollTop = Math.max(0, newHour * DAY_HOUR_H - 24);
      return;
    }
    const focusHour = isToday ? new Date().getHours() : 8;
    el.scrollTop = Math.max(0, focusHour * DAY_HOUR_H - 40);
  }, [dayKey, isToday, adding, newHour]);

  const addingTimed = adding && newHour != null;
  const addingAllDay = adding && newHour == null;

  return (
    <div className="day-view">
      {!adding && (
        <button type="button" className="add-top-btn" onClick={() => onStartAdd()}>+ 添加待办</button>
      )}
      {addingAllDay && (
        <AddForm
          inputRef={addInputRef}
          value={newTitle}
          onChange={onTitleChange}
          note={newNote}
          onNoteChange={onNoteChange}
          onCommit={onCommit}
          onCancel={onCancel}
          preview={parsedPreview}
          repeat={repeat}
          onRepeatChange={onRepeatChange}
          repeatUntil={repeatUntil}
          onRepeatUntilChange={onRepeatUntilChange}
        />
      )}

      <div
        className="day-allday"
        onMouseDown={(e) => e.stopPropagation()}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => { e.preventDefault(); onTodoDrop(dayKey, null); }}
      >
        <div className="day-allday-label">全天</div>
        <div className="day-allday-list">
          {allDayTodos.map((t) => (
            <TodoCard
              key={t.id}
              t={t}
              draggable
              dragging={draggingId === t.id}
              dayKey={dayKey}
              onDragStartTodo={onTodoDragStart}
              onDropAt={(id) => onTodoDrop(dayKey, id)}
              onToggle={onToggle}
              onDelete={onDelete}
              onEdit={onEdit}
              onSkip={onSkip}
            />
          ))}
          {allDayTodos.length === 0 && !adding && (
            <button type="button" className="day-allday-empty" onClick={() => onStartAdd()}>
              ＋ 添加全天待办
            </button>
          )}
        </div>
      </div>

      <div
        className="day-timeline"
        ref={timelineRef}
        onMouseDown={(e) => e.stopPropagation()}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => { e.preventDefault(); onTodoDrop(dayKey, null); }}
      >
        <div className="day-hours" style={{ height: 24 * DAY_HOUR_H }}>
          {Array.from({ length: 24 }, (_, h) => (
            <div
              key={h}
              className={`day-hour-row ${addingTimed && newHour === h ? "active" : ""}`}
              style={{ height: DAY_HOUR_H }}
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                onStartAdd(h);
              }}
            >
              <span className="day-hour-label">{`${pad(h)}:00`}</span>
              <div className="day-hour-track">
                {!adding && timedTodos.length === 0 && h === 9 && (
                  <span className="day-hour-hint">单击时间格添加</span>
                )}
              </div>
            </div>
          ))}
          {nowTop != null && <div className="day-now-line" style={{ top: nowTop }} />}
          <div className="day-timed-events">
            {timedTodos.map((t) => {
              const d = new Date(t.due_at!);
              const top = ((d.getHours() * 60 + d.getMinutes()) / 60) * DAY_HOUR_H;
              return (
                <div key={t.id} className="day-timed-item" style={{ top, minHeight: DAY_HOUR_H * 0.72 }}>
                  <TodoCard
                    t={t}
                    draggable
                    dragging={draggingId === t.id}
                    dayKey={dayKey}
                    onDragStartTodo={onTodoDragStart}
                    onDropAt={(id) => onTodoDrop(dayKey, id)}
                    onToggle={onToggle}
                    onDelete={onDelete}
                    onEdit={onEdit}
                    onSkip={onSkip}
                    hideTime
                  />
                </div>
              );
            })}
          </div>
          {addingTimed && (
            <div
              className="day-add-pop"
              style={{ top: Math.min(newHour! * DAY_HOUR_H + 4, 24 * DAY_HOUR_H - 140) }}
              onClick={(e) => e.stopPropagation()}
              onDoubleClick={(e) => e.stopPropagation()}
            >
              <AddForm
                inputRef={addInputRef}
                value={newTitle}
                onChange={onTitleChange}
                onCommit={onCommit}
                onCancel={onCancel}
                preview={parsedPreview}
                compact
                hourHint={newHour}
                repeat={repeat}
                onRepeatChange={onRepeatChange}
                repeatUntil={repeatUntil}
                onRepeatUntilChange={onRepeatUntilChange}
              />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/* =====================================================
   收起态横条（含实时时钟）
   ===================================================== */
function CollapsedBar({
  todos, locked, pomo, onTogglePomo, onResetPomo, onToggleLock, onExpand, onClose, onDragStart,
}: {
  todos: Todo[];
  locked: boolean;
  pomo: PomoSnap;
  onTogglePomo: () => void;
  onResetPomo: () => void;
  onToggleLock: () => void;
  onExpand: () => void;
  onClose: () => void;
  onDragStart: (e: React.MouseEvent) => void;
}) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const tick = () => setNow(new Date());
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, []);

  const info = lunarInfo(now.getFullYear(), now.getMonth() + 1, now.getDate());
  const displayText = info.term || info.festival || info.lFestival || info.day;
  const dow = ["日", "一", "二", "三", "四", "五", "六"][now.getDay()];
  const allCount = todos.filter((t) => !t.done).length;
  const timeText = now.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  return (
    <div className="app collapsed-bar" onMouseDown={onDragStart}>
      <span className="collapsed-info">
        {now.getMonth() + 1}月{now.getDate()}日 周{dow} · {displayText} · {timeText} · {allCount}个待办
      </span>
      <div className="collapsed-actions">
        <PomoChip snap={pomo} onToggle={onTogglePomo} onReset={onResetPomo} />
        <button
          className={`lock-btn mini ${locked ? "active" : ""}`}
          title={locked ? "解除锁定：可再次拖动窗口" : "锁定位置：禁止拖动窗口"}
          onClick={(e) => { e.stopPropagation(); onToggleLock(); }}
        >
          <IconLock locked={locked} />
        </button>
        <button className="expand-btn" onClick={onExpand}>▾</button>
        <button className="close-btn" title="关闭（最小化到托盘）" onClick={onClose}><IconClose /></button>
      </div>
    </div>
  );
}

/* =====================================================
   周视图内容 ── 7 列，每天显示自己的待办
   ===================================================== */
function WeekContent({ weekDays, todosOnDay, addingDay, newTitle, newNote, parsedPreview, addInputRef, startAdd, onTitleChange, onNoteChange, onCommit, onCancel, onToggle, onDelete, onEdit, onSkip, repeat, onRepeatChange, repeatUntil, onRepeatUntilChange, draggingId, onTodoDragStart, onTodoDrop }: {
  weekDays: Date[]; todosOnDay: (d: Date) => Todo[]; addingDay: string | null; newTitle: string; newNote: string; parsedPreview: string | null;
  addInputRef: React.RefObject<HTMLInputElement>; startAdd: (key: string, hour?: number | null) => void; onTitleChange: (v: string) => void; onNoteChange: (v: string) => void; onCommit: () => void; onCancel: () => void;
  onToggle: (t: Todo, dayKey?: string) => void; onDelete: (t: Todo) => void; onEdit: (id: string, p: Record<string, unknown>) => void; onSkip: (t: Todo) => void;
  repeat: RepeatVal; onRepeatChange: (v: RepeatVal) => void; repeatUntil: string | null; onRepeatUntilChange: (v: string | null) => void;
  draggingId: string | null; onTodoDragStart: (e: React.DragEvent, t: Todo, dayKey: string) => void; onTodoDrop: (targetDayKey: string, beforeId: string | null) => void;
}) {
  return (
    <div className="week-columns">
      {weekDays.map((day) => (
        <WeekDayColumn
          key={ymd(day)}
          day={day}
          list={todosOnDay(day)}
          isAdding={addingDay === ymd(day)}
          newTitle={newTitle}
          parsedPreview={parsedPreview}
          addInputRef={addInputRef}
          startAdd={startAdd}
          onTitleChange={onTitleChange}
          onCommit={onCommit}
          onCancel={onCancel}
          onToggle={onToggle}
          onDelete={onDelete}
          onEdit={onEdit}
          onSkip={onSkip}
          repeat={repeat}
          onRepeatChange={onRepeatChange}
          repeatUntil={repeatUntil}
          onRepeatUntilChange={onRepeatUntilChange}
          draggingId={draggingId}
          onTodoDragStart={onTodoDragStart}
          onTodoDrop={onTodoDrop}
        />
      ))}
    </div>
  );
}

/** 单日列：待办从顶往下排，加号固定在竖直中间；上半放不下的继续排到加号下方 */
function WeekDayColumn({
  day, list, isAdding, newTitle, parsedPreview, addInputRef, startAdd, onTitleChange, onCommit, onCancel,
  onToggle, onDelete, onEdit, onSkip, repeat, onRepeatChange, repeatUntil, onRepeatUntilChange,
  draggingId, onTodoDragStart, onTodoDrop,
}: {
  day: Date; list: Todo[]; isAdding: boolean; newTitle: string; parsedPreview: string | null;
  addInputRef: React.RefObject<HTMLInputElement>; startAdd: (key: string, hour?: number | null) => void;
  onTitleChange: (v: string) => void; onCommit: () => void; onCancel: () => void;
  onToggle: (t: Todo, dayKey?: string) => void; onDelete: (t: Todo) => void; onEdit: (id: string, p: Record<string, unknown>) => void; onSkip: (t: Todo) => void;
  repeat: RepeatVal; onRepeatChange: (v: RepeatVal) => void; repeatUntil: string | null; onRepeatUntilChange: (v: string | null) => void;
  draggingId: string | null; onTodoDragStart: (e: React.DragEvent, t: Todo, dayKey: string) => void; onTodoDrop: (targetDayKey: string, beforeId: string | null) => void;
}) {
  const key = ymd(day);
  const isWeekend = day.getDay() === 0 || day.getDay() === 6;
  const sorted = sortActiveFirst(list, key);
  const topRef = useRef<HTMLDivElement>(null);
  const [topCap, setTopCap] = useState(sorted.length);

  useLayoutEffect(() => {
    // 列表变化时先全部放到上半，再按实际高度切开，避免短暂空白
    setTopCap(sorted.length);
    const el = topRef.current;
    if (!el) return;

    const measure = () => {
      const gap = 4;
      const sample = el.querySelector(".todo-card") as HTMLElement | null;
      const cardH = sample?.offsetHeight || 40;
      const slot = cardH + gap;
      const cap = Math.max(0, Math.floor((el.clientHeight + gap) / slot));
      setTopCap((prev) => (prev === cap ? prev : cap));
    };

    // 等本帧布局完成后再量，保证上半区高度稳定
    const raf = requestAnimationFrame(measure);
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [sorted.length, key]);

  const above = sorted.slice(0, topCap);
  const below = sorted.slice(topCap);

  const renderCard = (t: Todo) => (
    <TodoCard
      key={t.id}
      t={t}
      onToggle={onToggle}
      onDelete={onDelete}
      onEdit={onEdit}
      onSkip={onSkip}
      compact
      dayKey={key}
      draggable
      dragging={draggingId === t.id}
      onDragStartTodo={onTodoDragStart}
      onDropAt={(id) => onTodoDrop(key, id)}
    />
  );

  return (
    <div
      className={`week-col ${isWeekend ? "weekend" : ""} ${isAdding ? "adding" : ""}`}
      onMouseDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        startAdd(key);
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => { e.preventDefault(); onTodoDrop(key, null); }}
    >
      <div className="week-col-half week-col-half-top" ref={topRef}>
        {above.map(renderCard)}
      </div>
      {!isAdding && (
        <button
          type="button"
          className="week-col-add-mid"
          title="添加待办"
          aria-label="添加待办"
          onClick={(e) => { e.stopPropagation(); startAdd(key); }}
          onDoubleClick={(e) => e.stopPropagation()}
        >+</button>
      )}
      <div className="week-col-half week-col-half-bot">
        {below.map(renderCard)}
      </div>
      {isAdding && (
        <div className="week-add-pop" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
          <AddForm
            inputRef={addInputRef}
            value={newTitle}
            onChange={onTitleChange}
            onCommit={onCommit}
            onCancel={onCancel}
            preview={parsedPreview}
            compact
            repeat={repeat}
            onRepeatChange={onRepeatChange}
            repeatUntil={repeatUntil}
            onRepeatUntilChange={onRepeatUntilChange}
          />
        </div>
      )}
    </div>
  );
}

/* =====================================================
   新建表单组件
   ===================================================== */
function AddForm({ inputRef, value, onChange, note, onNoteChange, onCommit, onCancel, preview, compact, hourHint, repeat, onRepeatChange, repeatUntil, onRepeatUntilChange }: {
  inputRef: React.RefObject<HTMLInputElement>; value: string; onChange: (v: string) => void;
  note?: string; onNoteChange?: (v: string) => void;
  onCommit: () => void; onCancel: () => void; preview: string | null; compact?: boolean; hourHint?: number | null;
  repeat?: RepeatVal; onRepeatChange?: (v: RepeatVal) => void;
  repeatUntil?: string | null; onRepeatUntilChange?: (v: string | null) => void;
}) {
  return (
    <div className={`add-form ${compact ? "add-form-compact" : ""}`}>
      <input ref={inputRef} className="add-input" type="text" value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter") onCommit(); if (e.key === "Escape") onCancel(); }}
        placeholder={compact ? "输入待办标题" : '输入待办（如"周六下午3点打球"）'} />
      {onNoteChange && !compact && (
        <textarea
          className="add-note"
          value={note ?? ""}
          onChange={(e) => onNoteChange(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") onCancel(); }}
          placeholder="备注（可选）"
          rows={2}
        />
      )}
      <div className="add-form-row">
        <select className="repeat-select" value={repeat ?? "none"} onChange={(e) => onRepeatChange?.(e.target.value as RepeatVal)}>
          <option value="none">不重复</option>
          <option value="daily">每天</option>
          <option value="weekly">每周</option>
          <option value="monthly">每月</option>
          <option value="yearly">每年</option>
        </select>
        {repeat && repeat !== "none" && (
          <label className="repeat-until-wrap" title="重复结束日期（留空表示一直重复）">
            <span className="repeat-until-label">至</span>
            <input
              type="date"
              className="repeat-until-input"
              value={repeatUntil ?? ""}
              onChange={(e) => onRepeatUntilChange?.(e.target.value || null)}
            />
          </label>
        )}
      </div>
      {hourHint != null && !preview && (
        <div className="parse-preview">将创建在 {pad(hourHint)}:00</div>
      )}
      {preview && <div className="parse-preview">{preview}</div>}
      <div className="add-actions">
        <button className="add-btn add-btn-confirm" onClick={onCommit}>确认</button>
        <button className="add-btn add-btn-cancel" onClick={onCancel}>取消</button>
      </div>
    </div>
  );
}

/* =====================================================
   待办卡片
   ===================================================== */
function TodoCard({ t, onToggle, onDelete, onEdit, onSkip, compact, dayKey, draggable, dragging, onDragStartTodo, onDropAt, hideTime }: {
  t: Todo; onToggle: (t: Todo, dayKey?: string) => void; onDelete: (t: Todo) => void;
  onEdit: (id: string, patch: Record<string, unknown>) => void; onSkip?: (t: Todo) => void; compact?: boolean; dayKey?: string;
  draggable?: boolean; dragging?: boolean; hideTime?: boolean;
  onDragStartTodo?: (e: React.DragEvent, t: Todo, dayKey: string) => void;
  onDropAt?: (beforeId: string | null) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(t.title);
  const [editNote, setEditNote] = useState(t.note ?? "");
  const inputRef = useRef<HTMLInputElement>(null);
  const doneForDay = dayKey ? todoDoneOnDay(t, dayKey) : t.done;

  const startEdit = () => {
    setEditTitle(t.title);
    setEditNote(t.note ?? "");
    setEditing(true);
    setTimeout(() => inputRef.current?.select(), 30);
  };
  const cancelEdit = () => {
    setEditing(false);
    setEditTitle(t.title);
    setEditNote(t.note ?? "");
  };
  const commitEdit = () => {
    const title = editTitle.trim();
    if (!title) { cancelEdit(); return; }
    const patch: Record<string, unknown> = {};
    if (title !== t.title) patch.title = title;
    if ((editNote || "") !== (t.note || "")) patch.note = editNote.trim();
    if (Object.keys(patch).length) onEdit(t.id, patch);
    setEditing(false);
  };

  return (
    <div
      className={`todo-card ${doneForDay ? "done" : ""} ${compact ? "todo-card-compact" : ""} ${dragging ? "dragging" : ""} ${editing ? "todo-card-editing" : ""}`}
      draggable={draggable && !editing}
      onDragStart={draggable && !editing ? (e: React.DragEvent) => { e.stopPropagation(); onDragStartTodo?.(e, t, dayKey || ""); } : undefined}
      onDragOver={draggable ? (e: React.DragEvent) => { e.preventDefault(); e.stopPropagation(); } : undefined}
      onDrop={draggable ? (e: React.DragEvent) => { e.preventDefault(); e.stopPropagation(); onDropAt?.(t.id); } : undefined}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <input type="checkbox" checked={doneForDay} onChange={() => onToggle(t, dayKey)} onClick={(e) => e.stopPropagation()} />
      {editing ? (
        <div
          className="todo-edit-block"
          onBlur={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node)) commitEdit();
          }}
        >
          <input
            ref={inputRef}
            className="todo-edit-input"
            type="text"
            value={editTitle}
            onChange={(e) => setEditTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") { e.preventDefault(); commitEdit(); }
              if (e.key === "Escape") { e.preventDefault(); cancelEdit(); }
            }}
          />
          <textarea
            className="todo-note-input"
            value={editNote}
            onChange={(e) => setEditNote(e.target.value)}
            placeholder="备注"
            rows={compact ? 1 : 2}
            onKeyDown={(e) => {
              if (e.key === "Escape") { e.preventDefault(); cancelEdit(); }
            }}
          />
        </div>
      ) : (
        <div className="todo-body" onDoubleClick={(e) => { e.stopPropagation(); startEdit(); }}>
          <span className="todo-title" title={t.title}>{t.title}</span>
          {compact && !hideTime && t.due_at && !isAllDay(t) && (
            <span className="todo-time todo-time-compact">
              {new Date(t.due_at).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}
            </span>
          )}
          {t.note ? <span className="todo-note" title={t.note}>{t.note}</span> : null}
        </div>
      )}
      {!compact && !hideTime && t.due_at && !isAllDay(t) && (
        <span className={`todo-time ${compact ? "todo-time-compact" : ""}`}>
          {new Date(t.due_at).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}
        </span>
      )}
      {t.repeat && t.repeat !== "none" && dayKey && onSkip && (
        <button className="todo-skip" title="今天不提醒（跳过这次重复）" onClick={(e) => { e.stopPropagation(); onSkip(t); }}>跳过</button>
      )}
      <button className="todo-del" onClick={() => onDelete(t)}>×</button>
    </div>
  );
}
