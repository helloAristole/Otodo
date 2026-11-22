use rusqlite::types::Value;
use rusqlite::{
    backup::Backup, params, params_from_iter, Connection, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Wry,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
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

#[derive(Deserialize)]
struct DateUpdate {
    id: String,
    due_at: Option<String>,
    due_date: Option<String>,
    all_day: bool,
}

/// 幽灵模式状态（鼠标穿透），前后端共享
#[derive(Default)]
struct GhostState(AtomicBool);

#[derive(Default)]
struct TemporaryWindowSize(AtomicBool);

impl TemporaryWindowSize {
    fn set_active(&self, active: bool) {
        self.0.store(active, Ordering::Release);
    }

    fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

const DATABASE_VERSION: i64 = 2;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DATABASE_BACKUPS: usize = 5;
const LEGACY_REQUIRED_COLUMNS: &[&str] = &["id", "title", "done"];
const VERSION_1_REQUIRED_COLUMNS: &[&str] = &[
    "id",
    "title",
    "note",
    "due_at",
    "all_day",
    "due_date",
    "done",
    "sort_order",
    "deleted",
    "created_at",
    "updated_at",
];
const CURRENT_REQUIRED_COLUMNS: &[&str] = &[
    "id",
    "title",
    "note",
    "due_at",
    "all_day",
    "due_date",
    "done",
    "sort_order",
    "deleted",
    "repeat",
    "done_dates",
    "repeat_until",
    "skip_dates",
    "dirty",
    "created_at",
    "updated_at",
];
type Migration = fn(&Transaction<'_>) -> rusqlite::Result<()>;

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法获取应用数据目录：{error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建应用数据目录：{error}"))?;
    Ok(dir)
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("otodo.db"))
}

fn open_connection(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|error| format!("无法打开数据库：{error}"))?;
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(|error| format!("无法设置数据库等待时间：{error}"))?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
        .map_err(|error| format!("无法设置数据库安全参数：{error}"))?;
    Ok(connection)
}

fn conn(app: &AppHandle) -> Result<Connection, String> {
    open_connection(&db_path(app)?)
}

fn table_exists(connection: &Connection, table_name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
        )",
        [table_name],
        |row| row.get(0),
    )
}

fn table_columns(connection: &Connection, table_name: &str) -> rusqlite::Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({table_name})");
    let mut statement = connection.prepare(&sql)?;
    let columns = statement
        .query_map([], |row| row.get(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(columns)
}

fn add_column_if_missing(
    transaction: &Transaction<'_>,
    existing_columns: &mut Vec<String>,
    column_name: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    if existing_columns.iter().any(|name| name == column_name) {
        return Ok(());
    }

    let sql = format!("ALTER TABLE todos ADD COLUMN {column_name} {definition}");
    transaction.execute(&sql, [])?;
    existing_columns.push(column_name.to_string());
    Ok(())
}

#[cfg(test)]
fn run_all_migrations_transaction(
    connection: &mut Connection,
    current_version: i64,
    migrations: &[(i64, Migration)],
) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (target_version, migrate) in migrations {
        if *target_version <= current_version {
            continue;
        }
        migrate(&transaction)?;
        transaction.pragma_update(None, "user_version", target_version)?;
    }
    transaction.commit()
}

fn migrate_to_version_1(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    if !table_exists(transaction, "todos")? {
        transaction.execute_batch(
            "CREATE TABLE todos (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                note TEXT,
                due_at TEXT,
                all_day INTEGER NOT NULL DEFAULT 0,
                due_date TEXT,
                done INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                deleted INTEGER NOT NULL DEFAULT 0,
                created_at TEXT,
                updated_at TEXT
            )",
        )?;
        return Ok(());
    }

    // 早期桌面版的基础列已在事务前验证；这里只补可安全推导的列，绝不重建或删除用户表。
    let mut columns = table_columns(transaction, "todos")?;
    add_column_if_missing(transaction, &mut columns, "note", "TEXT")?;
    add_column_if_missing(transaction, &mut columns, "due_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        &mut columns,
        "all_day",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, &mut columns, "due_date", "TEXT")?;
    add_column_if_missing(
        transaction,
        &mut columns,
        "sort_order",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        &mut columns,
        "deleted",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, &mut columns, "created_at", "TEXT")?;
    add_column_if_missing(transaction, &mut columns, "updated_at", "TEXT")?;

    transaction.execute(
        "UPDATE todos
         SET created_at = COALESCE(created_at, datetime('now')),
             updated_at = COALESCE(updated_at, datetime('now'))",
        [],
    )?;
    Ok(())
}

fn migrate_to_version_2(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let mut columns = table_columns(transaction, "todos")?;
    // 重复待办与同步标记属于可选升级列，统一在同一版本事务内补齐。
    add_column_if_missing(
        transaction,
        &mut columns,
        "repeat",
        "TEXT NOT NULL DEFAULT 'none'",
    )?;
    add_column_if_missing(
        transaction,
        &mut columns,
        "done_dates",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(transaction, &mut columns, "repeat_until", "TEXT")?;
    add_column_if_missing(transaction, &mut columns, "skip_dates", "TEXT")?;
    add_column_if_missing(
        transaction,
        &mut columns,
        "dirty",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

fn create_database_backup(source: &Connection, backup_directory: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_directory)
        .map_err(|error| format!("无法创建数据库备份目录：{error}"))?;
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("无法生成数据库备份时间戳：{error}"))?;
    let timestamp = format!("{:010}{:09}", elapsed.as_secs(), elapsed.subsec_nanos());
    let backup_path = backup_directory.join(format!("otodo-{timestamp}.db"));
    let mut destination =
        Connection::open(&backup_path).map_err(|error| format!("无法创建数据库备份：{error}"))?;

    let backup_result = (|| -> rusqlite::Result<()> {
        let backup = Backup::new(source, &mut destination)?;
        backup.run_to_completion(64, Duration::from_millis(10), None)
    })();
    if let Err(error) = backup_result {
        drop(destination);
        let _ = fs::remove_file(&backup_path);
        return Err(format!("数据库升级前备份失败：{error}"));
    }

    let integrity_result =
        destination.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0));
    drop(destination);
    handle_backup_integrity_result(&backup_path, integrity_result)?;

    prune_database_backups(backup_directory)?;
    Ok(backup_path)
}

fn handle_backup_integrity_result(
    backup_path: &Path,
    integrity_result: rusqlite::Result<String>,
) -> Result<(), String> {
    let error_detail = match integrity_result {
        Ok(result) if result == "ok" => return Ok(()),
        Ok(result) => format!("检查结果：{result}"),
        Err(error) => format!("无法执行检查：{error}"),
    };

    let cleanup_detail = match fs::remove_file(backup_path) {
        Ok(()) => String::new(),
        Err(error) => format!("；同时无法删除无效备份：{error}"),
    };
    Err(format!(
        "数据库备份完整性检查失败，{error_detail}{cleanup_detail}"
    ))
}

fn is_automatic_backup_name(name: &str) -> bool {
    let Some(timestamp) = name
        .strip_prefix("otodo-")
        .and_then(|value| value.strip_suffix(".db"))
    else {
        return false;
    };
    timestamp.len() == 19 && timestamp.bytes().all(|byte| byte.is_ascii_digit())
}

fn prune_database_backups(backup_directory: &Path) -> Result<(), String> {
    let mut backups = fs::read_dir(backup_directory)
        .map_err(|error| format!("无法读取数据库备份目录：{error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_automatic_backup_name)
        })
        .collect::<Vec<_>>();
    backups.sort();

    let remove_count = backups.len().saturating_sub(MAX_DATABASE_BACKUPS);
    for path in backups.into_iter().take(remove_count) {
        fs::remove_file(&path)
            .map_err(|error| format!("无法清理旧数据库备份 {}：{error}", path.display()))?;
    }
    Ok(())
}

fn required_columns_for_version(version: i64) -> &'static [&'static str] {
    match version {
        0 => LEGACY_REQUIRED_COLUMNS,
        1 => VERSION_1_REQUIRED_COLUMNS,
        _ => CURRENT_REQUIRED_COLUMNS,
    }
}

fn validate_todos_structure(connection: &Connection, version: i64) -> Result<(), String> {
    let columns = table_columns(connection, "todos")
        .map_err(|error| format!("无法读取 todos 表结构：{error}"))?;
    let missing = required_columns_for_version(version)
        .iter()
        .filter(|required| !columns.iter().any(|column| column == **required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "检测到无法安全迁移的 todos 表结构，缺少必需列：{}。为保护数据已停止启动",
        missing.join("、")
    ))
}

fn initialize_database(
    database_path: &Path,
    backup_directory: &Path,
) -> Result<Connection, String> {
    static INITIALIZATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _initialization_guard = INITIALIZATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|error| format!("无法锁定数据库初始化：{error}"))?;

    let mut connection = open_connection(database_path)?;
    let inspection = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始数据库初始化事务：{error}"))?;
    let has_todos = table_exists(&inspection, "todos")
        .map_err(|error| format!("无法检查数据库结构：{error}"))?;
    let current_version = inspection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| format!("无法读取数据库版本：{error}"))?;

    if current_version > DATABASE_VERSION {
        return Err(format!(
            "数据库版本 {current_version} 高于当前应用支持的版本 {DATABASE_VERSION}，请升级应用后重试"
        ));
    }

    if has_todos {
        validate_todos_structure(&inspection, current_version)?;
    } else if current_version > 0 {
        return Err("数据库版本已存在，但缺少 todos 表，为保护数据已停止启动".to_string());
    }

    if current_version == DATABASE_VERSION {
        inspection
            .commit()
            .map_err(|error| format!("无法完成数据库初始化事务：{error}"))?;
        return Ok(connection);
    }
    inspection
        .commit()
        .map_err(|error| format!("无法完成数据库初始化检查：{error}"))?;

    // 只有已有待办表的升级才需要备份；全新库不产生无意义的空备份。
    if has_todos {
        create_database_backup(&connection, backup_directory)?;
    }

    // 备份后以 IMMEDIATE 事务重新读取版本，避免基于事务外的旧版本重复迁移。
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始数据库迁移事务：{error}"))?;
    let current_version = transaction
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| format!("无法在迁移事务内复查数据库版本：{error}"))?;
    if current_version > DATABASE_VERSION {
        return Err(format!(
            "数据库版本 {current_version} 高于当前应用支持的版本 {DATABASE_VERSION}，请升级应用后重试"
        ));
    }
    if current_version == DATABASE_VERSION {
        transaction
            .commit()
            .map_err(|error| format!("无法完成数据库迁移复查：{error}"))?;
        return Ok(connection);
    }

    let migrations: &[(i64, Migration)] = &[(1, migrate_to_version_1), (2, migrate_to_version_2)];
    for (target_version, migrate) in migrations {
        if *target_version <= current_version {
            continue;
        }
        migrate(&transaction).map_err(|error| {
            format!("数据库迁移失败，所有版本变更均已回滚，原数据已保留：{error}")
        })?;
        transaction
            .pragma_update(None, "user_version", target_version)
            .map_err(|error| {
                format!("数据库迁移失败，所有版本变更均已回滚，原数据已保留：{error}")
            })?;
    }
    transaction
        .commit()
        .map_err(|error| format!("数据库迁移提交失败，所有版本变更均已回滚：{error}"))?;

    Ok(connection)
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

#[derive(Clone, Copy)]
struct WorkArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
}

impl WorkArea {
    const fn new(x: i32, y: i32, width: u32, height: u32, scale_factor: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale_factor,
        }
    }

    fn scale_factor(self) -> f64 {
        if self.scale_factor.is_finite() && self.scale_factor > 0.0 {
            self.scale_factor
        } else {
            1.0
        }
    }
}

struct RestoredGeometry {
    x: i32,
    y: i32,
    width: f64,
    height: f64,
}

const DEFAULT_WINDOW_WIDTH: f64 = 1000.0;
const DEFAULT_WINDOW_HEIGHT: f64 = 720.0;
const MIN_VISIBLE_WIDTH: i64 = 64;
const MIN_VISIBLE_HEIGHT: i64 = 40;

fn calculate_restored_geometry(
    saved: &WindowGeometry,
    work_areas: &[WorkArea],
    primary: WorkArea,
) -> RestoredGeometry {
    let requested_width = saved
        .width
        .filter(|value| value.is_finite() && *value > 1.0)
        .unwrap_or(DEFAULT_WINDOW_WIDTH);
    let requested_height = saved
        .height
        .filter(|value| value.is_finite() && *value > 1.0)
        .unwrap_or(DEFAULT_WINDOW_HEIGHT);

    let visible_size = work_areas.iter().copied().find_map(|area| {
        let scale = area.scale_factor();
        let width = requested_width.min(f64::from(area.width) / scale);
        let height = requested_height.min(f64::from(area.height) / scale);
        let physical_width = (width * scale).round() as i64;
        let physical_height = (height * scale).round() as i64;
        let left = i64::from(saved.x).max(i64::from(area.x));
        let top = i64::from(saved.y).max(i64::from(area.y));
        let right =
            (i64::from(saved.x) + physical_width).min(i64::from(area.x) + i64::from(area.width));
        let bottom =
            (i64::from(saved.y) + physical_height).min(i64::from(area.y) + i64::from(area.height));
        if right - left >= MIN_VISIBLE_WIDTH && bottom - top >= MIN_VISIBLE_HEIGHT {
            Some((width, height))
        } else {
            None
        }
    });

    if let Some((width, height)) = visible_size {
        return RestoredGeometry {
            x: saved.x,
            y: saved.y,
            width,
            height,
        };
    }

    let primary_scale = primary.scale_factor();
    let width = requested_width.min(f64::from(primary.width) / primary_scale);
    let height = requested_height.min(f64::from(primary.height) / primary_scale);
    let physical_width = (width * primary_scale).round() as i64;
    let physical_height = (height * primary_scale).round() as i64;
    RestoredGeometry {
        x: primary.x + ((i64::from(primary.width) - physical_width) / 2) as i32,
        y: primary.y + ((i64::from(primary.height) - physical_height) / 2) as i32,
        width,
        height,
    }
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
    #[serde(default)]
    due_reminders: bool,
    #[serde(default)]
    notification_prompted: bool,
    #[serde(default)]
    close_hint_shown: bool,
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

fn default_ui_settings() -> UiSettings {
    UiSettings {
        roll_over: true,
        pomo_focus_min: 25,
        pomo_break_min: 5,
        ..UiSettings::default()
    }
}

struct UiSettingsPersistence {
    lock: Mutex<()>,
}

impl UiSettingsPersistence {
    fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    fn read(&self, path: &Path) -> UiSettings {
        let Ok(_guard) = self.lock.lock() else {
            return default_ui_settings();
        };
        read_ui_settings_file(path)
    }

    fn update(
        &self,
        path: &Path,
        update: impl FnOnce(&mut UiSettings),
    ) -> Result<UiSettings, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|error| format!("无法锁定 UI 设置写入：{error}"))?;
        let mut settings = read_ui_settings_file(path);
        update(&mut settings);
        settings = normalize_ui_settings(settings);
        write_ui_settings_file(path, &settings)?;
        Ok(settings)
    }
}

fn read_ui_settings_file(path: &Path) -> UiSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .map(normalize_ui_settings)
        .unwrap_or_else(default_ui_settings)
}

fn write_ui_settings_file(path: &Path, settings: &UiSettings) -> Result<(), String> {
    let temp_path = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("无法序列化 UI 设置：{error}"))?;
    let result = (|| {
        use std::io::Write;
        let mut file = fs::File::create(&temp_path)
            .map_err(|error| format!("无法创建 UI 设置临时文件：{error}"))?;
        file.write_all(&text)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("无法写入 UI 设置临时文件：{error}"))?;
        drop(file);
        replace_file_atomically(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn read_ui_settings(app: &AppHandle) -> UiSettings {
    let Some(path) = ui_settings_path(app) else {
        return default_ui_settings();
    };
    app.state::<UiSettingsPersistence>().read(&path)
}

fn update_ui_settings(
    app: &AppHandle,
    update: impl FnOnce(&mut UiSettings),
) -> Result<UiSettings, String> {
    let Some(path) = ui_settings_path(app) else {
        return Err("无法获取 UI 设置路径".to_string());
    };
    app.state::<UiSettingsPersistence>().update(&path, update)
}

fn logical_inner_size(win: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let size = win.inner_size().ok()?;
    let scale = win.scale_factor().ok()?;
    if scale <= 0.0 {
        return None;
    }
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

fn should_save_window_geometry(is_minimized: bool, width: f64, height: f64) -> bool {
    !is_minimized && width.is_finite() && height.is_finite() && width > 1.0 && height > 1.0
}

fn save_window_geometry(app: &AppHandle, win: &tauri::WebviewWindow) {
    if app
        .try_state::<TemporaryWindowSize>()
        .is_some_and(|state| state.is_active())
    {
        return;
    }
    let Some((width, height)) = logical_inner_size(win) else {
        return;
    };
    if !should_save_window_geometry(win.is_minimized().unwrap_or(false), width, height) {
        return;
    }
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let Some(path) = window_geometry_path(app) else {
        return;
    };
    let geo = WindowGeometry {
        x: pos.x,
        y: pos.y,
        width: Some(width),
        height: Some(height),
    };
    if let Ok(text) = serde_json::to_string(&geo) {
        let _ = std::fs::write(path, text);
    }
}

fn restore_window_geometry(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Some(path) = window_geometry_path(app) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(geo) = serde_json::from_str::<WindowGeometry>(&text) else {
        return;
    };
    use tauri::{LogicalSize, PhysicalPosition, Position, Size};
    let primary_monitor = win.primary_monitor().ok().flatten();
    let available_monitors = win.available_monitors().unwrap_or_default();
    let Some(primary_monitor) = primary_monitor.or_else(|| available_monitors.first().cloned())
    else {
        let _ = win.set_position(Position::Physical(PhysicalPosition::new(geo.x, geo.y)));
        return;
    };
    let primary_rect = primary_monitor.work_area();
    let primary = WorkArea::new(
        primary_rect.position.x,
        primary_rect.position.y,
        primary_rect.size.width,
        primary_rect.size.height,
        primary_monitor.scale_factor(),
    );
    let work_areas = available_monitors
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            WorkArea::new(
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
                monitor.scale_factor(),
            )
        })
        .collect::<Vec<_>>();
    let restored = calculate_restored_geometry(&geo, &work_areas, primary);

    let _ = win.set_position(Position::Physical(PhysicalPosition::new(
        restored.x, restored.y,
    )));
    let _ = win.set_size(Size::Logical(LogicalSize::new(
        restored.width,
        restored.height,
    )));
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let temporary_size_active = app.state::<TemporaryWindowSize>().is_active();
        if !temporary_size_active {
            restore_window_geometry(app, &win);
            if read_ui_settings(app).collapsed {
                if let Some((width, _)) = logical_inner_size(&win) {
                    let _ = win.set_min_size(Some(tauri::LogicalSize::new(360.0, 40.0)));
                    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        width.max(360.0),
                        40.0,
                    )));
                }
            }
        }
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn hide_main_window(app: &AppHandle, win: &tauri::WebviewWindow) {
    save_window_geometry(app, win);
    let _ = win.hide();
}

fn close_hint_shown_after_notification(already_shown: bool, notification_succeeded: bool) -> bool {
    already_shown || notification_succeeded
}

fn notify_close_to_tray_once(app: &AppHandle) {
    let mut settings = read_ui_settings(app);
    if settings.close_hint_shown {
        return;
    }

    use tauri_plugin_notification::NotificationExt;
    let notification_succeeded = app
        .notification()
        .builder()
        .title("Otodo 仍在后台运行")
        .body("窗口已收至托盘；如需真正退出，请使用托盘菜单中的“退出”。")
        .show()
        .is_ok();
    settings.close_hint_shown =
        close_hint_shown_after_notification(settings.close_hint_shown, notification_succeeded);
    if settings.close_hint_shown {
        let _ = update_ui_settings(app, |current| {
            current.close_hint_shown = true;
        });
    }
}

fn reset_main_window_position(app: &AppHandle) -> Result<(), String> {
    use tauri::{LogicalSize, PhysicalPosition, Position, Size};
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "窗口不存在".to_string())?;
    let primary_monitor = win
        .primary_monitor()
        .map_err(|error| format!("无法读取主显示器：{error}"))?
        .ok_or_else(|| "未找到主显示器".to_string())?;
    let area = primary_monitor.work_area();
    let primary = WorkArea::new(
        area.position.x,
        area.position.y,
        area.size.width,
        area.size.height,
        primary_monitor.scale_factor(),
    );
    let (width, height) =
        logical_inner_size(&win).unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
    let geometry = WindowGeometry {
        x: i32::MAX,
        y: i32::MAX,
        width: Some(width),
        height: Some(height),
    };
    let restored = calculate_restored_geometry(&geometry, &[primary], primary);
    win.set_position(Position::Physical(PhysicalPosition::new(
        restored.x, restored.y,
    )))
    .map_err(|error| format!("无法重置窗口位置：{error}"))?;
    win.set_size(Size::Logical(LogicalSize::new(
        restored.width,
        restored.height,
    )))
    .map_err(|error| format!("无法重置窗口尺寸：{error}"))?;
    save_window_geometry(app, &win);
    Ok(())
}

fn mins_to_dur(mins: u32) -> Duration {
    Duration::from_secs(u64::from(mins.max(1)) * 60)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    revision: u64,
}

struct Pomo {
    phase: PomoPhase,
    running: bool,
    remaining: Duration,
    deadline: Option<Instant>,
    rounds_today: u32,
    rounds_date: String,
    focus: Duration,
    break_d: Duration,
    focus_min: u32,
    break_min: u32,
    revision: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedPomo {
    phase: PomoPhase,
    running: bool,
    deadline_unix_ms: Option<u64>,
    remaining_ms: u64,
    rounds_today: u32,
    rounds_date: String,
    focus_min: u32,
    break_min: u32,
}

fn local_day_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn local_day_bounds_ms(day_key: &str) -> Option<(u64, u64)> {
    use chrono::TimeZone;
    let date = chrono::NaiveDate::parse_from_str(day_key, "%Y-%m-%d").ok()?;
    let next_date = date.succ_opt()?;
    let start = chrono::Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    let next_start = chrono::Local
        .from_local_datetime(&next_date.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    Some((
        u64::try_from(start.timestamp_millis()).ok()?,
        u64::try_from(next_start.timestamp_millis()).ok()?,
    ))
}

fn count_periodic_events(first_ms: u64, period_ms: u64, start_ms: u64, end_ms: u64) -> u64 {
    if period_ms == 0 || start_ms > end_ms || first_ms > end_ms {
        return 0;
    }
    let first_index = if start_ms <= first_ms {
        0
    } else {
        start_ms
            .saturating_sub(first_ms)
            .saturating_add(period_ms - 1)
            / period_ms
    };
    let last_index = end_ms.saturating_sub(first_ms) / period_ms;
    if first_index > last_index {
        0
    } else {
        last_index - first_index + 1
    }
}

impl Pomo {
    fn new() -> Self {
        Self::with_durations(25, 5, &local_day_key())
    }

    fn with_durations(focus_min: u32, break_min: u32, today: &str) -> Self {
        let focus_min = clamp_mins(focus_min, 1, 180, 25);
        let break_min = clamp_mins(break_min, 1, 60, 5);
        let focus = mins_to_dur(focus_min);
        let break_d = mins_to_dur(break_min);
        Self {
            phase: PomoPhase::Idle,
            running: false,
            remaining: focus,
            deadline: None,
            rounds_today: 0,
            rounds_date: today.to_string(),
            focus,
            break_d,
            focus_min,
            break_min,
            revision: 0,
        }
    }

    fn restore(state: PersistedPomo, now_unix_ms: u64, today: &str) -> Self {
        let mut restored = Self::with_durations(state.focus_min, state.break_min, today);
        restored.phase = state.phase;
        restored.rounds_today = if state.rounds_date == today {
            state.rounds_today
        } else {
            0
        };
        restored.rounds_date = today.to_string();

        if state.running {
            let deadline_unix_ms = state.deadline_unix_ms.unwrap_or(now_unix_ms);
            if deadline_unix_ms > now_unix_ms {
                let remaining = Duration::from_millis(deadline_unix_ms - now_unix_ms);
                restored.running = true;
                restored.remaining = remaining;
                restored.deadline = Some(Instant::now() + remaining);
            } else {
                let elapsed_ms = now_unix_ms.saturating_sub(deadline_unix_ms);
                let focus_ms = restored.focus.as_millis() as u64;
                let break_ms = restored.break_d.as_millis() as u64;
                let cycle_ms = focus_ms.saturating_add(break_ms);
                let normalized_phase = if state.phase == PomoPhase::Idle {
                    PomoPhase::Focus
                } else {
                    state.phase
                };
                let cycle_offset = elapsed_ms % cycle_ms;
                let (phase, remaining_ms) = match normalized_phase {
                    PomoPhase::Focus if cycle_offset < break_ms => {
                        (PomoPhase::Break, break_ms - cycle_offset)
                    }
                    PomoPhase::Focus => (PomoPhase::Focus, cycle_ms - cycle_offset),
                    PomoPhase::Break if cycle_offset < focus_ms => {
                        (PomoPhase::Focus, focus_ms - cycle_offset)
                    }
                    PomoPhase::Break => (PomoPhase::Break, cycle_ms - cycle_offset),
                    PomoPhase::Idle => unreachable!(),
                };
                let first_focus_completion_ms = match normalized_phase {
                    PomoPhase::Focus => deadline_unix_ms,
                    PomoPhase::Break => deadline_unix_ms.saturating_add(focus_ms),
                    PomoPhase::Idle => unreachable!(),
                };
                let completed_focus_rounds = if state.rounds_date == today {
                    count_periodic_events(first_focus_completion_ms, cycle_ms, 0, now_unix_ms)
                } else if let Some((day_start_ms, next_day_start_ms)) = local_day_bounds_ms(today) {
                    count_periodic_events(
                        first_focus_completion_ms,
                        cycle_ms,
                        day_start_ms,
                        now_unix_ms.min(next_day_start_ms.saturating_sub(1)),
                    )
                } else {
                    0
                };
                restored.rounds_today = restored
                    .rounds_today
                    .saturating_add(completed_focus_rounds.min(u64::from(u32::MAX)) as u32);
                restored.phase = phase;
                restored.running = true;
                restored.remaining = Duration::from_millis(remaining_ms);
                restored.deadline = Some(Instant::now() + restored.remaining);
            }
        } else {
            restored.remaining = Duration::from_millis(state.remaining_ms);
        }
        restored
    }

    fn touch(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    fn persisted(&self, now_unix_ms: u64) -> PersistedPomo {
        let remaining = self.remaining_now();
        PersistedPomo {
            phase: self.phase,
            running: self.running,
            deadline_unix_ms: self
                .running
                .then_some(now_unix_ms.saturating_add(remaining.as_millis() as u64)),
            remaining_ms: remaining.as_millis() as u64,
            rounds_today: self.rounds_today,
            rounds_date: self.rounds_date.clone(),
            focus_min: self.focus_min,
            break_min: self.break_min,
        }
    }

    fn reset_rounds_for_day(&mut self, today: &str) -> bool {
        if self.rounds_date == today {
            return false;
        }
        self.rounds_date = today.to_string();
        self.rounds_today = 0;
        true
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
            revision: self.revision,
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
        let rounds_date = self.rounds_date.clone();
        let revision = self.revision;
        *self = Self {
            remaining: focus,
            focus,
            break_d,
            focus_min,
            break_min,
            rounds_today: rounds,
            rounds_date,
            revision,
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

fn pomodoro_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("pomodoro.json"))
}

fn read_pomodoro_state(
    path: &Path,
    fallback_focus_min: u32,
    fallback_break_min: u32,
    now_unix_ms: u64,
    today: &str,
) -> Pomo {
    let state = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<PersistedPomo>(&text).ok());
    match state {
        Some(state) => Pomo::restore(state, now_unix_ms, today),
        None => Pomo::with_durations(fallback_focus_min, fallback_break_min, today),
    }
}

fn replace_file_atomically(temp_path: &Path, target_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        extern "system" {
            fn MoveFileExW(
                existing_file_name: *const u16,
                new_file_name: *const u16,
                flags: u32,
            ) -> i32;
        }
        let source: Vec<u16> = temp_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target: Vec<u16> = target_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let result = unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            return Err(format!(
                "无法将临时文件 {} 原子替换为 {}：{}",
                temp_path.display(),
                target_path.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        fs::rename(temp_path, target_path).map_err(|error| {
            format!(
                "无法将临时文件 {} 原子替换为 {}：{error}",
                temp_path.display(),
                target_path.display()
            )
        })
    }
}

fn write_pomodoro_state(path: &Path, revision: u64, state: &PersistedPomo) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("pomodoro.json");
    let temp_path = path.with_file_name(format!("{file_name}.{revision}.tmp"));
    let text = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("无法序列化番茄钟状态：{error}"))?;
    let result = (|| {
        let mut file = fs::File::create(&temp_path)
            .map_err(|error| format!("无法创建番茄钟临时状态文件：{error}"))?;
        use std::io::Write;
        file.write_all(&text)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("无法写入番茄钟临时状态文件：{error}"))?;
        drop(file);
        replace_file_atomically(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

struct PomodoroPersistence {
    write_lock: Mutex<()>,
    highest_started_revision: AtomicU64,
    latest_written_revision: AtomicU64,
}

impl PomodoroPersistence {
    fn new() -> Self {
        Self {
            write_lock: Mutex::new(()),
            highest_started_revision: AtomicU64::new(0),
            latest_written_revision: AtomicU64::new(0),
        }
    }

    fn persist(&self, path: &Path, revision: u64, state: &PersistedPomo) -> Result<(), String> {
        let _write_guard = self
            .write_lock
            .lock()
            .map_err(|error| format!("无法锁定番茄钟状态写入：{error}"))?;
        let highest_started = self.highest_started_revision.load(Ordering::Acquire);
        let latest_written = self.latest_written_revision.load(Ordering::Acquire);
        if revision < highest_started || revision <= latest_written {
            return Ok(());
        }
        self.highest_started_revision
            .store(revision, Ordering::Release);
        write_pomodoro_state(path, revision, state)?;
        self.latest_written_revision
            .store(revision, Ordering::Release);
        Ok(())
    }
}

fn persist_pomodoro(app: &AppHandle, revision: u64, state: &PersistedPomo) -> Result<(), String> {
    let path = pomodoro_state_path(app)?;
    app.state::<PomodoroPersistence>()
        .persist(&path, revision, state)
}

struct TrayPomoUi {
    item: MenuItem<Wry>,
    last: Mutex<String>,
}

#[derive(Default)]
struct RevisionGate(Mutex<u64>);

impl RevisionGate {
    fn run_if_current(&self, revision: u64, update: impl FnOnce()) -> bool {
        let Ok(mut current) = self.0.lock() else {
            return false;
        };
        if revision <= *current {
            return false;
        }
        *current = revision;
        update();
        true
    }
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
    app.state::<RevisionGate>()
        .run_if_current(snap.revision, || {
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
        });
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

#[tauri::command]
fn send_due_notification(app: AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| format!("发送系统通知失败：{error}"))
}

fn with_pomo<T>(app: &AppHandle, f: impl FnOnce(&mut Pomo) -> T) -> Result<T, String> {
    let state = app.state::<Arc<Mutex<Pomo>>>();
    let mut p = state.lock().map_err(|e| e.to_string())?;
    Ok(f(&mut p))
}

#[tauri::command]
fn get_pomodoro(app: AppHandle) -> Result<PomoSnap, String> {
    let today = local_day_key();
    let (snap, persistence) = with_pomo(&app, |p| {
        let changed = p.reset_rounds_for_day(&today);
        let persistence = changed.then(|| {
            let revision = p.touch();
            (revision, p.persisted(unix_time_ms()))
        });
        (p.snapshot(), persistence)
    })?;
    if let Some((revision, state)) = persistence {
        persist_pomodoro(&app, revision, &state)?;
    }
    Ok(snap)
}

#[tauri::command]
fn pomodoro_toggle(app: AppHandle) -> Result<PomoSnap, String> {
    let today = local_day_key();
    let (snap, revision, state) = with_pomo(&app, |p| {
        p.reset_rounds_for_day(&today);
        p.toggle();
        let revision = p.touch();
        (p.snapshot(), revision, p.persisted(unix_time_ms()))
    })?;
    persist_pomodoro(&app, revision, &state)?;
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn pomodoro_reset(app: AppHandle) -> Result<PomoSnap, String> {
    let today = local_day_key();
    let (snap, revision, state) = with_pomo(&app, |p| {
        p.reset_rounds_for_day(&today);
        p.reset();
        let revision = p.touch();
        (p.snapshot(), revision, p.persisted(unix_time_ms()))
    })?;
    persist_pomodoro(&app, revision, &state)?;
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn pomodoro_set_durations(
    app: AppHandle,
    focus_min: u32,
    break_min: u32,
) -> Result<PomoSnap, String> {
    let today = local_day_key();
    let (snap, revision, state) = with_pomo(&app, |p| {
        p.reset_rounds_for_day(&today);
        p.set_durations(focus_min, break_min);
        let revision = p.touch();
        (p.snapshot(), revision, p.persisted(unix_time_ms()))
    })?;
    persist_pomodoro(&app, revision, &state)?;
    update_ui_settings(&app, |settings| {
        settings.pomo_focus_min = snap.focus_min;
        settings.pomo_break_min = snap.break_min;
    })?;
    emit_pomo(&app, &snap);
    Ok(snap)
}

#[tauri::command]
fn init_db(app: AppHandle) -> Result<(), String> {
    let database_path = db_path(&app)?;
    let backup_directory = data_dir(&app)?.join("backups");
    initialize_database(&database_path, &backup_directory)?;
    Ok(())
}

#[tauri::command]
fn get_data_dir(app: AppHandle) -> Result<String, String> {
    Ok(data_dir(&app)?.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_todos(app: AppHandle) -> Result<Vec<Todo>, String> {
    let c = conn(&app)?;
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
// 参数对应现有 IPC 字段，保留展开签名以避免改变前端接口。
#[allow(clippy::too_many_arguments)]
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
    let c = conn(&app)?;
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
    let mut connection = conn(&app)?;
    toggle_todo_atomically(&mut connection, &id, done, day.as_deref())
}

fn toggle_todo_atomically(
    connection: &mut Connection,
    id: &str,
    done: bool,
    day: Option<&str>,
) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始待办完成事务：{error}"))?;
    toggle_todo_in_connection(&transaction, id, done, day)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交待办完成事务：{error}"))
}

fn toggle_todo_in_connection(
    connection: &Connection,
    id: &str,
    done: bool,
    day: Option<&str>,
) -> Result<(), String> {
    // 重复待办：按天记录完成状态（done_dates 存空格分隔的 ymd 列表，与前端/服务端一致）
    let repeat = match connection.query_row(
        "SELECT repeat FROM todos WHERE id = ?1",
        params![id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(repeat) => repeat,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(format!("待办不存在：{id}"));
        }
        Err(error) => {
            return Err(format!("查询待办失败：{error}"));
        }
    };

    if repeat != "none" {
        if let Some(day) = day {
            let current_done_dates: String = connection
                .query_row(
                    "SELECT done_dates FROM todos WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("读取重复待办完成日期失败：{error}"))?;
            let mut set: Vec<&str> = current_done_dates
                .split(' ')
                .filter(|value| !value.is_empty())
                .collect();
            if done {
                if !set.contains(&day) {
                    set.push(day);
                }
            } else {
                set.retain(|value| *value != day);
            }
            let new_val = set.join(" ");
            let changed = connection
                .execute(
                    "UPDATE todos SET done_dates = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
                    params![new_val, id],
                )
                .map_err(|error| format!("更新重复待办失败：{error}"))?;
            require_one_affected_row(changed, id)?;
            return Ok(());
        }
    }
    let changed = connection
        .execute(
            "UPDATE todos SET done = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
            params![done as i32, id],
        )
        .map_err(|error| format!("更新待办完成状态失败：{error}"))?;
    require_one_affected_row(changed, id)
}

#[tauri::command]
fn delete_todo(app: AppHandle, id: String) -> Result<(), String> {
    let mut connection = conn(&app)?;
    delete_todo_in_connection(&mut connection, &id)
}

fn delete_todo_in_connection(connection: &mut Connection, id: &str) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始删除待办事务：{error}"))?;
    let changed = transaction
        .execute(
            "UPDATE todos SET deleted = 1, dirty = 1, updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )
        .map_err(|error| format!("删除待办失败：{error}"))?;
    require_one_affected_row(changed, id)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交删除待办事务：{error}"))
}

#[tauri::command]
fn restore_todo(app: AppHandle, id: String) -> Result<(), String> {
    let mut connection = conn(&app)?;
    restore_todo_in_connection(&mut connection, &id)
}

fn restore_todo_in_connection(connection: &mut Connection, id: &str) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始恢复待办事务：{error}"))?;
    let changed = transaction
        .execute(
            "UPDATE todos
             SET deleted = 0, dirty = 1, updated_at = datetime('now')
             WHERE id = ?1 AND deleted = 1",
            params![id],
        )
        .map_err(|error| format!("恢复待办失败：{error}"))?;
    if changed == 0 {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM todos WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| format!("检查待办恢复状态失败：{error}"))?;
        if exists {
            return Err(format!("待办 {id} 未处于删除状态"));
        }
        return Err(format!("待办不存在：{id}"));
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交恢复待办事务：{error}"))
}

/// 通用更新：fields 为部分字段对象，传入 null 表示「清空该字段」（修复 Bug 4）
#[tauri::command]
fn update_todo(app: AppHandle, id: String, fields: serde_json::Value) -> Result<(), String> {
    let mut connection = conn(&app)?;
    update_todo_in_connection(&mut connection, &id, &fields)
}

fn update_todo_in_connection(
    connection: &mut Connection,
    id: &str,
    fields: &serde_json::Value,
) -> Result<(), String> {
    let obj = fields
        .as_object()
        .ok_or_else(|| "fields 必须是对象".to_string())?;

    let mut clauses: Vec<String> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    for (key, value) in obj {
        let (column, converted) = validate_update_field(key, value)?;
        clauses.push(format!("{column} = ?"));
        vals.push(converted);
    }

    // 本地为真实数据源：任何修改都标记 dirty 并刷新 updated_at
    clauses.push("dirty = 1".to_string());
    clauses.push("updated_at = datetime('now')".to_string());

    vals.push(Value::Text(id.to_string()));
    let sql = format!("UPDATE todos SET {} WHERE id = ?", clauses.join(", "));
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始更新待办事务：{error}"))?;
    let changed = transaction
        .execute(&sql, params_from_iter(vals.iter()))
        .map_err(|error| format!("更新待办失败：{error}"))?;
    require_one_affected_row(changed, id)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交更新待办事务：{error}"))
}

fn validate_update_field(
    key: &str,
    value: &serde_json::Value,
) -> Result<(&'static str, Value), String> {
    match key {
        "title" => required_text_value(key, "title", value),
        "note" => nullable_text_value(key, "note", value),
        "due_at" => nullable_text_value(key, "due_at", value),
        "due_date" => nullable_text_value(key, "due_date", value),
        "all_day" => required_bool_value(key, "all_day", value),
        "done" => required_bool_value(key, "done", value),
        "sort_order" => required_integer_value(key, "sort_order", value),
        "repeat" => required_text_value(key, "repeat", value),
        "done_dates" => required_text_value(key, "done_dates", value),
        "repeat_until" => nullable_text_value(key, "repeat_until", value),
        "skip_dates" => nullable_text_value(key, "skip_dates", value),
        _ => Err(format!("不支持的更新字段：{key}")),
    }
}

fn required_text_value(
    key: &str,
    column: &'static str,
    value: &serde_json::Value,
) -> Result<(&'static str, Value), String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("字段 {key} 必须是字符串且不能为 null"))?;
    Ok((column, Value::Text(text.to_string())))
}

fn nullable_text_value(
    key: &str,
    column: &'static str,
    value: &serde_json::Value,
) -> Result<(&'static str, Value), String> {
    if value.is_null() {
        return Ok((column, Value::Null));
    }
    required_text_value(key, column, value)
}

fn required_bool_value(
    key: &str,
    column: &'static str,
    value: &serde_json::Value,
) -> Result<(&'static str, Value), String> {
    let boolean = value
        .as_bool()
        .ok_or_else(|| format!("字段 {key} 必须是布尔值且不能为 null"))?;
    Ok((column, Value::Integer(i64::from(boolean))))
}

fn required_integer_value(
    key: &str,
    column: &'static str,
    value: &serde_json::Value,
) -> Result<(&'static str, Value), String> {
    let integer = value
        .as_i64()
        .ok_or_else(|| format!("字段 {key} 必须是整数且不能为 null"))?;
    Ok((column, Value::Integer(integer)))
}

/// 批量重排顺序（修复 Bug 8：一次调用完成整列排序，避免 N 次串行请求）
#[tauri::command]
fn reorder_todos(app: AppHandle, orders: Vec<OrderItem>) -> Result<(), String> {
    let mut connection = conn(&app)?;
    reorder_todos_in_connection(&mut connection, &orders)
}

fn reorder_todos_in_connection(
    connection: &mut Connection,
    orders: &[OrderItem],
) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始待办排序事务：{error}"))?;
    reorder_todos_in_transaction(&transaction, orders)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交待办排序事务：{error}"))
}

fn reorder_todos_in_transaction(
    transaction: &Transaction<'_>,
    orders: &[OrderItem],
) -> Result<(), String> {
    for order in orders {
        let changed = transaction
            .execute(
            "UPDATE todos SET sort_order = ?1, dirty = 1, updated_at = datetime('now') WHERE id = ?2",
            params![order.sort_order, order.id],
        )
            .map_err(|error| format!("更新待办排序失败：{error}"))?;
        require_one_affected_row(changed, &order.id)?;
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
fn move_todo(
    app: AppHandle,
    id: String,
    due_at: Option<String>,
    due_date: Option<String>,
    all_day: bool,
    orders: Vec<OrderItem>,
) -> Result<(), String> {
    let mut connection = conn(&app)?;
    move_todo_in_connection(
        &mut connection,
        &id,
        due_at.as_deref(),
        due_date.as_deref(),
        all_day,
        &orders,
    )
}

fn move_todo_in_connection(
    connection: &mut Connection,
    id: &str,
    due_at: Option<&str>,
    due_date: Option<&str>,
    all_day: bool,
    orders: &[OrderItem],
) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始移动待办事务：{error}"))?;
    let changed = transaction
        .execute(
            "UPDATE todos
             SET due_at = ?1, due_date = ?2, all_day = ?3,
                 dirty = 1, updated_at = datetime('now')
             WHERE id = ?4",
            params![due_at, due_date, all_day as i32, id],
        )
        .map_err(|error| format!("更新待办日期失败：{error}"))?;
    require_one_affected_row(changed, id)?;
    reorder_todos_in_transaction(&transaction, orders)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交移动待办事务：{error}"))
}

#[tauri::command]
fn roll_over_todos(app: AppHandle, moves: Vec<DateUpdate>) -> Result<(), String> {
    let mut connection = conn(&app)?;
    roll_over_todos_in_connection(&mut connection, &moves)
}

fn roll_over_todos_in_connection(
    connection: &mut Connection,
    moves: &[DateUpdate],
) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("无法开始顺延待办事务：{error}"))?;
    for item in moves {
        let changed = transaction
            .execute(
                "UPDATE todos
                 SET due_at = ?1, due_date = ?2, all_day = ?3,
                     dirty = 1, updated_at = datetime('now')
                 WHERE id = ?4",
                params![item.due_at, item.due_date, item.all_day as i32, item.id],
            )
            .map_err(|error| format!("顺延待办失败：{error}"))?;
        require_one_affected_row(changed, &item.id)?;
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交顺延待办事务：{error}"))
}

fn require_one_affected_row(changed: usize, id: &str) -> Result<(), String> {
    if changed == 1 {
        return Ok(());
    }
    if changed == 0 {
        return Err(format!("待办不存在：{id}"));
    }
    Err(format!("待办 {id} 更新了 {changed} 行，已拒绝提交"))
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
fn resize_window(app: &AppHandle, width: f64, height: f64, temporary: bool) -> Result<(), String> {
    use tauri::{LogicalSize, Size};
    app.state::<TemporaryWindowSize>().set_active(temporary);
    if let Some(win) = app.get_webview_window("main") {
        // 改尺寸前记下位置，避免部分平台 resize 后窗口跳回默认位置
        let pos = win.outer_position().ok();
        let min_height = if height <= 40.0 { 40.0 } else { 320.0 };
        win.set_min_size(Some(LogicalSize::new(360.0, min_height)))
            .map_err(|e| e.to_string())?;
        win.set_size(Size::Logical(LogicalSize::new(width, height)))
            .map_err(|e| e.to_string())?;
        if let Some(p) = pos {
            use tauri::{PhysicalPosition, Position};
            let _ = win.set_position(Position::Physical(PhysicalPosition::new(p.x, p.y)));
            if !temporary {
                save_window_geometry(app, &win);
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn set_window_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    resize_window(&app, width, height, false)
}

#[tauri::command]
fn set_temporary_window_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    resize_window(&app, width, height, true)
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
        notify_close_to_tray_once(&app);
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
    update_ui_settings(&app, |current| {
        let pomo_focus_min = current.pomo_focus_min;
        let pomo_break_min = current.pomo_break_min;
        let close_hint_shown = current.close_hint_shown;
        *current = settings;
        current.pomo_focus_min = pomo_focus_min;
        current.pomo_break_min = pomo_break_min;
        current.close_hint_shown = close_hint_shown;
    })?;
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
        .manage(TemporaryWindowSize::default())
        .manage(Arc::new(Mutex::new(Pomo::new())))
        .manage(PomodoroPersistence::new())
        .manage(UiSettingsPersistence::new())
        .manage(RevisionGate::default())
        .invoke_handler(tauri::generate_handler![
            init_db,
            get_data_dir,
            get_todos,
            add_todo,
            update_todo,
            toggle_todo,
            delete_todo,
            restore_todo,
            reorder_todos,
            move_todo,
            roll_over_todos,
            drag_window,
            toggle_ghost,
            set_always_on_top,
            set_window_size,
            set_temporary_window_size,
            get_window_size,
            hide_window,
            show_window,
            get_ui_settings,
            save_ui_settings,
            get_pomodoro,
            pomodoro_toggle,
            pomodoro_reset,
            pomodoro_set_durations,
            send_due_notification
        ])
        .setup(|app| {
            // 启动时只恢复位置，保持隐藏；等前端首帧就绪后再 show，避免白闪。
            if let Some(win) = app.get_webview_window("main") {
                restore_window_geometry(app.handle(), &win);
                let settings = read_ui_settings(app.handle());
                let min_height = if settings.collapsed { 40.0 } else { 320.0 };
                let _ = win.set_min_size(Some(tauri::LogicalSize::new(360.0, min_height)));
                if settings.collapsed {
                    if let Some((width, _)) = logical_inner_size(&win) {
                        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                            width.max(360.0),
                            40.0,
                        )));
                    }
                }
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
                unsafe {
                    SetCurrentProcessExplicitAppUserModelID(aumid.as_ptr());
                }
            }

            // 注册全局快捷键 Alt+Space：显示/隐藏窗口（Bug 10）
            let _ = app
                .global_shortcut()
                .register(Shortcut::new(Some(Modifiers::ALT), Code::Space));

            // 开机自启：仅由安装程序询问或托盘菜单控制，启动时不再强制开启
            // ---- 系统托盘 (FR-14) ----
            let show_item = MenuItemBuilder::with_id("show", "显示窗口").build(app)?;
            let new_item = MenuItemBuilder::with_id("new_todo", "新建待办").build(app)?;
            let pomo_item = MenuItemBuilder::with_id("pomodoro", "开始专注  25:00").build(app)?;
            let reset_position_item =
                MenuItemBuilder::with_id("reset_position", "重置窗口位置").build(app)?;
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
                    &reset_position_item,
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
                    "reset_position" => {
                        if reset_main_window_position(app).is_ok() {
                            show_main_window(app);
                        }
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
                let today = local_day_key();
                let restored = pomodoro_state_path(app.handle())
                    .map(|path| {
                        read_pomodoro_state(
                            &path,
                            s.pomo_focus_min,
                            s.pomo_break_min,
                            unix_time_ms(),
                            &today,
                        )
                    })
                    .unwrap_or_else(|_| {
                        Pomo::with_durations(s.pomo_focus_min, s.pomo_break_min, &today)
                    });
                let restored_snapshot = {
                    let mut p = app.state::<Arc<Mutex<Pomo>>>().inner().lock().ok();
                    if let Some(ref mut p) = p {
                        **p = restored;
                        let revision = p.touch();
                        Some((p.snapshot(), revision, p.persisted(unix_time_ms())))
                    } else {
                        None
                    }
                };
                if let Some((snap, revision, state)) = restored_snapshot {
                    let _ = persist_pomodoro(app.handle(), revision, &state);
                    emit_pomo(app.handle(), &snap);
                }
            }

            let pomo_arc = app.state::<Arc<Mutex<Pomo>>>().inner().clone();
            let tick_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(200));
                let Ok(mut p) = pomo_arc.lock() else { continue };
                let day_changed = p.reset_rounds_for_day(&local_day_key());
                let running = p.running;
                let finished = p.finish_if_due();
                if !running && finished.is_none() && !day_changed {
                    continue;
                }
                let revision = p.touch();
                let snap = p.snapshot();
                let persisted = (finished.is_some() || day_changed)
                    .then(|| (revision, p.persisted(unix_time_ms())));
                drop(p);
                if let Some((revision, state)) = persisted {
                    let _ = persist_pomodoro(&tick_handle, revision, &state);
                }
                emit_pomo(&tick_handle, &snap);
                if let Some(phase) = finished {
                    notify_pomo(&tick_handle, phase, snap.break_min);
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                if window.label() == "main" {
                    if let Some(win) = window.app_handle().get_webview_window("main") {
                        save_window_geometry(window.app_handle(), &win);
                    }
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(win) = app.get_webview_window("main") {
                        hide_main_window(&app, &win);
                        notify_close_to_tray_once(&app);
                    }
                });
            }
            _ => {}
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
    use winapi::um::winuser::{ReleaseCapture, SendMessageW, HTCAPTION, WM_NCLBUTTONDOWN};
    if let Ok(raw) = win.hwnd() {
        let hwnd = raw.0 as HWND;
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
        }
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("otodo-{name}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("应能创建测试目录");
            Self { path }
        }

        fn database_path(&self) -> PathBuf {
            self.path.join("otodo.db")
        }

        fn backup_dir(&self) -> PathBuf {
            self.path.join("backups")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn columns(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare("PRAGMA table_info(todos)")
            .expect("应能读取表结构");
        statement
            .query_map([], |row| row.get(1))
            .expect("应能查询列")
            .collect::<rusqlite::Result<Vec<String>>>()
            .expect("列名应可读取")
    }

    fn backup_files(path: &Path) -> Vec<PathBuf> {
        if !path.exists() {
            return Vec::new();
        }
        fs::read_dir(path)
            .expect("应能读取备份目录")
            .map(|entry| entry.expect("备份目录项应有效").path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("db"))
            .collect()
    }

    #[test]
    fn fresh_database_is_created_without_backup() {
        let directory = TestDirectory::new("fresh");

        let connection = initialize_database(&directory.database_path(), &directory.backup_dir())
            .expect("全新数据库应初始化成功");

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .expect("应能读取版本"),
            DATABASE_VERSION
        );
        assert!(columns(&connection).contains(&"title".to_string()));
        assert!(backup_files(&directory.backup_dir()).is_empty());
    }

    #[test]
    fn current_database_reopens_without_losing_data() {
        let directory = TestDirectory::new("current");
        let database_path = directory.database_path();
        let backup_dir = directory.backup_dir();
        let connection =
            initialize_database(&database_path, &backup_dir).expect("首次初始化应成功");
        connection
            .execute(
                "INSERT INTO todos (id, title) VALUES ('current-1', '保留当前数据')",
                [],
            )
            .expect("应能写入当前库");
        drop(connection);

        let reopened = initialize_database(&database_path, &backup_dir).expect("当前库应可重开");

        let title: String = reopened
            .query_row(
                "SELECT title FROM todos WHERE id = 'current-1'",
                [],
                |row| row.get(0),
            )
            .expect("当前数据应保留");
        assert_eq!(title, "保留当前数据");
        assert!(backup_files(&backup_dir).is_empty());
    }

    #[test]
    fn legacy_database_missing_optional_columns_is_upgraded_in_place() {
        let directory = TestDirectory::new("legacy");
        let database_path = directory.database_path();
        let connection = Connection::open(&database_path).expect("应能创建旧库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO todos (id, title) VALUES ('legacy-1', '旧数据');",
            )
            .expect("应能准备旧结构");
        drop(connection);

        let upgraded =
            initialize_database(&database_path, &directory.backup_dir()).expect("旧库应升级成功");

        let upgraded_columns = columns(&upgraded);
        for name in [
            "note",
            "due_at",
            "all_day",
            "due_date",
            "sort_order",
            "deleted",
            "repeat",
            "done_dates",
            "repeat_until",
            "skip_dates",
            "dirty",
            "created_at",
            "updated_at",
        ] {
            assert!(
                upgraded_columns.contains(&name.to_string()),
                "缺少列 {name}"
            );
        }
        let title: String = upgraded
            .query_row("SELECT title FROM todos WHERE id = 'legacy-1'", [], |row| {
                row.get(0)
            })
            .expect("旧数据应保留");
        assert_eq!(title, "旧数据");
    }

    #[test]
    fn unknown_todos_structure_is_rejected_without_deleting_table() {
        let directory = TestDirectory::new("unknown");
        let database_path = directory.database_path();
        let connection = Connection::open(&database_path).expect("应能创建未知库");
        connection
            .execute_batch(
                "CREATE TABLE todos (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
                 INSERT INTO todos (id, payload) VALUES ('unknown-1', '必须保留');",
            )
            .expect("应能准备未知结构");
        drop(connection);

        let error = initialize_database(&database_path, &directory.backup_dir())
            .expect_err("未知结构必须停止迁移");

        assert!(error.contains("title"), "错误应说明缺少 title：{error}");
        let untouched = Connection::open(&database_path).expect("原库应仍可打开");
        let payload: String = untouched
            .query_row(
                "SELECT payload FROM todos WHERE id = 'unknown-1'",
                [],
                |row| row.get(0),
            )
            .expect("未知表及原数据不能被删除");
        assert_eq!(payload, "必须保留");
    }

    #[test]
    fn all_version_migrations_roll_back_together_when_later_version_fails() {
        let mut connection = Connection::open_in_memory().expect("应能创建内存库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO todos (id, title) VALUES ('rollback-1', '原数据');",
            )
            .expect("应能准备迁移前数据");

        fn version_2(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
            transaction.execute("THIS IS NOT VALID SQL", [])?;
            Ok(())
        }

        let migrations: &[(i64, Migration)] = &[(1, migrate_to_version_1), (2, version_2)];
        let result = run_all_migrations_transaction(&mut connection, 0, migrations);

        assert!(result.is_err());
        assert!(
            !columns(&connection).contains(&"note".to_string()),
            "真实版本 1 增加的列也必须整体回滚"
        );
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("应能读取回滚后的版本");
        assert_eq!(version, 0);
        let title: String = connection
            .query_row(
                "SELECT title FROM todos WHERE id = 'rollback-1'",
                [],
                |row| row.get(0),
            )
            .expect("失败后原数据应保留");
        assert_eq!(title, "原数据");
    }

    #[test]
    fn table_with_title_but_missing_required_base_column_is_rejected() {
        let directory = TestDirectory::new("unsafe-base");
        let database_path = directory.database_path();
        let connection = Connection::open(&database_path).expect("应能创建未知库");
        connection
            .execute_batch(
                "CREATE TABLE todos (title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO todos (title) VALUES ('缺少 id');",
            )
            .expect("应能准备缺少基础列的结构");
        drop(connection);

        let error = initialize_database(&database_path, &directory.backup_dir())
            .expect_err("缺少基础必需列的表必须拒绝迁移");

        assert!(error.contains("id"), "错误应列出缺少的基础列：{error}");
        let untouched = Connection::open(&database_path).expect("原库应仍可打开");
        let title: String = untouched
            .query_row("SELECT title FROM todos", [], |row| row.get(0))
            .expect("拒绝迁移后原数据应保留");
        assert_eq!(title, "缺少 id");
    }

    #[test]
    fn current_version_database_still_validates_required_columns() {
        let directory = TestDirectory::new("invalid-current");
        let database_path = directory.database_path();
        let connection = Connection::open(&database_path).expect("应能创建损坏的当前库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                 );
                 PRAGMA user_version = 2;",
            )
            .expect("应能准备版本号超前于结构的数据库");
        drop(connection);

        let error = initialize_database(&database_path, &directory.backup_dir())
            .expect_err("当前版本也必须验证完整结构");

        assert!(
            error.contains("repeat"),
            "错误应指出当前结构缺少必需列：{error}"
        );
    }

    #[test]
    fn toggle_todo_reports_missing_record() {
        let connection = Connection::open_in_memory().expect("应能创建内存库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    repeat TEXT NOT NULL DEFAULT 'none',
                    done_dates TEXT NOT NULL DEFAULT '',
                    done INTEGER NOT NULL DEFAULT 0,
                    dirty INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT
                 );",
            )
            .expect("应能准备待办表");

        let error = toggle_todo_in_connection(&connection, "missing", true, None)
            .expect_err("记录不存在必须明确报错");

        assert!(error.contains("不存在"), "错误应说明记录不存在：{error}");
    }

    #[test]
    fn toggle_todo_propagates_database_errors() {
        let connection = Connection::open_in_memory().expect("应能创建内存库");

        let error = toggle_todo_in_connection(&connection, "broken", true, None)
            .expect_err("数据库查询错误必须向上传播");

        assert!(
            error.contains("查询待办失败"),
            "错误应保留数据库失败上下文：{error}"
        );
    }

    fn create_write_test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("应能创建内存库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
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
                    updated_at TEXT
                 );",
            )
            .expect("应能准备待办表");
        connection
    }

    fn insert_write_test_todo(connection: &Connection, id: &str, repeat: &str, order: i32) {
        connection
            .execute(
                "INSERT INTO todos (id, title, repeat, sort_order, due_date)
                 VALUES (?1, ?2, ?3, ?4, '2026-11-20')",
                params![id, format!("待办 {id}"), repeat, order],
            )
            .expect("应能写入测试待办");
    }

    #[test]
    fn repeated_todo_completion_read_modify_write_is_atomic() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "repeat-1", "daily", 0);

        toggle_todo_atomically(&mut connection, "repeat-1", true, Some("2026-11-20"))
            .expect("第一次完成应成功");
        toggle_todo_atomically(&mut connection, "repeat-1", true, Some("2026-11-21"))
            .expect("第二次完成应保留第一次结果");

        let done_dates: String = connection
            .query_row(
                "SELECT done_dates FROM todos WHERE id = 'repeat-1'",
                [],
                |row| row.get(0),
            )
            .expect("应能读取完成日期");
        assert_eq!(done_dates, "2026-11-20 2026-11-21");
    }

    #[test]
    fn reorder_rolls_back_when_any_todo_is_missing() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "order-1", "none", 10);
        insert_write_test_todo(&connection, "order-2", "none", 20);
        let orders = vec![
            OrderItem {
                id: "order-1".to_string(),
                sort_order: 0,
            },
            OrderItem {
                id: "missing".to_string(),
                sort_order: 1,
            },
        ];

        reorder_todos_in_connection(&mut connection, &orders)
            .expect_err("任一待办不存在时必须整体失败");

        let original_order: i32 = connection
            .query_row(
                "SELECT sort_order FROM todos WHERE id = 'order-1'",
                [],
                |row| row.get(0),
            )
            .expect("应能读取回滚后的顺序");
        assert_eq!(original_order, 10);
    }

    #[test]
    fn cross_day_move_and_reorder_roll_back_together() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "move-1", "none", 10);
        insert_write_test_todo(&connection, "target-1", "none", 20);
        let orders = vec![
            OrderItem {
                id: "move-1".to_string(),
                sort_order: 0,
            },
            OrderItem {
                id: "missing".to_string(),
                sort_order: 1,
            },
        ];

        move_todo_in_connection(
            &mut connection,
            "move-1",
            None,
            Some("2026-11-21"),
            true,
            &orders,
        )
        .expect_err("排序失败时日期更新也必须回滚");

        let (due_date, sort_order): (String, i32) = connection
            .query_row(
                "SELECT due_date, sort_order FROM todos WHERE id = 'move-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("应能读取移动失败后的待办");
        assert_eq!(due_date, "2026-11-20");
        assert_eq!(sort_order, 10);
    }

    #[test]
    fn rollover_batch_rolls_back_when_any_todo_is_missing() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "roll-1", "none", 0);
        let moves = vec![
            DateUpdate {
                id: "roll-1".to_string(),
                due_at: None,
                due_date: Some("2026-11-21".to_string()),
                all_day: true,
            },
            DateUpdate {
                id: "missing".to_string(),
                due_at: None,
                due_date: Some("2026-11-21".to_string()),
                all_day: true,
            },
        ];

        roll_over_todos_in_connection(&mut connection, &moves)
            .expect_err("任一顺延失败时必须整体回滚");

        let due_date: String = connection
            .query_row(
                "SELECT due_date FROM todos WHERE id = 'roll-1'",
                [],
                |row| row.get(0),
            )
            .expect("应能读取顺延失败后的日期");
        assert_eq!(due_date, "2026-11-20");
    }

    #[test]
    fn update_todo_rejects_invalid_types_and_unknown_fields() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "update-1", "none", 0);

        let wrong_type = serde_json::json!({ "all_day": "yes" });
        let error = update_todo_in_connection(&mut connection, "update-1", &wrong_type)
            .expect_err("布尔字段不能接受字符串");
        assert!(error.contains("all_day"));

        let misspelled = serde_json::json!({ "due_data": "2026-11-21" });
        let error = update_todo_in_connection(&mut connection, "update-1", &misspelled)
            .expect_err("未知字段不能静默成功");
        assert!(error.contains("due_data"));
    }

    #[test]
    fn every_todo_update_requires_exactly_one_affected_row() {
        let mut connection = create_write_test_connection();

        let update_error = update_todo_in_connection(
            &mut connection,
            "missing",
            &serde_json::json!({ "title": "不存在" }),
        )
        .expect_err("通用更新必须检查 affected_rows");
        assert!(update_error.contains("不存在"));

        let delete_error = delete_todo_in_connection(&mut connection, "missing")
            .expect_err("删除必须检查 affected_rows");
        assert!(delete_error.contains("不存在"));
    }

    #[test]
    fn backup_rotation_keeps_manual_files() {
        let directory = TestDirectory::new("rotation");
        let backup_dir = directory.backup_dir();
        fs::create_dir_all(&backup_dir).expect("应能创建备份目录");
        for index in 0..6 {
            let name = format!("otodo-170000000000000000{index}.db");
            fs::write(backup_dir.join(name), b"automatic").expect("应能创建自动备份占位文件");
        }
        let manual_files = [
            "otodo-manual.db",
            "otodo-1700000000000000000-copy.db",
            "otodo-123.db",
            "important.db",
        ];
        for name in manual_files {
            fs::write(backup_dir.join(name), b"manual").expect("应能创建手工文件");
        }

        prune_database_backups(&backup_dir).expect("轮转应成功");

        let automatic_count = fs::read_dir(&backup_dir)
            .expect("应能读取备份目录")
            .filter_map(Result::ok)
            .filter(|entry| is_automatic_backup_name(&entry.file_name().to_string_lossy()))
            .count();
        assert_eq!(automatic_count, MAX_DATABASE_BACKUPS);
        for name in manual_files {
            assert!(backup_dir.join(name).exists(), "不能删除手工文件 {name}");
        }
    }

    #[test]
    fn failed_integrity_check_deletes_invalid_backup_and_reports_result() {
        let directory = TestDirectory::new("integrity");
        let backup_path = directory.path.join("otodo-1700000000000000000.db");
        fs::write(&backup_path, b"invalid backup").expect("应能创建无效备份占位文件");

        let error = handle_backup_integrity_result(
            &backup_path,
            Ok("row 2 missing from index todos_idx".to_string()),
        )
        .expect_err("完整性检查失败必须中止");

        assert!(!backup_path.exists(), "无效备份必须删除");
        assert!(
            error.contains("row 2 missing from index todos_idx"),
            "错误必须保留 SQLite 检查结果：{error}"
        );
    }

    #[test]
    fn upgrade_creates_readable_backup_with_original_data() {
        let directory = TestDirectory::new("backup");
        let database_path = directory.database_path();
        let connection = Connection::open(&database_path).expect("应能创建旧库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO todos (id, title) VALUES ('backup-1', '备份数据');",
            )
            .expect("应能准备旧数据");
        drop(connection);

        initialize_database(&database_path, &directory.backup_dir()).expect("升级应成功");

        let backups = backup_files(&directory.backup_dir());
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(&backups[0]).expect("备份数据库应可打开");
        let title: String = backup
            .query_row("SELECT title FROM todos WHERE id = 'backup-1'", [], |row| {
                row.get(0)
            })
            .expect("备份应包含升级前数据");
        assert_eq!(title, "备份数据");
    }

    #[test]
    fn concurrent_initialization_migrates_and_backs_up_only_once() {
        use std::sync::{Arc, Barrier};

        let directory = TestDirectory::new("concurrent-init");
        let database_path = directory.database_path();
        let backup_dir = directory.backup_dir();
        let connection = Connection::open(&database_path).expect("应能创建旧库");
        connection
            .execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO todos (id, title) VALUES ('concurrent-1', '并发迁移数据');",
            )
            .expect("应能准备旧数据");
        drop(connection);

        let barrier = Arc::new(Barrier::new(12));
        let mut workers = Vec::new();
        for _ in 0..12 {
            let barrier = barrier.clone();
            let database_path = database_path.clone();
            let backup_dir = backup_dir.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                initialize_database(&database_path, &backup_dir).map(|connection| drop(connection))
            }));
        }

        for worker in workers {
            worker
                .join()
                .expect("初始化线程不应崩溃")
                .expect("并发初始化不应 SQLITE_BUSY 或重复迁移");
        }

        assert_eq!(backup_files(&backup_dir).len(), 1);
        let connection = Connection::open(database_path).expect("迁移后数据库应可读取");
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .expect("应读取数据库版本"),
            DATABASE_VERSION,
        );
    }

    #[test]
    fn restore_todo_restores_only_a_soft_deleted_record() {
        let mut connection = create_write_test_connection();
        insert_write_test_todo(&connection, "restore-1", "none", 0);
        delete_todo_in_connection(&mut connection, "restore-1").expect("软删除应成功");

        restore_todo_in_connection(&mut connection, "restore-1").expect("撤销应恢复刚删除项");

        let (deleted, dirty): (i32, i32) = connection
            .query_row(
                "SELECT deleted, dirty FROM todos WHERE id = 'restore-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("应能读取恢复后的待办");
        assert_eq!(deleted, 0);
        assert_eq!(dirty, 1);

        let error = restore_todo_in_connection(&mut connection, "restore-1")
            .expect_err("不能恢复未删除的待办");
        assert!(error.contains("未处于删除状态"));
    }

    #[test]
    fn window_bounds_keep_visible_negative_coordinate_monitor() {
        let saved = WindowGeometry {
            x: -1500,
            y: 120,
            width: Some(1000.0),
            height: Some(720.0),
        };
        let areas = [
            WorkArea::new(0, 0, 1920, 1040, 1.0),
            WorkArea::new(-1920, 0, 1920, 1040, 1.0),
        ];

        let restored = calculate_restored_geometry(&saved, &areas, areas[0]);

        assert_eq!(restored.x, -1500);
        assert_eq!(restored.y, 120);
        assert_eq!(restored.width, 1000.0);
        assert_eq!(restored.height, 720.0);
    }

    #[test]
    fn window_bounds_center_on_primary_after_monitor_is_removed() {
        let saved = WindowGeometry {
            x: -1800,
            y: 100,
            width: Some(1000.0),
            height: Some(720.0),
        };
        let primary = WorkArea::new(0, 0, 1920, 1040, 1.0);

        let restored = calculate_restored_geometry(&saved, &[primary], primary);

        assert_eq!(restored.x, 460);
        assert_eq!(restored.y, 160);
    }

    #[test]
    fn window_bounds_handle_dpi_and_oversized_saved_window() {
        let saved = WindowGeometry {
            x: 2500,
            y: 100,
            width: Some(3000.0),
            height: Some(1800.0),
        };
        let primary = WorkArea::new(0, 0, 1920, 1040, 1.5);

        let restored = calculate_restored_geometry(&saved, &[primary], primary);

        assert_eq!(restored.width, 1280.0);
        assert!((restored.height - (1040.0 / 1.5)).abs() < 0.001);
        assert_eq!(restored.x, 0);
        assert_eq!(restored.y, 0);
    }

    #[test]
    fn window_bounds_recheck_visibility_after_clamping_oversized_window() {
        let saved = WindowGeometry {
            x: -3000,
            y: 100,
            width: Some(5000.0),
            height: Some(720.0),
        };
        let primary = WorkArea::new(0, 0, 1920, 1040, 1.0);

        let restored = calculate_restored_geometry(&saved, &[primary], primary);

        assert_eq!(restored.width, 1920.0);
        assert_eq!(restored.height, 720.0);
        assert_eq!(restored.x, 0);
        assert_eq!(restored.y, 160);
    }

    #[test]
    fn window_bounds_use_each_monitors_scale_for_visibility() {
        let saved = WindowGeometry {
            x: -3500,
            y: 100,
            width: Some(1000.0),
            height: Some(720.0),
        };
        let primary = WorkArea::new(0, 0, 1920, 1040, 1.0);
        let high_dpi_left = WorkArea::new(-2560, 0, 2560, 1400, 1.5);

        let restored = calculate_restored_geometry(&saved, &[primary, high_dpi_left], primary);

        assert_eq!(restored.x, -3500);
        assert_eq!(restored.y, 100);
        assert_eq!(restored.width, 1000.0);
        assert_eq!(restored.height, 720.0);
    }

    #[test]
    fn minimized_or_degenerate_window_geometry_is_not_persisted() {
        assert!(!should_save_window_geometry(true, 1000.0, 720.0));
        assert!(!should_save_window_geometry(false, 1.0, 720.0));
        assert!(!should_save_window_geometry(false, 1000.0, 1.0));
        assert!(should_save_window_geometry(false, 1000.0, 720.0));
        assert!(should_save_window_geometry(false, 1000.0, 40.0));
    }

    #[test]
    fn normal_window_resize_clears_temporary_recovery_marker() {
        let state = TemporaryWindowSize::default();

        state.set_active(true);
        assert!(state.is_active());

        state.set_active(false);
        assert!(!state.is_active());
    }

    #[test]
    fn pomodoro_revision_gate_rejects_stale_emit_and_tray_updates() {
        use std::sync::{mpsc, Arc, Barrier};

        let gate = Arc::new(RevisionGate::default());
        let entered_old_update = Arc::new(Barrier::new(2));
        let release_old_update = Arc::new(Barrier::new(2));
        let rendered_revision = Arc::new(AtomicU64::new(0));

        let old_worker = {
            let gate = gate.clone();
            let entered_old_update = entered_old_update.clone();
            let release_old_update = release_old_update.clone();
            let rendered_revision = rendered_revision.clone();
            std::thread::spawn(move || {
                assert!(gate.run_if_current(1, || {
                    entered_old_update.wait();
                    release_old_update.wait();
                    rendered_revision.store(1, Ordering::Release);
                }));
            })
        };

        entered_old_update.wait();
        let (new_update_finished_tx, new_update_finished_rx) = mpsc::channel();
        let new_worker = {
            let gate = gate.clone();
            let rendered_revision = rendered_revision.clone();
            std::thread::spawn(move || {
                assert!(gate.run_if_current(2, || {
                    rendered_revision.store(2, Ordering::Release);
                }));
                new_update_finished_tx
                    .send(())
                    .expect("应能报告新快照更新完成");
            })
        };
        let new_update_overtook_old = new_update_finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        release_old_update.wait();
        old_worker.join().expect("旧快照线程不应崩溃");
        new_worker.join().expect("新快照线程不应崩溃");

        assert!(
            !new_update_overtook_old,
            "新 revision 不能抢占正在执行的旧更新"
        );
        assert_eq!(rendered_revision.load(Ordering::Acquire), 2);
        assert!(!gate.run_if_current(1, || {
            rendered_revision.store(1, Ordering::Release);
        }));
        assert_eq!(rendered_revision.load(Ordering::Acquire), 2);
    }

    #[test]
    fn close_hint_is_persisted_only_after_notification_succeeds() {
        let after_failed_attempt = close_hint_shown_after_notification(false, false);
        assert!(!after_failed_attempt);

        let after_retry = close_hint_shown_after_notification(after_failed_attempt, true);
        assert!(after_retry);
        assert!(close_hint_shown_after_notification(true, false));
    }

    #[test]
    fn concurrent_ui_settings_updates_are_serialized_and_atomically_readable() {
        use std::sync::{Arc, Barrier};

        let directory = TestDirectory::new("ui-settings-concurrent");
        let path = directory.path.join("ui_settings.json");
        let persistence = Arc::new(UiSettingsPersistence::new());
        let barrier = Arc::new(Barrier::new(16));
        let mut workers = Vec::new();

        for _ in 0..16 {
            let persistence = persistence.clone();
            let barrier = barrier.clone();
            let path = path.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                persistence
                    .update(&path, |settings| {
                        settings.expanded_w = Some(settings.expanded_w.unwrap_or_default() + 1.0);
                    })
                    .expect("并发设置写入应成功");
            }));
        }

        for worker in workers {
            worker.join().expect("设置写入线程不应崩溃");
        }

        let saved: UiSettings =
            serde_json::from_str(&fs::read_to_string(&path).expect("最终设置文件应可读取"))
                .expect("最终设置文件不能被截断");
        assert_eq!(saved.expanded_w, Some(16.0));
        assert!(
            fs::read_dir(&directory.path)
                .expect("应能读取设置目录")
                .all(|entry| !entry
                    .expect("目录项应可读取")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")),
            "原子写入后不应遗留临时文件",
        );
    }

    #[test]
    fn paused_pomodoro_restores_remaining_time_and_custom_durations() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Focus,
            running: false,
            deadline_unix_ms: None,
            remaining_ms: 12_345,
            rounds_today: 2,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 40,
            break_min: 8,
        };

        let restored = Pomo::restore(persisted, 1_000_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Focus);
        assert!(!snapshot.running);
        assert_eq!(snapshot.remaining_ms, 12_345);
        assert_eq!(snapshot.rounds_today, 2);
        assert_eq!(snapshot.focus_min, 40);
        assert_eq!(snapshot.break_min, 8);
    }

    #[test]
    fn running_pomodoro_restores_from_wall_clock_deadline() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Break,
            running: true,
            deadline_unix_ms: Some(1_030_000),
            remaining_ms: 50_000,
            rounds_today: 1,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 25,
            break_min: 5,
        };

        let restored = Pomo::restore(persisted, 1_000_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert!(snapshot.running);
        assert!(snapshot.remaining_ms <= 30_000);
        assert!(snapshot.remaining_ms >= 29_900);
    }

    #[test]
    fn restore_advances_across_one_phase_boundary() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Focus,
            running: true,
            deadline_unix_ms: Some(1_000_000),
            remaining_ms: 60_000,
            rounds_today: 3,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 1,
            break_min: 1,
        };

        let restored = Pomo::restore(persisted, 1_030_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Break);
        assert!(snapshot.running);
        assert!((29_900..=30_000).contains(&snapshot.remaining_ms));
        assert_eq!(snapshot.rounds_today, 4);
    }

    #[test]
    fn restore_advances_across_two_phase_boundaries() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Focus,
            running: true,
            deadline_unix_ms: Some(1_000_000),
            remaining_ms: 60_000,
            rounds_today: 0,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 1,
            break_min: 1,
        };

        let restored = Pomo::restore(persisted, 1_060_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Focus);
        assert!(snapshot.running);
        assert!((59_900..=60_000).contains(&snapshot.remaining_ms));
        assert_eq!(snapshot.rounds_today, 1);
    }

    #[test]
    fn restore_skips_many_complete_cycles_without_per_second_iteration() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Focus,
            running: true,
            deadline_unix_ms: Some(1_000_000),
            remaining_ms: 60_000,
            rounds_today: 0,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 1,
            break_min: 1,
        };

        let restored = Pomo::restore(persisted, 1_630_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Break);
        assert!(snapshot.running);
        assert!((29_900..=30_000).contains(&snapshot.remaining_ms));
        assert_eq!(snapshot.rounds_today, 6);
    }

    #[test]
    fn restore_counts_only_focus_completions_on_the_restore_day() {
        use chrono::TimeZone;
        let deadline = chrono::Local
            .with_ymd_and_hms(2026, 11, 21, 0, 0, 30)
            .single()
            .expect("测试时间应唯一")
            .timestamp_millis() as u64;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 21, 0, 5, 0)
            .single()
            .expect("测试时间应唯一")
            .timestamp_millis() as u64;
        let persisted = PersistedPomo {
            phase: PomoPhase::Focus,
            running: true,
            deadline_unix_ms: Some(deadline),
            remaining_ms: 60_000,
            rounds_today: 9,
            rounds_date: "2026-11-20".to_string(),
            focus_min: 1,
            break_min: 1,
        };

        let restored = Pomo::restore(persisted, now, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Break);
        assert!((29_900..=30_000).contains(&snapshot.remaining_ms));
        assert_eq!(snapshot.rounds_today, 3);
    }

    #[test]
    fn pomodoro_rounds_reset_when_restored_on_another_day() {
        let persisted = PersistedPomo {
            phase: PomoPhase::Break,
            running: false,
            deadline_unix_ms: None,
            remaining_ms: 60_000,
            rounds_today: 4,
            rounds_date: "2026-11-20".to_string(),
            focus_min: 25,
            break_min: 5,
        };

        let restored = Pomo::restore(persisted, 1_000_000, "2026-11-21");

        assert_eq!(restored.snapshot().rounds_today, 0);
    }

    #[test]
    fn corrupt_pomodoro_file_falls_back_to_compatible_ui_durations() {
        let directory = TestDirectory::new("corrupt-pomodoro");
        let path = directory.path.join("pomodoro.json");
        fs::write(&path, b"{broken").expect("应能准备损坏状态文件");

        let restored = read_pomodoro_state(&path, 45, 9, 1_000_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Idle);
        assert!(!snapshot.running);
        assert_eq!(snapshot.remaining_ms, 45 * 60 * 1000);
        assert_eq!(snapshot.focus_min, 45);
        assert_eq!(snapshot.break_min, 9);
    }

    #[test]
    fn pomodoro_state_round_trips_through_atomic_file() {
        let directory = TestDirectory::new("pomodoro-round-trip");
        let path = directory.path.join("pomodoro.json");
        let state = PersistedPomo {
            phase: PomoPhase::Break,
            running: false,
            deadline_unix_ms: None,
            remaining_ms: 42_000,
            rounds_today: 3,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 50,
            break_min: 10,
        };

        let persistence = PomodoroPersistence::new();
        persistence
            .persist(&path, 1, &state)
            .expect("状态应原子写入");
        let restored = read_pomodoro_state(&path, 25, 5, 1_000_000, "2026-11-21");
        let snapshot = restored.snapshot();

        assert_eq!(snapshot.phase, PomoPhase::Break);
        assert_eq!(snapshot.remaining_ms, 42_000);
        assert_eq!(snapshot.rounds_today, 3);
        assert_eq!(snapshot.focus_min, 50);
        assert_eq!(snapshot.break_min, 10);
    }

    #[test]
    fn pomodoro_persistence_replaces_an_existing_target() {
        let directory = TestDirectory::new("pomodoro-replace");
        let path = directory.path.join("pomodoro.json");
        let persistence = PomodoroPersistence::new();
        let mut state = PersistedPomo {
            phase: PomoPhase::Focus,
            running: false,
            deadline_unix_ms: None,
            remaining_ms: 60_000,
            rounds_today: 1,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 25,
            break_min: 5,
        };

        persistence
            .persist(&path, 1, &state)
            .expect("首次写入应成功");
        state.rounds_today = 2;
        persistence
            .persist(&path, 2, &state)
            .expect("应替换已存在的目标文件");

        let saved: PersistedPomo =
            serde_json::from_str(&fs::read_to_string(path).expect("应读取最终状态"))
                .expect("最终状态应有效");
        assert_eq!(saved.rounds_today, 2);
    }

    #[test]
    fn failed_pomodoro_replace_removes_temporary_file() {
        let directory = TestDirectory::new("pomodoro-failed-replace");
        let target_directory = directory.path.join("pomodoro.json");
        fs::create_dir(&target_directory).expect("应能创建冲突目标目录");
        let state = PersistedPomo {
            phase: PomoPhase::Focus,
            running: false,
            deadline_unix_ms: None,
            remaining_ms: 60_000,
            rounds_today: 1,
            rounds_date: "2026-11-21".to_string(),
            focus_min: 25,
            break_min: 5,
        };

        let error =
            write_pomodoro_state(&target_directory, 7, &state).expect_err("替换目录目标必须失败");

        assert!(
            !directory.path.join("pomodoro.json.7.tmp").exists(),
            "写入失败必须清理临时文件",
        );
        assert!(error.contains(&target_directory.display().to_string()));
        assert!(!error.contains("番茄钟状态文件"));
    }

    #[test]
    fn concurrent_pomodoro_writes_cannot_overwrite_a_newer_snapshot() {
        use std::sync::{Arc, Barrier};
        let directory = TestDirectory::new("pomodoro-concurrent");
        let path = directory.path.join("pomodoro.json");
        let persistence = Arc::new(PomodoroPersistence::new());
        let barrier = Arc::new(Barrier::new(24));
        let mut workers = Vec::new();

        for version in 1..=24_u64 {
            let persistence = persistence.clone();
            let barrier = barrier.clone();
            let path = path.clone();
            workers.push(std::thread::spawn(move || {
                let state = PersistedPomo {
                    phase: PomoPhase::Focus,
                    running: false,
                    deadline_unix_ms: None,
                    remaining_ms: 60_000,
                    rounds_today: version as u32,
                    rounds_date: "2026-11-21".to_string(),
                    focus_min: 25,
                    break_min: 5,
                };
                barrier.wait();
                persistence
                    .persist(&path, version, &state)
                    .expect("并发持久化应成功");
            }));
        }
        for worker in workers {
            worker.join().expect("写入线程不应崩溃");
        }

        let saved: PersistedPomo =
            serde_json::from_str(&fs::read_to_string(path).expect("应读取最终状态"))
                .expect("最终状态应有效");
        assert_eq!(saved.rounds_today, 24);
    }
}
