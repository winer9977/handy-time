//! 重复规则：从"完成一条重复待办"推导下一条的到期日
//!
//! 纯函数、无 IO，便于单测。日期一律用 `NaiveDate`（墙上日期语义）。

use super::{Freq, Recurrence};
use chrono::{Datelike, NaiveDate, Weekday};
use lunar_lite::{leap_month, lunar_month_days, lunar_to_solar, solar_to_lunar, LunarDate, SolarDate};

/// 公历日 → 农历日
fn lunar_of(d: NaiveDate) -> Option<LunarDate> {
    solar_to_lunar(SolarDate {
        year: d.year(),
        month: d.month() as u8,
        day: d.day() as u8,
    })
    .ok()
}

/// 农历日 → 公历日（超范围/非法返回 None）
fn solar_of_lunar(l: LunarDate) -> Option<NaiveDate> {
    let s = lunar_to_solar(l).ok()?;
    NaiveDate::from_ymd_opt(s.year, s.month as u32, s.day as u32)
}

fn lunar_days(year: i32, month: u8, leap: bool) -> u8 {
    lunar_month_days(year, month, leap).unwrap_or(30)
}

/// 农历每月：保持"农历日"不变，农历月 +step。
///
/// 两个坑必须处理，否则展开会崩或跳错：
///   · 目标月是 29 天小月而基准是三十 → 回退到廿九（`min`），
///     绝不能拿不存在的三十去转换（lunar_to_solar 会 Err）。
///   · 一律用**普通月**（非闰）：农历每月推进跨闰月时若纠缠闰/不闰，
///     语义会歧义；"每月初一"这类需求本来就落在普通月序列上。
fn next_lunar_monthly(base: NaiveDate, interval: u16) -> Option<NaiveDate> {
    let l = lunar_of(base)?;
    let step = interval.max(1) as i32;
    let start = l.year * 12 + (l.month as i32 - 1);
    for k in 1..=60i32 {
        let idx = start + k * step;
        let y = idx.div_euclid(12);
        let m = idx.rem_euclid(12) + 1;
        let day = l.day.min(lunar_days(y, m as u8, false));
        let cand = solar_of_lunar(LunarDate {
            year: y,
            month: m as u8,
            day,
            is_leap_month: false,
        })?;
        if cand > base {
            return Some(cand);
        }
    }
    None
}

/// 农历每年：保持农历月+日不变，年 +step（农历生日、端午等）。
///
/// 闰月处理：基准若落在闰月，只有目标年**也有同名闰月**时才用闰月，
/// 否则回退到普通月（lunar 闰月几年一遇，不能强求）。同样 clamp 月长。
fn next_lunar_yearly(base: NaiveDate, interval: u16) -> Option<NaiveDate> {
    let l = lunar_of(base)?;
    let step = interval.max(1) as i32;
    for k in 1..=40i32 {
        let y = l.year + k * step;
        let leap = l.is_leap_month && leap_month(y).ok().flatten() == Some(l.month);
        let day = l.day.min(lunar_days(y, l.month, leap));
        let cand = solar_of_lunar(LunarDate {
            year: y,
            month: l.month,
            day,
            is_leap_month: leap,
        })?;
        // 农历新年在公历 1~2 月，同农历年可能对应公历上更早的日期，
        // 必须严格晚于 base 才算"下一次"
        if cand > base {
            return Some(cand);
        }
    }
    None
}

/// 解析 `YYYY-MM-DD`
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// 重复规则的中文描述（导出用）。
///
/// 与前端 `shared/rc-text.ts` 是同一套文案的两份实现 —— 进程边界所迫
/// （渲染期同步 IPC 不值当）。两边都有测试锚定，漂移会被测试抓住。
pub fn rc_label(r: &Recurrence) -> String {
    let unit = match r.freq {
        Freq::Daily => "每天",
        Freq::Weekly => "每周",
        Freq::Monthly => "每月",
        Freq::Yearly => "每年",
        Freq::Workday => "工作日",
        Freq::OfficialWorkday => "官方工作日",
        Freq::Holiday => "节假日",
        Freq::LunarMonthly => "农历每月",
        Freq::LunarYearly => "农历每年",
    };
    if r.freq == Freq::Weekly && !r.by_weekday.is_empty() {
        let names = "一二三四五六日";
        // by_weekday 0=周一 … 6=周日
        let joined: Vec<String> = r
            .by_weekday
            .iter()
            .map(|&d| names.chars().nth(d as usize % 7).unwrap_or('?').to_string())
            .collect();
        return format!("每周{}", joined.join("、"));
    }
    if r.freq == Freq::Monthly && !r.by_monthday.is_empty() {
        let joined: Vec<String> = r
            .by_monthday
            .iter()
            .map(|&md| if md == -1 { "末".into() } else { format!("{md}日") })
            .collect();
        return format!("每月{}", joined.join("/"));
    }
    if r.interval > 1 {
        let u = match r.freq {
            Freq::Daily => "天",
            Freq::Weekly => "周",
            Freq::Monthly => "月",
            Freq::Yearly => "年",
            Freq::Workday => "工作日",
            Freq::OfficialWorkday => "官方工作日",
            Freq::Holiday => "节假日",
            Freq::LunarMonthly => "农历月",
            Freq::LunarYearly => "农历年",
        };
        return format!("每{}{}", r.interval, u);
    }
    unit.to_string()
}

fn is_workday(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

fn next_workday(from: NaiveDate) -> NaiveDate {
    let mut d = from + chrono::Duration::days(1);
    for _ in 0..7 {
        if is_workday(d) {
            return d;
        }
        d += chrono::Duration::days(1);
    }
    d
}

fn add_months_clamped(d: NaiveDate, months: i32) -> NaiveDate {
    let total = d.year() * 12 + (d.month0() as i32) + months;
    let (y, m0) = (total.div_euclid(12), total.rem_euclid(12));
    let day = d.day().min(days_in_month(y, (m0 + 1) as u32));
    NaiveDate::from_ymd_opt(y, (m0 + 1) as u32, day).unwrap_or(d)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// monthly 且指定 by_monthday 时，找到严格晚于 `after` 的第一个满足日子。
/// 正数=当月第 N 日；负数 -1 = 当月最后一天。当月无对应日（如 2 月的 31 日）
/// 则当月跳过。interval 以基准日所在月为对齐原点。
fn next_monthday_after(base: NaiveDate, days: &[i8], interval: u16) -> Option<NaiveDate> {
    if days.is_empty() {
        return None;
    }
    let step = (interval.max(1) as i32).max(1); // 月步长
    let base_idx = base.year() * 12 + base.month0() as i32;
    // 从基准月起向后扫 240 个月（20 年）足够覆盖任何 interval 组合
    for k in 0..240i32 {
        let idx = base_idx + k * step;
        let year = idx.div_euclid(12);
        let month = idx.rem_euclid(12) + 1;
        if year > base.year() + 20 {
            break;
        }
        let last = days_in_month(year, month as u32);
        let mut cands: Vec<NaiveDate> = Vec::new();
        for &md in days {
            let day = if md > 0 {
                md as u32
            } else {
                last.saturating_sub((-md) as u32 - 1)
            };
            if day >= 1 && day <= last {
                if let Some(dt) = NaiveDate::from_ymd_opt(year, month as u32, day) {
                    cands.push(dt);
                }
            }
        }
        cands.sort_unstable();
        if let Some(first) = cands.into_iter().find(|dt| *dt > base) {
            return Some(first);
        }
    }
    None
}

/// 从 from（不含）起，找第一个满足 `pred` 的日子。上限 400 天防死循环
/// （连续长假或整月调休也远小于此）。
fn next_matching(from: NaiveDate, pred: impl Fn(NaiveDate) -> bool) -> Option<NaiveDate> {
    let mut d = from + chrono::Duration::days(1);
    for _ in 0..400 {
        if pred(d) {
            return Some(d);
        }
        d += chrono::Duration::days(1);
    }
    None
}

/// 官方工作日：周一至周五且非法定假日；调休上班的周末也算。
/// `!day_off` 正好表达这个语义（表内看 official，表外看是否周末）。
fn next_official_workday(from: NaiveDate) -> Option<NaiveDate> {
    let t = crate::holidays::table();
    next_matching(from, |d| !t.day_off(d))
}

/// 节假日：法定假日 + 普通周末（被调休上班占用的除外）。
fn next_holiday(from: NaiveDate) -> Option<NaiveDate> {
    let t = crate::holidays::table();
    next_matching(from, |d| t.day_off(d))
}

/// 按 step 次反复调用 `f`（step=1 就是"下一个"）。
fn step_times(
    base: NaiveDate,
    step: i64,
    f: impl Fn(NaiveDate) -> Option<NaiveDate>,
) -> Option<NaiveDate> {
    let mut d = base;
    for _ in 0..step.max(1) {
        d = f(d)?;
    }
    Some(d)
}

/// 到期日变更时，把提醒时点平移到新日期（HH:MM 保持不变）。
///
/// **为什么抽成共享函数**：有三条路径会改待办的到期日 ——
///   ① UI 改到期日（前端 TodoRow.applyDue）
///   ② 日历上拖"派生条目"改期（state::move_event）
///   ③ 拖待办到日历格生成日程（state::todo_to_event）
/// 三者必须**同一口径**处理提醒，否则同一条待办经不同路径得到不同结果：
/// 用户把待办拖到 5 号、提醒却还停在 3 号（实测在 ②③ 上各踩过一次）。
///
/// 同时复位触发位与额外提醒去重：时点整体平移后，旧的绝对 key 已指向不存在
/// 的时刻，留着会让同名新时点被误判"已弹过"而永不触发。持续提醒的节流基准
/// 也清零，让新时点重新计时。
///
/// `new_date` 为 None（清掉到期日）时**不动提醒** —— 提醒是独立设置，
/// 删掉到期日不代表想删提醒（UI 亦此行为）。
pub fn shift_reminders_to(t: &mut crate::domain::Todo, new_date: Option<&str>) {
    let Some(d) = new_date else { return };
    if let Some(r) = t.remind_at.as_deref() {
        t.remind_at = Some(match r.split_once('T') {
            Some((_, hhmm)) => format!("{d}T{hhmm}"),
            None => d.to_string(),
        });
    }
    t.extra_reminds = t
        .extra_reminds
        .iter()
        .map(|x| match x.split_once('T') {
            Some((_, hhmm)) => format!("{d}T{hhmm}"),
            None => d.to_string(),
        })
        .collect();
    t.reminded = false;
    t.reminded_keys.clear();
    t.last_reminded_at = None;
}

/// 给定基准日（通常是当前 due，或今天），算出下一条重复的到期日。
/// 返回 None 表示该规则不再有后续（超出 until，或规则无法推进）。
pub fn next_due(base: NaiveDate, r: &Recurrence) -> Option<NaiveDate> {
    let step = r.interval.max(1) as i64;
    let cand = match r.freq {
        Freq::Daily => base + chrono::Duration::days(step),
        Freq::Workday => {
            let mut d = next_workday(base);
            let mut n = 1;
            while n < step {
                d = next_workday(d);
                n += 1;
            }
            d
        }
        Freq::Weekly => {
            if r.by_weekday.is_empty() {
                base + chrono::Duration::weeks(step)
            } else {
                // 在未来 8 天内找下一个落在指定星期集合的日子
                let want: Vec<u32> = r.by_weekday.iter().map(|&v| (v % 7) as u32).collect();
                (1..=8)
                    .filter_map(|k| {
                        let d = base + chrono::Duration::days(k);
                        if want.contains(&(d.weekday().num_days_from_monday())) {
                            Some(d)
                        } else {
                            None
                        }
                    })
                    .next()?
            }
        }
        Freq::Monthly => {
            if r.by_monthday.is_empty() {
                add_months_clamped(base, step as i32)
            } else {
                next_monthday_after(base, &r.by_monthday, r.interval as u16)?
            }
        }
        Freq::Yearly => {
            let year = base.year() + step as i32;
            let day = base.day().min(days_in_month(year, base.month()));
            NaiveDate::from_ymd_opt(year, base.month(), day)?
        }
        Freq::LunarMonthly => next_lunar_monthly(base, r.interval)?,
        Freq::LunarYearly => next_lunar_yearly(base, r.interval)?,
        Freq::OfficialWorkday => step_times(base, step, next_official_workday)?,
        Freq::Holiday => step_times(base, step, next_holiday)?,
    };
    match r.until.as_deref().and_then(parse_date) {
        Some(u) if cand > u => None,
        _ => Some(cand),
    }
}

/// 把一条日程展开到 [from, to] 区间内的具体日期。
///
/// 重复日程**不落库多行**，而是查询时按规则虚拟展开（对齐小黄条
/// "灵活的重复日程，自动创建重复日程"的观感，同时避免长期重复项堆积数据）。
/// 无规则时退化为"就那一天"。
///
/// `cap` 是推进步数上限：起始日远在区间之前时避免无界循环。
pub fn expand_dates(
    start: NaiveDate,
    r: Option<&Recurrence>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let Some(rule) = r else {
        if (from..=to).contains(&start) {
            out.push(start);
        }
        return out;
    };
    if start > to {
        return out;
    }
    let mut cur = start;
    for _ in 0..4000 {
        if cur > to {
            break;
        }
        if cur >= from {
            out.push(cur);
        }
        match next_due(cur, rule) {
            Some(n) if n > cur => cur = n,
            _ => break, // 超出 until 或规则无法推进
        }
    }
    out
}

/// 倒数天数：目标日 - 今天。正数=还有 N 天，负数=已经 N 天。
pub fn countdown_days(today: NaiveDate, target: NaiveDate) -> i64 {
    (target - today).num_days()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Recurrence;

    fn d(s: &str) -> NaiveDate {
        parse_date(s).unwrap()
    }
    fn r(freq: Freq, interval: u16) -> Recurrence {
        Recurrence {
            freq,
            interval,
            by_weekday: vec![],
            by_monthday: vec![],
            until: None,
            auto_create: true,
        }
    }

    #[test]
    fn daily_steps() {
        assert_eq!(next_due(d("2026-01-05"), &r(Freq::Daily, 1)), Some(d("2026-01-06")));
        assert_eq!(next_due(d("2026-01-05"), &r(Freq::Daily, 3)), Some(d("2026-01-08")));
    }

    #[test]
    fn weekly_named_weekday() {
        // 2026-01-05 是周一
        let mut rec = r(Freq::Weekly, 1);
        rec.by_weekday = vec![4]; // 周五(0=周一)
        assert_eq!(next_due(d("2026-01-05"), &rec), Some(d("2026-01-09")));
    }

    #[test]
    fn monthly_clamps_leap() {
        let mut rec = r(Freq::Monthly, 1);
        rec.by_monthday = vec![31];
        // 1/31 之后应跳到 3/31（2 月无 31 日）
        let nx = next_due(d("2026-01-31"), &rec).unwrap();
        assert!(nx > d("2026-01-31"));
        assert_eq!(nx.day(), 31);
    }

    #[test]
    fn monthly_last_day_negative() {
        let mut rec = r(Freq::Monthly, 1);
        rec.by_monthday = vec![-1];
        // 基准 1/15，当月最后一天 1/31 在将来 → 应取 1/31 而非下月
        assert_eq!(next_due(d("2026-01-15"), &rec), Some(d("2026-01-31")));
        // 基准已是 1/31 → 下一个是 2/28
        assert_eq!(next_due(d("2026-01-31"), &rec), Some(d("2026-02-28")));
    }

    #[test]
    fn yearly_leap_feb29() {
        assert_eq!(next_due(d("2024-02-29"), &r(Freq::Yearly, 1)), Some(d("2025-02-28")));
    }

    #[test]
    fn lunar_monthly_keeps_lunar_day_across_months() {
        // 2026-03-19 = 农历二月初一 → 下次三月初一 = 2026-04-17
        // （2026 春节 = 2-17，公开可查，据此锚定所有农历断言）
        assert_eq!(
            next_due(d("2026-03-19"), &r(Freq::LunarMonthly, 1)),
            Some(d("2026-04-17"))
        );
        // 腊月初一（2026-01-01 是十一月十三 → 下次十二月初一... 用初一基准更直白）
        assert_eq!(
            next_due(d("2026-05-17"), &r(Freq::LunarMonthly, 1)),
            Some(d("2026-06-15")),
            "四月初一 → 五月初一"
        );
    }

    #[test]
    fn lunar_monthly_advances_one_lunar_month_per_step() {
        // 每次推进：农历月 +1（跨年回环），农历日保持（受目标月长 clamp）。
        let start = d("2026-01-01"); // 农历 2025 十一月十三
        let base = lunar_of(start).unwrap();
        assert_eq!((base.month, base.day), (11, 13));
        let mut n = start;
        for k in 1..=14i32 {
            n = next_due(n, &r(Freq::LunarMonthly, 1)).expect("应可连续推进");
            let l = lunar_of(n).unwrap();
            // 期望农历月 = 基准月 + k（1..12 回环）
            let want_month = ((11 - 1 + k) % 12 + 1) as u8;
            assert_eq!(l.month, want_month, "第 {k} 步应是农历{want_month}月，实际 {}", l.month);
            assert_eq!(l.day, base.day.min(lunar_days(l.year, l.month, false)));
            assert!(!l.is_leap_month, "每月推进一律走普通月序列");
        }
    }

    #[test]
    fn lunar_monthly_survives_short_target_months() {
        // 基准落在农历三十。目标月若是 29 天小月，必须 clamp 到廿九而非报错，
        // 否则 lunar_to_solar 返回 Err、整条展开链断掉。
        // 找一个"三十"的基准：从探测里取，再验证连续推进 12 步不断裂。
        // 找一个"三十"作基准：农历月 29/30 交替，扫 200 天必遇大月
        let mut n = d("2026-01-01");
        for _ in 0..200 {
            if lunar_of(n).map(|l| l.day == 30).unwrap_or(false) {
                break;
            }
            n += chrono::Duration::days(1);
        }
        let base = lunar_of(n).unwrap();
        assert_eq!(base.day, 30, "需要一个三十作基准，当前 {base:?}");
        for _ in 0..12 {
            let nx = next_due(n, &r(Freq::LunarMonthly, 1));
            assert!(nx.is_some(), "三十基准推进到小月不该失败（自 {n:?}）");
            let l = lunar_of(nx.unwrap()).unwrap();
            assert!(l.day == 30 || l.day == 29, "三十只会被 clamp 到廿九，实际 {}", l.day);
            n = nx.unwrap();
        }
    }

    #[test]
    fn lunar_yearly_repeats_same_lunar_date() {
        // 农历生日类：2026-01-01（农历乙巳年十一月十三）→ 次年同一农历日
        assert_eq!(
            next_due(d("2026-01-01"), &r(Freq::LunarYearly, 1)),
            Some(d("2026-12-21")),
            "农历 2025 十一月十三 → 农历 2026 十一月十三"
        );
        // 连续推进都保持农历月+日不变
        let base = lunar_of(d("2026-01-01")).unwrap();
        let mut n = d("2026-01-01");
        for _ in 0..4 {
            n = next_due(n, &r(Freq::LunarYearly, 1)).expect("农历每年应可连续推进");
            let l = lunar_of(n).unwrap();
            assert_eq!((l.month, l.day), (base.month, base.day), "农历月日须恒定");
            assert_eq!(l.is_leap_month, false, "目标年无同名闰月时回退普通月");
        }
    }

    #[test]
    fn lunar_yearly_survives_leap_month_years() {
        // 2025 有闰六月、2026 无：基准在普通六月，推进到闰六月年仍须可用
        assert_eq!(leap_month(2025).unwrap(), Some(6));
        assert_eq!(leap_month(2026).unwrap(), None);
        let mut n = d("2025-07-01"); // 农历二〇二五年六月初七（非闰）
        for _ in 0..3 {
            n = next_due(n, &r(Freq::LunarYearly, 1)).expect("闰月年不该让推进失败");
        }
        assert!(n > d("2025-07-01"));
    }

    #[test]
    fn lunar_freqs_expand_over_range() {
        // 展开路径（日历角标用的就是它）：农历每月在 60 天窗口里应出多次
        let start = d("2026-03-19"); // 二月初一
        let r1 = r(Freq::LunarMonthly, 1);
        let dates = expand_dates(start, Some(&r1), d("2026-03-19"), d("2026-08-31"));
        assert!(dates.len() >= 5, "5 个月应展开≥5 次，实际 {}", dates.len());
        assert!(dates.windows(2).all(|w| w[0] < w[1]), "展开须严格递增");
    }

    #[test]
    fn official_workday_lands_on_working_days_only() {
        // 不变量：产出的每一天都必须"非休"（!day_off），且严格前进。
        // 不硬编码具体日期（节假日排布是外部数据，会随年份变），断言谓词本身。
        let t = crate::holidays::table();
        let mut n = d("2026-01-05");
        for _ in 0..30 {
            n = next_due(n, &r(Freq::OfficialWorkday, 1)).expect("应可连续推进");
            assert!(!t.day_off(n), "{n:?} 不该是休息日");
        }
    }

    #[test]
    fn official_workday_skips_national_holiday() {
        // 2026 国庆 10-01..10-07 放假。从 09-30 出发，下一条必须跳过整段假期。
        let t = crate::holidays::table();
        // 前提自检：假期里确有休息日，否则这条测试没意义
        assert!(t.day_off(d("2026-10-01")), "10-01 应是法定假日");
        let nx = next_due(d("2026-09-30"), &r(Freq::OfficialWorkday, 1)).unwrap();
        assert!(nx > d("2026-10-07"), "应跳过整个国庆假期，实际 {nx:?}");
        assert!(!t.day_off(nx));
    }

    #[test]
    fn official_workday_counts_makeup_weekend() {
        // 调休上班的周末（表内 is_off_day=false）应算工作日。2026-10-10 是周六补班。
        let t = crate::holidays::table();
        // 自检：10-10 确为"周末 + 补班"
        assert!(matches!(d("2026-10-10").weekday(), Weekday::Sat));
        assert!(!t.day_off(d("2026-10-10")), "10-10 补班，不该被判休");
        // 从 10-09（周五）推进应落到 10-10（补班周六），而不是跳到下周一
        assert_eq!(
            next_due(d("2026-10-09"), &r(Freq::OfficialWorkday, 1)),
            Some(d("2026-10-10"))
        );
    }

    #[test]
    fn holiday_freq_lands_on_days_off() {
        let t = crate::holidays::table();
        let mut n = d("2026-01-05");
        for _ in 0..20 {
            n = next_due(n, &r(Freq::Holiday, 1)).expect("应可连续推进");
            assert!(t.day_off(n), "{n:?} 应是休息日");
        }
    }

    #[test]
    fn holiday_during_golden_week_advances_within_it() {
        // 国庆假期内，"每个节假日"应逐天走假期（10-01→10-02→…），
        // 而不是跳到假期后第一个周末。
        let a = next_due(d("2026-10-01"), &r(Freq::Holiday, 1)).unwrap();
        assert_eq!(a, d("2026-10-02"));
        let b = next_due(d("2026-10-06"), &r(Freq::Holiday, 1)).unwrap();
        assert_eq!(b, d("2026-10-07"));
    }

    #[test]
    fn workday_skips_weekend() {
        // 2026-01-09 周五 → 下一个工作日 周一 01-12
        assert_eq!(next_due(d("2026-01-09"), &r(Freq::Workday, 1)), Some(d("2026-01-12")));
    }

    #[test]
    fn until_blocks() {
        let mut rec = r(Freq::Daily, 1);
        rec.until = Some("2026-01-06".into());
        assert_eq!(next_due(d("2026-01-06"), &rec), None);
        assert_eq!(next_due(d("2026-01-05"), &rec), Some(d("2026-01-06")));
    }

    #[test]
    fn countdown_sign() {
        assert_eq!(countdown_days(d("2026-01-01"), d("2026-01-19")), 18);
        assert_eq!(countdown_days(d("2026-01-19"), d("2026-01-01")), -18);
    }

    /// shift_reminders_to 的最小待办（只关心提醒相关字段）
    fn todo_with(remind: Option<&str>, extras: &[&str]) -> crate::domain::Todo {
        crate::domain::Todo {
            id: "t".into(),
            list_id: "l".into(),
            title: "任务".into(),
            note: String::new(),
            done: false,
            done_at: None,
            pinned: false,
            order: 0,
            tags: vec![],
            due: Some("2026-05-03".into()),
            remind_at: remind.map(|s| s.into()),
            reminded: true,
            extra_reminds: extras.iter().map(|s| s.to_string()).collect(),
            reminded_keys: extras.iter().map(|s| s.to_string()).collect(),
            constant_remind: true,
            last_reminded_at: Some("2026-05-03T09:00".into()),
            countdown: None,
            priority: None,
            recurrence: None,
            created_at: "2026-05-01T10:00".into(),
            archived_at: None,
        }
    }

    #[test]
    fn shift_reminders_keeps_time_of_day() {
        // 三条改期路径（UI 改到期日 / 拖派生条目 / 拖进日历）共用这条规则：
        // 只换日期、HH:MM 必须原样保留。
        let mut t = todo_with(Some("2026-05-03T09:30"), &["2026-05-03T18:00"]);
        shift_reminders_to(&mut t, Some("2026-05-05"));
        assert_eq!(t.remind_at.as_deref(), Some("2026-05-05T09:30"));
        assert_eq!(t.extra_reminds, vec!["2026-05-05T18:00".to_string()]);
    }

    #[test]
    fn shift_reminders_resets_dedupe_state() {
        // 平移到新日期后旧的绝对 key 已指向不存在的时刻，必须清掉，
        // 否则同名新时点会被误判"已弹过"而永不触发。
        let mut t = todo_with(Some("2026-05-03T09:30"), &["2026-05-03T18:00"]);
        assert!(t.reminded, "前置：应处于已提醒状态");
        assert!(!t.reminded_keys.is_empty());
        assert!(t.last_reminded_at.is_some());
        shift_reminders_to(&mut t, Some("2026-05-05"));
        assert!(!t.reminded, "主提醒触发位应复位");
        assert!(t.reminded_keys.is_empty(), "额外提醒去重应清空");
        assert!(t.last_reminded_at.is_none(), "持续提醒计时应重新起算");
        // 但用户的提醒**配置**不能被清（那是他设的）
        assert_eq!(t.extra_reminds.len(), 1, "额外提醒时点须保留（只是换了日期）");
        assert!(t.constant_remind, "持续提醒开关须保留");
    }

    #[test]
    fn shift_reminders_tolerates_date_only_and_absent() {
        // 纯日期格式（reminder::parse_min 明确支持）也要能平移
        let mut t = todo_with(Some("2026-05-03"), &[]);
        shift_reminders_to(&mut t, Some("2026-05-05"));
        assert_eq!(t.remind_at.as_deref(), Some("2026-05-05"));
        // 没有提醒时不该凭空造出提醒
        let mut t2 = todo_with(None, &[]);
        shift_reminders_to(&mut t2, Some("2026-05-05"));
        assert_eq!(t2.remind_at, None, "无提醒时不得凭空产生");
        // new_date=None（清掉到期日）时不动提醒：提醒是独立设置
        let mut t3 = todo_with(Some("2026-05-03T09:30"), &["2026-05-03T18:00"]);
        shift_reminders_to(&mut t3, None);
        assert_eq!(t3.remind_at.as_deref(), Some("2026-05-03T09:30"), "清到期日不该删提醒");
        assert_eq!(t3.extra_reminds, vec!["2026-05-03T18:00".to_string()]);
        assert!(t3.reminded, "未改期就不该复位触发位");
    }
}
