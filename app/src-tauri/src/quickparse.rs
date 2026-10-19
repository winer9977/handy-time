//! 一句话智能识别（本地规则，不联网）
//!
//! 对齐小黄条"一句话输入日程，自动识别日期、时间"。
//! 支持：
//!   · `[标签] [另一个]` 前缀标签
//!   · 今天 / 明天 / 后天 / 大后天
//!   · 周X / 下周X / 这周X（周一起算；**当天正是周X → 就排今天**，与滴答一致）
//!   · 全写日期：`2026-10-20` / `2026/10/20` / `2026.10.20` / `2026年10月20日`
//!     （年份按字面取，不做"过期顺延一年"推断）
//!   · M月D日 / M-D（今年，过期则顺延一年；裸 `M.D` 不支持 —— 与小数无法区分）
//!   · HH:MM / 上午下午X点 / X点半（附着到上面解析出的日期，默认今天）

use crate::domain::recurring::fmt_date;
use crate::domain::Priority;
use chrono::{Duration, Local, NaiveDate, Datelike};

#[derive(Debug, Default)]
pub struct Parsed {
    pub title: String,
    pub tags: Vec<String>,
    pub due: Option<String>,
    /// 规范化 "HH:MM"；None = 未识别到时间
    pub time: Option<String>,
    /// 优先级旗标（!!! / !! / ! 或 P1/P2/P3）
    pub priority: Option<Priority>,
}

/// 解析输入串。（干净标题, 标签, 到期日）—— 不关心时间时的便捷入口
pub fn parse(input: &str) -> (String, Vec<String>, Option<String>) {
    let p = parse_full(input);
    (p.title, p.tags, p.due)
}

pub fn parse_full(input: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut text = input.trim().to_string();

    // 1) 前缀 [标签]（半角与全角）
    loop {
        let trimmed = text.trim_start();
        let full = trimmed.starts_with('［');
        if !trimmed.starts_with('[') && !full {
            break;
        }
        let close = if full { '］' } else { ']' };
        match trimmed.find(close) {
            Some(byte_end) => {
                let inner_start = if full { '［'.len_utf8() } else { 1 };
                let tag = trimmed[inner_start..byte_end].trim().to_string();
                if !tag.is_empty() {
                    out.tags.push(tag);
                }
                text = trimmed[byte_end + close.len_utf8()..].to_string();
            }
            None => break,
        }
    }

    let today = Local::now().date_naive();

    // 2) 优先级旗标：!!! / !! / ! 或 P1/P2/P3
    if let Some((pr, rest)) = find_priority(&text) {
        out.priority = Some(pr);
        text = rest;
    }

    // 3) 日期：全文扫描，命中即从文本中摘除（支持"交周报 8月1日""明天 开会"等任意位置）
    //    顺序：全写日期在前（它最明确），再 M月D日/M-D，再相对日/星期。
    for finder in [
        find_ymd_date as FindFn,
        find_md_date,
        find_relative_day,
        find_weekday,
    ] {
        if let Some((date, rest)) = finder(&text, today) {
            out.due = Some(fmt_date(date));
            text = rest;
            break;
        }
    }

    // 4) 时间：HH:MM / H:MM / 上午下午晚上X点(半|X分) / X点后
    //    附着到已解析出的日期；无日期则留空由调用方按"今天"处理
    if let Some((hhmm, rest)) = find_time(&text) {
        out.time = Some(hhmm);
        text = rest;
    }

    out.title = cleanup(&text);
    out
}

/// 时段词 → 12 小时制的加时偏移
const PERIODS: [(&str, u32); 6] = [
    ("凌晨", 0),
    ("早上", 0),
    ("上午", 0),
    ("中午", 12),
    ("下午", 12),
    ("晚上", 12),
];

/// 只抽标签，不改动正文。
///
/// 便签是自由多行文本，用户是"先建空便签再打字"，所以标签必须在**保存时**抽；
/// 而且不能像待办那样剥离 —— 删掉用户敲进去的字符比留着更糟。
/// 于是扫描全文所有 `[...]`（半角/全角），去重保序。
pub fn extract_tags(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '[' && chars[i] != '［' {
            i += 1;
            continue;
        }
        let close = if chars[i] == '［' { '］' } else { ']' };
        let Some(rel) = chars[i + 1..].iter().position(|&c| c == close) else {
            break;
        };
        let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
        // 标签不该跨行、不该为空、不该超长。
        // 必须在 trim 之前查换行 —— trim 会把首尾 \n 吃掉，之后再也查不到
        let tag = inner.trim();
        if !tag.is_empty()
            && !inner.contains('\n')
            && inner.chars().count() <= 24
        {
            if !out.iter().any(|t| t == tag) {
                out.push(tag.to_string());
            }
        }
        i += rel + 2;
    }
    out
}

/// 识别时间。命中返回（"HH:MM", 摘除该片段后的文本）
///
/// 覆盖：`09:30` `9:5` `15:20` `下午3：05` `下午3点` `晚上8点半` `早上7点10分` `中午12点`
/// 刻意保守：必须有 `点` / `:` / `：` 之一作锚，裸数字（"3折"、"5个"）不当时间。
fn find_time(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut s = 0usize;
    while s < n {
        // 只从数字串起点开始（避免把 12 拆成 1|2 两次误读）
        if !(chars[s].is_ascii_digit() && (s == 0 || !chars[s - 1].is_ascii_digit())) {
            s += 1;
            continue;
        }
        let Some((hv, after_h)) = read_upto(&chars, s, 2) else {
            while s < n && chars[s].is_ascii_digit() {
                s += 1; // 数字串过长（如年份 2026）：整串跳过
            }
            continue;
        };
        // 时段词紧贴数字（允许一个空格）？冒号式与"点"式都回看
        let (span_start, off) = look_period(&chars, s);
        let hit = if after_h < n && (chars[after_h] == ':' || chars[after_h] == '：') {
            // 冒号式 H:MM
            match read_upto(&chars, after_h + 1, 2) {
                Some((mv, end)) if mv < 60 && hv < 24 => {
                    let hour = apply_period(hv, off);
                    if hour > 23 {
                        None
                    } else {
                        Some((format!("{hour:02}:{mv:02}"), span_start, end))
                    }
                }
                _ => None,
            }
        } else if after_h < n && chars[after_h] == '点' && (1..=12).contains(&hv) {
            // 中文式 X点[半|Y分]
            let (mv, end) = read_minute(&chars, after_h + 1);
            let hour = apply_period(hv, off);
            if hour > 23 || mv > 59 {
                None
            } else {
                Some((format!("{hour:02}:{mv:02}"), span_start, end))
            }
        } else {
            None
        };
        if let Some((hhmm, a, b)) = hit {
            return Some((hhmm, splice(&chars, a, b)));
        }
        s = after_h.max(s + 1);
    }
    None
}

/// 回看紧贴 pos 之前的时段词（允许一个空格）。返回（片段起点, 加时偏移）
fn look_period(chars: &[char], pos: usize) -> (usize, Option<u32>) {
    let mut t = pos;
    if t > 0 && chars[t - 1] == ' ' {
        t -= 1;
    }
    for (kw, o) in PERIODS {
        let kc = kw.chars().count();
        if t >= kc && chars[t - kc..t].iter().copied().eq(kw.chars()) {
            return (t - kc, Some(o));
        }
    }
    (pos, None)
}

/// 12 小时制 + 时段偏移。中午12点保持 12，下午1点 → 13
fn apply_period(hv: u32, off: Option<u32>) -> u32 {
    match off {
        Some(12) if hv != 12 => hv + 12,
        Some(0) if hv == 12 => 0, // 凌晨12点视为 00:00
        _ => hv,
    }
}

/// 从 pos 起读最多 max 位数字；位数不足或超上限返回 None
fn read_upto(chars: &[char], pos: usize, max: usize) -> Option<(u32, usize)> {
    let mut j = pos;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j == pos || j - pos > max {
        return None;
    }
    chars[pos..j]
        .iter()
        .collect::<String>()
        .parse::<u32>()
        .ok()
        .map(|v| (v, j))
}

/// 「点」之后的分钟：`半` → 30；`Y分` → Y；否则 0。返回（分钟, 消费到的位置）
fn read_minute(chars: &[char], p: usize) -> (u32, usize) {
    if p < chars.len() && chars[p] == '半' {
        return (30, p + 1);
    }
    if let Some((mv, q)) = read_upto(chars, p, 2) {
        if q < chars.len() && chars[q] == '分' && mv < 60 {
            return (mv, q + 1);
        }
    }
    (0, p)
}

/// 识别优先级旗标。命中返回（优先级, 摘除该片段后的文本）
///
/// 语法：`!!!` 高 / `!!` 中 / `!` 低，或 `P1`/`P2`/`P3`（大小写均可）。
///
/// 误判代价极高（"买3个!" 被当成优先级），所以要求**独立成词**：
/// 前后必须是空白或串边界。这天然排除了三种情况：
///   · 中文句末全角 `！`（本来就不是 ASCII，且紧贴文字）
///   · 紧贴词的 `Hello!`、`买3个!`（前面不是空白）
///   · 代码式 `!=`、`a!b`（后面不是空白）
/// 多个旗标同时出现时取最高优先级。
fn find_priority(text: &str) -> Option<(Priority, String)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let standalone = |a: usize, b: usize| {
        (a == 0 || chars[a - 1].is_whitespace()) && (b >= n || chars[b].is_whitespace())
    };
    let mut best: Option<Priority> = None;
    let mut spans: Vec<(usize, usize)> = Vec::new();

    // ! !! !!!
    let mut i = 0usize;
    while i < n {
        if chars[i] != '!' {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && chars[j] == '!' {
            j += 1;
        }
        let len = j - i;
        if (1..=3).contains(&len) && standalone(i, j) {
            let p = match len {
                3 => Priority::High,
                2 => Priority::Mid,
                _ => Priority::Low,
            };
            if best.map(|b| p.rank() < b.rank()).unwrap_or(true) {
                best = Some(p);
            }
            spans.push((i, j));
        }
        i = j;
    }

    // P1 / P2 / P3
    let mut i = 0usize;
    while i < n {
        let is_p = chars[i] == 'p' || chars[i] == 'P';
        if !is_p || i + 1 >= n {
            i += 1;
            continue;
        }
        let d = chars[i + 1];
        let rank = match d {
            '1' => Some(Priority::High),
            '2' => Some(Priority::Mid),
            '3' => Some(Priority::Low),
            _ => None,
        };
        match rank {
            Some(p) if standalone(i, i + 2) => {
                if best.map(|b| p.rank() < b.rank()).unwrap_or(true) {
                    best = Some(p);
                }
                spans.push((i, i + 2));
                i += 2;
            }
            _ => i += 1,
        }
    }

    let best = best?;
    // 从后往前摘除，避免前面的摘除让后面的下标失效
    spans.sort_unstable();
    let mut chars2 = chars.clone();
    for (a, b) in spans.into_iter().rev() {
        chars2.drain(a..b);
    }
    Some((best, chars2.into_iter().collect()))
}

/// 摘除 [a, b) 这段字符，其余以空格相连
fn splice(chars: &[char], a: usize, b: usize) -> String {
    format!(
        "{} {}",
        chars[..a].iter().collect::<String>(),
        chars[b..].iter().collect::<String>()
    )
}

type FindFn = fn(&str, NaiveDate) -> Option<(NaiveDate, String)>;

/// 全写日期 —— `2026-10-20`、`2026/10/20`、`2026.10.20`、`2026年10月20日`（`号` 同）
///
/// 为什么必须支持：从别的软件/表格里复制一行日期，或者自己敲完整年月日，
/// 都是最常见的输入。此前 `2026-10-20` 一个都不认（`M-D` 扫描器把 4 位年
/// 当成"月"直接判非法，然后整串落空）—— 用户视角就是"我写了日期它没看见"，
/// 顶多把后面的 `09:30` 认成时间、到期日留空。
///
/// 三条边界，避免与既有规则撞车：
///   · **只有 4 位年份才启动**（1900..=2999），所以 `3.5折`、`v1.2`、`10-20`
///     这些仍然走各自的老路径，不误判；
///   · 分隔符允许 `年`/`-`/`/`/`.` 与 `月`/`-`/`/`/`.`，日尾可选 `日`/`号`；
///     不要求两段一致（`2026年10-20` 也算，代价极低）；
///   · 年份**按字面取**，不做 `M-D` 那套"过期顺延一年"——用户已经写了年份，
///     再推断就是擅自改他的意思。日历合法性由 `from_ymd_opt` 兜（`2031-02-30`
///     不认，继续交给后面的规则）。
///
/// 刻意**不**支持 `20261020`（8 位无分隔）：那种串与编号/卡号无法区分，
/// 误判会吃掉用户标题里的数字。
fn find_ymd_date(text: &str, _today: NaiveDate) -> Option<(NaiveDate, String)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // 年：必须正好 4 位。长数字串（含 8 位紧凑日期）整串跳过。
        let (year, after_y) = match read_upto(&chars, i, 4) {
            Some(v) => v,
            None => {
                while i < n && chars[i].is_ascii_digit() {
                    i += 1;
                }
                continue;
            }
        };
        if !(1900..=2999).contains(&year) || after_y >= n {
            i = after_y.max(i + 1);
            continue;
        }
        if !matches!(chars[after_y], '年' | '-' | '/' | '.') {
            i = after_y;
            continue;
        }
        let (month, after_m) = match read_upto(&chars, after_y + 1, 2) {
            Some(v) => v,
            None => {
                i = after_y + 1;
                continue;
            }
        };
        if after_m >= n || !matches!(chars[after_m], '月' | '-' | '/' | '.') {
            i = after_m;
            continue;
        }
        let (day, after_d) = match read_upto(&chars, after_m + 1, 2) {
            Some(v) => v,
            None => {
                i = after_m + 1;
                continue;
            }
        };
        // 尾随 日/号
        let mut end = after_d;
        if end < n && (chars[end] == '日' || chars[end] == '号') {
            end += 1;
        }
        if let Some(date) = NaiveDate::from_ymd_opt(year as i32, month, day) {
            let rest = format!(
                "{} {}",
                chars[..i].iter().collect::<String>(),
                chars[end..].iter().collect::<String>()
            );
            return Some((date, rest));
        }
        i = end.max(i + 1);
    }
    None
}

/// M月D日 / M-D / M.D —— 扫描串中第一处合法日期
fn find_md_date(text: &str, today: NaiveDate) -> Option<(NaiveDate, String)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // 读月数字
        let mut num = String::new();
        let mut j = i;
        while j < n && chars[j].is_ascii_digit() {
            num.push(chars[j]);
            j += 1;
        }
        let month: i32 = match num.parse() {
            Ok(m) => m,
            Err(_) => {
                i = j;
                continue;
            }
        };
        // 分隔符：月 或 -
        // 注意：不支持 "M.D"，因为与小数（3.5折、v1.2）无法区分，误判代价高
        let sep_ok = if j < n && chars[j] == '月' {
            j += 1;
            true
        } else if j < n && chars[j] == '-' {
            j += 1;
            true
        } else {
            false
        };
        if !sep_ok {
            i = j;
            continue;
        }
        // 读日数字
        let mut d = String::new();
        while j < n && chars[j].is_ascii_digit() {
            d.push(chars[j]);
            j += 1;
        }
        if d.is_empty() {
            i = j;
            continue;
        }
        let day: i32 = match d.parse() {
            Ok(v) => v,
            Err(_) => {
                i = j;
                continue;
            }
        };
        // 尾随 日/号
        if j < n && (chars[j] == '日' || chars[j] == '号') {
            j += 1;
        }
        if let Some(date) = resolve(month, day, today) {
            let rest = format!(
                "{} {}",
                chars[..i].iter().collect::<String>(),
                chars[j..].iter().collect::<String>()
            );
            return Some((date, rest));
        }
        i = j;
    }
    None
}

/// 今天/明天/后天/大后天 —— 命中即摘除
fn find_relative_day(text: &str, today: NaiveDate) -> Option<(NaiveDate, String)> {
    let offsets: [(&str, i64); 4] = [("大后天", 3), ("后天", 2), ("明天", 1), ("今天", 0)];
    for (kw, off) in offsets {
        if let Some(pos) = text.find(kw) {
            let date = today + Duration::days(off);
            let rest = format!("{} {}", &text[..pos], &text[pos + kw.len()..]);
            return Some((date, rest));
        }
    }
    None
}

/// 周X / 下周X / 周天 —— 命中即摘除
fn find_weekday(text: &str, today: NaiveDate) -> Option<(NaiveDate, String)> {
    let names = [
        ('一', 0), ('二', 1), ('三', 2), ('四', 3), ('五', 4), ('六', 5), ('日', 6), ('天', 6),
    ];
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    for i in 0..n {
        // 找 "周" 或 "下周" / "下个月周X" 简化为 "周X" 与 "下周X"
        let (start, next_week) = if chars[i] == '周' {
            (i, false)
        } else if chars[i] == '下' && i + 1 < n && chars[i + 1] == '周' {
            (i, true)
        } else if chars[i] == '下' && i + 2 < n && chars[i + 1] == '个' && chars[i + 2] == '周' {
            (i, true)
        } else {
            continue;
        };
        let zhou_idx = if chars[start] == '下' {
            // 下(个)?周
            if chars[start + 1] == '个' {
                start + 2
            } else {
                start + 1
            }
        } else {
            start
        };
        // zhou_idx 指向 "周"
        let after = zhou_idx + 1;
        if after >= n {
            continue;
        }
        let target = names.iter().find(|(c, _)| *c == chars[after]).map(|(_, v)| *v);
        if let Some(target) = target {
            let cur = today.weekday().num_days_from_monday() as i64;
            let diff = if next_week {
                (7 - cur) + target as i64
            } else {
                let d = target as i64 - cur;
                // ★ 当天正是周X → **就排今天**（与滴答一致）。以前 `d <= 0`
                // 一律 +7，配合下面那句"0 也算下周"，周五打「周五 复盘」
                // 得到的是**下周五**——用户以为识别错了。已过才顺延下周。
                if d < 0 {
                    d + 7
                } else {
                    d
                }
            };
            let date = today + Duration::days(diff);
            let rest = format!(
                "{} {}",
                chars[..start].iter().collect::<String>(),
                chars[after + 1..].iter().collect::<String>()
            );
            return Some((date, rest));
        }
    }
    None
}

fn resolve(month: i32, day: i32, today: NaiveDate) -> Option<NaiveDate> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = today.year();
    let candidate = NaiveDate::from_ymd_opt(year, month as u32, day as u32);
    match candidate {
        Some(d) if d >= today => Some(d),
        Some(d) => NaiveDate::from_ymd_opt(year + 1, month as u32, day as u32).or(Some(d)),
        None => None,
    }
}

fn cleanup(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_prefix() {
        let (t, tags, _) = parse("[工作][紧急] 写周报");
        assert_eq!(t, "写周报");
        assert_eq!(tags, vec!["工作", "紧急"]);
    }

    #[test]
    fn priority_bangs() {
        // 独立成词的 ! !! !!!
        let p = parse_full("!!! 修水管");
        assert_eq!(p.priority, Some(Priority::High));
        assert_eq!(p.title, "修水管");
        let p = parse_full("!! 修水管");
        assert_eq!(p.priority, Some(Priority::Mid));
        assert_eq!(p.title, "修水管");
        let p = parse_full("! 修水管");
        assert_eq!(p.priority, Some(Priority::Low));
        assert_eq!(p.title, "修水管");
        // 尾部也能识别
        let p = parse_full("写周报 !!!");
        assert_eq!(p.priority, Some(Priority::High));
        assert_eq!(p.title, "写周报");
    }

    #[test]
    fn priority_p_codes() {
        for (s, want) in [
            ("P1 上线", Priority::High),
            ("p2 复盘", Priority::Mid),
            ("P3 整理", Priority::Low),
        ] {
            let p = parse_full(s);
            assert_eq!(p.priority, Some(want), "{s}");
        }
    }

    #[test]
    fn priority_never_misfires_on_text() {
        // 这些都不该被当成优先级 —— 误判会吃掉用户敲进去的字符
        for s in [
            "买3个!",        // 紧贴词尾
            "Hello!",         // 英文句尾
            "太好了！",        // 全角感叹号
            "a != b",         // 代码
            "1!2!3",          // 阶乘/粘连
            "解释!!!一下",     // 前后都不是空白
            "!!不是独立词!!",  // 紧贴
        ] {
            let p = parse_full(s);
            assert_eq!(p.priority, None, "不该识别出优先级：{s} → {:?}", p);
        }
    }

    #[test]
    fn priority_coexists_with_tag_date_time() {
        let p = parse_full("[工作] !!! 明天 下午3点 开会");
        assert_eq!(p.priority, Some(Priority::High));
        assert_eq!(p.tags, vec!["工作"]);
        assert!(p.due.is_some(), "应识别出日期");
        assert_eq!(p.time.as_deref(), Some("15:00"));
        assert_eq!(p.title, "开会");
    }

    #[test]
    fn multiple_priority_tokens_take_highest() {
        let p = parse_full("! P1 综合");
        assert_eq!(p.priority, Some(Priority::High), "多旗标取最高");
        assert_eq!(p.title, "综合", "两个优先级 token 都该被摘掉");
    }

    #[test]
    fn no_priority_leaves_title_untouched() {
        let p = parse_full("普通任务");
        assert_eq!(p.priority, None);
        assert_eq!(p.title, "普通任务");
    }

    #[test]
    fn fullwidth_tag() {
        let (t, tags, _) = parse("［生活］买菜");
        assert_eq!(t, "买菜");
        assert_eq!(tags, vec!["生活"]);
    }

    #[test]
    fn md_date() {
        let p = parse_full("8月1日 体检");
        assert!(p.due.is_some());
        assert_eq!(p.title, "体检");
        let due = p.due.unwrap();
        assert_eq!(&due[5..7], "08");
        assert_eq!(&due[8..10], "01");
    }

    #[test]
    fn dash_date() {
        let p = parse_full("12-25 圣诞");
        assert_eq!(p.title, "圣诞");
        assert!(p.due.unwrap().ends_with("12-25"));
    }

    #[test]
    fn relative_days() {
        assert!(parse_full("明天交材料").due.is_some());
        assert!(parse_full("后天开会").due.is_some());
        assert!(parse_full("大后天走").due.is_some());
    }

    #[test]
    fn weekday() {
        let p = parse_full("周五复盘");
        assert_eq!(p.title, "复盘");
        assert!(p.due.is_some());
    }

    #[test]
    fn plain_text_untouched() {
        let (t, tags, due) = parse("普通一条待办");
        assert_eq!(t, "普通一条待办");
        assert!(tags.is_empty());
        assert!(due.is_none());
    }

    // ── 时间识别（对齐官网"自动识别日期、时间"）──────────

    #[test]
    fn hhmm_colon() {
        let p = parse_full("明天 9:30 站会");
        assert_eq!(p.time.as_deref(), Some("09:30"));
        assert!(p.due.is_some());
        assert_eq!(p.title, "站会");
    }

    #[test]
    fn fullwidth_colon_and_no_leading_zero() {
        let p = parse_full("下午3：05 面试");
        assert_eq!(p.time.as_deref(), Some("15:05"));
        assert_eq!(p.title, "面试");
    }

    #[test]
    fn cn_period_half() {
        let p = parse_full("晚上8点半 跑步");
        assert_eq!(p.time.as_deref(), Some("20:30"));
        assert_eq!(p.title, "跑步");
    }

    #[test]
    fn cn_period_minutes() {
        let p = parse_full("早上7点10分 交材料");
        assert_eq!(p.time.as_deref(), Some("07:10"));
        assert_eq!(p.title, "交材料");
    }

    #[test]
    fn noon_stays_12() {
        let p = parse_full("中午12点 吃饭");
        assert_eq!(p.time.as_deref(), Some("12:00"));
    }

    #[test]
    fn bare_hour_no_period() {
        // 无时段词时不臆测上下午，按字面值
        let p = parse_full("3点 开会");
        assert_eq!(p.time.as_deref(), Some("03:00"));
    }

    #[test]
    fn date_and_time_together() {
        let p = parse_full("[工作] 8月1日 14:00 评审");
        assert_eq!(p.tags, vec!["工作"]);
        assert_eq!(&p.due.unwrap()[5..], "08-01");
        assert_eq!(p.time.as_deref(), Some("14:00"));
        assert_eq!(p.title, "评审");
    }

    #[test]
    fn extract_tags_marks_without_stripping() {
        // 便签语义：标签要抽出来，但正文一个字都不能少
        let t = "买牛奶 [紧急] 记得带袋子";
        assert_eq!(extract_tags(t), vec!["紧急"]);
        assert!(t.contains("[紧急]"), "正文不应被改动");
    }

    #[test]
    fn extract_tags_handles_multiple_and_fullwidth() {
        assert_eq!(extract_tags("[a] x ［b］ y [a]"), vec!["a", "b"]); // 去重保序
        assert!(extract_tags("没有标签的一段话").is_empty());
        assert!(extract_tags("未闭合 [标签").is_empty());
        assert!(extract_tags("空 [] 标签").is_empty());
        assert!(extract_tags("跨行 [\n多行] 不算").is_empty());
    }

    #[test]
    fn times_are_not_dates_and_vice_versa() {
        // "9:30" 不能被日期扫描器当成 9月30日
        let p = parse_full("9:30 打卡");
        assert!(p.due.is_none());
        assert_eq!(p.time.as_deref(), Some("09:30"));
        assert_eq!(p.title, "打卡");
    }

    #[test]
    fn bogus_times_rejected() {
        assert_eq!(parse_full("25:00 怪").time, None);
        assert_eq!(parse_full("9:75 怪").time, None);
        assert_eq!(parse_full("13点 怪").time, None); // 12 小时制上限
        assert_eq!(parse_full("2026年 怪").time, None); // 年份不当小时
    }

    #[test]
    fn numbers_that_are_not_times() {
        // 裸数字没有 点/:/： 锚，不能误判
        assert_eq!(parse_full("打 3.5折 买入").time, None);
        assert_eq!(parse_full("买 5个 苹果").time, None);
        assert_eq!(parse_full("升级 v1.2 版本").time, None);
    }

    #[test]
    fn date_in_middle_is_extracted() {
        // 重写扫描器的初衷：日期在任意位置都能识别
        let p = parse_full("交周报 8月1日");
        assert!(p.due.is_some());
        assert_eq!(&p.due.unwrap()[5..], "08-01");
        assert_eq!(p.title, "交周报");
    }

    #[test]
    fn tag_then_date_then_title() {
        let p = parse_full("[工作] 明天 评审");
        assert_eq!(p.tags, vec!["工作"]);
        assert!(p.due.is_some());
        assert_eq!(p.title, "评审");
    }

    #[test]
    fn weekday_in_middle() {
        let p = parse_full("周五 复盘");
        assert_eq!(p.title, "复盘");
        assert!(p.due.is_some());
    }

    #[test]
    fn decimals_are_not_dates() {
        // "M.D" 已不支持：与小数（3.5折、v1.2）无法区分，误判代价高
        let p = parse_full("打 3.5折 买入");
        assert!(p.due.is_none(), "小数不应被识别为日期，实得 {:?}", p.due);
        let v = parse_full("升级 v1.2 版本");
        assert!(v.due.is_none(), "版本号不应被识别为日期，实得 {:?}", v.due);
    }

    #[test]
    fn year_rollover() {
        // 1月1日：03-02 应落在今年；而 12-30 因早于今天应顺延明年
        let p = parse_full("12-30 年终");
        if let Some(d) = p.due {
            let y: i32 = d[..4].parse().unwrap();
            assert!(y >= Local::now().year());
        }
    }

    // ── 全写日期（从别的软件复制一行日期 / 自己敲完整年月日）──────

    #[test]
    fn ymd_full_dates() {
        let cases = [
            ("2031-10-20 开会", "开会"),
            ("2031/10/20 开会", "开会"),
            ("2031.10.20 开会", "开会"),
            ("2031年10月20日 开会", "开会"),
            ("2031年10月20号开会", "开会"),
            ("交材料 2031-10-20", "交材料"),
        ];
        for (input, want_title) in cases {
            let p = parse_full(input);
            assert_eq!(p.due.as_deref(), Some("2031-10-20"), "{input} → {:?}", p.due);
            assert_eq!(p.title, want_title, "{input} 标题应剩「{want_title}」，实得 {:?}", p.title);
        }
    }

    #[test]
    fn ymd_with_tag_and_time() {
        // 与标签、时间、优先级共存（这是真实输入的常态组合）
        let p = parse_full("[工作] 2031-10-20 09:30 评审");
        assert_eq!(p.tags, vec!["工作"]);
        assert_eq!(p.due.as_deref(), Some("2031-10-20"));
        assert_eq!(p.time.as_deref(), Some("09:30"));
        assert_eq!(p.title, "评审");
    }

    #[test]
    fn ymd_year_is_taken_literally() {
        // 用户写了年份就按字面取，**不做** M-D 那套"过期顺延一年"
        let p = parse_full("1999-01-01 历史事件");
        assert_eq!(p.due.as_deref(), Some("1999-01-01"), "过去的全写日期照字面认");
    }

    #[test]
    fn ymd_rejects_illegal_and_ambiguous() {
        // 非法日历日不认（也不能被后面的 M-D 规则歪打正着）
        assert_eq!(parse_full("2031-02-30 怪").due, None);
        assert_eq!(parse_full("2031-13-40 怪").due, None);
        // 8 位紧凑日期刻意不支持：与编号/卡号无法区分，误判会吃掉标题里的数字
        assert_eq!(parse_full("单号 20311020 核对").due, None);
        assert_eq!(parse_full("单号 20311020 核对").time, None);
        // 版本号/小数照旧不受影响
        assert_eq!(parse_full("升级 v1.2 版本").due, None);
        assert_eq!(parse_full("打 3.5折 买入").due, None);
        // 没年份的 M-D 仍走老规则（含过期顺延一年）
        assert!(parse_full("12-25 圣诞").due.unwrap().ends_with("12-25"));
    }

    #[test]
    fn weekday_today_resolves_to_today() {
        // ★ 与滴答一致：当天正是周五时，「周五 复盘」= **今天**。
        // 以前 d<=0 一律 +7，周五打「周五」得到下周五，用户以为识别错了。
        let today = Local::now().date_naive();
        let names = ['一', '二', '三', '四', '五', '六', '日'];
        let name = names[today.weekday().num_days_from_monday() as usize];
        let want = fmt_date(today);
        let p = parse_full(&format!("周{name} 复盘"));
        assert_eq!(p.due.as_deref(), Some(want.as_str()), "周{name} 在今天该排今天");
        assert_eq!(p.title, "复盘");
        // 「下周X」不受这条影响（见下一条）
    }

    #[test]
    fn weekday_already_passed_rolls_forward() {
        // 只有**已经过去**的星期才顺延到下周
        let today = Local::now().date_naive();
        let cur = today.weekday().num_days_from_monday() as i64;
        let names = ['一', '二', '三', '四', '五', '六', '日'];
        for target in 0..7i64 {
            let p = parse_full(&format!("周{} 会", names[target as usize]));
            let want = if target >= cur {
                today + Duration::days(target - cur)
            } else {
                today + Duration::days(7 - cur + target)
            };
            let want = fmt_date(want);
            assert_eq!(p.due.as_deref(), Some(want.as_str()), "今天周{}，输入周{}", cur + 1, target + 1);
        }
    }

    #[test]
    fn next_week_always_at_least_seven_days_ahead() {
        let today = Local::now().date_naive();
        let cur = today.weekday().num_days_from_monday() as i64;
        let names = ['一', '二', '三', '四', '五', '六', '日'];
        for target in 0..7i64 {
            let p = parse_full(&format!("下周{} 会", names[target as usize]));
            let want = fmt_date(today + Duration::days((7 - cur) + target));
            assert_eq!(p.due.as_deref(), Some(want.as_str()));
        }
    }
}
