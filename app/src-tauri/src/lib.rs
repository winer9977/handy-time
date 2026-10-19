//! xhy 库根：应用装配
//!
//! 分层：domain(数据) / db(存储) / state(内存真源) / cmds(IPC) /
//! quickparse(智能识别) / holidays(节假日) / lunar(农历) / stats(周期统计) /
//! ai(模型润色，可选) / export(Markdown) / widgets(窗口生命周期) /
//! reminder(到期提醒) / drag(跨窗拖拽) / log(文件日志) / win(系统集成)

pub mod ai;
pub mod cmds;
pub mod db;
pub mod domain;
pub mod drag;
pub mod export;
pub mod holidays;
pub mod ids;
pub mod log;
pub mod lunar;
pub mod mdimport;
pub mod quickparse;
pub mod reminder;
pub mod stats;
pub mod state;
pub mod widgets;
pub mod win;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 产品显示名。**改名字要同时改三处**（各自独立，没法共用一份）：
///   · 这里（Rust 侧所有窗口标题、托盘提示、通知标题、导出文件头）
///   · `app/src-tauri/tauri.conf.json` 的 `productName`（决定 exe 名与安装目录，
///     那里刻意用 ASCII `Shoubian`：非 ASCII 路径在这条工具链上咬过两次，
///     见 README「NSIS 中文路径坑」与 `tools/verify-multiday-and-parse.ps1` 头注释）
///   · `app/src/shared/brand.ts`（Vue 侧界面里的名字）
/// `identifier`（dev.xhy.desktop）是**数据目录名**，绝不跟着改 —— 改了等于丢用户数据。
pub const BRAND: &str = "手边";

/// 本地日期字符串 `YYYY-MM-DD`
pub fn now_local_date_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 本地日期时间 `YYYY-MM-DDTHH:MM`
pub fn now_local_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M").to_string()
}

/// 本地日期时间（含秒），归档/备份文件名用
pub fn now_local_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 秒数取整分钟，提醒调度去抖用
pub fn floor_minute(secs: u64) -> u64 {
    secs / 60
}

pub const TICK: Duration = Duration::from_secs(20);

use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(widgets::global_shortcut_plugin())
        .invoke_handler(tauri::generate_handler![
            cmds::snapshot,
            cmds::add_list,
            cmds::update_list,
            cmds::remove_list,
            cmds::set_list_archived,
            cmds::set_list_exclude_stats,
            cmds::reorder_lists,
            cmds::add_todo,
            cmds::update_todo,
            cmds::patch_todo,
            cmds::remove_todo,
            cmds::trash_todo,
            cmds::restore_todo,
            cmds::purge_todo,
            cmds::empty_trash,
            cmds::set_done,
            cmds::toggle_pin,
            cmds::reorder_todo,
            cmds::set_countdown,
            cmds::set_priority,
            cmds::set_recurrence,
            cmds::add_event,
            cmds::update_event,
            cmds::remove_event,
            cmds::todo_to_event,
            cmds::move_event,
            cmds::events_in_range,
            cmds::add_note,
            cmds::update_note,
            cmds::remove_note,
            cmds::toggle_note_pin,
            cmds::reorder_note,
            cmds::upsert_widget,
            cmds::patch_widget,
            cmds::remove_widget,
            cmds::set_settings,
            cmds::export_data,
            cmds::export_markdown,
            cmds::save_markdown_file,
            cmds::import_data,
            cmds::import_markdown,
            cmds::import_markdown_file,
            cmds::data_dir,
            cmds::quick_parse,
            cmds::holidays_in_range,
            cmds::lunar_in_range,
            cmds::period_summary,
            cmds::done_trend,
            cmds::ai_summary,
            cmds::log_line,
            widgets::toggle_hide_all,
            widgets::set_widget_visible,
            widgets::show_all_widgets,
            widgets::save_widget_rect,
            widgets::set_input_mode,
            widgets::reapply_desktop_attach,
            widgets::set_autostart,
            widgets::show_launcher,
            widgets::show_main,
            widgets::auto_align_command,
            widgets::debug_geom,
            drag::drag_begin,
            drag::drag_cancel,
            drag::ghost_text,
        ])
        .setup(|app| {
            let data_root = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("xhy-data"));
            std::fs::create_dir_all(&data_root).ok();
            // 日志必须在数据目录就绪后立刻开：跨窗口拖拽这类问题只在真机出现，
            // 没有日志就只能靠猜（此前所有 tracing::warn! 都进了黑洞）。
            log::init(&data_root);
            log::line("=== 启动 ===");

            // 通知上那行"来源"是 Windows 按 AUMID 注册显示名渲染的，不注册就退回
            // 显示 exe 文件名（我们的是 xhy.exe —— productName 改不了 Cargo bin 名）。
            // 所以光把 title 写成「手边 待办提醒」不够，这里补注册。
            // 只写 HKCU、幂等、失败不影响功能（提醒照样弹），因此不 fail 启动。
            let aumid = app.config().identifier.clone();
            let ident = win::aumid::Identity {
                aumid: &aumid,
                display_name: BRAND,
                icon_path: None,
            };
            match win::aumid::apply(&ident) {
                Ok(key) => log::line(&format!("AUMID 显示名已注册：HKCU\\{key} = {BRAND}")),
                Err(e) => log::line(&format!("AUMID 注册失败（不影响提醒，仅来源名沿用 exe）：{e}")),
            }

            let state =
                state::AppState::open(data_root).map_err(|e| format!("初始化数据失败：{e}"))?;
            app.manage(state);

            // 顺序关键：先按默认挂件播种（rect 只是占位），sync_windows 第二步会用
    // 真实窗口 scale 重算逻辑工作区并把所有挂件 clamp/摆正 —— 启动早期拿不到
    // scale，强行在播种时算 DPI 反而会错位。
            widgets::ensure_default_widgets(app.handle());
            // 按 store 里的挂件表创建/销毁窗口并嵌入桌面层
            widgets::sync_windows(app.handle());
            widgets::build_tray(app.handle())?;
            widgets::register_hotkeys(app.handle());
            // 注册表实况对齐 store（用户开/关过才写；默认关，不擅自开机自启）
            widgets::sync_autostart(app.handle());

            reminder::spawn_scheduler(app.handle().clone());
            // 逾期自动延期：只在用户显式开启时做，且只在启动时一次
            if app.state::<state::AppState>().snapshot().settings.auto_postpone {
                match app.state::<state::AppState>().auto_postpone() {
                    Ok(n) if n > 0 => {
                        log::line(&format!("自动延期 {n} 条逾期待办"));
                        let _ = tauri::Emitter::emit(app.handle(), "xhy://changed", "todos");
                    }
                    Err(e) => log::line(&format!("自动延期失败：{e}")),
                    _ => {}
                }
            }
            widgets::edgedock::spawn_edge_poller(app.handle().clone());
            // 幽灵窗**不**预建：首次拖拽时由 drag_begin 异步派发创建（见 request_ghost）。
            // 预建等于常驻一个隐藏 webview，白占约 83 MB。
            drag::spawn_drag_poller(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 挂件/主窗口"关闭"= 隐藏。退出只走托盘，防止误关丢桌面挂件。
                // 主窗口同理：重建一个 webview 实测约 83 MB，不该白白丢。
                if widgets::hides_on_close(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("构建 xhy 应用失败");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            widgets::persist_all_rects(handle);
        }
    });
}
