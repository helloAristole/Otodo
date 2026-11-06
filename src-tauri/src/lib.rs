use rusqlite::{params, Connection, params_from_iter};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Wry,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
struct Todo {
    id: String,
    title: String,
    note: Option<String>,
    due_at: Option<String>,
    all_day: bool,
    due_date: Option<String>,
    done: bool,
    sort_order: i32,
    deleted: bool,
    repeat: String,
    done_dates: String,
    repeat_until: Option<String>,
    skip_dates: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct OrderItem {
    id: String,
    sort_order: i32,
}

/// 幽灵模式状态（鼠标穿透），前后端共享
#[derive(Default)]
struct GhostState(AtomicBool);

fn db_path(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .expect("无法获取应用数据目录");
    std::fs::create_dir_all(&dir).ok();
    dir.join("otodo.db")
}

fn conn(app: &AppHandle) -> Connection {
    Connection::open(db_path(app)).expect("无法打开数据库")
}

#[derive(Serialize, Deserialize, Default)]
struct WindowGeometry {
    x: i32,
    y: i32,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

fn window_geometry_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok();
    Some(dir.join("window.json"))
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct UiSettings {
    #[serde(default)]
    always_on_top: bool,
    #[serde(default = "default_true")]
    roll_over: bool,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    expanded_w: Option<f64>,
    #[serde(default)]
    expanded_h: Option<f64>,
    #[serde(default)]
    pomo_focus_min: u32,
    #[serde(default)]
    pomo_break_min: u32,
}

fn default_true() -> bool {
    true
}

fn clamp_mins(v: u32, lo: u32, hi: u32, fallback: u32) -> u32 {
    if (lo..=hi).contains(&v) {
        v
    } else {
        fallback
    }
}

fn normalize_ui_settings(mut s: UiSettings) -> UiSettings {
    s.pomo_focus_min = clamp_mins(s.pomo_focus_min, 1, 180, 25);
    s.pomo_break_min = clamp_mins(s.pomo_break_min, 1, 60, 5);
    s
}

fn ui_settings_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok();
    Some(dir.join("ui_settings.json"))
}

fn read_ui_settings(app: &AppHandle) -> UiSettings {
    let fallback = UiSettings {
        roll_over: true,
        pomo_focus_min: 25,
        pomo_break_min: 5,
        ..UiSettings::default()
    };
    let Some(path) = ui_settings_path(app) else {
        return fallback;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .map(normalize_ui_settings)
        .unwrap_or(fallback)
}

fn write_ui_settings(app: &AppHandle, settings: &UiSettings) {
    let Some(path) = ui_settings_path(app) else { return };
    if let Ok(text) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, text);
    }
}

fn logical_inner_size(win: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let size = win.inner_size().ok()?;
    let scale = win.scale_factor().ok()?;
    if scale <= 0.0 {
        return None;
    }
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

fn save_window_geometry(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Ok(pos) = win.outer_position() else { return };
    let Some(path) = window_geometry_path(app) else { return };
    let (width, height) = match logical_inner_size(win) {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };
    let geo = WindowGeometry {
        x: pos.x,
        y: pos.y,
        width,
        height,
    };
    if let Ok(text) = serde_json::to_string(&geo) {
        let _ = std::fs::write(path, text);
    }
}

fn restore_window_geometry(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Some(path) = window_geometry_path(app) else { return };
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let Ok(geo) = serde_json::from_str::<WindowGeometry>(&text) else { return };
    use tauri::{LogicalSize, PhysicalPosition, Position, Size};
    let _ = win.set_position(Position::Physical(PhysicalPosition::new(geo.x, geo.y)));
    if let (Some(w), Some(h)) = (geo.width, geo.height) {
        if w > 1.0 && h > 1.0 {
            let _ = win.set_size(Size::Logical(LogicalSize::new(w, h)));
        }
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        restore_window_geometry(app, &win);
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn hide_main_window(app: &AppHandle, win: &tauri::WebviewWindow) {
    save_window_geometry(app, win);
    let _ = win.hide();
}

const POMO_FOCUS: Duration = Duration::from_secs(25 * 60);
const POMO_BREAK: Duration = Duration::from_secs(5 * 60);

fn mins_to_dur(mins: u32) -> Duration {
    Duration::from_secs(u64::from(mins.max(1)) * 60)
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PomoPhase {
    Idle,
    Focus,
    Break,
}

#[derive(Clone, Serialize)]
struct PomoSnap {
    phase: PomoPhase,
    running: bool,
    remaining_ms: u64,
    rounds_today: u32,
    focus_min: u32,
    break_min: u32,
}

struct Pomo {
    phase: PomoPhase,
    running: bool,
    remaining: Duration,
    deadline: Option<Instant>,
    rounds_today: u32,
    focus: Duration,
    break_d: Duration,
    focus_min: u32,
    break_min: u32,
}

impl Pomo {
    fn new() -> Self {
        Self {
            phase: PomoPhase::Idle,
            running: false,
            remaining: POMO_FOCUS,
            deadline: None,
            rounds_today: 0,
            focus: POMO_FOCUS,
            break_d: POMO_BREAK,
            focus_min: 25,
            break_min: 5,
        }
    }

    fn remaining_now(&self) -> Duration {
        if self.running {
            if let Some(dl) = self.deadline {
                return dl.saturating_duration_since(Instant::now());
            }
        }
        self.remaining
    }

    fn snapshot(&self) -> PomoSnap {
        PomoSnap {
            phase: self.phase,
            running: self.running,
            remaining_ms: self.remaining_now().as_millis() as u64,
            rounds_today: self.rounds_today,
            focus_min: self.focus_min,
            break_min: self.break_min,
        }
    }

    fn pause(&mut self) {
        if !self.running {
            return;
        }
        self.remaining = self.remaining_now();
        self.running = false;
        self.deadline = None;
    }

    fn begin(&mut self, phase: PomoPhase, dur: Duration) {
        self.phase = phase;
        self.remaining = dur;
        self.running = true;
        self.deadline = Some(Instant::now() + dur);
    }

    fn set_durations(&mut self, focus_min: u32, break_min: u32) {
        self.focus_min = clamp_mins(focus_min, 1, 180, 25);
        self.break_min = clamp_mins(break_min, 1, 60, 5);
        self.focus = mins_to_dur(self.focus_min);
        self.break_d = mins_to_dur(self.break_min);
        if !self.running && self.phase == PomoPhase::Idle {
            self.remaining = self.focus;
        }
    }

    fn toggle(&mut self) {
        if self.running {
            self.pause();
            return;
        }
        let left = self.remaining_now();
        if left > Duration::from_millis(400) {
            if self.phase == PomoPhase::Idle {
                self.phase = PomoPhase::Focus;
            }
            self.running = true;
            self.deadline = Some(Instant::now() + left);
            return;
        }
        match self.phase {
            PomoPhase::Focus => self.begin(PomoPhase::Break, self.break_d),
            PomoPhase::Break | PomoPhase::Idle => self.begin(PomoPhase::Focus, self.focus),
        }
    }

    fn reset(&mut self) {
        let rounds = self.rounds_today;
        let focus = self.focus;
        let break_d = self.break_d;
        let focus_min = self.focus_min;
        let break_min = self.break_min;
        *self = Self {
            remaining: focus,
            focus,
            break_d,
            focus_min,
            break_min,
            rounds_today: rounds,
            ..Self::new()
        };
    }

    fn finish_if_due(&mut self) -> Option<PomoPhase> {
        if !self.running {
            return None;
        }
        if self.remaining_now() > Duration::ZERO {
            return None;
        }
        let done = self.phase;
        self.remaining = Duration::ZERO;
        self.running = false;
        self.deadline = None;
        if done == PomoPhase::Focus {
            self.rounds_today = self.rounds_today.saturating_add(1);
        }
        Some(done)
    }
}

struct TrayPomoUi {
    item: MenuItem<Wry>,
    last: Mutex<String>,
}

fn pomo_clock(ms: u64) -> String {
    let mm = ms / 60000;
    let ss = (ms / 1000) % 60;
    format!("{mm:02}:{ss:02}")
}

fn pomo_menu_label(snap: &PomoSnap) -> String {
    let time = pomo_clock(snap.remaining_ms);
    let done = !snap.running && snap.remaining_ms < 400 && snap.phase != PomoPhase::Idle;
    match (snap.phase, snap.running, done) {
        (PomoPhase::Idle, _, _) => format!("开始专注  {time}"),
        (PomoPhase::Focus, true, _) => format!("专注中  {time}"),
        (PomoPhase::Focus, false, true) => "专注结束".into(),
        (PomoPhase::Focus, false, _) => format!("专注已暂停  {time}"),
        (PomoPhase::Break, true, _) => format!("休息中  {time}"),
        (PomoPhase::Break, false, true) => "休息结束".into(),
        (PomoPhase::Break, false, _) => format!("休息已暂停  {time}"),
    }
}

fn emit_pomo(app: &AppHandle, snap: &PomoSnap) {
    let _ = app.emit("pomodoro-update", snap);
    let label = pomo_menu_label(snap);
    if let Some(tray) = app.tray_by_id("main") {
        let tip = if snap.phase == PomoPhase::Idle && !snap.running {
            "Otodo - 日历待办".into()
        } else {
            format!("Otodo · {label}")
        };
        let _ = tray.set_tooltip(Some(&tip));
    }
    if let Some(ui) = app.try_state::<TrayPomoUi>() {
        if let Ok(mut last) = ui.last.lock() {
            if *last != label {
                *last = label.clone();
                drop(last);
                let _ = ui.item.set_text(label);
            }
        }
    }
}

fn notify_pomo(app: &AppHandle, phase: PomoPhase, break_min: u32) {
    let (title, body) = match phase {
        PomoPhase::Focus => (
            "专注结束".to_string(),
            format!("可以休息 {break_min} 分钟，或再开一轮。"),
        ),
        PomoPhase::Break => (
            "休息结束".to_string(),
            "准备好就开始下一轮专注。".to_string(),
        ),
        PomoPhase::Idle => return,
    };
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(title).body(body).show();
}

fn with_pomo<T>(app: &AppHandle, f: impl FnOnce(&mut Pomo) -> T) -> Result<T, String> {
    let state = app.state::<Arc<Mutex<Pomo>>>();
    let mut p = state.lock().map_err(|e| e.to_string())?;
    Ok(f(&mut p))
}

#[tauri::command]
fn get_pomodoro(app: AppHandle) -> Result<PomoSnap, String> {
    with_pomo(&app, |p| p.snapshot())
}

#[tauri::command]
fn pomodoro_toggle(app: AppHandle) -> Result<PomoSnap, String> {
    let snap = with_pomo(&app, |p| {
        p.toggle();
        p.snapshot()
    })?;
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn pomodoro_reset(app: AppHandle) -> Result<PomoSnap, String> {
    let snap = with_pomo(&app, |p| {
        p.reset();
        p.snapshot()
    })?;
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn pomodoro_set_durations(app: AppHandle, focus_min: u32, break_min: u32) -> Result<PomoSnap, String> {
    let snap = with_pomo(&app, |p| {
        p.set_durations(focus_min, break_min);
        p.snapshot()
    })?;
    let mut settings = read_ui_settings(&app);
    settings.pomo_focus_min = snap.focus_min;
    settings.pomo_break_min = snap.break_min;
    write_ui_settings(&app, &settings);
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn init_db(app: AppHandle) -> Result<(), String> {
    let c = conn(&app);

    // 1) 先检测 todos 表是否存在 + 是否有 title 列
    let (table_exists, has_title) = {
        let mut stmt = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='todos'")
            .map_err(|e| e.to_string())?;
        let exists = stmt.exists([]).map_err(|e| e.to_string())?;
        if !exists {
            (false, false)
        } else {
            let mut stmt2 = c.prepare("PRAGMA table_info(todos)").map_err(|e| e.to_string())?;
            let rows = stmt2
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(|e| e.to_string())?;
            let cols: Vec<String> = rows.filter_map(|r| r.ok()).collect();
            (true, cols.iter().any(|c| c == "title"))
        }
    };

    // 2) 旧表存在但缺 title 列 → 删掉重建（schema 不兼容）
    if table_exists && !has_title {
        c.execute("DROP TABLE todos", []).map_err(|e| e.to_string())?;
    }

    // 3) CREATE IF NOT EXISTS —— 如果表被删了/不存在就新建
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS todos (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            note TEXT,
            due_at TEXT,
            all_day INTEGER NOT NULL DEFAULT 0,
            due_date TEXT,
            done INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0,
            repeat TEXT NOT NULL DEFAULT 'none',
            done_dates TEXT NOT NULL DEFAULT '',
            repeat_until TEXT,
            skip_dates TEXT,
            dirty INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .map_err(|e| e.to_string())?;

    // 为已存在的旧表补充新列（兼容升级）
    let mut info = c.prepare("PRAGMA table_info(todos)").map_err(|e| e.to_string())?;
    let existing: Vec<String> = info
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    if !existing.iter().any(|c| c == "repeat") {
        c.execute("ALTER TABLE todos ADD COLUMN repeat TEXT NOT NULL DEFAULT 'none'", [])
            .ok();
    }
    if !existing.iter().any(|c| c == "done_dates") {
        c.execute("ALTER TABLE todos ADD COLUMN done_dates TEXT NOT NULL DEFAULT ''", [])
            .ok();
    }
    if !existing.iter().any(|c| c == "repeat_until") {
        c.execute("ALTER TABLE todos ADD COLUMN repeat_until TEXT", []).ok();
    }
    if !existing.iter().any(|c| c == "skip_dates") {
        c.execute("ALTER TABLE todos ADD COLUMN skip_dates TEXT", []).ok();
    }
    if !existing.iter().any(|c| c == "dirty") {
        c.execute("ALTER TABLE todos ADD COLUMN dirty INTEGER NOT NULL DEFAULT 0", [])
            .ok();
    }
    // created_at / updated_at 在早期版本已存在；若没有则补充
    if !existing.iter().any(|c| c == "created_at") {
        c.execute(
            "ALTER TABLE todos ADD COLUMN created_at TEXT NOT NULL DEFAULT (datetime('now'))",
            [],
        )
        .ok();
    }
    if !existing.iter().any(|c| c == "updated_at") {
        c.execute(
            "ALTER TABLE todos ADD COLUMN updated_at TEXT NOT NULL DEFAULT (datetime('now'))",
            [],
        )
        .ok();
    }

    Ok(())
}

#[tauri::command]
fn get_todos(app: AppHandle) -> Result<Vec<Todo>, String> {
    let c = conn(&app);
    let mut stmt = c
        .prepare(
            "SELECT id, title, note, due_at, all_day, due_date, done, sort_order, deleted, repeat, done_dates, repeat_until, skip_dates, created_at, updated_at \
             FROM todos WHERE deleted = 0 ORDER BY due_at IS NULL, due_at, sort_order",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Todo {
                id: r.get(0)?,
                title: r.get(1)?,
                note: r.get(2)?,
                due_at: r.get(3)?,
                all_day: r.get::<_, i32>(4)? != 0,
                due_date: r.get(5)?,
                done: r.get::<_, i32>(6)? != 0,
                sort_order: r.get(7)?,
                deleted: r.get::<_, i32>(8)? != 0,
                repeat: r.get(9)?,
                done_dates: r.get(10)?,
                repeat_until: r.get(11)?,
                skip_dates: r.get(12)?,
                created_at: r.get(13)?,
                updated_at: r.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command(rename_all = "camelCase")]
fn add_todo(
    app: AppHandle,
    title: String,
    due_at: Option<String>,
    due_date: Option<String>,
    all_day: bool,
    note: Option<String>,
    repeat: String,
    repeat_until: Option<String>,
    skip_dates: Option<String>,
) -> Result<String, String> {
    let c = conn(&app);
    let id = Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO todos (id, title, note, due_at, due_date, all_day, repeat, repeat_until, skip_dates, done_dates, done, dirty, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', 0, 1, datetime('now'), datetime('now'))",
        params![id, title, note, due_at, due_date, all_day as i32, repeat, repeat_until, skip_dates],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
fn toggle_todo(app: AppHandle, id: String, done: bool, day: Option<String>) -> Result<(), String> {
    let c = conn(&app);
    // 重复待办：按天记录完成状态（done_dates 存空格分隔的 ymd 列表，与前端/服务端一致）
    let repeat: String = c
        .query_row("SELECT repeat FROM todos WHERE id = ?1", params![id], |r| r.get(0))
        .unwrap_or_else(|_| "none".to_string());
    if repeat != "none" {
        if let Some(day) = day {
            let cur: String = c
                .query_row("SELECT done_dates FROM todos WHERE id = ?1", params![id], |r| r.get(0))
                .unwrap_or_default();
            let mut set: Vec<&str> = cur.split(' ').filter(|s| !s.is_empty()).collect();
            if done {
                if !set.contains(&day.as_str()) {
                    set.push(day.as_str());
                }
            } else {
                set.retain(|s| *s != day.as_str());
            }
            let new_val = set.join(" ");
            c.execute(
                "UPDATE todos SET done_dates = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
                params![new_val, id],
            )
            .map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    c.execute(
        "UPDATE todos SET done = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
        params![done as i32, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_todo(app: AppHandle, id: String) -> Result<(), String> {
    let c = conn(&app);
    c.execute(
        "UPDATE todos SET deleted = 1, dirty = 1, updated_at = datetime('now') WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 通用更新：fields 为部分字段对象，传入 null 表示「清空该字段」（修复 Bug 4）
#[tauri::command]
fn update_todo(app: AppHandle, id: String, fields: serde_json::Value) -> Result<(), String> {
    let c = conn(&app);
    let obj = fields
        .as_object()
        .ok_or_else(|| "fields 必须是对象".to_string())?;

    let mut clauses: Vec<String> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();

    // 通用更新：fields 为部分字段对象；传入 null 表示「清空该字段」（修复 Bug 4）。
    // kind: 0=文本, 1=布尔, 2=整数
    let mut take = |key: &str, col: &str, kind: u8| {
        if let Some(v) = obj.get(key) {
            clauses.push(format!("{} = ?", col));
            if v.is_null() {
                vals.push(Value::Null);
            } else if kind == 0 {
                vals.push(Value::Text(v.as_str().unwrap_or("").to_string()));
            } else if kind == 1 {
                vals.push(Value::Integer(if v.as_bool().unwrap_or(false) { 1 } else { 0 }));
            } else {
                vals.push(Value::Integer(v.as_i64().unwrap_or(0)));
            }
        }
    };
    take("title", "title", 0);
    take("note", "note", 0);
    take("due_at", "due_at", 0);
    take("due_date", "due_date", 0);
    take("all_day", "all_day", 1);
    take("done", "done", 1);
    take("sort_order", "sort_order", 2);
    take("repeat", "repeat", 0);
    take("done_dates", "done_dates", 0);
    take("repeat_until", "repeat_until", 0);
    take("skip_dates", "skip_dates", 0);

    // 本地为真实数据源：任何修改都标记 dirty 并刷新 updated_at
    clauses.push("dirty = 1".to_string());
    clauses.push("updated_at = datetime('now')".to_string());

    vals.push(Value::Text(id.clone()));
    let sql = format!("UPDATE todos SET {} WHERE id = ?", clauses.join(", "));
    c.execute(&sql, params_from_iter(vals.iter()))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 批量重排顺序（修复 Bug 8：一次调用完成整列排序，避免 N 次串行请求）
#[tauri::command]
fn reorder_todos(app: AppHandle, orders: Vec<OrderItem>) -> Result<(), String> {
    let c = conn(&app);
    for o in orders {
        c.execute(
            "UPDATE todos SET sort_order = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
            params![o.sort_order, o.id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn drag_window(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(win) = app.get_webview_window("main") {
            start_window_drag(&win);
        }
    }
    Ok(())
}

/// FR-13 幽灵模式：切换鼠标穿透
#[tauri::command]
fn toggle_ghost(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<GhostState>();
    let current = state.0.load(Ordering::SeqCst);
    let next = !current;
    state.0.store(next, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("main") {
        win.set_ignore_cursor_events(next)
            .map_err(|e| e.to_string())?;
    }
    Ok(next)
}

/// 切换窗口置顶
#[tauri::command(rename_all = "camelCase")]
fn set_always_on_top(app: AppHandle, on_top: bool) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.set_always_on_top(on_top).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 调整窗口大小（收起/展开）
#[tauri::command]
fn set_window_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    use tauri::{LogicalSize, Size};
    if let Some(win) = app.get_webview_window("main") {
        // 改尺寸前记下位置，避免部分平台 resize 后窗口跳回默认位置
        let pos = win.outer_position().ok();
        win.set_size(Size::Logical(LogicalSize::new(width, height)))
            .map_err(|e| e.to_string())?;
        if let Some(p) = pos {
            use tauri::{PhysicalPosition, Position};
            let _ = win.set_position(Position::Physical(PhysicalPosition::new(p.x, p.y)));
            save_window_geometry(&app, &win);
        }
    }
    Ok(())
}

/// 读取当前窗口逻辑尺寸（用于收起前记住展开大小）
#[tauri::command]
fn get_window_size(app: AppHandle) -> Result<(f64, f64), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "窗口不存在".to_string())?;
    logical_inner_size(&win).ok_or_else(|| "无法读取窗口尺寸".to_string())
}

/// 关闭窗口到托盘：隐藏而非退出（真正退出仅由托盘「退出」完成）
#[tauri::command]
fn hide_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        hide_main_window(&app, &win);
    }
    Ok(())
}

/// 前端首帧就绪后显示主窗口（避免启动白闪）
#[tauri::command]
fn show_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// 读取 UI 设置（锁定/置顶等），比 WebView localStorage 更可靠
#[tauri::command]
fn get_ui_settings(app: AppHandle) -> UiSettings {
    read_ui_settings(&app)
}

/// 持久化 UI 设置到应用数据目录
#[tauri::command]
fn save_ui_settings(app: AppHandle, settings: UiSettings) -> Result<(), String> {
    let settings = normalize_ui_settings(settings);
    write_ui_settings(&app, &settings);
    let _ = with_pomo(&app, |p| {
        p.set_durations(settings.pomo_focus_min, settings.pomo_break_min);
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 必须尽早注册：第二次启动时只唤醒已有实例，禁止再开一个窗口
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if let Some(win) = app.get_webview_window("main") {
                            if win.is_visible().unwrap_or(false) {
                                hide_main_window(app, &win);
                            } else {
                                show_main_window(app);
                            }
                        }
                    }
                })
                .build(),
        )
        .manage(GhostState::default())
        .manage(Arc::new(Mutex::new(Pomo::new())))
        .invoke_handler(tauri::generate_handler![
            init_db,
            get_todos,
            add_todo,
            update_todo,
            toggle_todo,
            delete_todo,
            reorder_todos,
            drag_window,
            toggle_ghost,
            set_always_on_top,
            set_window_size,
            get_window_size,
            hide_window,
            show_window,
            get_ui_settings,
            save_ui_settings,
            get_pomodoro,
            pomodoro_toggle,
            pomodoro_reset,
            pomodoro_set_durations
        ])
        .setup(|app| {
            // 启动时只恢复位置，保持隐藏；等前端首帧就绪后再 show，避免白闪。
            if let Some(win) = app.get_webview_window("main") {
                restore_window_geometry(app.handle(), &win);
            }

            #[cfg(target_os = "windows")]
            {
                // 声明 AppUserModelID，否则 Windows 通知系统不识别 Otodo
                use std::ffi::OsStr;
                use std::os::windows::ffi::OsStrExt;
                let aumid: Vec<u16> = OsStr::new("com.otodo.desktop")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                extern "system" {
                    fn SetCurrentProcessExplicitAppUserModelID(pcwszAppID: *const u16) -> i32;
                }
                unsafe { SetCurrentProcessExplicitAppUserModelID(aumid.as_ptr()); }
            }

            // 注册全局快捷键 Alt+Space：显示/隐藏窗口（Bug 10）
            let _ = app.global_shortcut().register(Shortcut::new(Some(Modifiers::ALT), Code::Space));

            // 开机自启：仅由安装程序询问或托盘菜单控制，启动时不再强制开启
            // ---- 系统托盘 (FR-14) ----
            let show_item = MenuItemBuilder::with_id("show", "显示窗口").build(app)?;
            let new_item = MenuItemBuilder::with_id("new_todo", "新建待办").build(app)?;
            let pomo_item = MenuItemBuilder::with_id("pomodoro", "开始专注  25:00").build(app)?;
            let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart_item = CheckMenuItemBuilder::with_id("autostart", "开机自启")
                .checked(autostart_on)
                .build(app)?;
            let sep = tauri::menu::PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let tray_menu = MenuBuilder::new(app)
                .items(&[
                    &show_item,
                    &new_item,
                    &pomo_item,
                    &autostart_item,
                    &sep,
                    &quit_item,
                ])
                .build()?;

            let autostart_item_for_event = autostart_item.clone();
            let _tray = TrayIconBuilder::with_id("main")
                .icon(make_tray_icon())
                .tooltip("Otodo - 日历待办")
                .menu(&tray_menu)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        show_main_window(app);
                    }
                    "new_todo" => {
                        show_main_window(app);
                        let _ = app.emit("tray-new-todo", ());
                    }
                    "pomodoro" => {
                        let _ = pomodoro_toggle(app.clone());
                    }
                    "autostart" => {
                        let launcher = app.autolaunch();
                        let enabled = launcher.is_enabled().unwrap_or(false);
                        if enabled {
                            if launcher.disable().is_ok() {
                                let _ = autostart_item_for_event.set_checked(false);
                            }
                        } else if launcher.enable().is_ok() {
                            let _ = autostart_item_for_event.set_checked(true);
                        }
                    }
                    "quit" => {
                        if let Some(win) = app.get_webview_window("main") {
                            save_window_geometry(app, &win);
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            app.manage(TrayPomoUi {
                item: pomo_item.clone(),
                last: Mutex::new("开始专注  25:00".into()),
            });

            {
                let s = read_ui_settings(app.handle());
                let snap = {
                    let mut p = app.state::<Arc<Mutex<Pomo>>>().inner().lock().ok();
                    if let Some(ref mut p) = p {
                        p.set_durations(s.pomo_focus_min, s.pomo_break_min);
                        Some(p.snapshot())
                    } else {
                        None
                    }
                };
                if let Some(snap) = snap {
                    emit_pomo(app.handle(), &snap);
                }
            }

            let pomo_arc = app.state::<Arc<Mutex<Pomo>>>().inner().clone();
            let tick_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(200));
                let Ok(mut p) = pomo_arc.lock() else { continue };
                let running = p.running;
                let finished = p.finish_if_due();
                if !running && finished.is_none() {
                    continue;
                }
                let snap = p.snapshot();
                drop(p);
                emit_pomo(&tick_handle, &snap);
                if let Some(phase) = finished {
                    notify_pomo(&tick_handle, phase, snap.break_min);
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    if window.label() == "main" {
                        if let Some(win) = window.app_handle().get_webview_window("main") {
                            save_window_geometry(window.app_handle(), &win);
                        }
                    }
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        api.prevent_close();
                        let app = window.app_handle().clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(win) = app.get_webview_window("main") {
                                hide_main_window(&app, &win);
                            }
                        });
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 托盘图标：从 PNG 字节加载
fn make_tray_icon() -> Image<'static> {
    Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("托盘图标加载失败")
}

#[cfg(target_os = "windows")]
fn start_window_drag(win: &tauri::WebviewWindow) {
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{ReleaseCapture, SendMessageW, WM_NCLBUTTONDOWN, HTCAPTION};
    if let Ok(raw) = win.hwnd() {
        let hwnd = raw.0 as HWND;
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
        }
    }
}
