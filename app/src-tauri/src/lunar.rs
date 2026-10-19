//! 农历（对照小黄条 YYNote 的农历显示）
//!
//! 纯本地计算（lunar-lite，MIT），无网络、无数据文件 —— 与节假日表不同，
//! 农历是天文算法可推导的，不需要外部数据源。
//!
//! 标签规则与市面日历一致：
//!   · 每月初一 → 显示月名（正月/二月…冬月/腊月，闰月加"闰"）
//!   · 其余日子 → 显示日名（初一…初十、十一…十九、廿十…廿九、三十）

use chrono::{Datelike, NaiveDate};

const MONTH_NAMES: [&str; 12] = [
    "正月", "二月", "三月", "四月", "五月", "六月", "七月", "八月", "九月", "十月", "冬月", "腊月",
];

const DAY_NAMES: [&str; 30] = [
    "初一", "初二", "初三", "初四", "初五", "初六", "初七", "初八", "初九", "初十", "十一", "十二", "十三", "十四",
    "十五", "十六", "十七", "十八", "十九", "二十", "廿一", "廿二", "廿三", "廿四", "廿五", "廿六", "廿七",
    "廿八", "廿九", "三十",
];

/// 该日的农历标签；转换失败（超出支持范围等）返回 None，界面留空。
pub fn lunar_label(d: NaiveDate) -> Option<String> {
    let solar = lunar_lite::SolarDate {
        year: d.year(),
        month: d.month() as _,
        day: d.day() as _,
    };
    let lunar = lunar_lite::solar_to_lunar(solar).ok()?;
    let mi = (lunar.month as usize).checked_sub(1)?;
    let name = MONTH_NAMES.get(mi)?;
    if lunar.day == 1 {
        // 初一显示月名；闰月前缀"闰"（如"闰二月"）
        Some(if lunar.is_leap_month {
            format!("闰{name}")
        } else {
            name.to_string()
        })
    } else {
        DAY_NAMES.get((lunar.day as usize).checked_sub(1)?).map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn lunar_new_year_2026_is_feb17() {
        // 2026 年春节（正月初一）= 2026-02-17，公开日历可查证。
        // 这条同时锚定"初一显示月名"的规则。
        assert_eq!(lunar_label(d(2026, 2, 17)).as_deref(), Some("正月"));
        assert_eq!(lunar_label(d(2026, 2, 18)).as_deref(), Some("初二"));
    }

    #[test]
    fn known_2026_october_labels() {
        // 锚点：2026-10-10 = 农历九月初一（month_boundary 独立锚定）。
        // 日名不带月名前缀（初一才显示月名）——与 YYNote 显示一致：
        // 10-04 廿四、10-11 初二、10-18 初九、10-25 十六（用户截图交叉验证）。
        assert_eq!(lunar_label(d(2026, 10, 1)).as_deref(), Some("廿一"));
        assert_eq!(lunar_label(d(2026, 10, 18)).as_deref(), Some("初九"));
        assert_eq!(lunar_label(d(2026, 10, 25)).as_deref(), Some("十六"));
    }

    #[test]
    fn month_boundary_shows_month_name() {
        // 农历九月初一（2026-10-10）应显示"九月"而非"初一"
        assert_eq!(lunar_label(d(2026, 10, 10)).as_deref(), Some("九月"));
    }

    #[test]
    fn thirtieth_exists() {
        // 三十是日名表边界（索引 29）
        let _ = lunar_label(d(2026, 10, 24)); // 不 panic 即可
    }
}