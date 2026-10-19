//! 逻辑像素 ↔ 物理像素，与多显示器坐标
//!
//! 数据里一律存**逻辑像素**（跨 DPI/缩放迁移不破相）。显示时才换算。
//! Tauri 的 outer_position / inner_size 返回**物理像素**，
//! 所以进出都要过一次 scaleFactor。

use crate::domain::RectL;

/// 一个显示器的逻辑工作区（已减任务栏）
#[derive(Debug, Clone, Copy, Default)]
pub struct MonitorLogical {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

pub fn to_physical(rect: RectL, scale: f64) -> (i32, i32, u32, u32) {
    (
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        (rect.w * scale).round().max(1.0) as u32,
        (rect.h * scale).round().max(1.0) as u32,
    )
}

pub fn to_logical(x: i32, y: i32, w: u32, h: u32, scale: f64) -> RectL {
    let s = if scale > 0.0 { scale } else { 1.0 };
    RectL {
        x: x as f64 / s,
        y: y as f64 / s,
        w: w as f64 / s,
        h: h as f64 / s,
    }
}

/// 把矩形夹进工作区，保证整块可见（贴边隐藏模式下允许略微出界）
pub fn clamp_to_monitor(mut r: RectL, m: MonitorLogical, keep_visible: f64) -> RectL {
    let min_w = keep_visible.min(r.w);
    let min_h = keep_visible.min(r.h);
    if r.x + r.w > m.x + m.w {
        r.x = m.x + m.w - r.w;
    }
    if r.y + r.h > m.y + m.h {
        r.y = m.y + m.h - r.h;
    }
    if r.x < m.x - (r.w - min_w) {
        r.x = m.x - (r.w - min_w);
    }
    if r.y < m.y - (r.h - min_h) {
        r.y = m.y - (r.h - min_h);
    }
    r
}

/// 包含给定点的显示器索引（物理坐标）
pub fn monitor_containing(mons: &[MonitorLogical], x: f64, y: f64) -> usize {
    mons.iter()
        .position(|m| x >= m.x && x < m.x + m.w && y >= m.y && y < m.y + m.h)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let r = RectL {
            x: 100.0,
            y: 50.5,
            w: 300.0,
            h: 400.25,
        };
        let scale = 1.5;
        let p = to_physical(r, scale);
        let l = to_logical(p.0, p.1, p.2, p.3, scale);
        // 物理像素取整不可避免：误差上限 = 0.5 / scale
        let tol = 0.5 / scale + 1e-9;
        assert!((l.x - r.x).abs() <= tol, "x 误差 {}", (l.x - r.x).abs());
        assert!((l.y - r.y).abs() <= tol, "y 误差 {}", (l.y - r.y).abs());
        assert!((l.w - r.w).abs() <= tol, "w 误差 {}", (l.w - r.w).abs());
        assert!((l.h - r.h).abs() <= tol, "h 误差 {}", (l.h - r.h).abs());
    }

    #[test]
    fn physical_is_integer_at_150pct() {
        let r = RectL { x: 100.0, y: 50.5, w: 300.0, h: 400.25 };
        let (x, y, w, h) = to_physical(r, 1.5);
        assert_eq!((x, y, w, h), (150, 76, 450, 600));
    }

    #[test]
    fn scale_zero_guards() {
        let l = to_logical(0, 0, 100, 100, 0.0);
        assert_eq!(l.w, 100.0);
    }

    #[test]
    fn clamp_keeps_on_screen() {
        let m = MonitorLogical {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
            scale: 1.0,
        };
        let r = RectL {
            x: 3000.0,
            y: -500.0,
            w: 300.0,
            h: 400.0,
        };
        let c = clamp_to_monitor(r, m, 40.0);
        assert!(c.x + c.w <= 1920.0 + 0.001);
        assert!(c.y >= -400.0 + 40.0 - 0.001);
    }

    #[test]
    fn edge_hidden_rect_survives_clamp() {
        // 贴右边缘半隐藏：允许 x 超出屏幕，但必须保留 40px 可见
        let m = MonitorLogical { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0 };
        let r = RectL { x: 1900.0, y: 100.0, w: 300.0, h: 400.0 };
        let c = clamp_to_monitor(r, m, 40.0);
        assert!(c.x + c.w >= 1920.0 - 40.0 - 0.001);
    }

    #[test]
    fn monitor_hit_test() {
        let mons = vec![
            MonitorLogical { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0 },
            MonitorLogical { x: 1920.0, y: 0.0, w: 1280.0, h: 1024.0, scale: 1.0 },
        ];
        assert_eq!(monitor_containing(&mons, 2000.0, 10.0), 1);
        assert_eq!(monitor_containing(&mons, 100.0, 10.0), 0);
        // 越界点回落到 0 号屏
        assert_eq!(monitor_containing(&mons, -50.0, 5000.0), 0);
    }
}
