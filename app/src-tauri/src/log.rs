//! 极简文件日志：追加到数据目录下的 xhy.log
//!
//! 为什么不用 tracing-subscriber：本项目此前**没有任何日志订阅器**，
//! 所有 tracing::warn! 都进了黑洞 —— 跨窗口拖拽这类只在真机出现的
//! 问题因此完全无法诊断，只能靠猜。
//!
//! 写失败一律静默退化：日志是诊断辅助，绝不能反过来搞挂主流程。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 单份日志上限：256 KB 足够装下几千行诊断信息
const MAX_BYTES: u64 = 256 * 1024;

fn log_path() -> &'static OnceLock<Option<PathBuf>> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    &P
}

/// 初始化日志文件位置（setup 里调一次，传数据目录根）
pub fn init(root: &std::path::Path) {
    let _ = log_path().set(Some(root.join("xhy.log")));
}

/// 追加一行。前缀是 UNIX 毫秒时间戳，便于与前端时序对齐。
pub fn line(msg: &str) {
    let Some(path) = log_path().get_or_init(|| None).as_ref() else {
        return;
    };
    // 目录可能还不存在（首启早期）
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 体积上限：诊断日志不能把用户磁盘写满。超阈值就整份轮替一份旧的。
    if let Ok(md) = std::fs::metadata(path) {
        if md.len() > MAX_BYTES {
            let old = path.with_extension("log.old");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(path, &old);
        }
    }
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(f, "[{now}] {msg}");
}

/// 记录一次拖拽相关事件
pub fn drag(msg: &str) {
    line(&format!("DRAG {msg}"));
}

/// 记录一次贴边状态机跃迁（"拖不出来"这类真机问题靠它定位）
pub fn dock(msg: &str) {
    line(&format!("DOCK {msg}"));
}

/// 清空日志（诊断脚本用，避免读到上一次的记录）
pub fn clear() {
    if let Some(path) = log_path().get_or_init(|| None).as_ref() {
        let _ = std::fs::remove_file(path);
    }
}
