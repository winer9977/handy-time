//! 贴边自动隐藏的运行时驱动：把 win::edgehide 的纯几何接到真实窗口上
//!
//! 语义对齐「小黄条」"贴近屏幕边缘自动隐藏"：**拖到边上就自动停靠**，
//! 无需预先开关 —— 检测到贴边即停靠，并把边写回 store（重启后保持）。
//!
//! 状态机（每挂件独立）：
//!   Free ──(贴边且已停手)──> Docked ──(光标进入边条)──> Revealed
//!     ^                        <──(光标离开驻留区)────────┘
//!
//! "是否停手"用**位置轨迹**判断（位置一变就重置计时），不依赖前端上报：
//! startDragging 期间 webview 收不到 pointerup，任何前端协作都不可靠。
//!
//! 轮询而非事件驱动：Win32 没有"光标进入窗口外区域"的廉价通知，
//! Rainmeter / DisplayFusion 同样低频轮询。120ms 一次，肉眼无感。

use crate::domain::RectL;
use crate::state::AppState;
use crate::win::edgehide::{self, Edge, Pt};
use crate::win::geometry::MonitorLogical;
use crate::widgets::{label_of, monitors_of};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, LogicalPosition, Manager};

#[derive(Debug, Clone)]
pub enum DockState {
    /// 停靠收起中。`user` = 完整在屏的用户摆放位；`docked` = 实际收起位
    Docked { edge: Edge, user: RectL, docked: RectL },
    /// 已唤出显示。`user` = 唤出后所处位置（完整在屏、贴边）
    Revealed { edge: Edge, user: RectL },
}

/// 唤出后，光标在挂件矩形外扩 HOLD 内不收回
const HOLD: f64 = 14.0;
/// 停靠时留在屏内的可见条宽度（逻辑像素）
const KEEP: f64 = 16.0;
/// "已停手"静默期：拖动途中不做停靠判定
const SETTLE: Duration = Duration::from_millis(320);
/// 判定"收起态被用户拖走"的位移阈值
const DRAG_AWAY: f64 = 40.0;
/// 轨迹比较的位置变化容差（逻辑像素）
const MOVE_EPS: f64 = 1.0;

/// 每挂件轨迹：最后观测位置、最后变化时刻、本会话内是否被移动过
#[derive(Debug, Clone)]
struct Track {
    rect: RectL,
    at: Instant,
    moved: bool,
}

fn docks() -> &'static Mutex<HashMap<String, DockState>> {
    static D: OnceLock<Mutex<HashMap<String, DockState>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

fn tracks() -> &'static Mutex<HashMap<String, Track>> {
    static T: OnceLock<Mutex<HashMap<String, Track>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

pub fn spawn_edge_poller(app: AppHandle) {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || loop {
        tick(&app);
        std::thread::sleep(Duration::from_millis(120));
    });
}

/// 前端回写 rect 时调用：若该挂件正处停靠/唤出态，说明用户动了它 ——
/// 丢弃停靠状态，由下一轮 tick 基于新位置重新判定，避免把"半出界"位置
/// 当成用户意图存下来。
pub fn is_docked(id: &str) -> bool {
    docks().lock().map(|d| d.contains_key(id)).unwrap_or(false)
}

/// 挂件被删/关时清理其轨迹与状态
pub fn forget(id: &str) {
    if let Ok(mut d) = docks().lock() {
        d.remove(id);
    }
    if let Ok(mut t) = tracks().lock() {
        t.remove(id);
    }
}

/// 更新轨迹，返回 `(已停手, 本会话内被移动过)`。
///
/// "被移动过"是停靠的**前置条件**：静态摆放（首次启动播种、程序自己移动）
/// 恰好贴着边时不该自动收起 —— 只有用户亲手把它拖到边上才是"我要贴边隐藏"。
/// 注意程序自己 move_to 造成的位移不算：那种位移方向是"推出屏幕"，
/// 用 `program_moves` 记录我们主动下发的位置，比对时跳过。
fn note_position(id: &str, cur: RectL, now: Instant) -> (bool, bool) {
    let mut t = match tracks().lock() {
        Ok(g) => g,
        Err(_) => return (false, false),
    };
    let ours = match program_moves().lock() {
        Ok(mut p) => p.remove(id),
        Err(_) => None,
    };
    // 我们刚下发的位置：只刷新基准，不算用户移动
    if let Some(o) = &ours {
        if same_rect(o, &cur) {
            if let Some(tr) = t.get_mut(id) {
                tr.rect = cur;
                tr.at = now;
            }
            return (false, t.get(id).map(|x| x.moved).unwrap_or(false));
        }
    }
    match t.get_mut(id) {
        Some(tr) => {
            let moved = !same_rect(&tr.rect, &cur);
            if moved {
                tr.rect = cur;
                tr.at = now;
                tr.moved = true;
                return (false, true);
            }
            (now.duration_since(tr.at) >= SETTLE, tr.moved)
        }
        None => {
            t.insert(
                id.to_string(),
                Track {
                    rect: cur,
                    at: now,
                    moved: false,
                },
            );
            (false, false) // 首次见到：先记录，下一轮再判
        }
    }
}

fn same_rect(a: &RectL, b: &RectL) -> bool {
    (a.x - b.x).abs() <= MOVE_EPS
        && (a.y - b.y).abs() <= MOVE_EPS
        && (a.w - b.w).abs() <= MOVE_EPS
        && (a.h - b.h).abs() <= MOVE_EPS
}

/// 本线程主动下发的位置，用于区分"用户拖动"与"程序摆放"
fn program_moves() -> &'static Mutex<HashMap<String, RectL>> {
    static P: OnceLock<Mutex<HashMap<String, RectL>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// 单次扫描
fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    if *state.hidden_all.read() {
        return; // 快速隐藏期间不做贴边逻辑
    }
    let store = state.snapshot();
    let mons = monitors_of(app);
    let cursor = match app.cursor_position() {
        Ok(p) => p,
        Err(_) => return,
    };
    let now = Instant::now();

    let mut map = match docks().lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    // 挂件被删/隐藏 → 丢状态与轨迹
    let alive: Vec<String> = store
        .widgets
        .iter()
        .filter(|w| w.visible)
        .map(|w| w.id.clone())
        .collect();
    map.retain(|id, _| alive.iter().any(|a| a == id));

    for w in store.widgets.iter().filter(|w| w.visible) {
        let Some(win) = app.get_webview_window(&label_of(&w.id)) else {
            continue;
        };
        let Ok(scale) = win.scale_factor() else { continue };
        let scale = scale.max(0.01);
        let (Ok(pos), Ok(size)) = (win.outer_position(), win.inner_size()) else {
            continue;
        };
        let cur = RectL {
            x: pos.x as f64 / scale,
            y: pos.y as f64 / scale,
            w: size.width as f64 / scale,
            h: size.height as f64 / scale,
        };
        let (settled, moved) = note_position(&w.id, cur, now);
        let m = monitor_for(&mons, cur);
        let cp = Pt {
            x: cursor.x / scale,
            y: cursor.y / scale,
        };
        let threshold = store.settings.edge_hide_threshold;

        // 停靠资格：用户本会话拖过，或上次运行就停在边上（重启恢复）。
        // 首次启动播种的位置即使贴边也不算 —— 那不是用户"要贴边隐藏"的意图。
        let eligible = w.auto_dock && (moved || w.edge_hide.is_some());

        match map.get(&w.id).cloned() {
            Some(DockState::Revealed { edge, user }) => {
                // settled 前置：窗口在动 = 用户正拖着它，此刻绝不收回/收起，
                // 否则与系统拖拽互相抢位置（修"拖不出来"的关键之一）
                if settled && edgehide::should_hide(cp, cur, HOLD) {
                    let d = edgehide::docked_rect(user, m, edge, KEEP);
                    move_to(app, &w.id, d);
                    map.insert(w.id.clone(), DockState::Docked { edge, user, docked: d });
                } else if settled && edgehide::detect_edge(cur, m, threshold).is_none() {
                    // 唤出后用户把它拖离边缘 → 自由。位置必须落库：停靠/唤出
                    // 期间 is_docked 为真，前端回写被忽略，不落就丢位置了。
                    crate::log::dock(&format!("{} 唤出态拖离 → 自由", w.id));
                    map.remove(&w.id);
                    free_widget(app, &w.id, cur);
                }
            }
            Some(DockState::Docked { edge, user, docked }) => {
                if !eligible {
                    // 用户关了贴边开关：还原到屏内位置
                    move_to(app, &w.id, edgehide::restored_rect(user, edge, m));
                    map.remove(&w.id);
                    continue;
                }
                // 按轴分离判断（修"拖不出来"）：
                //   along —— 沿停靠轴（把窗口往外拽）
                //   cross —— 交叉轴（沿边滑动）
                // 两者的正确反应完全不同，混成一个"位移量"必然出错。
                let (along, cross) = axis_deltas(&cur, &docked, edge);
                if along.abs() > DRAG_AWAY {
                    // 沿停靠轴拖出去且已停手 = 用户要摆脱贴边
                    if !settled {
                        continue; // 还在拖，别抢
                    }
                    // 先把它从停靠轴拉回屏内，否则"溢出状态"会被 detect_edge
                    // 判成贴边 → 立刻又吸回去，用户怎么拖都拖不出来
                    // （实测复现的根因）。gap 必须 > 阈值，否则拉回来仍算贴边。
                    let gap = threshold.max(KEEP) + 10.0;
                    let freed = edgehide::pull_away_from(cur, m, edge, gap);
                    if !same_rect(&freed, &cur) {
                        move_to(app, &w.id, freed);
                    }
                    match edgehide::detect_edge(freed, m, threshold) {
                        Some(e) => {
                            let d = edgehide::docked_rect(freed, m, e, KEEP);
                            move_to(app, &w.id, d);
                            remember_edge(app, &w.id, e);
                            crate::log::dock(&format!("{} 换边停靠 {}", w.id, e.as_str()));
                            map.insert(
                                w.id.clone(),
                                DockState::Docked { edge: e, user: freed, docked: d },
                            );
                        }
                        None => {
                            // 拖到屏内非边缘处：彻底自由。位置与 edgeHide 一起
                            // 落库 —— 只清标记不落位置，重启后窗口会开在屏外。
                            crate::log::dock(&format!("{} 脱离贴边 → 自由", w.id));
                            map.remove(&w.id);
                            free_widget(app, &w.id, freed);
                        }
                    }
                    continue;
                }
                if cross.abs() > MOVE_EPS {
                    // 沿边滑动：只把交叉轴基线挪到新位置，**不动窗口**。
                    // 早先这里会 move_to 重算的收起位，那会把 along 分量
                    // 一并拽回边条 —— 用户"往外拽一半"就被弹回去。
                    // 停靠轴分量保持原样（cur 在该轴上等于 docked）。
                    let follow = RectL {
                        x: if is_vertical(edge) { cur.x } else { docked.x },
                        y: if is_vertical(edge) { docked.y } else { cur.y },
                        w: cur.w,
                        h: cur.h,
                    };
                    map.insert(
                        w.id.clone(),
                        DockState::Docked {
                            edge,
                            // user 语义 = 完整在屏的摆放位（关贴边开关时还原用）
                            user: edgehide::restored_rect(follow, edge, m),
                            docked: follow,
                        },
                    );
                    continue;
                }
                // 悬停唤出：必须「已停手」。用户按住边条起拖的瞬间光标就在
                // 热区内，若此刻无条件 move_to 弹出，系统拖拽会丢失起点 ——
                // 窗口跳走、鼠标还按着，表现就是"拖不动/拖不出来"。
                if settled && edgehide::should_reveal(cp, cur, edge, KEEP + HOLD) {
                    let back = edgehide::restored_rect(cur, edge, m);
                    move_to(app, &w.id, back);
                    crate::log::dock(&format!("{} 唤出 {}", w.id, edge.as_str()));
                    map.insert(w.id.clone(), DockState::Revealed { edge, user: back });
                }
            }
            None => {
                let Some(edge) = edgehide::detect_edge(cur, m, threshold) else {
                    // 不在边上：清掉过期的 edgeHide 标记
                    if settled && w.edge_hide.is_some() {
                        clear_edge(app, &w.id);
                    }
                    continue;
                };
                if !eligible || !settled {
                    continue; // 无停靠资格，或还在拖
                }
                let d = edgehide::docked_rect(cur, m, edge, KEEP);
                move_to(app, &w.id, d);
                remember_edge(app, &w.id, edge);
                map.insert(w.id.clone(), DockState::Docked { edge, user: cur, docked: d });
            }
        }
    }
}

/// 把检测到的边持久化，重启后仍保持停靠。
///
/// 一律走 `patch_widget`（字段级）：此前是"读整个 Widget → 改一个字段 →
/// 写回整个对象"，与并发的显隐/尺寸写者互相抹掉（lost update）。
fn remember_edge(app: &AppHandle, id: &str, edge: Edge) {
    let state = app.state::<AppState>();
    let cur = state.snapshot().widgets.into_iter().find(|x| x.id == id);
    let Some(w) = cur else { return };
    if w.edge_hide.as_deref() == Some(edge.as_str()) {
        return; // 没变就别白写一次盘 + 广播
    }
    let _ = state.patch_widget(
        id,
        crate::domain::WidgetPatch {
            edge_hide: Some(Some(edge.as_str().into())),
            ..Default::default()
        },
    );
}

fn clear_edge(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let Some(w) = state.snapshot().widgets.into_iter().find(|x| x.id == id) else {
        return;
    };
    if w.edge_hide.is_none() {
        return;
    }
    let _ = state.patch_widget(
        id,
        crate::domain::WidgetPatch {
            edge_hide: Some(None),
            ..Default::default()
        },
    );
}

/// 彻底脱离贴边：位置与 edgeHide 一起落库。
///
/// 只清标记不落位置 = 重启后窗口仍开在屏外（用户视角"挂件丢了"）。
fn free_widget(app: &AppHandle, id: &str, r: RectL) {
    let state = app.state::<AppState>();
    let Some(w) = state.snapshot().widgets.into_iter().find(|x| x.id == id) else {
        return;
    };
    crate::log::dock(&format!(
        "free_widget {id} 旧rect=({},{}) → 新rect=({:.1},{:.1})",
        w.rect.x, w.rect.y, r.x, r.y
    ));
    let _ = state.patch_widget(
        id,
        crate::domain::WidgetPatch {
            rect: Some(r),
            edge_hide: Some(None),
            free: Some(true),
            ..Default::default()
        },
    );
    let _ = tauri::Emitter::emit(app, "xhy://layout", id.to_string());
}

/// 停靠边是否为竖边（左/右）——决定"沿边滑动"走 y 轴还是 x 轴
fn is_vertical(edge: Edge) -> bool {
    matches!(edge, Edge::Left | Edge::Right)
}

/// 相对收起位的两个轴位移：`(沿停靠轴, 交叉轴)`。
///
/// 混成一个标量必然出错：沿停靠轴拖出去 = 用户想摆脱贴边；
/// 交叉轴移动 = 只是沿边滑一滑，窗口该跟着走但不该被拽回边上。
fn axis_deltas(cur: &RectL, docked: &RectL, edge: Edge) -> (f64, f64) {
    if is_vertical(edge) {
        (cur.x - docked.x, cur.y - docked.y)
    } else {
        (cur.y - docked.y, cur.x - docked.x)
    }
}

/// 收起态窗口被用户拖走的判据：沿停靠轴位移明显
/// （tick 直接用 axis_deltas 分轴判断，此函数供单测锚定语义）
#[cfg(test)]
fn dragged_away(cur: &RectL, docked: &RectL, edge: Edge) -> bool {
    axis_deltas(cur, docked, edge).0.abs() > DRAG_AWAY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> RectL {
        RectL { x, y, w, h }
    }

    #[test]
    fn right_edge_splits_axes() {
        let docked = r(1500.0, 100.0, 388.0, 500.0);
        // 沿停靠轴（x）往外拽 → along 大、cross 零
        let (a, c) = axis_deltas(&r(1400.0, 100.0, 388.0, 500.0), &docked, Edge::Right);
        assert_eq!((a, c), (-100.0, 0.0));
        assert!(dragged_away(&r(1400.0, 100.0, 388.0, 500.0), &docked, Edge::Right));
        // 交叉轴（y）沿边滑 → along 零，不算拖走（tick 应跟随而非拽回）
        let (a2, c2) = axis_deltas(&r(1500.0, 200.0, 388.0, 500.0), &docked, Edge::Right);
        assert_eq!((a2, c2), (0.0, 100.0));
        assert!(!dragged_away(&r(1500.0, 200.0, 388.0, 500.0), &docked, Edge::Right));
    }

    #[test]
    fn top_edge_splits_axes() {
        let docked = r(300.0, -484.0, 388.0, 500.0);
        // 顶边：停靠轴 = y，交叉轴 = x
        let (a, c) = axis_deltas(&r(300.0, -400.0, 388.0, 500.0), &docked, Edge::Top);
        assert_eq!((a, c), (84.0, 0.0));
        assert!(dragged_away(&r(300.0, -400.0, 388.0, 500.0), &docked, Edge::Top));
        // 沿顶边横移 100px 不该判成拖走
        assert!(!dragged_away(&r(400.0, -484.0, 388.0, 500.0), &docked, Edge::Top));
    }

    #[test]
    fn small_along_drag_is_not_away() {
        // "往外拽一半"（< 阈值）：既不唤出也不该被弹回 —— tick 靠这个
        // 判断把窗口留在用户放下的位置
        let docked = r(1500.0, 100.0, 388.0, 500.0);
        assert!(!dragged_away(&r(1480.0, 100.0, 388.0, 500.0), &docked, Edge::Right));
        assert!(dragged_away(&r(1450.0, 100.0, 388.0, 500.0), &docked, Edge::Right));
    }
}

fn monitor_for(mons: &[MonitorLogical], r: RectL) -> MonitorLogical {
    let i = crate::win::geometry::monitor_containing(mons, r.x + r.w / 2.0, r.y + r.h / 2.0);
    mons.get(i).copied().unwrap_or(mons[0])
}

fn move_to(app: &AppHandle, id: &str, r: RectL) {
    let Some(win) = app.get_webview_window(&label_of(id)) else {
        return;
    };
    // 登记为"程序下发的位置"：下一轮观测到它就说明不是用户在拖，
    // 否则贴边收起会被误判成用户移动，导致状态机自己跟自己打架。
    if let Ok(mut p) = program_moves().lock() {
        p.insert(id.to_string(), r);
    }
    let _ = win.set_position(LogicalPosition::new(r.x, r.y));
    // 移动会破坏 z-order，重新归位：
    // 已嵌入 Progman 时抬到父内最前（压底会被壁纸盖住），顶层时压到最底
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        unsafe {
            crate::win::desktop::reseat(windows::Win32::Foundation::HWND(h.0 as *mut _))
        };
    }
}
