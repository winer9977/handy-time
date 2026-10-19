//! 挂件窗口生命周期：创建/销毁、桌面嵌入、快速隐藏、自动对齐、托盘、热键
//!
//! 一个挂件 = 一个 `WebviewWindow`，label = `w-<widgetId>`。
//! 前端通过 `getCurrentWindow().label` 认出自己的身份，再决定渲染哪种挂件，
//! 这样就不依赖 URL query（dev 与打包行为一致）。
//!
//! 几何一律以**逻辑像素**存于 store；摆放窗口全程用 LogicalSize/LogicalPosition，
//! DPI 由 Tauri 负责换算（自己再乘一次 scale 是双重换算 bug 之源）。

pub mod edgedock;

use crate::domain::{AttachMode, CalendarView, RectL, Widget, WidgetKind, WidgetSize};
use crate::state::{AppState, SettingsPatch};
use crate::win::geometry::{self, MonitorLogical};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

#[cfg(windows)]
use crate::win::desktop;

pub fn label_of(id: &str) -> String {
    format!("w-{id}")
}

pub fn id_of(label: &str) -> Option<&str> {
    label.strip_prefix("w-")
}

/// 各档位尺寸（逻辑像素）
pub fn size_of(kind: WidgetKind, s: WidgetSize) -> (f64, f64) {
    let w = match s {
        WidgetSize::S => 250.0,
        WidgetSize::M => 310.0,
        WidgetSize::L => 390.0,
        WidgetSize::Xl => 500.0,
    };
    let h = match (kind, s) {
        (WidgetKind::Calendar, WidgetSize::S) => 320.0,
        (WidgetKind::Calendar, WidgetSize::M) => 400.0,
        (WidgetKind::Calendar, WidgetSize::L) => 500.0,
        (WidgetKind::Calendar, WidgetSize::Xl) => 620.0,
        (WidgetKind::Todo, WidgetSize::S) => 290.0,
        (WidgetKind::Todo, WidgetSize::M) => 370.0,
        (WidgetKind::Todo, WidgetSize::L) => 470.0,
        (WidgetKind::Todo, WidgetSize::Xl) => 580.0,
        (WidgetKind::Note, WidgetSize::S) => 210.0,
        (WidgetKind::Note, WidgetSize::M) => 270.0,
        (WidgetKind::Note, WidgetSize::L) => 350.0,
        (WidgetKind::Note, WidgetSize::Xl) => 460.0,
    };
    (w, h)
}

// ── 显示器 ────────────────────────────────────────────────

/// 逻辑工作区列表
///
/// `Monitor::position()/size()` 返回**物理**像素，`Monitor::scale_factor()`
/// 是该屏 DPI 缩放。这里一次性换算成逻辑像素；此后全程只用
/// `LogicalSize/LogicalPosition`，绝不再手工乘除 scale（双重换算是 bug 之源）。
pub fn monitors_of(app: &AppHandle) -> Vec<MonitorLogical> {
    let mut out: Vec<MonitorLogical> = Vec::new();
    if let Ok(mons) = app.available_monitors() {
        for m in mons {
            let scale = {
                let s = m.scale_factor();
                if s > 0.0 { s } else { 1.0 }
            };
            let pos = m.position();
            let size = m.size();
            out.push(MonitorLogical {
                x: pos.x as f64 / scale,
                y: pos.y as f64 / scale,
                w: size.width as f64 / scale,
                h: size.height as f64 / scale,
                scale,
            });
        }
    }
    if out.is_empty() {
        out.push(MonitorLogical {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
            scale: 1.0,
        });
    }
    out
}

/// 主屏（第一块），供播种与对齐用
fn primary(mons: &[MonitorLogical]) -> MonitorLogical {
    mons.first().copied().unwrap_or(MonitorLogical {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
        scale: 1.0,
    })
}

// ── 播种与同步 ────────────────────────────────────────────

/// 首次运行：右上角待办 + 其下日历，左上角便签
pub fn ensure_default_widgets(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.snapshot().widgets.is_empty() {
        return;
    }
    let m = primary(&monitors_of(app));
    let list_id = state.snapshot().lists.first().map(|l| l.id.clone());
    let mk = |kind: WidgetKind, size: WidgetSize, x: f64, y: f64| {
        let (w, h) = size_of(kind, size);
        Widget {
            id: crate::ids::new(),
            kind,
            list_id: if kind == WidgetKind::Todo {
                list_id.clone()
            } else {
                None
            },
            view: (kind == WidgetKind::Calendar).then_some(CalendarView::Month),
            filter: None,
            tag: None,
            size,
            rect: RectL { x, y, w, h },
            visible: true,
            auto_dock: true,
            edge_hide: None,
            free: false,
        }
    };
    let seed = [
        mk(WidgetKind::Todo, WidgetSize::M, m.x + m.w - 330.0, m.y + 24.0),
        mk(WidgetKind::Calendar, WidgetSize::M, m.x + m.w - 330.0, m.y + 410.0),
        mk(WidgetKind::Note, WidgetSize::M, m.x + 24.0, m.y + 24.0),
    ];
    for w in seed {
        let _ = state.upsert_widget(w);
    }
    // 便签挂件要有承载对象才可写；只播种挂件会让首启用户面对一个
    // 敲了字却存不下来的空框（"便签不能输入"的第二个成因）
    if state.snapshot().notes.is_empty() {
        let _ = state.add_note(String::new());
    }
}

/// store.widgets → 真实窗口（建缺失 / 关多余 / 摆几何 / 嵌桌面）
///
/// 顺序很关键：**先建窗口**，再读各窗口自带的 scale_factor 算逻辑工作区，
/// 最后统一摆位。反过来（先算再建）在启动早期拿不到 scale，播种坐标会错。
/// 把窗口层调和到 store 描述的期望状态。**幂等 + 自愈**。
///
/// 架构约定（这是"同一错误反复出现"的根治）：
///   · `store.widgets[].visible` 是**唯一真相源**；窗口存在与否、可见与否
///     一律由本函数推导，任何地方都不许直接 show/hide/close 挂件窗口。
///   · 本函数**不做早退**：每次调用都把每个挂件重新调和一遍。因此即便
///     某次事件循环错过、或窗口被外部因素改成别的状态，下一次 sync 就
///     自动修好 —— 不存在"store 说可见、窗口却藏着"且永远修不回来的漂移态。
///   · 新建窗口用 build() 返回的句柄就地处理（窗口表注册是异步的，
///     事后再查表可能查不到 —— 这正是上一版"点了显示但窗口不出现"的成因）。
/// 调和串行化锁。async 命令跑在线程池上，两个 sync_windows 并发时会都
/// 认为"窗口不存在"而重复 build（后者报 label already in use，挂件反而
/// 建不出来）。调和本身幂等，排队跑完最终状态一定正确。
static SYNC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn sync_windows(app: &AppHandle) {
    // 中毒也要继续：锁内没有任何会永久失败的路径，放弃调和反而让窗口卡住
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let state = app.state::<AppState>();
    let store = state.snapshot();
    let mons = monitors_of(app);
    let attach = store.settings.attach_mode;
    // 快速隐藏态（Alt+Shift+H）：整体收起但不改 store.visible，
    // 所以期望可见 = store.visible && !hidden_all。
    let soft_hidden = *state.hidden_all.read();
    let mut built_now: Vec<String> = Vec::new();

    // ① 期望隐藏的：hide。
    //
    // 为什么不是 destroy（实机验证过两次，都栽在这）：destroy 是**异步**的
    // （派发给主线程处理），而本函数紧接着的 ③ 就要查窗口表。"收起后立刻
    // 再显示"时，销毁还没完成 → ③ 查到那个正在消失的窗口，以为它还在，
    // 于是只 show 不重建 → 结果 store.visible=true 而窗口根本不存在，
    // 用户看到"点了显示没反应"。hide 是同步的，不存在这个窗口期。
    //
    // 之所以曾想用 destroy，是为了消灭"活着的隐藏挂件用陈旧快照写回
    // visible"这个陈旧写者 —— 但那件事已由**字段级 patch_widget** 从根上
    // 解决（各写者只碰自己那一个字段，抹不掉 visible），不需要靠销毁窗口。
    for w in &store.widgets {
        let label = label_of(&w.id);
        if w.visible && !soft_hidden {
            continue;
        }
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.hide();
        }
    }

    // ② store 里已删除的挂件：窗口一并销毁
    for (label, win) in app.webview_windows() {
        if let Some(id) = id_of(&label) {
            if !store.widgets.iter().any(|x| x.id == id) {
                let _ = win.destroy();
            }
        }
    }

    dump_geom(app, &mons); // 启动诊断：真实 scale/monitor 写文件，排查 DPI

    // ③ 期望可见的：没有就建，有就重新摆位显示。
    for w in store.widgets.iter().filter(|w| w.visible && !soft_hidden) {
        let label = label_of(&w.id);
        // 停靠态下位置由贴边状态机掌管，别把它拽回屏内
        let docked = edgedock::is_docked(&w.id);
        match app.get_webview_window(&label) {
            Some(win) => {
                if !docked && !built_now.contains(&label) {
                    apply_widget_geometry(app, &win, w, &mons, attach);
                }
            }
            None => {
                match build_widget_window(app, w, &label) {
                    Ok(win) => {
                        apply_widget_geometry(app, &win, w, &mons, attach);
                        built_now.push(label);
                    }
                    Err(e) => tracing::warn!("创建挂件窗口 {label} 失败：{e}"),
                }
            }
        }
    }

}

/// 建挂件窗口（初始不可见，几何随后由 apply_widget_geometry 落定）
fn build_widget_window(
    app: &AppHandle,
    w: &crate::domain::Widget,
    label: &str,
) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title_of(w.kind))
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(false)
        .skip_taskbar(true)
        .resizable(true)
        .minimizable(false)
        .maximizable(false)
        .closable(true)
        .visible(false)
        .inner_size(w.rect.w.max(120.0), w.rect.h.max(120.0))
        .position(w.rect.x, w.rect.y)
        .build()
}

/// 把一个挂件窗口摆到 store 记录的几何上并显示、嵌入桌面层。
fn apply_widget_geometry(
    app: &AppHandle,
    win: &tauri::WebviewWindow,
    w: &crate::domain::Widget,
    mons: &[MonitorLogical],
    attach: AttachMode,
) {
    let rect = geometry::clamp_to_monitor(w.rect, mon_at(mons, w.rect), 48.0);
    // 全程逻辑像素：DPI 由 Tauri 负责，绝不再自己乘 scale
    let _ = win.set_size(LogicalSize::new(rect.w, rect.h));
    let _ = win.set_position(LogicalPosition::new(rect.x, rect.y));
    let _ = win.show();
    attach_one(app, win, attach);
}

fn mon_at(mons: &[MonitorLogical], r: RectL) -> MonitorLogical {
    let i = geometry::monitor_containing(mons, r.x + r.w / 2.0, r.y + r.h / 2.0);
    *mons.get(i).unwrap_or(&mons[0])
}

/// 启动诊断：把窗口真实 scale + monitor 物理尺寸写文件，供排查 DPI
fn dump_geom(app: &AppHandle, mons: &[MonitorLogical]) {
    let mut s = String::new();
    s.push_str(&format!("logical monitors: {:?}\n", mons));
    if let Ok(am) = app.available_monitors() {
        for m in am {
            s.push_str(&format!(
                "monitor phys pos=({},{}) size=({}x{}) scale={}\n",
                m.position().x, m.position().y, m.size().width, m.size().height,
                m.scale_factor()
            ));
        }
    }
    for (label, w) in app.webview_windows() {
        let sc = w.scale_factor().unwrap_or(0.0);
        let pos = w.outer_position().ok();
        let sz = w.inner_size().ok();
        s.push_str(&format!(
            "win {label} scale={sc} pos={:?} size={:?}\n",
            pos.map(|p| (p.x, p.y)),
            sz.map(|z| (z.width, z.height))
        ));
    }
    let path = app
        .path()
        .app_data_dir()
        .unwrap_or_default()
        .join("geom-dump.txt");
    let _ = std::fs::write(path, s);
}

fn title_of(kind: WidgetKind) -> &'static str {
    match kind {
        WidgetKind::Todo => "待办清单",
        WidgetKind::Calendar => "日历清单",
        WidgetKind::Note => "便签",
    }
}

/// 把一个窗口嵌进桌面层；失败自动降级并广播
fn attach_one(app: &AppHandle, win: &tauri::WebviewWindow, want: AttachMode) {
    #[cfg(windows)]
    {
        let want_eff = match want {
            AttachMode::WorkerW => desktop::Effective::WorkerW,
            AttachMode::BottomZ => desktop::Effective::BottomZ,
        };
        if let Ok(h) = win.hwnd() {
            let hwnd = windows::Win32::Foundation::HWND(h.0 as *mut _);
            let actual = unsafe { desktop::apply(hwnd, want_eff) };
            if actual != want_eff {
                tracing::warn!("桌面嵌入已降级：期望 {want_eff:?}，实际 {actual:?}");
                let _ = app.emit("xhy://attach", "downgraded");
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (app, win, want);
}

// ── 命令：几何回写 / 重贴 / 隐藏 ──────────────────────────

#[tauri::command]
pub fn save_widget_rect(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    rect: RectL,
) -> Result<(), String> {
    // 停靠/唤出态下窗口位置由贴边状态机掌管，前端回写的是"半出界"坐标，
    // 存下来会污染用户真实摆放位 —— 直接忽略，交给 edgedock 处理。
    if edgedock::is_docked(&id) {
        crate::log::dock(&format!("save_widget_rect {id} 忽略（停靠态）rect=({:.1},{:.1})", rect.x, rect.y));
        return Ok(());
    }
    let mons = monitors_of(&app);
    let m = mon_at(&mons, rect);
    let clamped = geometry::clamp_to_monitor(rect, m, 48.0);
    crate::log::dock(&format!(
        "save_widget_rect {id} 收到=({:.1},{:.1}) clamp后=({:.1},{:.1})",
        rect.x, rect.y, clamped.x, clamped.y
    ));
    // 字段级补丁：只落 rect/free。整对象写回会连带把 visible 等字段
    // 用快照时的旧值覆盖掉（lost update）。
    state.patch_widget(
        &id,
        crate::domain::WidgetPatch {
            rect: Some(clamped),
            free: Some(true),
            ..Default::default()
        },
    )?;
    let _ = app.emit("xhy://layout", id);
    Ok(())
}

#[tauri::command]
pub fn reapply_desktop_attach(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let store = state.snapshot();
    for w in store.widgets.iter().filter(|w| w.visible) {
        if let Some(win) = app.get_webview_window(&label_of(&w.id)) {
            attach_one(&app, &win, store.settings.attach_mode);
        }
    }
    Ok(match store.settings.attach_mode {
        AttachMode::WorkerW => "已嵌入桌面壁纸层".into(),
        AttachMode::BottomZ => "已贴到窗口最底层".into(),
    })
}

/// 显示 / 隐藏单个挂件（**持久**，写进 store）。
///
/// 为什么需要命令而不是前端直接 `win.hide()`：那样会有两套可见性真相 ——
/// Tauri 内部状态 vs `store.visible`，两者一旦漂移 ✕ 就变空操作。
/// 现在 ✕ 只改数据，窗口生死由 `sync_windows` 统一决定。
///
/// **必须 async**：Tauri 2 的同步命令跑在主线程，而 `sync_windows` 可能新建
/// 窗口 —— 建窗要等主线程事件循环推进，同步命令占着主线程等自己 → **自锁**，
/// 表现是"整个应用所有按钮一起点了没反应"。本项目在拖拽幽灵窗上栽过同一课
/// （见 drag.rs:144 注释），这是第二次，别再改回同步。
/// async 命令跑在工作线程，主线程空闲，建窗能正常完成。
#[tauri::command]
pub async fn set_widget_visible(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    visible: bool,
) -> Result<(), String> {
    let Some(w) = state.snapshot().widgets.into_iter().find(|x| x.id == id) else {
        return Ok(()); // 挂件已删：无事可做
    };
    // 值相同也照样调和：早退会让"窗口与 store 已漂移"的状态**永远修不回来**
    // （点已显示的挂件 → 直接 return → 不 sync → 窗口再也不出现）。
    // 调和是幂等的，多跑一次没有副作用。
    if w.visible != visible {
        // 走字段级方法：upsert_widget 现在**保留**已存在挂件的 visible
        // （防前端陈旧快照 lost-update），所以这里不能用它改 visible。
        state.set_widget_visible(&id, visible)?;
    }
    // 显式要显示某个挂件 = 用户想退出"快速隐藏"态，否则 sync 会因
    // soft_hidden 跳过它，点了没反应。代价：其他 visible 的挂件一起回来
    // —— 这与"显示全部"语义一致，比"点开了却没窗口"好解释。
    if visible && *state.hidden_all.read() {
        *state.hidden_all.write() = false;
        let _ = app.emit("xhy://hidden", false);
    }
    sync_windows(&app);
    let _ = app.emit("xhy://changed", "widgets");
    Ok(())
}

/// 把调和动作派发到工作线程执行。**主线程调用者（托盘回调、热键 handler）
/// 必须用这个**，不能直接调 sync_windows。
///
/// 原因（实测踩过，就是"整个应用按钮全没反应"的成因）：Tauri 2 的同步命令
/// 与托盘/热键回调都跑在**主线程**，而 sync_windows 可能新建窗口，建窗要等
/// 主线程事件循环推进 → 主线程等自己 → 死锁，此后所有 IPC 与事件都投不出去。
/// 见 drag.rs 同一课的注释。
fn schedule_sync(app: &AppHandle) {
    let a = app.clone();
    std::thread::spawn(move || sync_windows(&a));
}

/// 显示全部挂件（托盘/主窗口入口）：把每个挂件的 `visible` 置真并同步。
///
/// 必须同时清 `hidden_all`：sync_windows 在快速隐藏态下会跳过所有挂件
/// （见那里的 `!soft_hidden`），若不清，用户"显示全部"点了没反应。
/// async：见 set_widget_visible 的自锁说明。
#[tauri::command]
pub async fn show_all_widgets(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut n = 0usize;
    for w in state.snapshot().widgets {
        if !w.visible {
            // 同 set_widget_visible：upsert_widget 保留 visible，须走字段级
            state.set_widget_visible(&w.id, true)?;
            n += 1;
        }
    }
    let was_hidden = *state.hidden_all.read();
    *state.hidden_all.write() = false;
    if n > 0 || was_hidden {
        schedule_sync(&app);
        let _ = app.emit("xhy://changed", "widgets");
        if was_hidden {
            let _ = app.emit("xhy://hidden", false);
        }
        crate::log::line(&format!("显示全部挂件：恢复 {n} 个（快速隐藏态 {was_hidden}→false）"));
    }
    Ok(())
}

#[tauri::command]
pub fn toggle_hide_all(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<bool, String> {
    let hide = !*state.hidden_all.read();
    *state.hidden_all.write() = hide;
    // 只翻 hidden_all，窗口层交给 sync_windows 调和（期望可见 =
    // store.visible && !hidden_all）。
    //
    // 此前这里另有一条 set_all_visible 直接 show/hide 每个窗口 —— 那是
    // **绕过调和器的第二写者**：它 `filter(w.visible)` 后 get_webview_window
    // 查不到已销毁窗口就 `continue` 跳过，于是"显示全部"点了没反应
    // （用户报的"待办清单显示不了"）。可见性只允许 sync_windows 一处推导。
    schedule_sync(&app);
    let _ = app.emit("xhy://hidden", hide);
    Ok(hide)
}

/// 退出前把窗口实际位置写回 store（逻辑像素）
pub fn persist_all_rects(app: &AppHandle) {
    let state = app.state::<AppState>();
    for w in state.snapshot().widgets {
        let Some(win) = app.get_webview_window(&label_of(&w.id)) else {
            continue;
        };
        let (Ok(pos), Ok(size), Ok(scale)) = (
            win.outer_position(),
            win.inner_size(),
            win.scale_factor(),
        ) else {
            continue;
        };
        let mut w2 = w.clone();
        w2.rect = geometry::to_logical(pos.x, pos.y, size.width, size.height, scale);
        if w2.rect == w.rect {
            continue; // 没挪动就别写盘
        }
        // 只落 rect：退出路径上整对象写回会把运行期间改过但尚未落库的
        // 其他字段（如刚切的筛选）一并抹掉
        let _ = state.patch_widget(
            &w.id,
            crate::domain::WidgetPatch {
                rect: Some(w2.rect),
                ..Default::default()
            },
        );
    }
}

// ── 自动对齐 ──────────────────────────────────────────────

/// 未自由摆放的挂件按屏幕工作区重新排布（吸附间隙 = settings.snap_gap）
///
/// `#[tauri::command]` 的薄壳是下面的 `auto_align_command`（命令参数需要
/// 持有型 AppHandle，业务逻辑收在这个引用版里）。
pub fn auto_align(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let store = state.snapshot();
    let gap = store.settings.snap_gap.max(0.0);
    let mons = monitors_of(app);
    let m = primary(&mons);

    let mut right_bottom = m.y + 24.0;
    let mut left_bottom = m.y + 24.0;

    for mut w in store.widgets.clone() {
        if !w.visible || w.free {
            continue;
        }
        // 对齐即"回到屏内正常位"：先丢掉贴边状态并清 edgeHide 标记，
        // 否则状态机下一轮会把它重新吸回边上（这是卡死挂件的逃生口）。
        edgedock::forget(&w.id);
        w.edge_hide = None;
        let (sw, sh) = size_of(w.kind, w.size);
        w.rect.w = sw;
        w.rect.h = sh;
        if w.kind == WidgetKind::Note {
            w.rect.x = m.x + 24.0;
            w.rect.y = left_bottom;
            left_bottom += sh + gap;
        } else {
            w.rect.x = m.x + m.w - sw - 24.0;
            w.rect.y = right_bottom;
            right_bottom += sh + gap;
        }
        // 全程逻辑像素，DPI 交给 Tauri
        if let Some(win) = app.get_webview_window(&label_of(&w.id)) {
            let _ = win.set_size(LogicalSize::new(w.rect.w, w.rect.h));
            let _ = win.set_position(LogicalPosition::new(w.rect.x, w.rect.y));
        }
        // 字段级：只落这次真正改的 rect 与 edge_hide
        state.patch_widget(
            &w.id,
            crate::domain::WidgetPatch {
                rect: Some(w.rect),
                edge_hide: Some(None),
                ..Default::default()
            },
        )?;
    }
    let _ = app.emit("xhy://aligned", ());
    Ok(())
}

/// IPC 薄壳：命令参数需持有型 AppHandle，内部函数用引用
#[tauri::command]
pub fn auto_align_command(app: tauri::AppHandle) -> Result<(), String> {
    auto_align(&app)
}

/// 诊断：把 Tauri 视角的显示器/窗口几何原样吐出来，排查 DPI 换算
#[derive(serde::Serialize)]
pub struct GeomReport {
    pub monitors: Vec<(i32, i32, u32, u32)>,
    pub windows: Vec<(String, f64, i32, i32, u32, u32)>,
}

#[tauri::command]
pub fn debug_geom(app: tauri::AppHandle) -> Result<GeomReport, String> {
    let mut monitors = Vec::new();
    for m in app.available_monitors().map_err(|e| e.to_string())? {
        monitors.push((m.position().x, m.position().y, m.size().width, m.size().height));
    }
    let mut windows = Vec::new();
    for (label, w) in app.webview_windows() {
        let scale = w.scale_factor().unwrap_or(1.0);
        let pos = w.outer_position().ok();
        let size = w.inner_size().ok();
        if let (Some(p), Some(s)) = (pos, size) {
            windows.push((label, scale, p.x, p.y, s.width, s.height));
        }
    }
    Ok(GeomReport { monitors, windows })
}

// ── 主程序窗口（对照滴答三栏：列表 / 看板 / 时间线）────────
//
// 与挂件的本质区别：**普通顶层窗口**，原生标题栏、可缩放、进任务栏。
// 刻意不透明、不嵌桌面层 —— 看板/时间线要横向铺开，388px 挂件装不下，
// 硬塞只会把两个形态都做坏。
pub const MAIN: &str = "main";

/// **必须 async**，理由同 set_widget_visible：同步命令占着主线程等自己
/// 推进窗口创建 → 自锁。托盘与前端都调它，卡死时表现是"整个应用没反应"。
#[tauri::command]
pub async fn show_main(app: tauri::AppHandle) -> Result<(), String> {
    show_main_sync(app)
}

/// 实体逻辑。**托盘回调必须直接调这个**：async 命令在回调里 `let _ =`
/// 会丢弃 Future 根本不执行（cargo 不报错，工作台静默打不开）。
pub fn show_main_sync(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(MAIN) {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::App("index.html".into()))
        .title(format!("{} 工作台", crate::BRAND))
        .decorations(true)
        .transparent(false)
        .shadow(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .resizable(true)
        .min_inner_size(820.0, 520.0)
        .inner_size(1080.0, 700.0)
        .center()
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

/// 挂件窗口"关闭"= 隐藏（防误关丢桌面挂件）；主窗口同理，
/// 免得每次重开都重建一个 webview（实测每个约 83 MB）。
/// 返回 true 表示这个 label 该被拦下关闭。
pub fn hides_on_close(label: &str) -> bool {
    id_of(label).is_some() || label == MAIN
}

// ── 快速添加窗 ────────────────────────────────────────────

pub const LAUNCHER: &str = "launcher";

/// 实体逻辑（可同步调用：托盘/热键回调专用）。
/// 含 build() —— **主线程直接调会自锁**，回调必须经 `schedule_launcher`。
pub fn show_launcher_sync(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(LAUNCHER) {
        place_at_cursor(app, &win);
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(
        app,
        LAUNCHER,
        WebviewUrl::App("index.html".into()),
    )
    .title(format!("{} 快速添加", crate::BRAND))
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .inner_size(460.0, 132.0)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    place_at_cursor(app, &win);
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

/// **必须 async**：见 show_main 的自锁说明（同步命令占主线程等 build）。
#[tauri::command]
pub async fn show_launcher(app: AppHandle) -> Result<(), String> {
    show_launcher_sync(&app)
}

/// 托盘/热键回调专用：派发到工作线程执行，绝不在主线程直接 build。
fn schedule_launcher(app: &AppHandle) {
    let a = app.clone();
    std::thread::spawn(move || {
        let _ = show_launcher_sync(&a);
    });
}

fn place_at_cursor(app: &AppHandle, win: &tauri::WebviewWindow) {
    let mons = monitors_of(app);
    // cursor_position 是物理像素 → 用窗口自身 scale 转逻辑
    let scale = win.scale_factor().unwrap_or(1.0).max(0.01);
    let (cx, cy) = match app.cursor_position() {
        Ok(p) => (p.x / scale, p.y / scale),
        Err(_) => {
            let m = primary(&mons);
            (m.x + m.w / 2.0, m.y + m.h / 3.0)
        }
    };
    let m = mon_at(&mons, RectL { x: cx, y: cy, w: 460.0, h: 132.0 });
    let x = cx.clamp(m.x, m.x + m.w - 460.0);
    let y = cy.clamp(m.y, m.y + m.h - 132.0);
    let _ = win.set_position(LogicalPosition::new(x, y));
}

// ── 托盘 ──────────────────────────────────────────────────

/// 托盘图标事件是否应触发"打开工作台"。
///
/// 抽成纯函数是为了**可测**：托盘在系统任务栏里，合成鼠标消息够不到，
/// 本沙箱的 `SendInput` 又被完整性级别拦截（早前实测确认），所以这条
/// 交互规则只能靠单测锁住 —— 否则"单击还是双击""左右键"全靠肉眼。
///
/// 规则：**左键 + 松开**才开窗。
///   · 只认 Left：右键要弹菜单，绝不能顺手把工作台也开了；
///   · 只认 Up（松开）：`WM_LBUTTONUP → MouseButtonState::Up`
///     （已从 tray-icon 的 Windows 实现源码确认；tauri 的文档注释把
///     Up/Down 写反了，别信注释）。按下即触发会造成重复开窗。
pub fn tray_opens_main(
    ev: &tauri::tray::TrayIconEvent,
) -> bool {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    matches!(
        ev,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

/// 建托盘
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let m_show = MenuItem::with_id(app, "show", "显示 / 隐藏全部挂件", true, None::<&str>)?;
    let m_restore = MenuItem::with_id(app, "restore", "恢复被收起的挂件", true, None::<&str>)?;
    let m_work = MenuItem::with_id(app, "work", "打开工作台", true, None::<&str>)?;
    let m_add = MenuItem::with_id(app, "add", "快速添加…", true, None::<&str>)?;
    let m_align = MenuItem::with_id(app, "align", "自动对齐桌面", true, None::<&str>)?;
    let m_refresh = MenuItem::with_id(app, "refresh", "重新嵌入桌面层", true, None::<&str>)?;
    let m_export = MenuItem::with_id(app, "export", "导出数据…", true, None::<&str>)?;
    let m_md = MenuItem::with_id(app, "exportmd", "导出 Markdown…", true, None::<&str>)?;
    let m_import = MenuItem::with_id(app, "import", "导入数据…", true, None::<&str>)?;
    let m_mdimport = MenuItem::with_id(app, "importmd", "导入 Markdown…", true, None::<&str>)?;
    let m_quit = MenuItem::with_id(app, "quit", format!("退出 {}", crate::BRAND), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &m_show,
            &m_restore,
            &m_work,
            &m_add,
            &m_align,
            &m_refresh,
            &m_export,
            &m_md,
            &m_import,
            &m_mdimport,
            &m_quit,
        ],
    )?;

    let mut b = TrayIconBuilder::with_id("xhy-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(format!("{} · 桌面待办（单击打开工作台）", crate::BRAND))
        // **左键单击**托盘图标 = 打开工作台（用户要求；对照常见桌面工具习惯）。
        // 右键仍弹菜单（show_menu_on_left_click(false) 保证左键不弹）。
        //
        // 为什么用 Click 而不是 DoubleClick：单击更顺手，且不需要用户被
        // 教"要双击"。TrayIconEvent::Click 带 button 字段，必须判 Left ——
        // 否则右键也会被当成打开窗口。
        //
        // 回调在**主线程**：show_main_sync 含 build()，直接调会自锁
        // （审计实锤同一模式），所以派发到工作线程。
        .on_tray_icon_event(|tray, event| {
            if tray_opens_main(&event) {
                let app = tray.app_handle().clone();
                std::thread::spawn(move || {
                    let _ = show_main_sync(app);
                });
            }
        })
        .on_menu_event(|app: &AppHandle, event| match event.id().as_ref() {
            "show" => {
                // 托盘回调在主线程：toggle_hide_all 现在只翻 hidden_all +
                // schedule_sync（派发到工作线程），可同步调用
                let state = app.state::<AppState>();
                let _ = toggle_hide_all(app.clone(), state);
            }
            "restore" => {
                // show_all_widgets 是 async 命令，这里 `let _ =` 会**静默丢弃
                // Future 根本不执行**（cargo 不报错）——上线以来托盘这一项
                // 一直是坏的。改为只做数据侧 + schedule_sync。
                let state = app.state::<AppState>();
                let mut n = 0usize;
                for w in state.snapshot().widgets {
                    if !w.visible {
                        let _ = state.set_widget_visible(&w.id, true);
                        n += 1;
                    }
                }
                *state.hidden_all.write() = false;
                schedule_sync(app);
                let _ = app.emit("xhy://changed", "widgets");
                let _ = app.emit("xhy://hidden", false);
                crate::log::line(&format!("托盘恢复挂件：{n} 个（含清除快速隐藏态）"));
            }
            "work" => {
                // 主线程回调：必须调同步版，async 版在这里 `let _ =` 会丢弃
                // Future、工作台根本打不开（cargo 不报错）
                let _ = show_main_sync(app.clone());
            }
            "add" => {
                // show_launcher 含 build()，主线程直接调会自锁（审计实锤 #2）
                schedule_launcher(app);
            }
            "align" => {
                // auto_align 只 set_size/position 不建窗，无自锁；但持写锁
                // 写盘仍不宜在主线程，统一派发
                let a = app.clone();
                std::thread::spawn(move || {
                    let _ = auto_align(&a);
                });
            }
            "refresh" => {
                let _ = reapply_desktop_attach(app.clone());
            }
            "export" => save_data(app),
            "exportmd" => save_markdown(app),
            "import" => open_data(app),
            "importmd" => import_md(app),
            "quit" => {
                persist_all_rects(app);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

fn save_data(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    let handle = app.clone();
    app.dialog().file().save_file(move |path| {
        let Some(p) = path.as_ref().and_then(|f| f.as_path()) else { return };
        let state = handle.state::<AppState>();
        let json = state.export();
        let _ = std::fs::write(p, json);
    });
}

/// 导出 Markdown（对照 Kite 待办）。与 JSON 导出同一套对话框流程。
/// 托盘菜单与设置页共用这个入口。
pub fn save_markdown(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    let handle = app.clone();
    app.dialog().file().save_file(move |path| {
        let Some(p) = path.as_ref().and_then(|f| f.as_path()) else { return };
        let state = handle.state::<AppState>();
        let md = crate::export::to_markdown(&state.snapshot());
        let _ = std::fs::write(p, md);
    });
}

/// 选一个 `.md` 导入待办（合并语义，不整库替换）。
///
/// 摘要走 `xhy://md-import` 事件回界面：对话框回调是异步的，命令本身早就返回了，
/// 没有别的路径能把结果带回按钮旁边。读文件失败（非 UTF-8 的 GBK 导出等）
/// 明确报错，**不做 lossy 解码** —— 中文变乱码被导进来比失败更难收拾。
pub fn import_md(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    let handle = app.clone();
    app.dialog().file().pick_file(move |path| {
        let Some(p) = path.as_ref().and_then(|f| f.as_path()) else { return };
        let text = match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                let _ = handle.emit("xhy://md-import", &format!("读不了这个文件：{e}"));
                return;
            }
        };
        let state = handle.state::<AppState>();
        let msg = match state.import_markdown(&text, "导入待办") {
            Ok(st) => format!(
                "已导入：新增 {} 条、跳过重复 {} 条、新建清单 {} 个{}",
                st.added,
                st.dup,
                st.lists,
                if st.empty > 0 {
                    format!("、忽略空条目 {} 条", st.empty)
                } else {
                    String::new()
                }
            ),
            Err(e) => format!("导入失败：{e}"),
        };
        crate::log::line(&format!("[md-import] {msg}"));
        let _ = handle.emit("xhy://md-import", &msg);
        let _ = handle.emit("xhy://changed", "todos");
    });
}

fn open_data(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    let handle = app.clone();
    app.dialog().file().pick_file(move |path| {
        let Some(p) = path.as_ref().and_then(|f| f.as_path()) else { return };
        let Ok(text) = std::fs::read_to_string(p) else {
            return;
        };
        let state = handle.state::<AppState>();
        if state.import(&text).is_ok() {
            // 对话框回调在主线程：不能直接 sync_windows（建窗会自锁）
            schedule_sync(&handle);
            let _ = handle.emit("xhy://changed", "all");
        }
    });
}

// ── 全局热键 ──────────────────────────────────────────────

/// 在 Builder 阶段创建插件实例（handler 里按按键分派）
///
/// 必须按 `shortcut.key()` 分派：handler 对所有已注册热键都会触发，
/// 不区分就会把"快速添加"也当成"隐藏全部"。
pub fn global_shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_global_shortcut::{Builder, Code, ShortcutState};
    Builder::new()
        .with_handler(|app: &AppHandle, shortcut, ev| {
            if ev.state() != ShortcutState::Pressed {
                return;
            }
            match shortcut.key {
                Code::KeyH => {
                    let state = app.state::<AppState>();
                    let _ = toggle_hide_all(app.clone(), state);
                }
                Code::KeyN => {
                    // 热键回调在主线程：show_launcher 含 build()，直接调自锁
                    schedule_launcher(app);
                }
                _ => {}
            }
        })
        .build()
}

/// setup 阶段注册具体按键
pub fn register_hotkeys(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
    let hide = Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyH);
    match app.global_shortcut().register(hide) {
        Ok(()) => crate::log::line("热键 Alt+Shift+H 注册成功"),
        Err(e) => crate::log::line(&format!("注册全局热键 Alt+Shift+H 失败：{e}")),
    }
    let add = Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyN);
    match app.global_shortcut().register(add) {
        Ok(()) => crate::log::line("热键 Alt+Shift+N 注册成功"),
        Err(e) => crate::log::line(&format!("注册全局热键 Alt+Shift+N 失败：{e}")),
    }
}

/// 切换挂件的"可输入"态。
///
/// 前端在可编辑元素收到 pointerdown/focus 时传 `on=true`，编辑结束传 `false`。
/// 详见 `win::desktop::set_input_mode` —— 不加这个，挂件里的文本框一律打不了字。
#[tauri::command]
pub fn set_input_mode(app: tauri::AppHandle, label: String, on: bool) -> Result<(), String> {
    let Some(win) = app.get_webview_window(&label) else {
        return Ok(()); // 窗口已销毁：无事可做
    };
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        // 必须切到主线程：force_foreground 靠 AttachThreadInput 绕过前台锁定，
        // 而该技巧要求"调用线程就是窗口所属线程"。Tauri 命令跑在工作线程上，
        // 在那里调用会静默失效（实测：样式清掉了但窗口没成为前台）。
        let raw = h.0 as isize; // isize 是 Send，可安全跨线程捕获
        let _ = app.run_on_main_thread(move || unsafe {
            desktop::set_input_mode(windows::Win32::Foundation::HWND(raw as *mut _), on);
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (win, on);
    }
    Ok(())
}

/// 让注册表实况与 store 里的 autostart 设置对齐。
///
/// 不设这一步的话：设置默认 `autostart: true`，但 Run 键从来没写过，
/// "开机即用"就只是界面上一个好看的开关。反向也要同步（用户关掉后
/// 要真的删掉），否则关掉只生效一次。
pub fn sync_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let want = app.state::<AppState>().snapshot().settings.autostart;
    let m = app.autolaunch();
    match m.is_enabled() {
        Ok(have) if have == want => {}
        _ => {
            let r = if want { m.enable() } else { m.disable() };
            if let Err(e) = r {
                tracing::warn!("同步开机自启失败（want={want}）：{e}");
            }
        }
    }
}

#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let m = app.autolaunch();
    if enable {
        m.enable().map_err(|e| e.to_string())?;
    } else {
        m.disable().map_err(|e| e.to_string())?;
    }
    app.state::<AppState>()
        .set_settings(SettingsPatch {
            autostart: Some(enable),
            ..Default::default()
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::tray_opens_main;
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

    fn click(button: MouseButton, state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: "xhy-tray".into(),
            position: tauri::PhysicalPosition::new(0.0f64, 0.0f64),
            rect: tauri::Rect {
                position: tauri::Position::Physical(tauri::PhysicalPosition::new(0i32, 0i32)),
                size: tauri::Size::Physical(tauri::PhysicalSize::new(16u32, 16u32)),
            },
            button,
            button_state: state,
        }
    }

    #[test]
    fn tray_left_up_opens_main() {
        // 用户要求：**单击**托盘打开工作台（原来是双击）
        assert!(tray_opens_main(&click(MouseButton::Left, MouseButtonState::Up)));
    }

    #[test]
    fn tray_left_down_does_not_open() {
        // 按下瞬间不开窗：否则一次点击可能触发两次（按下 + 松开），
        // 而且用户可能只是想按住看看
        assert!(!tray_opens_main(&click(MouseButton::Left, MouseButtonState::Down)));
    }

    #[test]
    fn tray_right_click_never_opens_main() {
        // 右键是弹菜单用的，绝不能顺手把工作台也开了
        assert!(!tray_opens_main(&click(MouseButton::Right, MouseButtonState::Up)));
        assert!(!tray_opens_main(&click(MouseButton::Right, MouseButtonState::Down)));
    }

    #[test]
    fn tray_middle_click_never_opens_main() {
        assert!(!tray_opens_main(&click(MouseButton::Middle, MouseButtonState::Up)));
    }

    #[test]
    fn tray_double_click_is_no_longer_the_trigger() {
        // 显式锁住语义变更：双击不再是打开条件（用户要的是单击）。
        // 若将来有人改回双击，这条会红。
        let dbl = TrayIconEvent::DoubleClick {
            id: "xhy-tray".into(),
            position: tauri::PhysicalPosition::new(0.0f64, 0.0f64),
            rect: tauri::Rect {
                position: tauri::Position::Physical(tauri::PhysicalPosition::new(0i32, 0i32)),
                size: tauri::Size::Physical(tauri::PhysicalSize::new(16u32, 16u32)),
            },
            button: MouseButton::Left,
        };
        assert!(!tray_opens_main(&dbl), "双击不该再是打开条件");
    }
}
