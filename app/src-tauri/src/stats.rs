//! 周/月总结的本地统计（对照滴答清单的"回顾"）
//!
//! 刻意与 AI 解耦：**统计数字必须永远算得出、且可单测**，AI 只负责把这些
//! 数字写成一段人话。没配 key、断网、模型抽风，都不该影响这一层。
//!
//! 统计口径（一个周期 = [from, to] 闭区间，按墙上日期）：
//!   · 完成   —— done_at 落在区间内的待办（含已归档的，否则月总结看不到
//!              被归档走的历史完成项）
//!   · 新建   —— created_at 落在区间内
//!   · 遗留   —— 到期日在区间内但至今未完成（该做没做的，最值得看）
//!   · 完成率 —— 完成 / (完成 + 遗留)：分母是"这段时间本该完成的事"
//!   · 分布   —— 按清单、按标签、按天（哪天最 productive）
//!
//! 便签**不**计入：它没有"完成"语义，且缺 created_at 字段（只有 updated_at，
//! 会把"编辑过老便签"错算成"新建"）。为边缘指标动 schema 不值当。

use crate::domain::{Store, Todo};
use chrono::{Duration, NaiveDate};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodStats {
    pub from: String,
    pub to: String,
    pub done: usize,
    pub created: usize,
    /// 到期在区间内、至今仍未完成
    pub overdue_left: usize,
    /// 完成 / (完成 + 遗留)
    pub completion_rate: f64,
    pub event_count: usize,
    /// 遗留项标题（最多 12 条，给人看"欠了什么"）
    pub left_titles: Vec<String>,
    /// (清单名, 完成数) 降序
    pub by_list: Vec<(String, usize)>,
    /// (标签, 完成数) 降序，取前 8
    pub by_tag: Vec<(String, usize)>,
    /// (优先级, 完成数) 固定序：高→中→低。滴答分析页的"优先级分布"
    #[serde(default)]
    pub by_priority: Vec<(String, usize)>,
    /// (日期, 完成数) 按日期升序，画小柱状图用
    pub per_day: Vec<(String, usize)>,
    /// 完成最多的一天（什么都没完成时 None）
    pub best_day: Option<(String, usize)>,
    /// 环比上一周期（None = 上一周期无数据可比）
    pub prev: Option<Box<PeriodStats>>,
}

impl PeriodStats {
    /// 完成率的环比变化（百分点），正=比上期好
    pub fn rate_delta(&self) -> Option<f64> {
        self.prev.as_ref().map(|p| self.completion_rate - p.completion_rate)
    }
}

fn d10_str(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

fn d10(s: &Option<String>) -> Option<NaiveDate> {
    d10_str(s.as_deref()?)
}

/// 纯函数统计：所有输入显式传入，不读时钟、不读盘，便于单测。
///
/// `archived` = 归档分片（archive/*.jsonl）里的待办。归档只有这一处来源
/// （`store.archived` 索引是从不写入的死字段，已删）。
pub fn compute(store: &Store, archived: &[Todo], from: NaiveDate, to: NaiveDate) -> PeriodStats {
    compute_list(store, archived, from, to, None)
}

/// 带清单过滤的统计。`list` = Some 时只算该清单：
///   · 待办按 `list_id` 归属；
///   · 日程里手工创建的（无 todo_id）**不属于任何清单，恒计入** —— 与
///     `state::calendar_entries` 的规则逐条对齐，否则用户在日历看到 3 条、
///     总结写 5 条，互相矛盾比数字不准更糟。
///   · 拖上日历生成的（带 todo_id）按关联待办的清单归属。
pub fn compute_list(
    store: &Store,
    archived: &[Todo],
    from: NaiveDate,
    to: NaiveDate,
    list: Option<&str>,
) -> PeriodStats {
    compute_inner(store, archived, from, to, true, list)
}

fn compute_inner(
    store: &Store,
    archived: &[Todo],
    from: NaiveDate,
    to: NaiveDate,
    with_prev: bool,
    list: Option<&str>,
) -> PeriodStats {
    let in_range = |d: Option<NaiveDate>| matches!(d, Some(d) if d >= from && d <= to);
    // 待办只有两处来源：主表（活动）+ 归档分片（archive/*.jsonl）。
    // 曾还有第三个来源 `store.archived` 索引，但**它从不被写入**
    // （run_archive 只写分片），是只读死字段 —— 为它写的读取路径与测试
    // 永远不会为真，还会误导后人以为归档数据有两处来源。已删字段。
    let all_todos = || store.todos.iter().chain(archived.iter());
    // 被标记"不参与统计"的清单：它们的待办与日程一律跳过。
    // 与上面的 `list` 参数（用户临时只看某一个清单）是两个正交维度：
    //   · list  = 收窄到某一个（临时视图选择）
    //   · excluded = 永久排除若干（清单自身的属性）
    // 两者叠加时取交集 —— 即"看这个清单，但它若被排除就什么都不显示"，
    // 语义一致、不冲突。
    let excluded: std::collections::HashSet<&str> = store
        .lists
        .iter()
        .filter(|l| l.exclude_from_stats)
        .map(|l| l.id.as_str())
        .collect();
    let in_list = |t: &Todo| {
        if excluded.contains(t.list_id.as_str()) {
            return false;
        }
        match list {
            None => true,
            Some(l) => t.list_id == l,
        }
    };
    // 日程归属：优先看日程自己记的 list_id（母记录），回落查待办；
    // 归属不明（手工日程）恒计入。判据与 state::event_list 一致，
    // 否则日历显示 3 条、总结写 5 条，互相矛盾比数字不准更糟。
    //
    // 注意"归属不明"的判定要先排除被排除清单的待办：若某日程的关联待办
    // 在一个被排除的清单里，这条日程也应当被排除（否则用户会看到
    // "我排除了那个清单，总结里还有它的日程"）。
    let event_in_list = |e: &crate::domain::Event| {
        let owner = e.list_id.as_deref().map(|s| s.to_string()).or_else(|| {
            e.todo_id
                .as_deref()
                .and_then(|tid| store.todos.iter().find(|t| t.id == tid))
                .map(|t| t.list_id.clone())
        });
        if let Some(oid) = owner.as_deref() {
            if excluded.contains(oid) {
                return false;
            }
        }
        match list {
            None => true,
            Some(l) => match owner.as_deref() {
                None => true, // 既没记清单也查不到待办：归属不明，恒计入
                Some(oid) => oid == l,
            },
        }
    };

    // 已完成：主表（活动） + 归档分片
    let done_todos: Vec<&Todo> = all_todos()
        .filter(|t| t.done && in_range(d10(&t.done_at)))
        .filter(|t| in_list(t))
        .collect();

    let created = all_todos()
        .filter(|t| in_range(d10_str(&t.created_at)))
        .filter(|t| in_list(t))
        .count();

    // 遗留：到期在区间内，但至今没完成（只看主表 —— 归档的都是已完成的）
    let left: Vec<&Todo> = store
        .todos
        .iter()
        .filter(|t| !t.done && in_range(d10(&t.due)))
        .filter(|t| in_list(t))
        .collect();

    let mut left_titles: Vec<String> = left.iter().map(|t| t.title.clone()).collect();
    left_titles.sort();
    left_titles.truncate(12);

    let name_of = |id: &str| -> String {
        store
            .lists
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "（已删清单）".into())
    };
    let mut list_map: std::collections::BTreeMap<String, usize> = Default::default();
    let mut tag_map: std::collections::BTreeMap<String, usize> = Default::default();
    let mut pr_map: std::collections::BTreeMap<String, usize> = Default::default();
    for t in &done_todos {
        *list_map.entry(name_of(&t.list_id)).or_default() += 1;
        for tag in &t.tags {
            *tag_map.entry(tag.clone()).or_default() += 1;
        }
        if let Some(p) = &t.priority {
            // 用 serde 名（high/mid/low），**不是** Priority::as_str()（那是
            // "高/中/低"中文展示串，导出用的）。前端 AnalysisPane 的
            // PR_LABEL/PR_COLOR 按 serde 名索引，用错就静默查不到色与文案。
            // 走 to_value 让键名与类型定义同源，杜绝又一处手工映射漂移。
            if let Some(k) = serde_json::to_value(p).ok().and_then(|v| v.as_str().map(String::from)) {
                *pr_map.entry(k).or_default() += 1;
            }
        }
    }
    let mut by_list: Vec<(String, usize)> = list_map.into_iter().collect();
    by_list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut by_tag: Vec<(String, usize)> = tag_map.into_iter().collect();
    by_tag.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    by_tag.truncate(8);
    // 优先级固定序（高→中→低），没出现的档位不占位
    let by_priority: Vec<(String, usize)> = ["high", "mid", "low"]
        .iter()
        .filter_map(|k| pr_map.get(*k).map(|n| ((*k).to_string(), *n)))
        .collect();

    // 按天：区间内每天都占一格（空闲日显示 0，否则"三天打鱼"看不出来）
    let mut per_day: Vec<(String, usize)> = Vec::new();
    let mut best: Option<(String, usize)> = None;
    let mut d = from;
    while d <= to {
        let key = d.format("%Y-%m-%d").to_string();
        let n = done_todos
            .iter()
            .filter(|t| d10_str(t.done_at.as_deref().unwrap_or("")) == Some(d))
            .count();
        if best.as_ref().map(|(_, bn)| n > *bn).unwrap_or(n > 0) {
            best = Some((key.clone(), n));
        }
        per_day.push((key, n));
        d += Duration::days(1);
    }

    let done = done_todos.len();
    let denom = done + left.len();
    let event_count = store
        .events
        .iter()
        .filter(|e| in_range(d10_str(&e.date)))
        .filter(|e| event_in_list(e))
        .count();

    // 环比：上一同长度周期。递归一层即止（with_prev=false），且沿用同一清单过滤
    let prev = if with_prev {
        let (pf, pl) = prev_period(from, to);
        let p = compute_inner(store, archived, pf, pl, false, list);
        // 上一周期啥都没有就别显示"环比 0%"，会误导
        if p.done + p.overdue_left + p.created + p.event_count == 0 {
            None
        } else {
            Some(Box::new(p))
        }
    } else {
        None
    };

    PeriodStats {
        from: from.format("%Y-%m-%d").to_string(),
        to: to.format("%Y-%m-%d").to_string(),
        done,
        created,
        overdue_left: left.len(),
        completion_rate: if denom == 0 {
            0.0
        } else {
            done as f64 / denom as f64
        },
        event_count,
        left_titles,
        by_list,
        by_tag,
        by_priority,
        per_day,
        best_day: best.filter(|(_, n)| *n > 0),
        prev,
    }
}

/// 近 n 天（含今天）的完成趋势：直接复用 compute 的 per_day ——
/// 口径（什么算完成、归档怎么算）只有一份，分析页与总结页永不打架。
///
/// `list` 必须与同页的分布统计传同一个值。漏了会出现同一屏里
/// 趋势柱显示"全局 2 项"、下方分布显示"本清单 0 项"的自相矛盾
/// （实机截图抓到过）。
pub fn trend(store: &Store, archived: &[Todo], today: NaiveDate, n: i64, list: Option<&str>) -> Vec<(String, usize)> {
    let n = n.clamp(7, 366);
    let from = today - Duration::days(n - 1);
    compute_list(store, archived, from, today, list).per_day
}

/// 本周一 / 本周日（墙上日期，周一为一周之始）
pub fn week_bounds(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    use chrono::Datelike;
    let offset = (today.weekday().number_from_monday() as i64) - 1;
    let mon = today - Duration::days(offset);
    (mon, mon + Duration::days(6))
}

/// 本月首日 / 末日
pub fn month_bounds(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    use chrono::Datelike;
    let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let last_day = match today.month() {
        12 => NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap_or(today),
        m => NaiveDate::from_ymd_opt(today.year(), m + 1, 1)
            .map(|x| x - Duration::days(1))
            .unwrap_or(today),
    };
    (first, last_day)
}

/// 上一个同长度周期（做"比上周/比上月"环比）
pub fn prev_period(from: NaiveDate, to: NaiveDate) -> (NaiveDate, NaiveDate) {
    let len = (to - from).num_days() + 1;
    (from - Duration::days(len), from - Duration::days(1))
}

/// 把统计摘要成紧凑文本：既是给模型看的提示词素材，也是**没配 AI 时的
/// 降级展示** —— 所以它自己就得是一份能读的总结。
pub fn digest(s: &PeriodStats, label: &str) -> String {
    let pct = (s.completion_rate * 100.0).round() as i32;
    let mut out = format!(
        "{label}（{} ~ {}）\n完成 {} 项，新建 {} 项，遗留未完成 {} 项，完成率 {pct}%\n日程 {} 条\n",
        s.from, s.to, s.done, s.created, s.overdue_left, s.event_count
    );
    if let Some(pp) = &s.prev {
        let ppct = (pp.completion_rate * 100.0).round() as i32;
        let delta = s.completion_rate - pp.completion_rate;
        let dp = (delta * 100.0).round() as i32;
        let arrow = if dp > 0 {
            format!("↑{dp} 个百分点")
        } else if dp < 0 {
            format!("↓{} 个百分点", -dp)
        } else {
            "持平".into()
        };
        out.push_str(&format!(
            "上一周期完成 {} 项、完成率 {ppct}%（本期{arrow}）\n",
            pp.done
        ));
    }
    if !s.by_list.is_empty() {
        let parts: Vec<String> = s.by_list.iter().map(|(k, v)| format!("{k} {v}")).collect();
        out.push_str(&format!("按清单：{}\n", parts.join("、")));
    }
    if !s.by_tag.is_empty() {
        let parts: Vec<String> = s.by_tag.iter().map(|(k, v)| format!("#{k} {v}")).collect();
        out.push_str(&format!("按标签：{}\n", parts.join("、")));
    }
    if let Some((d, n)) = &s.best_day {
        out.push_str(&format!("产出最高的一天：{d}（{n} 项）\n"));
    }
    if !s.left_titles.is_empty() {
        out.push_str(&format!("遗留：{}\n", s.left_titles.join("、")));
    }
    out
}

/// 一次总结的完整返回：数字 + 可直接展示的摘要文本。
/// 合成一个结构是为了少一次 IPC：前端要同时渲染统计卡和文字摘要。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub kind: String,
    pub label: String,
    pub stats: PeriodStats,
    /// 本地摘要（无 AI 时这就是最终展示文本）
    pub digest: String,
}

/// 按 kind（"week" / "month"）算出周期并生成摘要。
/// `list` = Some 时只统计该清单（主窗口选中清单时总结要跟着变）。
pub fn summary(store: &Store, archived: &[Todo], today: NaiveDate, kind: &str) -> Summary {
    summary_list(store, archived, today, kind, None)
}

pub fn summary_list(
    store: &Store,
    archived: &[Todo],
    today: NaiveDate,
    kind: &str,
    list: Option<&str>,
) -> Summary {
    let (from, to) = match kind {
        "month" => month_bounds(today),
        _ => week_bounds(today),
    };
    let stats = compute_list(store, archived, from, to, list);
    let label = match kind {
        "month" => "本月",
        _ => "本周",
    };
    Summary {
        kind: kind.to_string(),
        label: label.to_string(),
        digest: digest(&stats, label),
        stats,
    }
}

/// AppState 侧的入口：读归档 + 注入今天，返回统计。
pub fn period_for(
    store: &Store,
    archived: &[Todo],
    today: NaiveDate,
    kind: &str,
) -> PeriodStats {
    let (from, to) = match kind {
        "month" => month_bounds(today),
        _ => week_bounds(today),
    };
    compute(store, archived, from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn todo(title: &str, due: Option<&str>, done: bool, done_at: Option<&str>) -> Todo {
        Todo {
            id: title.into(),
            list_id: "l1".into(),
            title: title.into(),
            note: String::new(),
            done,
            done_at: done_at.map(|s| s.into()),
            pinned: false,
            order: 0,
            tags: vec![],
            due: due.map(|s| s.into()),
            remind_at: None,
            reminded: false,
            extra_reminds: vec![],
            reminded_keys: vec![],
            constant_remind: false,
            last_reminded_at: None,
            countdown: None,
            priority: None,
            recurrence: None,
            created_at: "2026-01-01T10:00".into(),
            archived_at: None,
        }
    }

    fn store_with(todos: Vec<Todo>) -> Store {
        Store {
            version: 1,
            lists: vec![TodoList {
                id: "l1".into(),
                name: "工作".into(),
                color: "#fff".into(),
                order: 0,
                collapsed: false,
                archived: false,
                exclude_from_stats: false,
            }],
            todos,
            ..Default::default()
        }
    }

    #[test]
    fn counts_done_and_left_in_window() {
        let st = store_with(vec![
            todo("本周完成", Some("2026-03-10"), true, Some("2026-03-10T15:00")),
            todo("上周完成", Some("2026-03-02"), true, Some("2026-03-02T09:00")),
            todo("遗留", Some("2026-03-11"), false, None),
            todo("未来的事", Some("2026-04-01"), false, None),
        ]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, 1, "只有 done_at 落在区间的算本期完成");
        assert_eq!(s.overdue_left, 1, "到期在区间内且未完成 = 遗留");
        assert!((s.completion_rate - 0.5).abs() < 1e-9, "完成率 = 1/(1+1)");
        assert_eq!(s.left_titles, vec!["遗留".to_string()]);
    }

    #[test]
    fn archived_todos_count_toward_period() {
        // 归档里的项必须计入 —— 否则月总结看不到被归档走的历史完成项
        let st = store_with(vec![]);
        let arch = vec![todo("归档的旧事", None, true, Some("2026-03-12T10:00"))];
        let s = compute(&st, &arch, d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, 1, "已归档的完成项也要统计到");
    }

    #[test]
    fn per_day_covers_every_day_and_finds_best() {
        let st = store_with(vec![
            todo("a", None, true, Some("2026-03-10T08:00")),
            todo("b", None, true, Some("2026-03-10T09:00")),
            todo("c", None, true, Some("2026-03-12T09:00")),
        ]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.per_day.len(), 7, "周区间应覆盖 7 天");
        assert_eq!(s.per_day[0], ("2026-03-09".into(), 0), "空闲日该显示 0");
        assert_eq!(s.per_day[1], ("2026-03-10".into(), 2));
        assert_eq!(s.best_day, Some(("2026-03-10".into(), 2)));
        assert_eq!(s.done, 3);
    }

    #[test]
    fn empty_period_has_zero_rate_not_nan() {
        let st = store_with(vec![]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, 0);
        assert_eq!(s.completion_rate, 0.0, "分母为 0 时该给 0 而不是 NaN");
        assert!(s.completion_rate.is_finite());
        assert_eq!(s.best_day, None, "什么都没完成时不该有最佳日");
        assert!(s.prev.is_none(), "上一周期无数据时不该给出环比");
    }

    #[test]
    fn prev_period_comparison_is_reported() {
        let st = store_with(vec![
            todo("上期完成", None, true, Some("2026-03-03T10:00")),
            todo("本期完成", None, true, Some("2026-03-10T10:00")),
            todo("本期遗留", Some("2026-03-11"), false, None),
        ]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        let pp = s.prev.as_ref().expect("上期有数据，应有环比");
        assert_eq!(pp.done, 1);
        assert_eq!(pp.overdue_left, 0);
        // 本期 1/(1+1)=50%，上期 1/1=100% → 下降 50 个百分点
        assert!((s.rate_delta().unwrap() + 0.5).abs() < 1e-9);
        assert_eq!((pp.from.as_str(), pp.to.as_str()), ("2026-03-02", "2026-03-08"));
    }

    #[test]
    fn prev_does_not_recurse_forever() {
        // 环比里再套环比会指数爆炸；断言只有一层。
        // 两期都要有数据，否则按设计 prev=None（空期不给环比）
        let st = store_with(vec![
            todo("本期", None, true, Some("2026-03-10T10:00")),
            todo("上期", None, true, Some("2026-03-03T10:00")),
        ]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        let pp = s.prev.as_ref().expect("两期都有数据，应有环比");
        assert!(pp.prev.is_none(), "环比只算一层");
    }

    #[test]
    fn week_bounds_start_on_monday() {
        // 2026-03-11 是周三 → 周一 03-09、周日 03-15
        assert_eq!(week_bounds(d(2026, 3, 11)), (d(2026, 3, 9), d(2026, 3, 15)));
        assert_eq!(week_bounds(d(2026, 3, 9)).0, d(2026, 3, 9), "周一不该减出上一周");
        assert_eq!(week_bounds(d(2026, 3, 15)), (d(2026, 3, 9), d(2026, 3, 15)), "周日");
    }

    #[test]
    fn month_bounds_handles_feb_and_december() {
        assert_eq!(month_bounds(d(2026, 3, 20)), (d(2026, 3, 1), d(2026, 3, 31)));
        assert_eq!(month_bounds(d(2024, 2, 10)), (d(2024, 2, 1), d(2024, 2, 29)), "闰年");
        assert_eq!(month_bounds(d(2026, 2, 10)), (d(2026, 2, 1), d(2026, 2, 28)), "平年");
        assert_eq!(month_bounds(d(2026, 12, 5)), (d(2026, 12, 1), d(2026, 12, 31)), "12 月不溢出");
    }

    #[test]
    fn prev_period_is_same_length_immediately_before() {
        assert_eq!(prev_period(d(2026, 3, 9), d(2026, 3, 15)), (d(2026, 3, 2), d(2026, 3, 8)));
        let (f, l) = prev_period(d(2026, 3, 1), d(2026, 3, 31));
        assert_eq!((f, l), (d(2026, 1, 29), d(2026, 2, 28)));
        assert_eq!((l - f).num_days() + 1, 31, "长度必须一致，否则环比没意义");
    }

    #[test]
    fn digest_contains_the_numbers() {
        let st = store_with(vec![todo("写周报", Some("2026-03-10"), true, Some("2026-03-10T18:00"))]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        let txt = digest(&s, "本周");
        assert!(txt.contains("本周"), "{txt}");
        assert!(txt.contains("完成 1 项"), "{txt}");
        assert!(txt.contains("工作 1"), "清单分布应出现: {txt}");
        assert!(txt.contains("100%"), "完成率: {txt}");
    }

    #[test]
    fn digest_shows_period_change_direction() {
        let st = store_with(vec![
            todo("上期", None, true, Some("2026-03-03T10:00")),
            todo("本期", Some("2026-03-11"), false, None),
        ]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        let txt = digest(&s, "本周");
        assert!(txt.contains("上一周期"), "应含环比: {txt}");
        assert!(txt.contains("↓"), "本期更差应有下降箭头: {txt}");
    }

    #[test]
    fn tags_and_events_are_counted() {
        let mut t = todo("带标签", None, true, Some("2026-03-10T10:00"));
        t.tags = vec!["重要".into()];
        let mut st = store_with(vec![t]);
        st.events.push(Event {
            id: "e1".into(),
            date: "2026-03-11".into(),
            time: None,
            duration_min: 0,
            title: "评审".into(),
            color: "#fff".into(),
            source: "manual".into(),
            todo_id: None,
            span_days: 1,
            span_index: 0,
            span_total: 1,
            recurrence: None,
            list_id: None,
        });
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.by_tag, vec![("重要".into(), 1)]);
        assert_eq!(s.event_count, 1, "区间内的日程计数");
    }

    #[test]
    fn deleted_list_falls_back_instead_of_panicking() {
        // 清单删了但待办还在归档里：不该 panic，也不该丢统计
        let mut t = todo("孤儿", None, true, Some("2026-03-10T10:00"));
        t.list_id = "gone".into();
        let st = store_with(vec![t]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, 1);
        assert_eq!(s.by_list, vec![("（已删清单）".into(), 1)]);
    }

    #[test]
    fn created_at_within_window_is_counted() {
        let mut t = todo("新建的", None, false, None);
        t.created_at = "2026-03-10T10:00".into();
        let st = store_with(vec![t]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.created, 1);
    }

    #[test]
    fn malformed_dates_are_ignored_not_fatal() {
        // 归档是 append-only 日志，坏数据不该让整份总结崩掉
        let mut t = todo("坏日期", Some("not-a-date"), true, Some("???"));
        t.created_at = String::new();
        let st = store_with(vec![t]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, 0, "坏 done_at 不计入");
        assert_eq!(s.created, 0, "坏 created_at 不计入");
        assert_eq!(s.overdue_left, 0, "坏 due 不计入");
    }

    #[test]
    fn period_for_picks_week_or_month() {
        let st = store_with(vec![]);
        let w = period_for(&st, &[], d(2026, 3, 11), "week");
        assert_eq!((w.from.as_str(), w.to.as_str()), ("2026-03-09", "2026-03-15"));
        let m = period_for(&st, &[], d(2026, 3, 11), "month");
        assert_eq!((m.from.as_str(), m.to.as_str()), ("2026-03-01", "2026-03-31"));
        // 未知 kind 退回周（前端只会传这两个，防御性）
        assert_eq!(period_for(&st, &[], d(2026, 3, 11), "?").from, "2026-03-09");
    }

    #[test]
    fn by_priority_is_fixed_order_and_skips_unset() {
        // 分析页的优先级分布：固定"高→中→低"序（不是按数量排，那会让
        // 柱状图每次刷新都换顺序）；没设优先级的完成项不占位。
        let mut a = todo("高优先", None, true, Some("2026-03-10T10:00"));
        a.priority = Some(Priority::High);
        let mut b = todo("低优先1", None, true, Some("2026-03-10T11:00"));
        b.priority = Some(Priority::Low);
        let mut c = todo("低优先2", None, true, Some("2026-03-11T11:00"));
        c.priority = Some(Priority::Low);
        let mut m = todo("中优先", None, true, Some("2026-03-12T11:00"));
        m.priority = Some(Priority::Mid);
        let st = store_with(vec![a, b, c, m, todo("没设优先级", None, true, Some("2026-03-12T12:00"))]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(
            s.by_priority,
            vec![("high".into(), 1), ("mid".into(), 1), ("low".into(), 2)],
            "须按高→中→低固定序，且不含未设优先级的"
        );
    }

    #[test]
    fn by_priority_empty_when_nothing_flagged() {
        let st = store_with(vec![todo("裸任务", None, true, Some("2026-03-10T10:00"))]);
        let s = compute(&st, &[], d(2026, 3, 9), d(2026, 3, 15));
        assert!(s.by_priority.is_empty(), "全无优先级时不该造出零值占位");
    }

    #[test]
    fn trend_covers_every_day_in_window() {
        // 趋势必须逐日占格（空闲日为 0），否则"三天打鱼"看不出来
        let mut t1 = todo("周一干的", None, true, Some("2026-03-09T10:00"));
        t1.id = "t1".into();
        let mut t2 = todo("周三干的", None, true, Some("2026-03-11T10:00"));
        t2.id = "t2".into();
        let st = store_with(vec![t1, t2]);
        let tr = trend(&st, &[], d(2026, 3, 15), 7, None);
        assert_eq!(tr.len(), 7, "7 天窗口就该有 7 格");
        assert_eq!(tr[0], ("2026-03-09".into(), 1));
        assert_eq!(tr[1], ("2026-03-10".into(), 0), "空闲日也要占位");
        assert_eq!(tr[2], ("2026-03-11".into(), 1));
        assert_eq!(tr[6], ("2026-03-15".into(), 0), "末格是今天");
    }

    #[test]
    fn trend_clamps_absurd_windows() {
        // 前端只传 7/30/90，但命令参数来自 IPC：坏值不能让它去展开几万天
        let st = store_with(vec![]);
        assert_eq!(trend(&st, &[], d(2026, 3, 15), 1, None).len(), 7, "过小抬到 7");
        assert_eq!(trend(&st, &[], d(2026, 3, 15), -5, None).len(), 7, "负数抬到 7");
        assert_eq!(trend(&st, &[], d(2026, 3, 15), 99999, None).len(), 366, "过大压到 366");
    }

    #[test]
    fn trend_counts_archived_and_matches_summary_semantics() {
        // 同一份完成口径：趋势之和必须等于同区间 compute 的 done。
        // 两处若各算各的，分析页与总结页数字就会打架（本项目最怕的双实现漂移）。
        let st = store_with(vec![todo("活的", None, true, Some("2026-03-12T10:00"))]);
        let arch = vec![todo("归档的", None, true, Some("2026-03-10T10:00"))];
        let tr = trend(&st, &arch, d(2026, 3, 15), 7, None);
        assert_eq!(tr.iter().map(|(_, n)| n).sum::<usize>(), 2, "归档项须计入趋势");
        let s = compute(&st, &arch, d(2026, 3, 9), d(2026, 3, 15));
        assert_eq!(s.done, tr.iter().map(|(_, n)| n).sum::<usize>(), "与总结同口径");
    }

    #[test]
    fn trend_respects_list_filter() {
        // 实机截图抓到的 bug：分析页趋势柱按全局算、下方分布按选中清单算，
        // 同一屏出现"共完成 2 项 / 本清单 0 项"的自相矛盾。
        // 根因是 trend 没接 list。这条锁住它。
        let mut a = todo("收集箱的", None, true, Some("2026-03-12T10:00"));
        a.id = "a".into();
        a.list_id = "l1".into();
        let mut b = todo("别的清单的", None, true, Some("2026-03-12T11:00"));
        b.id = "b".into();
        b.list_id = "other".into();
        let st = store_with(vec![a, b]);

        let all = trend(&st, &[], d(2026, 3, 15), 7, None);
        assert_eq!(all.iter().map(|(_, n)| n).sum::<usize>(), 2, "全局两条");
        let one = trend(&st, &[], d(2026, 3, 15), 7, Some("l1"));
        assert_eq!(one.iter().map(|(_, n)| n).sum::<usize>(), 1, "选中清单只算它的");
        let none = trend(&st, &[], d(2026, 3, 15), 7, Some("empty-list"));
        assert_eq!(none.iter().map(|(_, n)| n).sum::<usize>(), 0, "空清单应为 0，与同页分布一致");
    }
}
