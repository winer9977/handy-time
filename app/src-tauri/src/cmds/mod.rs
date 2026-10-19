//! #[tauri::command] 层：前端唯一入口
//!
//! 约定：命令一律 `&AppState` 注入，返回 `Result<T, String>`（错误消息为中文，
//! 直接给 UI 展示）。写操作成功后广播 `xhy://changed`，负载指明变更范围，
//! 各挂件窗口按需局部刷新。

use crate::domain::{Countdown, Event, Note, Recurrence, Settings, Store, Todo, TodoList, Widget};
use crate::state::{AppState, SettingsPatch};
use tauri::Emitter;

const EV_CHANGED: &str = "xhy://changed";

fn emit(app: &tauri::AppHandle, scope: &str) {
    let _ = app.emit(EV_CHANGED, scope);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub store: Store,
    pub hidden_all: bool,
    pub calendar_view: String,
    pub calendar_focus: String,
}

#[tauri::command]
pub fn snapshot(state: tauri::State<AppState>) -> Result<Snapshot, String> {
    Ok(Snapshot {
        store: state.snapshot(),
        hidden_all: *state.hidden_all.read(),
        calendar_view: serde_json::to_string(&*state.calendar_view.read())
            .unwrap_or_else(|_| "\"month\"".into())
            .trim_matches('"')
            .to_string(),
        calendar_focus: state.calendar_focus.read().clone(),
    })
}

// ── 清单 ──────────────────────────────────────────────────
#[tauri::command]
pub fn add_list(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    name: String,
    color: String,
) -> Result<TodoList, String> {
    let r = state.add_list(name, color)?;
    emit(&app, "lists");
    Ok(r)
}

#[tauri::command]
pub fn update_list(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    list: TodoList,
) -> Result<(), String> {
    state.update_list(list)?;
    emit(&app, "lists");
    Ok(())
}

#[tauri::command]
pub fn remove_list(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.remove_list(id)?;
    emit(&app, "lists");
    Ok(())
}

/// 归档 / 取消归档清单。归档后不在侧栏主区显示，其待办也不进活动列表，
/// 但数据保留（可在「已归档」区恢复）。对照滴答的清单归档。
#[tauri::command]
pub fn set_list_archived(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    archived: bool,
) -> Result<(), String> {
    state.set_list_archived(&id, archived)?;
    emit(&app, "lists");
    Ok(())
}

/// 设置某清单是否**不参与统计**：周/月总结与分析页会跳过它的待办与日程。
/// 用户的典型诉求："我有多个清单，其中一两个不想算进总结/完成率"。
#[tauri::command]
pub fn set_list_exclude_stats(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    exclude: bool,
) -> Result<(), String> {
    state.set_list_exclude_stats(&id, exclude)?;
    emit(&app, "lists");
    Ok(())
}

#[tauri::command]
pub fn reorder_lists(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    ids: Vec<String>,
) -> Result<(), String> {
    state.reorder_lists(ids)?;
    emit(&app, "lists");
    Ok(())
}

// ── 待办 ──────────────────────────────────────────────────
#[tauri::command]
pub fn add_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    list_id: String,
    title: String,
) -> Result<Todo, String> {
    let r = state.add_todo(list_id, title)?;
    emit(&app, "todos");
    Ok(r)
}

/// 整对象写入：**仅保留给导入/迁移**。日常改字段用 patch_todo（见其注释）。
#[tauri::command]
pub fn update_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    todo: Todo,
) -> Result<(), String> {
    state.update_todo(todo)?;
    emit(&app, "todos");
    Ok(())
}

/// **字段级补丁**：只改给出的字段，避免前端陈旧快照抹掉并发写者的改动
/// （提醒线程置 reminded、set_done 写 done_at、重复规则生成新行……）。
#[tauri::command]
pub fn patch_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    patch: crate::domain::TodoPatch,
) -> Result<(), String> {
    state.patch_todo(&id, patch)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn remove_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.remove_todo(id)?;
    emit(&app, "todos");
    Ok(())
}

// ── 回收站（对照滴答「垃圾桶」）──────────────────────────
#[tauri::command]
pub fn trash_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.trash_todo(id)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn restore_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<Option<Todo>, String> {
    let t = state.restore_todo(id)?;
    emit(&app, "todos");
    Ok(t)
}

#[tauri::command]
pub fn purge_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.purge_todo(id)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn empty_trash(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<usize, String> {
    let n = state.empty_trash()?;
    emit(&app, "todos");
    Ok(n)
}

#[tauri::command]
pub fn set_done(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    done: bool,
) -> Result<(), String> {
    state.set_done(id, done)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn toggle_pin(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.toggle_pin(id)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn reorder_todo(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    moved_id: String,
    target_id: Option<String>,
    after: bool,
) -> Result<(), String> {
    state.reorder_todo(moved_id, target_id, after)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn set_countdown(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    countdown: Option<Countdown>,
) -> Result<(), String> {
    state.set_countdown(id, countdown)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn set_priority(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    priority: Option<crate::domain::Priority>,
) -> Result<(), String> {
    state.set_priority(id, priority)?;
    emit(&app, "todos");
    Ok(())
}

#[tauri::command]
pub fn set_recurrence(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    recurrence: Option<Recurrence>,
) -> Result<(), String> {
    state.set_recurrence(id, recurrence)?;
    emit(&app, "todos");
    Ok(())
}

// ── 日程 ──────────────────────────────────────────────────
#[tauri::command]
pub fn add_event(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    event: Event,
) -> Result<Event, String> {
    let r = state.add_event(event)?;
    emit(&app, "events");
    Ok(r)
}

#[tauri::command]
pub fn update_event(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    event: Event,
) -> Result<(), String> {
    state.update_event(event)?;
    emit(&app, "events");
    Ok(())
}

#[tauri::command]
pub fn remove_event(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.remove_event(id)?;
    emit(&app, "events");
    Ok(())
}

/// 拖待办到日历格
#[tauri::command]
pub fn todo_to_event(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    todo_id: String,
    date: String,
) -> Result<Event, String> {
    let r = state.todo_to_event(todo_id, date)?;
    emit(&app, "all");
    Ok(r)
}

#[tauri::command]
pub fn move_event(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    event_id: String,
    date: String,
) -> Result<(), String> {
    state.move_event(event_id, date)?;
    emit(&app, "events");
    Ok(())
}

// ── 便签 ──────────────────────────────────────────────────
#[tauri::command]
pub fn add_note(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    body: String,
) -> Result<Note, String> {
    let r = state.add_note(body)?;
    emit(&app, "notes");
    Ok(r)
}

#[tauri::command]
pub fn update_note(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    note: Note,
) -> Result<(), String> {
    state.update_note(note)?;
    emit(&app, "notes");
    Ok(())
}

#[tauri::command]
pub fn remove_note(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.remove_note(id)?;
    emit(&app, "notes");
    Ok(())
}

#[tauri::command]
pub fn toggle_note_pin(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.toggle_note_pin(id)?;
    emit(&app, "notes");
    Ok(())
}

#[tauri::command]
pub fn reorder_note(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    target_id: Option<String>,
    after: bool,
) -> Result<(), String> {
    state.reorder_note(id, target_id, after)?;
    emit(&app, "notes");
    Ok(())
}

// ── 挂件与设置 ────────────────────────────────────────────
/// 整对象写入：仅用于**新建**挂件（设置页添加挂件时）。
/// 改已有挂件请用 patch_widget —— 整对象覆盖会把别人刚改的字段抹掉。
#[tauri::command]
pub fn upsert_widget(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    widget: Widget,
) -> Result<(), String> {
    state.upsert_widget(widget)?;
    emit(&app, "widgets");
    Ok(())
}

/// **字段级补丁**（架构修正）：只改 patch 里给出的字段。
///
/// 此前所有挂件写者（显隐、尺寸、筛选、标签、贴边、清单绑定）都发
/// "整个 Widget 快照"覆盖服务端，两个写者并发时后写的把先写的抹掉
/// （lost update）—— 这是"同一类错误反复出现"的根因。改为补丁后，
/// 各写者只碰自己那一个字段，互不影响。
#[tauri::command]
pub fn patch_widget(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    patch: crate::domain::WidgetPatch,
) -> Result<(), String> {
    state.patch_widget(&id, patch)?;
    emit(&app, "widgets");
    Ok(())
}

#[tauri::command]
pub fn remove_widget(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<(), String> {
    state.remove_widget(id)?;
    emit(&app, "widgets");
    Ok(())
}

#[tauri::command]
pub fn set_settings(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let r = state.set_settings(patch)?;
    emit(&app, "settings");
    Ok(r)
}

// ── 导入导出 ──────────────────────────────────────────────
#[tauri::command]
pub fn export_data(state: tauri::State<AppState>) -> Result<String, String> {
    Ok(state.export())
}

/// 导出为 Markdown（给人读/贴笔记软件；完整备份请用 export_data 的 JSON）
#[tauri::command]
pub fn export_markdown(state: tauri::State<AppState>) -> Result<String, String> {
    let g = state.snapshot();
    Ok(crate::export::to_markdown(&g))
}

/// 弹"另存为"对话框导出 Markdown（设置页入口；托盘菜单走同一实现）
#[tauri::command]
pub fn save_markdown_file(app: tauri::AppHandle) -> Result<(), String> {
    crate::widgets::save_markdown(&app);
    Ok(())
}

#[tauri::command]
pub fn import_data(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    json: String,
) -> Result<(), String> {
    state.import(&json)?;
    emit(&app, "all");
    Ok(())
}

/// 导入 Markdown 待办（**合并**：同名清单归并、条目按标题+到期日去重）。
/// 与 `import_data`（JSON 整库替换）刻意是两条路，避免"导入 md 把库洗了"。
/// 返回摘要给界面显示；设置页的选文件入口走 `import_markdown_file`。
#[tauri::command]
pub fn import_markdown(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    md: String,
    list_name: Option<String>,
) -> Result<String, String> {
    let fb = list_name.unwrap_or_else(|| "导入待办".into());
    let st = state.import_markdown(&md, &fb)?;
    emit(&app, "todos");
    Ok(format!(
        "新增 {} 条，跳过重复 {} 条，新建清单 {} 个{}",
        st.added,
        st.dup,
        st.lists,
        if st.empty > 0 { format!("，忽略空条目 {} 条", st.empty) } else { String::new() }
    ))
}

/// 弹"打开"对话框选一个 .md 再导入（前端没装 dialog 的 JS 包，选文件只能在 Rust 侧）
#[tauri::command]
pub fn import_markdown_file(app: tauri::AppHandle) -> Result<(), String> {
    crate::widgets::import_md(&app);
    Ok(())
}

/// 一句话智能解析预览（与 add_todo 走同一个解析器，前后端零规则漂移）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsePreview {
    pub title: String,
    pub tags: Vec<String>,
    pub due: Option<String>,
    /// 识别到的时间 "HH:MM"
    pub time: Option<String>,
}

#[tauri::command]
pub fn quick_parse(text: String) -> Result<ParsePreview, String> {
    let p = crate::quickparse::parse_full(&text);
    Ok(ParsePreview {
        title: p.title,
        tags: p.tags,
        due: p.due,
        time: p.time,
    })
}

/// 前端日志桥：把 webview 侧的关键事件写进同一个 xhy.log
///
/// 跨窗口拖拽的断点可能在 Rust（广播）也可能在前端（坐标换算/命中），
/// 只有一侧日志等于没有日志。前端刻意做"宁可不投也不猜"的静默分支，
/// 更需要这里留痕。
#[tauri::command]
pub fn log_line(source: String, msg: String) -> Result<(), String> {
    crate::log::line(&format!("[{source}] {msg}"));
    Ok(())
}

/// 打开数据目录（便于用户自行备份/排查）
#[tauri::command]
pub fn data_dir(state: tauri::State<AppState>) -> Result<String, String> {
    Ok(state.db.root().to_string_lossy().to_string())
}

/// 区间内日程，**含重复日程的虚拟展开**。
///
/// 日历按可见窗口（月 42 格 / 周 7 天 / 日 1 天）调这个命令取数，
/// 而不是在前端过滤快照里的原始 events —— 那样重复规则要在 TS 再实现一遍，
/// 两份规则必然漂移。展开只有一份，在 `state::expand_events`。
#[tauri::command]
pub fn events_in_range(
    state: tauri::State<AppState>,
    from: String,
    to: String,
    list: Option<String>,
) -> Result<Vec<Event>, String> {
    Ok(state.events_in_range(&from, &to, list.as_deref()))
}

/// 区间内每天的农历标签（前端日历渲染用）。
/// 无标签的日子不返回；转换失败的天也跳过，绝不返回猜测值。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LunarInfo {
    pub date: String,
    pub label: String,
}

/// 区间查询的天数上限。参数来自 IPC（前端 localStorage 的视图态可被改），
/// 不设上限时 `from=1900-01-01, to=2999-12-31` 会循环 40 万次并返回巨大数组
/// —— 应用直接卡死。日历最长视图是"年"，366 天绰绰有余；给足余量到 800 天。
const RANGE_DAYS_CAP: i64 = 800;

/// 把 IPC 传来的区间规整成安全值：解析失败或顺序颠倒都返回 None（调用方返回空），
/// 超长区间则截断到上限，避免无界循环。
fn clamp_range(from: &str, to: &str) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
    use chrono::NaiveDate;
    let a = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let b = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    if b < a {
        return None; // 顺序颠倒：宁可返回空，也不静默给出错误结果
    }
    let end = if (b - a).num_days() + 1 > RANGE_DAYS_CAP {
        a + chrono::Duration::days(RANGE_DAYS_CAP - 1)
    } else {
        b
    };
    Some((a, end))
}

#[tauri::command]
pub fn lunar_in_range(from: String, to: String) -> Result<Vec<LunarInfo>, String> {
    use chrono::Duration;
    let Some((mut d, end)) = clamp_range(&from, &to) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    while d <= end {
        if let Some(label) = crate::lunar::lunar_label(d) {
            out.push(LunarInfo {
                date: d.format("%Y-%m-%d").to_string(),
                label,
            });
        }
        d += Duration::days(1);
    }
    Ok(out)
}

/// 某日期区间内的法定节假日/调休（前端月历角标用）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HolidayInfo {
    pub date: String,
    pub name: String,
    /// true = 放假；false = 调休上班
    pub off: bool,
}

#[tauri::command]
pub fn holidays_in_range(from: String, to: String) -> Result<Vec<HolidayInfo>, String> {
    let table = crate::holidays::table();
    // 同 lunar_in_range：区间来自 IPC，必须限长（无界循环会让应用卡死）
    let Some((mut d, end)) = clamp_range(&from, &to) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    while d <= end {
        if let Some((name, off)) = table.get(d) {
            out.push(HolidayInfo {
                date: d.format("%Y-%m-%d").to_string(),
                name: name.clone(),
                off: *off,
            });
        }
        d += chrono::Duration::days(1);
    }
    Ok(out)
}

// ── 周/月总结（对照滴答清单的"回顾"）──────────────────────

/// 本地统计摘要。纯计算 + 读归档文件，快，同步命令即可。
#[tauri::command]
pub fn period_summary(
    state: tauri::State<AppState>,
    kind: String,
    list: Option<String>,
) -> Result<crate::stats::Summary, String> {
    Ok(state.period_summary(&kind, list.as_deref()))
}

/// 近 n 天完成趋势（分析页）。返回 (日期, 完成数) 升序。
/// 复用 stats::compute，口径与总结页一致，两处数字永不打架。
/// list 与同页分布同值（见 stats::trend 注释）。
#[tauri::command]
pub fn done_trend(
    state: tauri::State<AppState>,
    days: i64,
    list: Option<String>,
) -> Result<Vec<(String, usize)>, String> {
    Ok(state.trend(days, list.as_deref()))
}

/// AI 润色版总结。**必须 async**：同步命令跑在主线程，里面一个 25s 的
/// 阻塞 HTTP 会把整个 app 冻住（拖拽死锁就是这么来的，同一课）。
///
/// 没配 key 直接返回 Err —— 前端据此降级显示 `period_summary` 的本地摘要，
/// 那本来就是份能读的总结。
#[tauri::command]
pub async fn ai_summary(
    state: tauri::State<'_, AppState>,
    kind: String,
    list: Option<String>,
) -> Result<String, String> {
    let cfg = state.ai_config();
    if !crate::ai::ready(&cfg.base_url, &cfg.api_key, &cfg.model) {
        return Err("未配置 AI（需要 base URL / API Key / 模型名）".into());
    }
    // 先在锁内算完，再释放 State 借用去发请求。
    // list 一并传下去：AI 润色的必须是面板上正显示的那份数字，
    // 否则选"待办"清单却拿到全库摘要，文不对题。
    let summary = state.period_summary(&kind, list.as_deref());
    let body = crate::ai::build_body(&cfg.model, &summary.digest, &summary.label);
    let base = cfg.base_url.clone();
    let key = cfg.api_key.clone();
    // 阻塞式 ureq 挪到阻塞线程，别占着 async runtime 的工作线程
    let text = tauri::async_runtime::spawn_blocking(move || crate::ai::ask(&base, &key, &body))
        .await
        .map_err(|e| format!("任务调度失败：{e}"))??;
    crate::log::line(&format!(
        "AI 总结成功 kind={kind} len={}",
        text.chars().count()
    ));
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_range_rejects_bad_and_reversed() {
        // 参数来自 IPC（前端 localStorage 视图态可被改），坏值必须安全返回
        assert!(clamp_range("不是日期", "2026-01-01").is_none());
        assert!(clamp_range("2026-01-01", "坏值").is_none());
        // 顺序颠倒：宁可返回空，也不静默给出错误结果
        assert!(
            clamp_range("2026-12-31", "2026-01-01").is_none(),
            "from > to 应被拒（否则静默返回空，用户以为没数据）"
        );
    }

    #[test]
    fn clamp_range_caps_absurd_span() {
        // 无界循环防护：40 万天会让应用卡死
        let (a, b) = clamp_range("1900-01-01", "2999-12-31").expect("应被截断而非拒绝");
        assert_eq!(a.to_string(), "1900-01-01");
        assert_eq!(
            (b - a).num_days() + 1,
            RANGE_DAYS_CAP,
            "超长区间应截到上限"
        );
        // 正常区间原样透传
        let (c, d) = clamp_range("2026-10-01", "2026-10-31").unwrap();
        assert_eq!((d - c).num_days() + 1, 31);
    }

    #[test]
    fn lunar_and_holiday_ranges_are_bounded() {
        // 端到端确认两个命令在极端入参下也不会卡（返回量受上限约束）
        let n = lunar_in_range("1900-01-01".into(), "2999-12-31".into())
            .unwrap()
            .len();
        assert!(n as i64 <= RANGE_DAYS_CAP, "农历标签数不该超过天数上限，实得 {n}");
        let h = holidays_in_range("1900-01-01".into(), "2999-12-31".into())
            .unwrap()
            .len();
        assert!(h as i64 <= RANGE_DAYS_CAP, "节假日数不该超过天数上限，实得 {h}");
        // 坏入参返回空而非报错/panic
        assert!(lunar_in_range("x".into(), "y".into()).unwrap().is_empty());
        assert!(holidays_in_range("x".into(), "y".into()).unwrap().is_empty());
    }

    #[test]
    fn lunar_range_only_returns_inside_span() {
        // 正常区间：只返回区间内的天，不越界
        let v = lunar_in_range("2026-10-01".into(), "2026-10-03".into()).unwrap();
        assert!(v
            .iter()
            .all(|x| x.date.as_str() >= "2026-10-01" && x.date.as_str() <= "2026-10-03"));
        assert!(!v.is_empty(), "农历每天都该有标签");
    }
}
