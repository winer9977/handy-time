//! 到期提醒 + 定时归档
//!
//! 挂件常驻，提醒不能靠前端 setInterval（webview 休眠/最小化会漏）。
//! 用 Rust 侧线程轮询：每 20s 扫一遍 remind_at 命中当前分钟的待办，
//! 发 Windows Toast 并置 reminded=true（重启不重复弹）。
//! 跨天触发一次按完成日期归档 + 复位 reminded。

use crate::state::AppState;
use chrono::{Local, NaiveDateTime};
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

pub fn spawn_scheduler(app: AppHandle) {
    // 幂等：dev 热重启避免起多个线程
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    thread::spawn(move || {
        let mut last_day = Local::now().format("%Y-%m-%d").to_string();
        loop {
            let now = Local::now();
            let state = app.state::<AppState>();

            let store = state.snapshot();
            let cur_min = now.format("%Y-%m-%dT%H:%M").to_string();
            let fired = plan_fires(&store.todos, &cur_min);
            let new_day = now.format("%Y-%m-%d").to_string() != last_day;
            drop(store);

            for (id, title, act) in fired {
                notify(&app, &title);
                match act {
                    Act::Main => {
                        let _ = state.mark_reminded_full(&id, &cur_min);
                    }
                    Act::Extra(x) => {
                        let _ = state.mark_extra_reminded(&id, &x, &cur_min);
                    }
                    Act::Constant => {
                        let _ = state.mark_constant_reminded(&id, &cur_min);
                    }
                }
                let _ = app.emit("xhy://changed", "todos");
            }

            if new_day {
                last_day = now.format("%Y-%m-%d").to_string();
                let _ = state.run_archive();
                let _ = state.reset_reminded();
                let _ = app.emit("xhy://changed", "all");
            }

            thread::sleep(crate::TICK);
        }
    });
}

/// 一轮内要执行的提醒动作类别
#[derive(Debug, PartialEq, Eq)]
pub enum Act {
    /// 主提醒（remind_at）
    Main,
    /// 某个额外提醒时点，携带其时间串
    Extra(String),
    /// 持续提醒重弹
    Constant,
}

/// 规划这一轮该弹哪些提醒。**纯函数**：不读时钟、不写库，所以多提醒 /
/// 持续提醒 / 去重 / 节流这套组合行为能被完整单测覆盖（调度线程本身没法测）。
///
/// 优先级刻意互斥：同一轮里主提醒 > 额外提醒 > 持续重弹，只出最高优先级的
/// 一种。否则"到期又该重弹"会一次弹两条，用户看到的是重复通知。
pub fn plan_fires(todos: &[crate::domain::Todo], cur_min: &str) -> Vec<(String, String, Act)> {
    let mut out = Vec::new();
    for t in todos.iter().filter(|t| !t.done) {
        // ① 主提醒
        if !t.reminded && t.remind_at.as_deref().is_some_and(|r| remind_hit(r, cur_min)) {
            out.push((t.id.clone(), t.title.clone(), Act::Main));
            continue;
        }
        // ② 额外提醒时点（对照滴答"多个提醒时间"）。每个时点独立去重，
        //    同一轮里多个到期就各弹一条。
        let mut hit_extra = false;
        for x in &t.extra_reminds {
            if !t.reminded_keys.iter().any(|k| k == x) && remind_hit(x, cur_min) {
                out.push((t.id.clone(), t.title.clone(), Act::Extra(x.clone())));
                hit_extra = true;
            }
        }
        if hit_extra {
            continue;
        }
        // ③ 持续提醒（对照滴答"Constant Reminder"）：主提醒已弹过但仍未完成
        //    → 每隔 CONSTANT_EVERY_MIN 分钟重弹，直到完成。
        if t.constant_remind
            && t.reminded
            && t.remind_at.as_deref().is_some_and(|r| remind_hit(r, cur_min))
            && constant_due(&t.last_reminded_at, cur_min, CONSTANT_EVERY_MIN)
        {
            out.push((t.id.clone(), t.title.clone(), Act::Constant));
        }
    }
    out
}

/// 持续提醒重弹间隔（分钟）。轮询是 20s，不节流会把"持续提醒"变成"疯狂弹窗"。
const CONSTANT_EVERY_MIN: i64 = 2;

/// 持续提醒节流：距上次弹窗 >= every 分钟才再弹。
/// `last` 为空（从没弹过）视为该弹。
fn constant_due(last: &Option<String>, current_min: &str, every: i64) -> bool {
    let Some(l) = last.as_deref().and_then(parse_min) else {
        return true;
    };
    match parse_min(current_min) {
        Some(c) => (c - l).num_minutes() >= every,
        None => false,
    }
}

/// remind_at ≤ 当前分钟即触发（逾期也会提醒一次，靠 reminded 去重）
fn remind_hit(remind_at: &str, current_min: &str) -> bool {
    match (parse_min(remind_at), parse_min(current_min)) {
        (Some(r), Some(c)) => r <= c,
        _ => false,
    }
}

fn parse_min(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .ok()
        // 容忍纯日期：视为当天 09:00
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_hms_opt(9, 0, 0).unwrap())
        })
}

/// 发系统通知（正文固定带软件名，便于在通知中心识别）
fn notify(app: &AppHandle, title: &str) {
    use tauri_plugin_notification::NotificationExt;
    let body = format!("『{title}』到期了");
    let _ = app
        .notification()
        .builder()
        .title(format!("{} 待办提醒", crate::BRAND))
        .body(&body)
        .show();
    // 同时广播，让挂件 UI 里该行高亮闪烁
    let _ = app.emit("xhy://remind", title);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Todo;

    /// 只关心提醒相关字段，其余给默认
    fn todo(f: impl FnOnce(&mut Todo)) -> Todo {
        let mut t = Todo {
            id: "t1".into(),
            list_id: "l1".into(),
            title: "任务".into(),
            note: String::new(),
            done: false,
            done_at: None,
            pinned: false,
            order: 0,
            tags: vec![],
            due: None,
            remind_at: None,
            reminded: false,
            extra_reminds: vec![],
            reminded_keys: vec![],
            constant_remind: false,
            last_reminded_at: None,
            countdown: None,
            priority: None,
            recurrence: None,
            created_at: "2026-10-17T08:00".into(),
            archived_at: None,
        };
        f(&mut t);
        t
    }

    const NOW: &str = "2026-10-17T10:00";

    #[test]
    fn main_reminder_fires_once_then_deduped() {
        let due = todo(|t| t.remind_at = Some("2026-10-17T09:30".into()));
        let got = plan_fires(&[due], NOW);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, Act::Main);

        // 已弹过 → 不再弹（没开持续提醒时）
        let once = todo(|t| {
            t.remind_at = Some("2026-10-17T09:30".into());
            t.reminded = true;
        });
        assert!(plan_fires(&[once], NOW).is_empty(), "reminded 后不该重弹");
    }

    #[test]
    fn future_reminder_does_not_fire() {
        let later = todo(|t| t.remind_at = Some("2026-10-17T18:00".into()));
        assert!(plan_fires(&[later], NOW).is_empty(), "未到点不该弹");
    }

    #[test]
    fn done_todo_never_fires() {
        let d = todo(|t| {
            t.done = true;
            t.remind_at = Some("2026-10-17T09:30".into());
            t.constant_remind = true;
        });
        assert!(plan_fires(&[d], NOW).is_empty(), "已完成绝不该弹");
    }

    #[test]
    fn multiple_extra_reminders_fire_independently() {
        // 对照滴答"多个提醒时间"：三个时点都到期 → 各弹一条
        let t = todo(|x| {
            x.remind_at = Some("2026-10-17T09:00".into());
            x.reminded = true; // 主提醒已处理，让额外提醒成为主角
            x.extra_reminds = vec![
                "2026-10-17T09:15".into(),
                "2026-10-17T09:30".into(),
                "2026-10-17T18:00".into(), // 未到点
            ];
        });
        let got = plan_fires(&[t], NOW);
        assert_eq!(got.len(), 2, "两个已到点该各弹一次");
        assert!(got.iter().all(|(_, _, a)| matches!(a, Act::Extra(_))));
        // 携带的是正确的时间串
        let mut keys: Vec<String> = got
            .iter()
            .filter_map(|(_, _, a)| match a {
                Act::Extra(x) => Some(x.clone()),
                _ => None,
            })
            .collect();
        keys.sort();
        assert_eq!(keys, vec!["2026-10-17T09:15", "2026-10-17T09:30"]);
    }

    #[test]
    fn extra_reminder_deduped_per_key() {
        // 已记进 reminded_keys 的时点不再弹
        let t = todo(|x| {
            x.remind_at = Some("2026-10-17T09:00".into());
            x.reminded = true;
            x.extra_reminds = vec!["2026-10-17T09:30".into()];
            x.reminded_keys = vec!["2026-10-17T09:30".into()];
        });
        assert!(plan_fires(&[t], NOW).is_empty(), "该额外时点已弹过");
    }

    #[test]
    fn main_and_extra_are_mutex_per_round() {
        // 同一轮主提醒优先，不叠加额外 —— 否则用户一次收到两条重复通知
        let t = todo(|x| {
            x.remind_at = Some("2026-10-17T09:30".into());
            x.extra_reminds = vec!["2026-10-17T09:31".into()];
        });
        let got = plan_fires(&[t], NOW);
        assert_eq!(got.len(), 1, "同一轮只出主提醒");
        assert_eq!(got[0].2, Act::Main);
    }

    #[test]
    fn constant_reminder_refires_after_interval() {
        let on = |t: &mut Todo| {
            t.remind_at = Some("2026-10-17T09:30".into());
            t.reminded = true;
            t.constant_remind = true;
        };
        // 从没弹过（last 为空）→ 该弹
        let a = todo(on);
        let got = plan_fires(&[a], NOW);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, Act::Constant);

        // 刚弹过（1 分钟前 < 2 分钟阈值）→ 被节流
        let b = todo(|t| {
            on(t);
            t.last_reminded_at = Some("2026-10-17T09:59".into());
        });
        assert!(plan_fires(&[b], NOW).is_empty(), "未到重弹间隔该被节流");

        // 3 分钟前 ≥ 阈值 → 再弹
        let c = todo(|t| {
            on(t);
            t.last_reminded_at = Some("2026-10-17T09:57".into());
        });
        assert_eq!(plan_fires(&[c], NOW)[0].2, Act::Constant, "超间隔应重弹");
    }

    #[test]
    fn constant_needs_main_already_fired() {
        // 主提醒还没弹过时，持续提醒不该抢跑（否则会跳过正常节奏）
        let t = todo(|x| {
            x.remind_at = Some("2026-10-17T09:30".into());
            x.reminded = false;
            x.constant_remind = true;
        });
        let got = plan_fires(&[t], NOW);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, Act::Main, "首轮应记为主提醒");
    }

    #[test]
    fn constant_without_remind_at_never_fires() {
        // 没有 remind_at 就无从判断"逾期"，持续提醒不该凭空弹
        let t = todo(|x| {
            x.constant_remind = true;
            x.reminded = true;
        });
        assert!(plan_fires(&[t], NOW).is_empty());
    }

    #[test]
    fn constant_due_handles_garbage_and_bounds() {
        assert!(constant_due(&None, NOW, 2), "从没弹过 → 该弹");
        // 坏时间戳与"从没弹过"同义（都不知道上次何时弹），故同样放行：
        // 弹完会写入合法时间戳，自愈。给它另一套行为才是武断的。
        assert!(
            constant_due(&Some("乱码".into()), NOW, 2),
            "坏时间戳不该 panic，按未弹过处理"
        );
        assert!(constant_due(&Some("2026-10-17".into()), NOW, 2), "纯日期视为 09:00，已过 2 分钟");
        // 未来时刻（时钟回拨/手工改库）→ 不弹，避免负间隔疯弹
        assert!(!constant_due(&Some("2026-10-17T11:00".into()), NOW, 2));
    }

    #[test]
    fn pure_date_reminder_treated_as_nine_am() {
        // 只存日期（无 HH:MM）时视为 09:00：10:00 已过点 → 该弹
        let mk = || todo(|x| x.remind_at = Some("2026-10-17".into()));
        assert_eq!(plan_fires(&[mk()], NOW)[0].2, Act::Main);
        // 08:00 未到 09:00 → 不弹
        assert!(plan_fires(&[mk()], "2026-10-17T08:00").is_empty());
    }
}
