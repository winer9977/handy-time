//! 贴屏幕边缘自动隐藏（dock / reveal）
//!
//! 语义：用户把挂件拖到贴近某条屏边（左/右/上）且距离 < 阈值时，
//! 该挂件进入"停靠"状态 —— 大部分滑出屏幕，只留一条 `keep_visible`
//! 的边条；光标移到边条附近时滑出显示，离开后收回。
//!
//! 本模块只做**纯几何判断**（可单测）；实际移动窗口由 `widgets` 侧的
//! 轮询线程驱动，避免把 Win32 调用混进逻辑里。

use crate::domain::RectL;
use crate::win::geometry::MonitorLogical;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
}

impl Edge {
    pub fn parse(s: &str) -> Option<Edge> {
        match s {
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            "top" => Some(Edge::Top),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
        }
    }
}

/// 光标位置
#[derive(Debug, Clone, Copy)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

/// 该矩形是否贴到某条边（在阈值内）。返回优先级最高的边。
///
/// 角落同时贴两边时取"更贴"的那条（距离更小者）。
pub fn detect_edge(r: RectL, m: MonitorLogical, threshold: f64) -> Option<Edge> {
    let mut best: Option<(f64, Edge)> = None;
    let mut consider = |d: f64, e: Edge| {
        if d <= threshold && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, e));
        }
    };
    consider(r.x - m.x, Edge::Left);
    consider((m.x + m.w) - (r.x + r.w), Edge::Right);
    consider(r.y - m.y, Edge::Top);
    best.map(|(_, e)| e)
}

/// 停靠后的矩形：沿该边推到屏幕外，只保留 `keep` 逻辑像素可见
pub fn docked_rect(r: RectL, m: MonitorLogical, edge: Edge, keep: f64) -> RectL {
    let keep = keep.min(if edge == Edge::Top { r.h } else { r.w }.max(1.0));
    match edge {
        Edge::Left => RectL {
            x: m.x - (r.w - keep),
            y: r.y,
            w: r.w,
            h: r.h,
        },
        Edge::Right => RectL {
            x: m.x + m.w - keep,
            y: r.y,
            w: r.w,
            h: r.h,
        },
        Edge::Top => RectL {
            x: r.x,
            y: m.y - (r.h - keep),
            w: r.w,
            h: r.h,
        },
    }
}

/// 还原矩形：停靠前用户摆放的位置（去掉出界分量后贴回边上）
pub fn restored_rect(docked: RectL, edge: Edge, m: MonitorLogical) -> RectL {
    match edge {
        Edge::Left => RectL { x: m.x, ..docked },
        Edge::Right => RectL {
            x: m.x + m.w - docked.w,
            ..docked
        },
        Edge::Top => RectL { y: m.y, ..docked },
    }
}

/// 把矩形从 `edge` 方向拉回屏内，并保证与该边至少隔 `gap`。
///
/// "拖不出来"的根因就在这里：收起态窗口被往外拽之后，只要它**还溢出**那条
/// 边，`detect_edge` 就仍判定它贴该边（负距离 ≤ 阈值），于是松手即被吸回去。
/// 用户再怎么拖都拖不出来。脱离贴边时必须显式把它放到"不再贴那条边"的位置。
pub fn pull_away_from(r: RectL, m: MonitorLogical, edge: Edge, gap: f64) -> RectL {
    match edge {
        // 左缘：x 至少要 m.x + gap
        Edge::Left => RectL {
            x: r.x.max(m.x + gap),
            ..r
        },
        // 右缘：右边界至多 m.x+m.w-gap
        Edge::Right => RectL {
            x: (m.x + m.w - gap - r.w).min(r.x),
            ..r
        },
        // 顶缘：y 至少要 m.y + gap
        Edge::Top => RectL {
            y: r.y.max(m.y + gap),
            ..r
        },
    }
}

/// 光标是否落在"该唤出"的热区。
///
/// 热区 = 停靠矩形本身（其可见部分即边条）沿出界方向再外扩 `pad`。
/// 出界一侧的坐标本就够不着，外扩只是让"贴着屏幕边"稳定命中。
/// 交叉轴取停靠矩形范围，避免屏幕别处的光标误触发。
pub fn should_reveal(p: Pt, docked: RectL, edge: Edge, pad: f64) -> bool {
    let r = match edge {
        Edge::Left => RectL {
            x: docked.x,
            y: docked.y,
            w: docked.w + pad,
            h: docked.h,
        },
        Edge::Right => RectL {
            x: docked.x - pad,
            y: docked.y,
            w: docked.w + pad,
            h: docked.h,
        },
        Edge::Top => RectL {
            x: docked.x,
            y: docked.y,
            w: docked.w,
            h: docked.h + pad,
        },
    };
    p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
}

/// 光标是否已离开"驻留区"（唤出后的完整矩形 + hold 余量），用于决定收回
pub fn should_hide(p: Pt, shown: RectL, hold: f64) -> bool {
    let r = RectL {
        x: shown.x - hold,
        y: shown.y - hold,
        w: shown.w + hold * 2.0,
        h: shown.h + hold * 2.0,
    };
    !(p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> MonitorLogical {
        MonitorLogical { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0 }
    }
    fn r(x: f64, y: f64, w: f64, h: f64) -> RectL {
        RectL { x, y, w, h }
    }

    #[test]
    fn detects_right_edge() {
        assert_eq!(detect_edge(r(1600.0, 100.0, 310.0, 370.0), m(), 24.0), Some(Edge::Right));
    }

    #[test]
    fn detects_left_edge() {
        assert_eq!(detect_edge(r(5.0, 100.0, 310.0, 370.0), m(), 24.0), Some(Edge::Left));
    }

    #[test]
    fn detects_top_edge() {
        assert_eq!(detect_edge(r(500.0, 2.0, 310.0, 370.0), m(), 24.0), Some(Edge::Top));
    }

    #[test]
    fn middle_is_not_docked() {
        assert_eq!(detect_edge(r(800.0, 400.0, 310.0, 370.0), m(), 24.0), None);
    }

    #[test]
    fn threshold_respected() {
        // 距右边缘 40px，阈值 24 → 不触发
        assert_eq!(detect_edge(r(1570.0, 100.0, 310.0, 370.0), m(), 24.0), None);
        // 阈值 50 → 触发
        assert_eq!(detect_edge(r(1570.0, 100.0, 310.0, 370.0), m(), 50.0), Some(Edge::Right));
    }

    #[test]
    fn corner_picks_closer_edge() {
        // 距顶 3px、距右 10px → 取更贴的顶边
        assert_eq!(detect_edge(r(1600.0, 3.0, 310.0, 370.0), m(), 24.0), Some(Edge::Top));
    }

    #[test]
    fn docked_right_keeps_sliver() {
        let d = docked_rect(r(1600.0, 100.0, 310.0, 370.0), m(), Edge::Right, 24.0);
        // 只留 24px 在屏内：右边缘 = 1920，故 x = 1920-24 = 1896
        assert_eq!(d.x, 1896.0);
        assert_eq!(1920.0 - d.x, 24.0, "屏内可见 24px");
        assert_eq!(d.x + d.w - 1920.0, 310.0 - 24.0, "出界 286px");
    }

    #[test]
    fn docked_left_offscreens_most() {
        let d = docked_rect(r(5.0, 100.0, 310.0, 370.0), m(), Edge::Left, 24.0);
        assert_eq!(d.x, -(310.0 - 24.0));
        assert_eq!(d.x + d.w, 24.0); // 可见 24px
    }

    #[test]
    fn docked_top_offscreens_most() {
        let d = docked_rect(r(500.0, 2.0, 310.0, 370.0), m(), Edge::Top, 24.0);
        assert_eq!(d.y, -(370.0 - 24.0));
        assert_eq!(d.y + d.h, 24.0);
    }

    #[test]
    fn keep_cannot_exceed_size() {
        // 挂件比 keep 还窄时，不能推出"负出界"
        let d = docked_rect(r(0.0, 0.0, 10.0, 10.0), m(), Edge::Left, 40.0);
        assert_eq!(d.x, 0.0 - (10.0f64 - 10.0));
        assert!(d.x + d.w <= 10.0);
    }

    #[test]
    fn reveal_zone_covers_sliver() {
        let d = docked_rect(r(1600.0, 100.0, 310.0, 370.0), m(), Edge::Right, 24.0);
        // 光标在可见边条上 → 唤出
        assert!(should_reveal(Pt { x: 1900.0, y: 200.0 }, d, Edge::Right, 6.0));
        // 光标在屏幕中间 → 不唤出
        assert!(!should_reveal(Pt { x: 900.0, y: 500.0 }, d, Edge::Right, 6.0));
        // 光标在边条上方（y 超范围）→ 不唤出
        assert!(!should_reveal(Pt { x: 1900.0, y: 20.0 }, d, Edge::Right, 6.0));
    }

    #[test]
    fn reveal_zone_includes_offscreen_margin() {
        let d = docked_rect(r(5.0, 100.0, 310.0, 370.0), m(), Edge::Left, 24.0);
        // 左停靠：可见条 x∈[0,24]；光标贴最左缘（x=0）应唤出
        assert!(should_reveal(Pt { x: 0.0, y: 200.0 }, d, Edge::Left, 6.0));
    }

    #[test]
    fn hide_when_cursor_leaves_shown_rect() {
        let shown = r(1610.0, 100.0, 310.0, 370.0);
        // 光标在挂件内 → 不收回
        assert!(!should_hide(Pt { x: 1700.0, y: 200.0 }, shown, 12.0));
        // 光标跑到屏幕中间 → 收回
        assert!(should_hide(Pt { x: 800.0, y: 500.0 }, shown, 12.0));
    }

    #[test]
    fn edge_strings_roundtrip() {
        for s in ["left", "right", "top"] {
            assert_eq!(Edge::parse(s).unwrap().as_str(), s);
        }
        assert_eq!(Edge::parse("bottom"), None);
        assert_eq!(Edge::parse(""), None);
    }

    #[test]
    fn pull_away_breaks_the_redetect_loop() {
        // 根因回归测试：收起态被往外拽后仍溢出右缘，detect_edge 会再次判定
        // 贴右 → 吸回去，用户永远拖不出来。pull_away_from 必须让它脱离判定。
        let m = MonitorLogical { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0 };
        let docked = r(1904.0, 100.0, 388.0, 500.0); // 收起态：只留 16px 在屏内
        assert_eq!(detect_edge(docked, m, 24.0), Some(Edge::Right));
        // 用户往外拖 260px（越过 DRAG_AWAY），仍然溢出右缘
        let dragged = r(1660.0, 100.0, 388.0, 500.0);
        assert_eq!(
            detect_edge(dragged, m, 24.0),
            Some(Edge::Right),
            "前提：溢出状态确实会被判成贴边（这就是 bug 的成因）"
        );
        // 拉回屏内后必须不再贴边
        let freed = pull_away_from(dragged, m, Edge::Right, 34.0);
        assert_eq!(detect_edge(freed, m, 24.0), None, "拉回后不该再被判成贴边");
        assert!(freed.x + freed.w <= 1920.0 - 24.0, "右缘必须留有余量");
        // 已经离边的不该被多推
        let already = r(1000.0, 100.0, 388.0, 500.0);
        assert_eq!(pull_away_from(already, m, Edge::Right, 34.0), already);
    }

    #[test]
    fn pull_away_handles_left_and_top() {
        let m = MonitorLogical { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0 };
        // 左缘：往外拖 = x 变负
        let l = pull_away_from(r(-200.0, 100.0, 388.0, 500.0), m, Edge::Left, 34.0);
        assert_eq!(l.x, 34.0);
        assert_eq!(detect_edge(l, m, 24.0), None);
        // 顶缘：往外拖 = y 变负
        let t = pull_away_from(r(300.0, -400.0, 388.0, 500.0), m, Edge::Top, 34.0);
        assert_eq!(t.y, 34.0);
        assert_eq!(detect_edge(t, m, 24.0), None);
    }

    #[test]
    fn works_on_secondary_monitor_with_offset() {
        // 副屏原点在 (1920,0)：贴其左缘 = x≈1920
        let mm = MonitorLogical { x: 1920.0, y: 0.0, w: 1280.0, h: 1024.0, scale: 1.0 };
        let rect = r(1922.0, 50.0, 310.0, 370.0);
        assert_eq!(detect_edge(rect, mm, 24.0), Some(Edge::Left));
        let d = docked_rect(rect, mm, Edge::Left, 24.0);
        // 停靠后只留 24px 在副屏内 → 右缘 = 1920+24
        assert_eq!(d.x + d.w, 1944.0);
    }
}
