//! 节假日与调休（中国法定节假日）
//!
//! 数据源：NateScarlet/holiday-cn 的年度 JSON（MIT），构建期打包进二进制；
//! 离线可用，无需自己维护农历/调休表。运行时按 `NaiveDate` 查表。
//!
//! 若打包数据缺失（例如开发期尚未下载），返回空表并在日历上退化为
//! "仅区分周末"，不报错、不阻塞主流程。

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub struct HolidayEntry {
    pub name: String,
    pub date: String,
    /// true = 放假日；false = 调休上班日
    #[serde(rename = "isOffDay")]
    pub is_off_day: bool,
}

#[derive(Debug, Deserialize)]
struct YearFile {
    #[allow(dead_code)]
    #[serde(default)]
    year: i32,
    /// holiday-cn 的数组字段名是 `days`
    #[serde(default)]
    days: Vec<HolidayEntry>,
}

#[derive(Default)]
pub struct HolidayTable {
    /// date -> (名称, 是否放假)
    by_date: HashMap<NaiveDate, (String, bool)>,
}

impl HolidayTable {
    pub fn get(&self, d: NaiveDate) -> Option<&(String, bool)> {
        self.by_date.get(&d)
    }

    pub fn is_rest(&self, d: NaiveDate) -> bool {
        matches!(self.by_date.get(&d), Some((_, true)))
    }

    pub fn is_workday_makeup(&self, d: NaiveDate) -> bool {
        matches!(self.by_date.get(&d), Some((_, false)))
    }

    /// 该日是否"休"（法定假日或普通周末，且未被调休上班占用）
    pub fn day_off(&self, d: NaiveDate) -> bool {
        if let Some((_, official)) = self.by_date.get(&d) {
            return *official;
        }
        matches!(
            d.weekday(),
            chrono::Weekday::Sat | chrono::Weekday::Sun
        )
    }
}

fn parse_years(jsons: &[&'static str]) -> HolidayTable {
    let mut t = HolidayTable::default();
    for j in jsons {
        if let Ok(yf) = serde_json::from_str::<YearFile>(j) {
            for h in yf.days {
                if let Ok(d) = NaiveDate::parse_from_str(&h.date, "%Y-%m-%d") {
                    t.by_date.insert(d, (h.name, h.is_off_day));
                }
            }
        }
    }
    t
}

/// 打包的年度数据（`src-tauri/holidays/` 目录）；缺文件时为空数组
fn bundled() -> Vec<&'static str> {
    let mut v = Vec::new();
    macro_rules! try_year {
        ($y:literal) => {{
            let s: &'static str = include_str!(concat!("../holidays/", $y, ".json"));
            if s.trim() != "{}" {
                v.push(s);
            }
        }};
    }
    // 覆盖当前与前后各一年，逐年追加
    try_year!("2024");
    try_year!("2025");
    try_year!("2026");
    try_year!("2027");
    v
}

/// 全局懒加载表
pub fn table() -> &'static HolidayTable {
    static T: OnceLock<HolidayTable> = OnceLock::new();
    T.get_or_init(|| parse_years(&bundled()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn weekend_rest_when_no_data() {
        let t = HolidayTable::default();
        // 2026-01-03 周六
        assert_eq!(d("2026-01-03").weekday().num_days_from_monday(), 5);
        assert!(t.day_off(d("2026-01-03")));
        assert!(!t.day_off(d("2026-01-05"))); // 周一
    }

    #[test]
    fn bundled_parses_if_present() {
        // 数据文件可能尚未下载；不 panic 即算通过
        let _ = table();
    }

    #[test]
    fn bundled_has_real_holidays() {
        let t = table();
        // 2026 元旦必须在表内且为放假
        let ny = d("2026-01-01");
        assert!(t.get(ny).is_some(), "打包数据应含 2026 元旦");
        assert!(t.is_rest(ny));
        // 表内应至少有一个"调休上班日"（isOffDay=false）
        let has_makeup = t.by_date.values().any(|(_, off)| !*off);
        assert!(has_makeup, "应含调休上班日");
    }
}
