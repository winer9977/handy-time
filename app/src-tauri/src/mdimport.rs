//! Markdown 待办导入（**纯解析**，不碰存储；落库合并见 `state::import_markdown`）
//!
//! 为什么要单独一个模块而不是塞进 `export.rs`：导出是"把结构写成给人看的文本"，
//! 导入是"把任意来源的文本猜回结构"，后者的规则全是边界判断，必须能单独测。
//!
//! 认三种排版（覆盖"我们自己的导出"与第三方待办工具的导出）：
//!
//! ```text
//! ## 📋 收集箱            → 清单分组：下面的条目属于「收集箱」
//! ## 2026-10-19           → 日期分组：下面的条目到期日都是这天（清单走兜底名）
//! ## 工作                 → 普通名字分组，同 📋
//! - [ ] 交周报 `月报` · 📅 2026-10-20 · 🔔 2026-10-20T09:30
//! - [x] 打电话沟通: 对方没接
//! ```
//!
//! 三条刻意取舍，都各有测试盯着：
//! 1. **字段分隔符用 ` · `**（中点 + 空格），与我们导出端逐字一致。标题里本来就带
//!    ` · ` 的条目会被拆开，但拆出来的非标记段**原样拼回去**，所以标题不会变形。
//! 2. **备注只在"半角冒号 + 空格"且冒号前不是数字**时才切。`12:00` 里也有冒号，
//!    按"第一个冒号"切会把时间咬掉；`约见教练: 没接上` 这种才是真的备注。全角 `：`
//!    一律不切（中文正文里"第一阶段：完成选材清单"的冒号属于标题的一部分）。
//! 3. **重复规则不还原**（`↻每周` 是给人看的文案，反推 `Recurrence` 结构容易猜错）。
//!    猜错比不猜更糟，所以原样搬进备注，让用户看得见、自己在界面上点。

/// 一条解析出来的待办
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdTodo {
    pub title: String,
    pub note: String,
    pub done: bool,
    /// `YYYY-MM-DD`；分组日期与行内 `📅` 取后者优先
    pub due: Option<String>,
    /// `YYYY-MM-DDTHH:MM`
    pub remind_at: Option<String>,
    pub done_at: Option<String>,
    pub tags: Vec<String>,
    /// 无法还原的标记原文（如 `↻每周`），落库时并进备注，不静默丢弃
    pub extra: Vec<String>,
}

/// 一个分组（`##` 标题 + 它下面的条目）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdGroup {
    /// 清单名；日期分组时为 None（由调用方给兜底清单）
    pub list: Option<String>,
    /// 该分组的日期；名字分组时为 None
    pub date: Option<String>,
    pub todos: Vec<MdTodo>,
}

/// `## 📋 收集箱` → `收集箱`；`## 2026-10-19` → 日期分组
fn header_kind(text: &str) -> MdGroup {
    let t = text.trim().trim_start_matches('#').trim();
    // 去掉我们导出时加的图标前缀（📋 清单 / 🗓️ 日程），剩下的才是名字
    let name = t
        .trim_start_matches('\u{1F4CB}')
        .trim_start_matches("\u{1F5D3}\u{FE0F}")
        .trim_start_matches('\u{1F5D3}')
        .trim_start_matches('\u{1F4C5}')
        .trim()
        .to_string();
    if is_date(&name) {
        return MdGroup { list: None, date: Some(name), todos: vec![] };
    }
    MdGroup { list: Some(name), date: None, todos: vec![] }
}

/// 严格 `YYYY-MM-DD`（月份/日期范围也查，避免把 `2026-99-99` 当日期）
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && s.bytes().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == b'-' } else { c.is_ascii_digit() })
        && s[5..7].parse::<u32>().map_or(false, |m| (1..=12).contains(&m))
        && s[8..10].parse::<u32>().map_or(false, |d| (1..=31).contains(&d))
}

/// `2026-10-19T09:30` / `09:30` 这种时间戳原样收下（合法性交给落库端与提醒调度器）
fn is_dt(s: &str) -> bool {
    s.len() >= 16 && is_date(&s[..10]) && s.as_bytes()[10] == b'T'
}

/// 剥出行内 `` `标签` ``，返回（去掉标签后的文本，标签集）
fn take_tags(text: &str) -> (String, Vec<String>) {
    let mut tags = Vec::new();
    let mut out = String::new();
    let mut rest = text;
    while let Some(st) = rest.find('`') {
        out.push_str(&rest[..st]);
        match rest[st + 1..].find('`') {
            Some(en) => {
                let tag = rest[st + 1..st + 1 + en].trim().to_string();
                if !tag.is_empty() && !tags.contains(&tag) {
                    tags.push(tag);
                }
                rest = &rest[st + 1 + en + 1..];
            }
            // 落单的引号：原样留着，别把后面的正文吃掉
            None => {
                out.push('`');
                rest = &rest[st + 1..];
            }
        }
    }
    out.push_str(rest);
    (out.trim().to_string(), tags)
}

/// 冒号前是数字就当时间（`12:00`），不当备注分隔
fn split_note(text: &str) -> (String, String) {
    let Some(i) = text.find(": ") else {
        return (text.trim().to_string(), String::new());
    };
    let before = &text[..i];
    let after = text[i + 2..].trim().to_string();
    let prev_is_digit = before.chars().last().map_or(false, |c| c.is_ascii_digit());
    if before.is_empty() || prev_is_digit || after.is_empty() {
        return (text.trim().to_string(), String::new());
    }
    (before.trim_end().to_string(), after)
}

/// 解析一行条目；不是 `- [ ]` / `- [x]` 形态就返回 None
fn parse_item(line: &str) -> Option<MdTodo> {
    let t = line.trim_start();
    let rest = t
        .strip_prefix("- [")
        .or_else(|| t.strip_prefix("* ["))
        .or_else(|| t.strip_prefix("+ ["))?;
    // 方括号里只有一个标记字符，且必须紧跟 `]`：`- [?] xx` 这种不是任务清单
    let mark = rest.chars().next()?;
    if !matches!(mark, ' ' | 'x' | 'X') {
        return None;
    }
    let after = rest[1..].strip_prefix(']')?;
    let body = after.trim_start();
    if body.is_empty() {
        return None;
    }
    let done = matches!(mark, 'x' | 'X');

    let mut todo = MdTodo {
        title: String::new(),
        note: String::new(),
        done,
        due: None,
        remind_at: None,
        done_at: None,
        tags: Vec::new(),
        extra: Vec::new(),
    };
    // 我们导出端的字段分隔符是 ` · `；第一段是标题（可能带 `` `tag` ``），
    // 后面每段要么是标记要么是标题里的中点 —— 认不出的拼回标题。
    let mut title_parts: Vec<String> = Vec::new();
    for seg in body.split(" · ") {
        let s = seg.trim();
        if let Some(v) = s.strip_prefix("📅").map(str::trim) {
            if is_date(v) {
                todo.due = Some(v.to_string());
                continue;
            }
        }
        if let Some(v) = s.strip_prefix("🔔").map(str::trim) {
            if is_dt(v) {
                todo.remind_at = Some(v.to_string());
                continue;
            }
        }
        if let Some(v) = s.strip_prefix("✅").map(str::trim) {
            if is_dt(v) {
                todo.done_at = Some(v.to_string());
                continue;
            }
        }
        if s.starts_with('↻') || s.starts_with("重复") {
            todo.extra.push(s.to_string());
            continue;
        }
        title_parts.push(seg.to_string());
    }
    let (untagged, tags) = take_tags(&title_parts.join(" · "));
    todo.tags = tags;
    let (title, note) = split_note(&untagged);
    todo.title = title;
    let mut notes: Vec<String> = Vec::new();
    if !note.is_empty() {
        notes.push(note);
    }
    notes.extend(todo.extra.drain(..));
    todo.note = notes.join("\n");
    if todo.title.is_empty() {
        return None;
    }
    Some(todo)
}

/// 解析结果：分组 + **被丢掉的空条目数**。
///
/// 空条目（`- [ ]` 后面没字）第三方导出里真会出现（实测就有一份末尾挂着一条空条目）。
/// 不建空待办是对的，但**静默丢掉**用户会以为"少了几条却没地方找"，
/// 所以计数一路带到导入摘要里显示出来。
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub groups: Vec<MdGroup>,
    pub empty_dropped: usize,
}

/// 像任务条目吗（`- [x]` / `* [ ]` / `+ [X]`）
fn looks_like_item(t: &str) -> bool {
    let Some(rest) = t
        .strip_prefix("- [")
        .or_else(|| t.strip_prefix("* ["))
        .or_else(|| t.strip_prefix("+ ["))
    else {
        return false;
    };
    matches!(rest.chars().next(), Some(' ') | Some('x') | Some('X')) && rest[1..].starts_with(']')
}

/// 解析整份 Markdown → 分组列表（空分组也会被保留，由落库端决定要不要建清单）
pub fn parse(md: &str) -> Parsed {
    // 别的工具（尤其 Windows 上的）常给 UTF-8 文件加 BOM；不剥掉的话
    // 第一个 `## 分组` 会被当成普通文本丢掉，整份导入少一组。
    let md = md.trim_start_matches('\u{feff}');
    let mut out = Parsed::default();
    let mut groups: Vec<MdGroup> = Vec::new();
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("##") {
            groups.push(header_kind(t));
            continue;
        }
        // 单个 `#` 是文档标题（`# xhy 导出数据`），不是分组
        if t.starts_with('#') || t.starts_with("> ") || t.is_empty() {
            continue;
        }
        if let Some(item) = parse_item(line) {
            if groups.is_empty() {
                // 没有任何 `##` 分组：全部进一个无名组（落库端用兜底清单）
                groups.push(MdGroup { list: None, date: None, todos: vec![] });
            }
            groups.last_mut().expect("刚保证非空").todos.push(item);
        } else if looks_like_item(t) {
            out.empty_dropped += 1;
        }
    }
    groups.retain(|g| !g.todos.is_empty());
    out.groups = groups;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_groups_become_due_and_name_groups_become_lists() {
        let md = "## 2026-08-18\n\n- [x] 交季度报表\n- [x] 约客户看方案\n\n## 2026-10-19\n\n- [ ] 出差上海\n";
        let g = parse(md).groups;
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].date.as_deref(), Some("2026-08-18"));
        assert_eq!(g[0].todos.len(), 2);
        assert!(g[0].todos.iter().all(|t| t.done), "[x] 都是已完成");
        assert_eq!(g[1].todos[0].due.as_deref(), None, "日期由分组给，不逐条重复写");
        assert!(!g[1].todos[0].done);
    }

    #[test]
    fn our_own_export_round_trips_fields() {
        // 与 export::to_markdown 的排版逐字对齐：` · ` 分隔 + 📅//✅/`` `tag` ``
        let md = "## 📋 工作\n\n- [ ] 交周报 `月报` `重要` · 📅 2026-10-20 · 🔔 2026-10-20T09:30\n\
                  - [x] 已完成的 · ✅ 2026-10-01T08:00\n";
        let g = parse(md).groups;
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].list.as_deref(), Some("工作"), "📋 前缀要剥掉");
        let a = &g[0].todos[0];
        assert_eq!(a.title, "交周报");
        assert_eq!(a.tags, vec!["月报", "重要"]);
        assert_eq!(a.due.as_deref(), Some("2026-10-20"));
        assert_eq!(a.remind_at.as_deref(), Some("2026-10-20T09:30"));
        let b = &g[0].todos[1];
        assert_eq!(b.title, "已完成的");
        assert_eq!(b.done_at.as_deref(), Some("2026-10-01T08:00"));
    }

    #[test]
    fn note_splits_on_halfwidth_colon_only_when_clearly_not_a_time() {
        let md = "## 2026-09-14\n\n- [x] 约健身教练: 本周时间冲突\n\
                  - [x] 周一下午 14:00 约见教练\n\
                  - [x] 装修方案 第一阶段：完成选材清单\n";
        let g = parse(md).groups;
        let todos = &g[0].todos;
        assert_eq!(todos[0].title, "约健身教练");
        assert_eq!(todos[0].note, "本周时间冲突");
        // 冒号前是数字 → 那是时间，不许从中间咬断
        assert_eq!(todos[1].title, "周一下午 14:00 约见教练");
        assert_eq!(todos[1].note, "");
        // 全角冒号一律不算分隔
        assert_eq!(todos[2].title, "装修方案 第一阶段：完成选材清单");
    }

    #[test]
    fn spaced_colon_and_title_with_middot_survive() {
        let md = "## 2026-07-03\n\n- [x] 整理调研报告 \"工具\" : 未被采用\n- [ ] A · B 这种标题\n";
        let g = parse(md).groups;
        assert_eq!(g[0].todos[0].title, "整理调研报告 \"工具\"");
        assert_eq!(g[0].todos[0].note, "未被采用");
        assert_eq!(g[0].todos[1].title, "A · B 这种标题", "认不出的 ` · ` 段要原样拼回标题");
    }

    #[test]
    fn recurrence_marker_is_preserved_in_note_instead_of_guessed() {
        let md = "## 2026-09-01\n\n- [ ] 交材料 · ↻每周\n";
        let g = parse(md).groups;
        assert_eq!(g[0].todos[0].title, "交材料");
        assert_eq!(g[0].todos[0].note, "↻每周", "重复规则不猜，搬进备注让用户看得见");
    }

    #[test]
    fn junk_and_headings_are_ignored_and_empty_groups_dropped() {
        let md = "# 导出数据\n\n> 导出于 2026-10-19\n\n## 🗓️ 日程\n\n- 09:30 评审 · 📅 2026-10-19\n\n\
                  普通段落，不是条目\n- 没有方括号的也不算\n- [ ] \n";
        assert!(parse(md).groups.is_empty(), "非条目行与空分组都不产出");
    }

    #[test]
    fn items_before_any_header_still_import() {
        let g = parse("- [ ] 冒烟：文件开头没有 ## 分组").groups;
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].list, None);
        assert_eq!(g[0].todos[0].title, "冒烟：文件开头没有 ## 分组");
    }

    #[test]
    fn empty_items_are_dropped_but_counted_not_silent() {
        // 第三方真实导出末尾就有一条 `- [ ] `（空标题）。不建空待办是对的，
        // 但必须把"丢了几条"报出来，否则用户只会觉得"我 41 条怎么少了 1 条"。
        let p = parse("## 2026-10-19\n\n- [ ] 有内容\n- [ ] \n- [x]\n");
        assert_eq!(p.groups[0].todos.len(), 1);
        assert_eq!(p.empty_dropped, 2, "两条空条目都要数得出来");
    }

    #[test]
    fn is_date_rejects_lookalikes() {
        assert!(is_date("2026-02-30"), "结构合法；日历合法性由落库端判");
        assert!(!is_date("2026-2-3"), "月/日必须补零");
        assert!(!is_date("20260-02-03"));
        assert!(!is_date("2026-13-01"), "月份越界");
        assert!(!is_date("a026-02-01"), "非数字位");
        assert!(!is_dt("2026-10-19 09:30"), "分隔符必须是 T");
    }
}
