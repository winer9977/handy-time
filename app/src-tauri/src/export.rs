//! Markdown 导出（对照 Kite 待办的"导出 Markdown"）
//!
//! 与 JSON 导出的分工：JSON 是完整备份（可导入还原），Markdown 是给人读/
//! 贴到笔记软件里的。刻意**不**追求 round-trip —— 那该用 JSON。
//!
//! 结构：按清单分节；已完成项用 `- [x]` 并带完成日期；日程按日期排序成
//! 独立一节；便签各自成段。纯函数，单元测试直接断言输出。

use crate::domain::recurring::{fmt_date, parse_date, rc_label};
use crate::domain::{Event, Store};

/// 定时日程写成 `09:30–10:30`；不定长只写开始时刻；全天返回 None（调用方写"全天"）
fn event_time(e: &Event) -> Option<String> {
    let start = e.time.as_deref()?;
    // 时刻串解析不出来（脏数据/手改库）就**原样输出**，绝不退化成"全天" ——
    // 明明有时刻却写全天，是比少个结束时刻更糟的谎。
    let (hh, mm) = match start.split_once(':') {
        Some(v) => v,
        None => return Some(start.to_string()),
    };
    let (Ok(h), Ok(m)) = (hh.parse::<u32>(), mm.parse::<u32>()) else {
        return Some(start.to_string());
    };
    if e.duration_min == 0 {
        return Some(start.to_string());
    }
    let total = h * 60 + m + e.duration_min;
    // 跨零点标 +1：`23:40–00:40+1`，与界面上的写法一致
    let cross = if total >= 1440 { "+1" } else { "" };
    Some(format!("{start}–{:02}:{:02}{cross}", (total / 60) % 24, total % 60))
}

/// 跨天日程的最后一天 = 起始日 + N - 1
fn span_end(date: &str, days: u16) -> Option<String> {
    let d = parse_date(date)? + chrono::Duration::days(days.max(1) as i64 - 1);
    Some(fmt_date(d))
}

/// 导出为 Markdown 文本
pub fn to_markdown(s: &Store) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} 导出数据\n\n> 导出于 {}\n\n", crate::BRAND, crate::now_local_str()));

    // ── 待办：按清单分节 ──
    let mut lists: Vec<_> = s.lists.iter().collect();
    lists.sort_by_key(|l| l.order);
    for l in lists {
        let mut todos: Vec<_> = s
            .todos
            .iter()
            .filter(|t| t.list_id == l.id)
            .collect();
        // 未完成在前，其后按到期日、手动顺序
        todos.sort_by_key(|t| (t.done, t.due.clone().unwrap_or_else(|| "9999".into()), t.order));
        if todos.is_empty() {
            continue;
        }
        out.push_str(&format!("## 📋 {name}\n\n", name = l.name));
        for t in todos {
            let box_mark = if t.done { "x" } else { " " };
            out.push_str(&format!("- [{box_mark}] {}", t.title));
            if !t.tags.is_empty() {
                for tag in &t.tags {
                    out.push_str(&format!(" `{tag}`"));
                }
            }
            if let Some(d) = &t.due {
                out.push_str(&format!(" · 📅 {d}"));
            }
            if let Some(r) = &t.remind_at {
                out.push_str(&format!(" · 🔔 {r}"));
            }
            if let Some(rc) = &t.recurrence {
                out.push_str(&format!(" · ↻{}", rc_label(rc)));
            }
            if t.done {
                if let Some(d) = &t.done_at {
                    out.push_str(&format!(" · ✅ {d}"));
                }
            }
            out.push('\n');
            if !t.note.is_empty() {
                out.push_str(&format!("  > {}\n", t.note.replace('\n', "\n  > ")));
            }
        }
        out.push('\n');
    }

    // ── 日程：按日期排序 ──
    if !s.events.is_empty() {
        out.push_str("## 🗓️ 日程\n\n");
        let mut evs: Vec<_> = s.events.iter().collect();
        evs.sort_by_key(|e| (e.date.clone(), e.time.clone().unwrap_or_default()));
        for e in evs {
            let time = match event_time(e) {
                Some(t) => t,
                None => "全天".into(),
            };
            out.push_str(&format!("- **{date}** {time} · {title}", date = e.date, title = e.title));
            // 跨天日程：只写起始日会让人以为一天就完了
            if e.span_days > 1 {
                match span_end(&e.date, e.span_days) {
                    Some(d) => out.push_str(&format!(" · 📅 {} → {d}（{} 天）", e.date, e.span_days)),
                    None => out.push_str(&format!(" · 📅 连占 {} 天", e.span_days)),
                }
            }
            if let Some(rc) = &e.recurrence {
                out.push_str(&format!(" · ↻{}", rc_label(rc)));
            }
            if e.source == "todo" {
                out.push_str(" · 来自待办");
            }
            out.push('\n');
        }
        out.push('\n');
    }

    // ── 便签 ──
    if !s.notes.is_empty() {
        out.push_str("## 📝 便签\n\n");
        let mut notes: Vec<_> = s.notes.iter().collect();
        // 排序必须与界面上看到的一致（前端 store.ts 的 notes getter）：
        // **置顶在前**，其后按手动 order 升序。
        //
        // 旧代码写成 `(n.pinned, Reverse(n.order))`：bool 升序是 false 在前，
        // 于是导出里置顶的反而排最后，连 order 方向也是反的 —— 导出的顺序
        // 与用户界面完全颠倒（实测发现，且此前测试完全没覆盖便签顺序）。
        notes.sort_by_key(|n| (!n.pinned, n.order));
        for (i, n) in notes.iter().enumerate() {
            out.push_str(&format!("### 便签 {}\n\n", i + 1));
            if !n.tags.is_empty() {
                for tag in &n.tags {
                    out.push_str(&format!("`{tag}` "));
                }
                out.push('\n');
            }
            out.push_str(n.body.trim_end());
            out.push_str("\n\n");
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(time: Option<&str>, duration_min: u32, span_days: u16) -> Event {
        Event {
            id: "e1".into(),
            date: "2026-05-01".into(),
            time: time.map(|s| s.to_string()),
            duration_min,
            title: "评审".into(),
            color: "#fff".into(),
            source: "manual".into(),
            todo_id: None,
            span_days,
            span_index: 0,
            span_total: span_days,
            recurrence: None,
            list_id: None,
        }
    }

    fn one(e: Event) -> String {
        let mut s = Store::default();
        s.events.push(e);
        to_markdown(&s)
    }

    #[test]
    fn duration_and_span_show_up_in_markdown() {
        let md = one(ev(Some("09:30"), 90, 1));
        assert!(md.contains("09:30–11:00"), "结束时刻要跟着时长: {md}");
        let md2 = one(ev(Some("23:40"), 60, 1));
        assert!(md2.contains("23:40–00:40+1"), "跨零点标 +1: {md2}");
        let md3 = one(ev(None, 0, 3));
        assert!(md3.contains("全天"), "{md3}");
        assert!(md3.contains("2026-05-01 → 2026-05-03（3 天）"), "跨天写出头尾: {md3}");
        // 不定长 + 不跨天：保持原来的简洁一行
        let md4 = one(ev(Some("09:30"), 0, 1));
        assert!(md4.contains("09:30 · 评审"), "{md4}");
        assert!(!md4.contains("→"), "不跨天不该有多余箭头: {md4}");
    }

    #[test]
    fn bad_time_string_does_not_panic() {
        // 库里可能是手改过的脏数据：解析不出来就退回只显示开始时刻，绝不 panic
        let md = one(ev(Some("乱码"), 30, 1));
        assert!(md.contains("乱码"), "{md}");
    }
}
