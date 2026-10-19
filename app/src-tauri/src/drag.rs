//! 跨窗口拖拽：幽灵窗 + 全局落点广播
//!
//! 为什么不能用 HTML5 DnD / 纯前端指针事件：
//! 每个挂件是独立 `WebviewWindow`，指针事件与 DOM 都只在自己窗口内有效，
//! "从待办拖到日历"这种跨窗口拖拽前端根本拿不到对方窗口的坐标。
//!
//! 方案：拖拽由 Rust 侧统一接管 ——
//!   1. 前端 `drag_begin(payload)` 起拖
//!   2. Rust 建一个透明、置顶、鼠标穿透的幽灵窗跟随光标显示标签
//!   3. 16ms 轮询检测左键抬起 / Esc，向**所有**窗口广播 `xhy://drag-drop`（全局物理坐标）
//!   4. 各窗口把全局坐标换算成自己的 CSS 坐标，命中自己注册的落点就执行
//!
//! 同窗口排序与跨窗口投放走同一套逻辑，无特例。
//!
//! 幽灵文本的传递刻意不用 `navigate`（`WebviewUrl::App` 只收 PathBuf，
//! 塞 `#fragment` 解析不了），改为前端挂载时主动 `ghost_text()` 拉一次 +
//! 之后靠 `emit_to` 定向推送，避开"窗口还没加载完就 emit"的竞态。

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(windows)]
use windows::Win32::Foundation::POINT;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

pub const GHOST: &str = "drag-ghost";

/// 当前拖拽（None = 没有拖）
fn active() -> &'static Mutex<Option<Active>> {
    static A: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone)]
struct Active {
    label: String,
    payload: serde_json::Value,
}

#[derive(Clone, Serialize)]
struct DropEvent {
    /// 全局物理坐标（各窗口自己换算成 CSS 坐标）
    x: f64,
    y: f64,
    payload: serde_json::Value,
}

#[derive(Clone, Serialize)]
struct MoveEvent {
    x: f64,
    y: f64,
}

#[derive(Serialize)]
pub struct GhostText {
    pub label: String,
    pub dragging: bool,
}

/// 前端挂载时拉当前幽灵文本；也用于 emit 丢失时兜底
#[tauri::command]
pub fn ghost_text() -> Result<GhostText, String> {
    let g = active().lock().map_err(|e| e.to_string())?;
    Ok(match &*g {
        Some(a) => GhostText {
            label: a.label.clone(),
            dragging: true,
        },
        None => GhostText {
            label: String::new(),
            dragging: false,
        },
    })
}

#[tauri::command]
pub async fn drag_begin(app: AppHandle, label: String, payload: serde_json::Value) -> Result<(), String> {
    crate::log::drag(&format!("begin label={label:?}"));
    {
        let mut g = active().lock().map_err(|e| e.to_string())?;
        *g = Some(Active {
            label: label.clone(),
            payload,
        });
    }
    // 异步派发建窗，命令立刻返回（见 request_ghost 的注释）
    request_ghost(&app);
    let _ = app.emit_to(GHOST, "xhy://ghost-text", &label);
    crate::log::drag("begin ok");
    Ok(())
}

#[tauri::command]
pub fn drag_cancel(app: AppHandle) -> Result<(), String> {
    *active().lock().map_err(|e| e.to_string())? = None;
    hide_ghost(&app);
    Ok(())
}

/// 建（或复用）幽灵窗。**只能在主线程调用**（setup 里，或 run_on_main_thread 内）。
pub fn ensure_ghost(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(GHOST).is_some() {
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(app, GHOST, WebviewUrl::App("index.html".into()))
        .title(crate::BRAND)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .focused(false)
        .visible(false)
        .inner_size(260.0, 44.0)
        .build()
        .map_err(|e| e.to_string())?;
    // 鼠标穿透：幽灵窗绝不能挡住落点判定
    let _ = win.set_ignore_cursor_events(true);
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        unsafe {
            crate::win::desktop::apply_window_styles(windows::Win32::Foundation::HWND(h.0 as *mut _))
        };
    }
    crate::log::drag("ghost 已创建");
    Ok(())
}

/// 确保幽灵窗**将会**存在，但绝不等待它建好。
///
/// 为什么不能等待：`ensure_ghost` 要主线程推进窗口创建，而调用它的
/// `drag_begin` 曾经是**同步命令**（跑在主线程）—— 主线程等自己 → 自锁。
/// 后果不是报错，而是 `app.emit` 的事件全投递不出去、前端 `invoke` 永不返回，
/// 表现为"拖不动且毫无提示"。现在 `drag_begin` 已改 async（跑工作线程），
/// 但这里仍坚持异步派发：多层保险，且不必依赖调用方是不是 async。
/// 也不在 setup 里预建：幽灵窗是个隐藏 webview，会常驻多占一个渲染进程
/// （实测每个约 150–200MB），对没用过拖拽的用户是白付的开销。
///
/// 于是惰性 + 异步：首次拖拽时派到主线程创建，命令立刻返回。
/// 轮询器本来就容忍"窗口还不存在"（`if let Some(g) = ...`），
/// 所以最坏情况是第一次拖拽的头一两百毫秒没有幽灵标签，落点判定不受影响。
fn request_ghost(app: &AppHandle) {
    if app.get_webview_window(GHOST).is_some() {
        return;
    }
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Err(e) = ensure_ghost(&handle) {
            crate::log::drag(&format!("ghost 创建失败：{e}"));
        }
    }) {
        crate::log::drag(&format!("ghost 创建派发失败：{e}"));
    }
}

fn hide_ghost(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(GHOST) {
        let _ = win.hide();
    }
}

#[cfg(windows)]
fn cursor_phys() -> Option<(f64, f64)> {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f64, p.y as f64))
}
#[cfg(not(windows))]
fn cursor_phys() -> Option<(f64, f64)> {
    None
}

/// 按键当前是否按下（GetAsyncKeyState 高位）
#[cfg(windows)]
fn down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0 }
}

pub fn spawn_drag_poller(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        // 上一次广播的光标位置：只在移动 >2px 时再发，省掉无变化的空事件
        let mut last: Option<(f64, f64)> = None;
        loop {
            let cur = active()
                .lock()
                .map(|g| g.clone())
                .unwrap_or(None);

            match cur {
                Some(drag) => {
                    if let Some((cx, cy)) = cursor_phys() {
                        if let Some(g) = app.get_webview_window(GHOST) {
                            let _ = g.show();
                            if let Ok(scale) = g.scale_factor() {
                                let s = scale.max(0.01);
                                let _ = g
                                    .set_position(LogicalPosition::new(cx / s + 14.0, cy / s + 16.0));
                            }
                        }
                        let moved = last
                            .map(|(lx, ly)| (cx - lx).abs() > 2.0 || (cy - ly).abs() > 2.0)
                            .unwrap_or(true);
                        if moved {
                            last = Some((cx, cy));
                            // 各窗口据此判定"当前悬停在我的哪个落点"
                            let _ = app.emit("xhy://drag-move", MoveEvent { x: cx, y: cy });
                        }
                    }
                    // 左键抬起 = 落点；Esc = 取消
                    if !down(VK_LBUTTON) {
                        last = None;
                        finish(&app, &drag, cursor_phys());
                    } else if down(VK_ESCAPE) {
                        last = None;
                        if let Ok(mut g) = active().lock() {
                            *g = None;
                        }
                        hide_ghost(&app);
                        let _ = app.emit("xhy://drag-end", ());
                    }
                }
                None => {
                    last = None;
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    });
}

fn finish(app: &AppHandle, drag: &Active, at: Option<(f64, f64)>) {
    if let Ok(mut g) = active().lock() {
        *g = None;
    }
    hide_ghost(app);
    match at {
        Some((x, y)) => {
            crate::log::drag(&format!("drop at ({x:.0},{y:.0}) payload={}", drag.payload));
            let _ = app.emit(
                "xhy://drag-drop",
                DropEvent {
                    x,
                    y,
                    payload: drag.payload.clone(),
                },
            );
        }
        // 拿不到光标位置就什么都不发 —— 前端会静默无反应，必须留痕
        None => crate::log::drag("drop SKIPPED: cursor position unavailable"),
    }
    // 落点判定后清掉各窗口的悬停高亮
    let _ = app.emit("xhy://drag-end", ());
}

static RUNNING: AtomicBool = AtomicBool::new(false);
