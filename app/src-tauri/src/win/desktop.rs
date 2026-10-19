//! Windows 桌面层集成：扩展样式 + Bottom-Z + WorkerW 嵌入（可降级）
//!
//! 设计原则：桌面层手法属于未公开行为，宁可降级为"贴底普通窗"，
//! 也绝不让插件不可见或让程序崩溃。所有函数在无权限环境下静默失败并返回降级结果。

#![cfg(windows)]

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, FindWindowExW, FindWindowW, GetForegroundWindow,
    GetParent, GetWindowLongPtrW, GetWindowThreadProcessId, GWL_EXSTYLE, HWND_BOTTOM, HWND_TOP,
    SendMessageW, SetForegroundWindow, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// 实际生效的嵌入模式（可能与用户设置不同）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    BottomZ,
    WorkerW,
}

/// null HWND（SetParent 传 None 语义 = 脱离父窗口）
const NULL_H: HWND = HWND(std::ptr::null_mut());

/// 让窗口不抢焦点、不进 Alt+Tab
///
/// # Safety
/// hwnd 必须是本进程内有效窗口句柄，且在主线程调用。
pub unsafe fn apply_window_styles(hwnd: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex = ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
    }
}

/// 压到 z-order 最底，且不激活
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn force_bottom(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// 查找 Progman（始终存在）
unsafe fn progman() -> Option<HWND> {
    unsafe { FindWindowW(w!("Progman"), PCWSTR::null()).ok() }.filter(|h| !h.is_invalid())
}

/// 承载 SHELLDLL_DefView 的窗口（Progman 或 WorkerW）
///
/// 标准 Win32 桌面：DefView 是 Progman 的子窗口。
/// 开启桌面幻灯片后，Windows 另建一层 WorkerW 承载 DefView，壁纸画在它上面；
/// 此时只有挂到 WorkerW 才能位于壁纸/图标之下。
unsafe fn defview_host() -> Option<HWND> {
    unsafe {
        let pm = progman()?;
        // 催生成 WorkerW（Rainmeter 手法）；失败无副作用
        let _ = SendMessageW(pm, 0x052C, WPARAM(0x000D), LPARAM(0x0001));
        scan_workerw(pm).or(Some(pm))
    }
}

struct EnumCtx {
    progman: HWND,
    found: Option<HWND>,
}

unsafe extern "system" fn find_workerw(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let ctx = &mut *(lparam.0 as *mut EnumCtx);
        // 跳过 Progman 本身：只要"另一个"承载 DefView 的顶层窗口
        if hwnd.0 != ctx.progman.0 {
            if let Ok(dv) = FindWindowExW(hwnd, NULL_H, w!("SHELLDLL_DefView"), PCWSTR::null()) {
                if !dv.is_invalid() {
                    ctx.found = Some(hwnd);
                    return BOOL(0); // 停止枚举
                }
            }
        }
        BOOL(1) // 继续
    }
}

/// 扫描 Progman 之外、承载 DefView 的顶层窗口（即 WorkerW）
unsafe fn scan_workerw(progman: HWND) -> Option<HWND> {
    unsafe {
        let mut ctx = EnumCtx {
            progman,
            found: None,
        };
        let p = &mut ctx as *mut EnumCtx as isize;
        let _ = EnumWindows(Some(find_workerw), LPARAM(p));
        ctx.found
    }
}

/// 定位可用作嵌入父窗口的桌面层窗口；找不到返回 None
pub fn desktop_parent() -> Option<HWND> {
    unsafe { defview_host() }
}

/// 把窗口挂到桌面层父窗口（真·嵌入）。成功返回 true
///
/// 挂进去后必须用 **HWND_TOP**：父窗口 Progman 的子 z-order 里，
/// `SHELLDLL_DefView`（壁纸 + 桌面图标的绘制者）排在最上，
/// 若把挂件压到 HWND_BOTTOM 就会被壁纸整个盖住 —— 窗口存在但看不见。
/// 挂件仍在 Progman 之内，所以对普通应用窗口而言依旧"贴在桌面上"。
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn attach(hwnd: HWND, parent: HWND) -> bool {
    unsafe {
        if SetParent(hwnd, parent).is_ok() {
            raise_in_parent(hwnd);
            return true;
        }
        false
    }
}

/// 在**当前父窗口内**抬到最前；无父（顶层）时压到全局最底
///
/// 两种模式的语义都是"待在桌面上、不抢普通窗口的地方"：
///   · 已嵌入：父是 Progman，抬到父内最前 → 盖住壁纸与图标，可点击
///   · 未嵌入：顶层窗口，压到 z-order 最底 → 在所有窗口之下
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn reseat(hwnd: HWND) {
    unsafe {
        // GetParent 在无父窗口时返回 Err（包装层把 null 视作失败）
        match GetParent(hwnd) {
            Ok(_) => raise_in_parent(hwnd),
            Err(_) => force_bottom(hwnd),
        }
    }
}

/// 父窗口内抬到最前
unsafe fn raise_in_parent(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// 从桌面层脱离，恢复为普通顶层窗口
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn detach(hwnd: HWND) {
    unsafe {
        let _ = SetParent(hwnd, NULL_H);
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
}

/// 绕过 Windows 前台锁定，强制让 hwnd 成为前台。
///
/// 关键点：挂件进程**不是**前台进程（`WS_EX_NOACTIVATE` 就是为这个），
/// 而 Windows 默认只允许前台进程调 SetForegroundWindow —— 直接调会被静默拒绝。
/// 标准解法是把本线程输入临时附加到当前前台线程，让请求看起来由前台进程发起。
///
/// # Safety 见 [`apply_window_styles`]
unsafe fn force_foreground(hwnd: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        let cur = GetCurrentThreadId();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        // 先试附加输入队列（跨进程前台锁定的标准解法）。Attach 可能因桌面/
        // 完整性级别被拒，**不能**因此跳过激活 —— 实测真鼠标点击后本线程
        // 持有"最后输入权"，裸 SetForegroundWindow 就会被放行。
        let attached = fg_tid != 0 && fg_tid != cur && AttachThreadInput(fg_tid, cur, BOOL(1)).as_bool();
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetActiveWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(fg_tid, cur, BOOL(0));
        }
    }
}

/// 切换"可输入"状态。
///
/// 背景：挂件平时带 `WS_EX_NOACTIVATE`，点它不会激活窗口 —— 这是桌面挂件
/// "不抢焦点"的关键。但副作用是**点文本框也拿不到键盘焦点，完全无法输入**。
/// 所以按需切换：用户点到可编辑元素时清除该样式并激活窗口，编辑结束再恢复。
///
/// 返回切换后是否处于"可输入"态。
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn set_input_mode(hwnd: HWND, on: bool) -> bool {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let next = if on {
            ex & !WS_EX_NOACTIVATE.0
        } else {
            ex | WS_EX_NOACTIVATE.0
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
        if on {
            // 必须绕过前台锁定，否则非前台进程的调用会被静默拒绝
            force_foreground(hwnd);
        } else {
            // 编辑结束：退回桌面层原位，别赖在前台
            reseat(hwnd);
        }
        on
    }
}

/// 应用期望模式，返回实际生效模式。WorkerW 不可用时自动降级 BottomZ。
///
/// # Safety 见 [`apply_window_styles`]
pub unsafe fn apply(hwnd: HWND, want: Effective) -> Effective {
    unsafe {
        apply_window_styles(hwnd);
        match want {
            Effective::WorkerW => match desktop_parent() {
                Some(parent) if attach(hwnd, parent) => Effective::WorkerW,
                _ => {
                    detach(hwnd);
                    force_bottom(hwnd);
                    Effective::BottomZ
                }
            },
            Effective::BottomZ => {
                detach(hwnd);
                force_bottom(hwnd);
                Effective::BottomZ
            }
        }
    }
}

// 未使用但保留给后续子类化（贴边隐藏钉 z-order）用
#[allow(dead_code)]
type WndProcPrev = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_hwnd_is_invalid() {
        assert!(NULL_H.is_invalid());
    }

    #[test]
    #[ignore = "需要真实桌面会话，手动跑"]
    fn finds_desktop_parent() {
        let h = desktop_parent();
        assert!(h.is_some(), "应能找到 Progman/WorkerW");
    }
}
