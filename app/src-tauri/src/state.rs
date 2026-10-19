//! 应用状态：内存中的 Store + 派生索引
//!
//! 单一真源。前端所有写操作经 `cmds` 走这里；写成功即落盘并广播变更，
//! 挂件窗口按需局部刷新。读侧提供派生查询（今日待办、某日日程…），
//! 避免前端各自过滤导致口径不一。

use crate::db::JsonStore;
use crate::domain::{CalendarView, Event, Note, Store, Todo, TodoList, Widget};
use crate::{ids, now_local_date_str, now_local_str};
use chrono::{Local, NaiveDate};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    pub store: RwLock<Store>,
    pub db: Arc<JsonStore>,
    /// 挂件是否被"快速隐藏"整体收起
    pub hidden_all: RwLock<bool>,
    /// 当前使用的日历视图（跨窗口共享，切视图时广播）
    pub calendar_view: RwLock<CalendarView>,
    /// 日历聚焦日期
    pub calendar_focus: RwLock<String>,
}

/// 带到期日的待办 → 日历上的派生条目。
///
/// 用户写完"明天 交周报"期待日历上能看到，是自然预期；此前日历只读
/// `events` 表，待办再多也一片空白。
///
/// 刻意**派生而非落库**：不复制数据，就没有"改了待办忘了改日程"的同步问题，
/// 删掉待办日历上也就自动消失。
///
/// 去重：已被显式排进日历的待办（拖拽或「排到…」会建一条带 `todo_id` 的日程）
/// 不再派生，否则同一天出现两次。
fn todo_derived_events(todos: &[Todo], events: &[Event], from: &str, to: &str) -> Vec<Event> {
    use std::collections::HashSet;
    let linked: HashSet<&str> = events.iter().filter_map(|e| e.todo_id.as_deref()).collect();
    todos
        .iter()
        .filter(|t| !t.done)
        .filter(|t| !linked.contains(t.id.as_str()))
        .filter(|t| {
            t.due
                .as_deref()
                .map(|d| d >= from && d <= to)
                .unwrap_or(false)
        })
        .map(|t| Event {
            // 前缀区分派生条目，避免与真实日程 id 撞车；日视图双击改标题
            // 会作用到日程上，因此这里给的是只读语义的 id
            id: format!("todo:{}", t.id),
            date: t.due.clone().unwrap_or_default(),
            time: t
                .remind_at
                .as_deref()
                .and_then(|r| r.split_once('T'))
                .map(|(_, hhmm)| hhmm.to_string()),
            duration_min: 0,
            title: t.title.clone(),
            color: "#9aa0a6".into(),
            source: "todo".into(),
            todo_id: Some(t.id.clone()),
            span_days: 1,
            // 派生条目永远只占到期那一天（重复由待办本身推进，不在这展开）
            span_index: 0,
            span_total: 1,
            // 带上待办自己的重复规则。**不是**拿它展开的 —— 派生条目永远只
            // 落在 due 那一天（重复由待办本身推进），这里要它是因为：
            //   ① 日历上看得出"这条每天重复"（此前派生条目一律不显示 ↻）；
            //   ② 日视图的 ↻ 面板要能编辑待办的规则，而整对象回传时必须区分
            //      「这次没改规则」与「改成不重复」（见 update_event 的 todo: 分支）。
            recurrence: t.recurrence.clone(),
            list_id: Some(t.list_id.clone()),
        })
        .collect()
}

/// 已完成待办的 id 集合。
///
/// 日历一致性关键：拖进过日历的待办，其日程带 `todo_id`。若查询不看
/// 关联待办的完成态，就会出现"待办勾掉了、日历还挂着"——与派生条目
/// （完成即消失）行为不一致，用户视角就是"有的还在有的没了"。
/// 统一规则：**关联待办已完成的日程，日历上不显示**；取消完成即恢复。
fn done_todo_ids(todos: &[Todo]) -> std::collections::HashSet<&str> {
    todos.iter().filter(|t| t.done).map(|t| t.id.as_str()).collect()
}

/// 日历查询的统一入口：日程（含重复展开）+ 待办派生，且都剔除已完成项。
/// 派生日程的清单归属：**优先用日程自己记的 `list_id`**。
/// 为什么不能只靠 todo_id 反查待办：待办被删除（含软删进回收站）后
/// 反查必然落空，于是这条日程在任何清单下都不显示 —— 用户视角就是
/// "删一条待办，日历上别的清单的日程也没了"。母记录落库时把当时的
/// 清单写进去，清单被删则回落到待办的 list_id。
fn event_list(store: &Store, e: &Event) -> Option<String> {
    if let Some(l) = e.list_id.as_deref() {
        return Some(l.to_string());
    }
    e.todo_id
        .as_deref()
        .and_then(|tid| store.todos.iter().find(|t| t.id == tid))
        .map(|t| t.list_id.clone())
}

/// 日历查询的统一入口：日程（含重复展开）+ 待办派生，且都剔除已完成项。
///
/// `list` 过滤的归属规则（主窗口选中清单时日历/总结都走这一套）：
///   · 日程 → 按 `event_list` 归属；归属为空（既没记清单也查不到待办）
///     视为"不属于任何清单"，恒显示。手工日程正是这种。
///   · 待办派生条目 → 按待办的 list_id 过滤。
fn calendar_entries(store: &Store, from: &str, to: &str, list: Option<&str>) -> Vec<Event> {
    let done = done_todo_ids(&store.todos);
    let in_list = |e: &Event| match list {
        None => true,
        Some(l) => match event_list(store, e) {
            None => true, // 归属不明：宁可多显示，不可凭空消失
            Some(el) => el == l,
        },
    };
    let live: Vec<Event> = store
        .events
        .iter()
        .filter(|e| match e.todo_id.as_deref() {
            Some(id) => !done.contains(id),
            None => true,
        })
        .filter(|e| in_list(e))
        .cloned()
        .collect();
    let mut v = expand_events(&live, from, to);
    let mut derived = todo_derived_events(&store.todos, &live, from, to);
    if list.is_some() {
        derived.retain(in_list);
    }
    v.extend(derived);
    v
}

/// 把日程（含重复规则 + 跨天占用）展开到 [from, to] 区间内的**每一天**。
///
/// 三层展开，缺一层就会出"跨天日程只显示第一天"那种看得见字段、等不到功能的 bug：
///   ① 重复规则给出各个**起始日** occurrence（`recurring::expand_dates`）；
///   ② 每个起始日按 `span_days` 往后铺 N 天 —— 跨天日程的语义就是"这几天都占着"；
///   ③ 与查询区间求交，落在区间内的每一天出一条，`span_index` 说明它是第几天
///      （0 = 起始日，>0 = 续日）。
///
/// ★ ②之前先把取起始日的窗口**往前放宽 span-1 天**：否则"上月 29 号开始、跨到
/// 本月 2 号"的日程，本月查询里起始日不在区间内 → `expand_dates` 一条都不给 →
/// 尾巴那几天照样是空的（这就是"跨天只在第一天出现"的另一半成因）。
///
/// 同一 (id, date) 只出一条（"每天重复 + 跨 3 天"会大量自重叠）。
fn expand_events(events: &[Event], from: &str, to: &str) -> Vec<Event> {
    use crate::domain::recurring::{expand_dates, fmt_date, parse_date};
    let (Some(from_d), Some(to_d)) = (parse_date(from), parse_date(to)) else {
        return Vec::new();
    };
    let mut out: Vec<Event> = Vec::new();
    let mut seen: std::collections::HashMap<(String, String), usize> = Default::default();
    for e in events {
        let Some(start) = parse_date(&e.date) else { continue };
        let span = e.span_days.max(1).clamp(1, MAX_SPAN_DAYS) as i64;
        let win_from = if span > 1 {
            from_d - chrono::Duration::days(span - 1)
        } else {
            from_d
        };
        for d in expand_dates(start, e.recurrence.as_ref(), win_from, to_d) {
            for k in 0..span {
                let day = d + chrono::Duration::days(k);
                if day > to_d {
                    break;
                }
                if day < from_d {
                    continue; // 尾巴从区间之前伸进来：只发区间内的天
                }
                let date = fmt_date(day);
                let occ = Event {
                    date,
                    span_index: k as u16,
                    span_total: span as u16,
                    ..e.clone()
                };
                match seen.get(&(occ.id.clone(), occ.date.clone())) {
                    // 同一天已有一条：只有"这次是新一轮的起始日"才替换 ——
                    // 否则"每天重复 + 跨 2 天"时，06-02 会被 06-01 的续日占掉，
                    // 06-02 自己那条长条就画不出来了。
                    Some(&pos) => {
                        if k == 0 && out[pos].span_index != 0 {
                            out[pos] = occ;
                        }
                    }
                    None => {
                        seen.insert((occ.id.clone(), occ.date.clone()), out.len());
                        out.push(occ);
                    }
                }
            }
        }
    }
    out
}

/// 跨天上限与时长上限。`span_days` 与 `duration_min` 都可以由前端整对象写入，
/// 不夹的话一条"跨 65535 天"的日程会让区间展开循环爆掉、`duration_min` 溢出成
/// 奇怪的结束时刻。上限取"两个月/一整天"，比这更大的需求应该拆成多条。
const MAX_SPAN_DAYS: u16 = 60;
const MAX_DURATION_MIN: u32 = 24 * 60;

/// 落库前收拾一条日程：业务字段夹范围，两个**运行时字段**清零。
///
/// `span_index`/`span_total` 是查询展开时算出来给前端画长条用的，不是用户输入。
/// 前端 `updateEvent` 是整对象回传（会把它们原样带回来），照单全收就会把
/// "第 3 天"这种一次性上下文写进库 —— 下次展开时它已经与 date 不一致了。
fn sanitize_event_for_store(e: &mut Event) {
    e.span_days = e.span_days.max(1).clamp(1, MAX_SPAN_DAYS);
    e.duration_min = e.duration_min.min(MAX_DURATION_MIN);
    e.span_index = 0;
    e.span_total = e.span_days;
}

impl AppState {
    pub fn open(root: PathBuf) -> Result<Self, crate::db::StoreError> {
        let db = Arc::new(JsonStore::open(root)?);
        let store = db.load()?;
        Ok(Self {
            store: RwLock::new(store),
            db,
            hidden_all: RwLock::new(false),
            calendar_view: RwLock::new(CalendarView::Month),
            calendar_focus: RwLock::new(now_local_date_str()),
        })
    }

    /// 变更后统一持久化。闭包可以带返回值（`import_markdown` 要在**同一把锁**里
    /// 数出"新增几条、跳过几条"，捕获可变状态是不行的：`move` 会把 Copy 的计数器
    /// 复制进闭包，改完整个人回到外面还是 0 —— 编译器只会给你一条 warning）。
    fn persist<R>(&self, f: impl FnOnce(&mut Store) -> R) -> Result<R, String> {
        let mut guard = self.store.write();
        let r = f(&mut guard);
        self.db.write(&guard).map_err(|e| e.to_string())?;
        Ok(r)
    }

    // ── 读取 ────────────────────────────────────────────────

    pub fn snapshot(&self) -> Store {
        self.store.read().clone()
    }

    /// 某清单的活动待办（未归档），置顶优先，其次 order
    pub fn todos_of(&self, list_id: Option<&str>) -> Vec<Todo> {
        let g = self.store.read();
        let today = now_local_date_str();
        // 归档清单的待办不参与"活动待办"：显式点名某清单时照给（用户点进
        // 「已归档」区就是要看它），全局聚合时排除。与前端 shared/scope.ts
        // 同一口径 —— 归档规则只能有一处定义，否则前后端必然漂移
        // （"日历/总结串清单"那个 bug 就是这么来的）。
        let explicit = list_id.is_some();
        let archived: std::collections::HashSet<&str> = g
            .lists
            .iter()
            .filter(|l| l.archived)
            .map(|l| l.id.as_str())
            .collect();
        let mut v: Vec<Todo> = g
            .todos
            .iter()
            .filter(|t| !t.done)
            .filter(|t| list_id.map_or(true, |l| t.list_id == l))
            .filter(|t| explicit || !archived.contains(t.list_id.as_str()))
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                // 优先级：高→低→无。放在 order 之前 —— 用户特意标了旗标，
                // 就是要它压过手动排序；置顶仍然最优先（那是更强的意图）
                .then_with(|| match (a.priority, b.priority) {
                    (Some(x), Some(y)) => x.rank().cmp(&y.rank()),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then(a.order.cmp(&b.order))
                // 到期近的在前；无到期排最后
                .then_with(|| due_key(&a.due, &today).cmp(&due_key(&b.due, &today)))
        });
        v
    }

    /// 已完成待办（本周回顾）
    pub fn done_recent(&self, days: i64) -> Vec<Todo> {
        let g = self.store.read();
        let cutoff = Local::now().date_naive() - chrono::Duration::days(days);
        g.todos
            .iter()
            .filter(|t| {
                t.done && t
                    .done_at
                    .as_deref()
                    .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%dT%H:%M").ok())
                    .or_else(|| s_to_date(t.done_at.as_deref()))
                    .map(|d| d >= cutoff)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// 某日日程，按 time 排序（全天在前）。
    /// 含重复日程的虚拟 occurrence + 带到期日的待办派生条目；
    /// 已完成的待办、及其关联日程，一律不显示。
    pub fn events_on(&self, date: &str) -> Vec<Event> {
        let g = self.store.read();
        // 单日视图（挂件/日双击）没有"选中清单"语境，保持全局
        let mut v = calendar_entries(&g, date, date, None);
        v.sort_by(|a, b| {
            (&a.time, &a.title).cmp(&(&b.time, &b.title))
        });
        v
    }

    /// 区间内每天的日程（重复展开 + 待办派生，剔除已完成）供前端渲染角标。
    /// `list` = Some 时只保留该清单派生的待办项（手工日程不受影响）。
    pub fn events_in_range(&self, from: &str, to: &str, list: Option<&str>) -> Vec<Event> {
        let g = self.store.read();
        calendar_entries(&g, from, to, list)
    }

    /// 今日到期 + 已逾期的活动待办
    pub fn today_and_overdue(&self) -> Vec<Todo> {
        let today = now_local_date_str();
        self.todos_of(None)
            .into_iter()
            .filter(|t| t.due.as_deref().map(|d| d <= today.as_str()).unwrap_or(false))
            .collect()
    }

    /// 逾期自动延期（对照 Kite 待办）：未完成且到期日早于今天的，
    /// 把到期日推到今天；有提醒时间的同步平移并重置提醒位，
    /// 使其在今天仍能提醒一次。返回被延期的条数。
    ///
    /// 只在设置开启时由启动流程调用一次 —— 它会改写用户的到期日，
    /// 属于"动数据"的操作，默认关。
    ///
    /// 先读快照算出名单、再按 id 写（run_archive 同款）：闭包捕获的
    /// 计数器副本改不到外面，直接在里面累加会永远返回 0。
    pub fn auto_postpone(&self) -> Result<usize, String> {
        let today = now_local_date_str();
        let ids: Vec<String> = self
            .store
            .read()
            .todos
            .iter()
            .filter(|t| {
                !t.done
                    && t.due
                        .as_deref()
                        .map(|d| d < today.as_str())
                        .unwrap_or(false)
            })
            .map(|t| t.id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(0);
        }
        let n = ids.len(); // ids 会被 move 进闭包，长度先取出来
        self.persist(move |s| {
            for t in s.todos.iter_mut().filter(|t| ids.contains(&t.id)) {
                t.due = Some(today.clone());
                if let Some(r) = t.remind_at.clone() {
                    // remind_at 支持两种格式（见 reminder::parse_min）：
                    // 带时间的 `YYYY-MM-DDTHH:MM` 与纯日期的 `YYYY-MM-DD`。
                    // 旧代码只处理带 `T` 的那种，纯日期时**连 reminded 都不重置**
                    // —— 延期到今天的项当天再也不会提醒（实测发现的漏网）。
                    t.remind_at = Some(match r.split_once('T') {
                        Some((_, hhmm)) => format!("{today}T{hhmm}"),
                        None => today.clone(),
                    });
                    t.reminded = false;
                    // 额外提醒按**绝对时点**去重，一并清：否则昨天没做完的项
                    // 其额外提醒今天永不再响（旧 key 已过期却仍在列表里）。
                    t.reminded_keys.clear();
                    // 持续提醒的节流基准归零，让今天重新起算
                    t.last_reminded_at = None;
                }
            }
        })?;
        Ok(n)
    }

    // ── 写入：清单 ──────────────────────────────────────────

    pub fn add_list(&self, name: String, color: String) -> Result<TodoList, String> {
        let list = TodoList {
            id: ids::new(),
            name,
            color,
            // 接在**最大** order 之后，而不是拿条数当序号：删过中间清单后
            // 条数会与现存序号撞号（两条 order 相同时，排序结果取决于数组
            // 顺序，新建的清单可能挤到中间去）。add_todo 用的是同一种算法。
            order: self.store.read().lists.iter().map(|l| l.order).max().unwrap_or(-1) + 1,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        };
        let out = list.clone();
        self.persist(move |s| s.lists.push(list))?;
        Ok(out)
    }

    /// 该清单是否不计入周/月总结与分析页。
    /// 字段级写入（与 set_list_archived 同款），避免整对象覆盖。
    pub fn set_list_exclude_stats(&self, id: &str, exclude: bool) -> Result<(), String> {
        let id = id.to_string();
        self.persist(move |s| {
            if let Some(l) = s.lists.iter_mut().find(|l| l.id == id) {
                l.exclude_from_stats = exclude;
            }
        })
    }

    /// 归档 / 取消归档清单（对照滴答）。字段级写入，避免整对象覆盖。
    pub fn set_list_archived(&self, id: &str, archived: bool) -> Result<(), String> {
        let id = id.to_string();
        self.persist(move |s| {
            if let Some(l) = s.lists.iter_mut().find(|l| l.id == id) {
                l.archived = archived;
            }
        })
    }

    pub fn update_list(&self, list: TodoList) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(slot) = s.lists.iter_mut().find(|l| l.id == list.id) {
                *slot = list.clone();
            }
        })
    }

    pub fn remove_list(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            // 至少留一个清单（收集箱是数据锚点：恢复待办、新建待办都指向它）
            if s.lists.len() <= 1 {
                return;
            }
            s.lists.retain(|l| l.id != id);
            // 该清单的待办移入第一个剩余清单，避免数据凭空消失。
            // （审计实锤：旧代码先迁移再 retain 删掉，迁移是死代码，
            //   实际效果是删清单连带删光其中全部待办——用户数据丢失级。）
            let Some(target) = s.lists.first().map(|l| l.id.clone()) else {
                return;
            };
            let moved: Vec<String> = s
                .todos
                .iter()
                .filter(|t| t.list_id == id)
                .map(|t| t.title.clone())
                .collect();
            for t in s.todos.iter_mut() {
                if t.list_id == id {
                    t.list_id = target.clone();
                }
            }
            // 日程与挂件里同样可能存着被删清单的 id，**必须一并修**：
            //   · 日程（母记录 list_id）：待办已迁入新清单，但拖它生成的日程
            //     仍记着旧清单 → 在主窗口选新清单看日历时这些日程**不显示**
            //     （过滤落空），用户视角"待办还在，日历上却没了"。
            //   · 挂件（绑定的 listId）：恒指向不存在的清单 → 列表永远为空，
            //     用户看到"删个清单，挂件就空了"，且不知道点「全」能恢复。
            for e in s.events.iter_mut() {
                if e.list_id.as_deref() == Some(id.as_str()) {
                    e.list_id = Some(target.clone());
                }
            }
            for w in s.widgets.iter_mut() {
                if w.list_id.as_deref() == Some(id.as_str()) {
                    // 挂件回落到"全部清单"（None）而不是第一个清单：
                    // 挂件与清单是"看哪个"的绑定关系，清单没了就退回看全部，
                    // 比硬塞进某个名称完全无关的清单更符合用户预期。
                    w.list_id = None;
                }
            }
            if !moved.is_empty() {
                tracing::info!("清单 {id} 删除，{} 条待办移入 {target}：{}", moved.len(), moved.join("、"));
            }
        })
    }

    pub fn reorder_lists(&self, ids: Vec<String>) -> Result<(), String> {
        self.persist(move |s| {
            for (i, id) in ids.iter().enumerate() {
                if let Some(l) = s.lists.iter_mut().find(|l| l.id == *id) {
                    l.order = i as i32;
                }
            }
            s.lists.sort_by_key(|l| l.order);
        })
    }

    // ── 写入：待办 ──────────────────────────────────────────

    pub fn add_todo(&self, list_id: String, title: String) -> Result<Todo, String> {
        let order = self
            .store
            .read()
            .todos
            .iter()
            .filter(|t| t.list_id == list_id)
            .map(|t| t.order)
            .max()
            .unwrap_or(0)
            + 1;
        let title = title.trim().to_string();
        // 一句话智能识别：[标签] 前缀 + 日期/时间自然语言
        let p = crate::quickparse::parse_full(&title);
        // **没写日期就是"今天要做的"**，不是"待定"（用户口径：快速添加不该产出无到期日的
        // 待办 —— 那种条目既不在"今天"也不在任何时间桶里，记完就像丢了）。
        let due = p.due.unwrap_or_else(crate::now_local_date_str);
        // 只有识别到**时刻**才落成提醒：没写时刻就不该弹提醒，否则每记一条都响一次。
        let remind_at = p.time.map(|t| format!("{due}T{t}"));
        let todo = Todo {
            id: ids::new(),
            list_id,
            title: p.title,
            note: String::new(),
            done: false,
            done_at: None,
            pinned: false,
            order,
            tags: p.tags,
            due: Some(due),
            remind_at,
            reminded: false,
            extra_reminds: vec![],
            reminded_keys: vec![],
            constant_remind: false,
            last_reminded_at: None,
            countdown: None,
            priority: p.priority,
            recurrence: None,
            created_at: now_local_str(),
            archived_at: None,
        };
        let out = todo.clone();
        self.persist(move |s| s.todos.push(todo))?;
        Ok(out)
    }

    /// 整对象写入：**仅用于导入/迁移等整体替换场景**。
    /// 日常改字段一律走 `patch_todo` —— 整对象覆盖会把并发写者
    /// （提醒线程、set_done、重复规则生成）刚改的字段抹回旧值。
    pub fn update_todo(&self, todo: Todo) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(slot) = s.todos.iter_mut().find(|t| t.id == todo.id) {
                *slot = todo.clone();
            }
        })
    }

    /// 字段级补丁：只改 patch 里给出的字段，其余保持服务端当前值。
    /// 这是日常编辑（标题/备注/标签/到期/提醒/优先级/倒数/重复）的正路。
    pub fn patch_todo(&self, id: &str, p: crate::domain::TodoPatch) -> Result<(), String> {
        let id = id.to_string();
        self.persist(move |s| {
            let Some(t) = s.todos.iter_mut().find(|t| t.id == id) else {
                return; // 已被删/归档：静默忽略，前端可能持有旧引用
            };
            if let Some(v) = p.title {
                let v = v.trim().to_string();
                if !v.is_empty() {
                    t.title = v;
                }
            }
            if let Some(v) = p.note {
                t.note = v;
            }
            if let Some(v) = p.done {
                t.done = v;
                t.done_at = if v { Some(now_local_str()) } else { None };
            }
            if let Some(v) = p.pinned {
                t.pinned = v;
            }
            if let Some(v) = p.tags {
                t.tags = v;
            }
            if let Some(v) = p.due {
                t.due = v;
            }
            if let Some(v) = p.remind_at {
                t.remind_at = v;
            }
            if let Some(v) = p.extra_reminds {
                t.extra_reminds = v;
            }
            if let Some(v) = p.reminded_keys {
                t.reminded_keys = v;
            }
            if let Some(v) = p.constant_remind {
                t.constant_remind = v;
            }
            if let Some(v) = p.reminded {
                t.reminded = v;
            }
            if let Some(v) = p.last_reminded_at {
                t.last_reminded_at = v;
            }
            if let Some(v) = p.countdown {
                t.countdown = v;
            }
            if let Some(v) = p.priority {
                t.priority = v;
            }
            if let Some(v) = p.recurrence {
                t.recurrence = v;
            }
        })
    }

    pub fn remove_todo(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            s.todos.retain(|t| t.id != id);
            // 关联日程解除绑定但保留日程本身（用户可能已改写过内容）
            for e in s.events.iter_mut() {
                if e.todo_id.as_deref() == Some(&id) {
                    e.todo_id = None;
                    e.source = "manual".into();
                }
            }
        })
    }

    /// 软删除：移进回收站（对照滴答「垃圾桶」）。
    ///
    /// 与 remove_todo 的分工：垃圾桶是用户日常删除的默认去向（可恢复），
    /// 彻底删除才走 remove_todo。所以这里**不**解绑日程 —— 恢复后
    /// 待办与日程的关联应当原样回来。
    pub fn trash_todo(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            if s.trashed.iter().any(|x| x.todo.id == id) {
                return; // 已在回收站，别塞出两条
            }
            let Some(pos) = s.todos.iter().position(|t| t.id == id) else {
                return;
            };
            let todo = s.todos.remove(pos);
            s.trashed.push(crate::domain::TrashedTodo {
                todo,
                trashed_at: now_local_str(),
            });
            trim_trash(&mut s.trashed);
        })
    }

    /// 从回收站恢复到原清单。清单若已被删，落回第一个清单（收集箱）。
    pub fn restore_todo(&self, id: String) -> Result<Option<Todo>, String> {
        // 与 set_done 同一模式：先在只读快照里算好要插回的完整条目，
        // 写闭包里只做搬移 —— 否则值被闭包 move 走就读不出来了。
        let prepared: Option<Todo> = {
            let g = self.store.read();
            g.trashed.iter().find(|x| x.todo.id == id).map(|x| {
                let mut t = x.todo.clone();
                if !g.lists.iter().any(|l| l.id == t.list_id) {
                    if let Some(first) = g.lists.first() {
                        t.list_id = first.id.clone();
                    }
                }
                // 排序键接回表尾，避免与现存项撞号
                t.order = g.todos.iter().map(|y| y.order).max().unwrap_or(0) + 1;
                t
            })
        };
        let Some(todo) = prepared else {
            return Ok(None); // 不在回收站
        };
        let out = todo.clone();
        let tid = todo.id.clone();
        self.persist(move |s| {
            s.trashed.retain(|x| x.todo.id != tid);
            s.todos.push(todo);
        })?;
        Ok(Some(out))
    }

    /// 彻底删除回收站里的某一条（不可恢复）
    pub fn purge_todo(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            s.trashed.retain(|x| x.todo.id != id);
        })
    }

    /// 清空回收站
    pub fn empty_trash(&self) -> Result<usize, String> {
        let n = self.store.read().trashed.len();
        self.persist(move |s| {
            s.trashed.clear();
        })?;
        Ok(n)
    }

    /// 完成 → 处理重复规则自动生成 → 可选立即归档
    pub fn set_done(&self, id: String, done: bool) -> Result<Option<Todo>, String> {
        // 先在只读快照里算出要生成的下一条（避免在写闭包里回传值）
        let next_todo: Option<Todo> = if done {
            let g = self.store.read();
            g.todos.iter().find(|t| t.id == id).and_then(|t| {
                let rc = t.recurrence.clone()?;
                if !rc.auto_create {
                    return None;
                }
                let base = t
                    .due
                    .as_deref()
                    .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                    .unwrap_or_else(|| Local::now().date_naive());
                let next = crate::domain::recurring::next_due(base, &rc)?;
                let order = g.todos.iter().map(|x| x.order).max().unwrap_or(0) + 1;
                let mut nt = t.clone();
                nt.id = ids::new();
                nt.done = false;
                nt.done_at = None;
                nt.pinned = false;
                let next_due = crate::domain::recurring::fmt_date(next);
                // 提醒必须跟着新到期日搬 —— 这是**第四处**同口径点
                // （前两处在 move_event / todo_to_event，第三处在 UI）。
                // 漏了它的后果在"每天 9 点站会"这种最常见用法上立刻可见：
                // 新条目的 remind_at 仍指着**旧那一天**（早已过期），下一轮
                // 轮询就误报，用户刚勾完完成又收到一遍提醒。
                // 走共享函数顺手把 reminded_keys/last_reminded_at 也清干净。
                crate::domain::recurring::shift_reminders_to(&mut nt, Some(&next_due));
                nt.due = Some(next_due);
                nt.created_at = now_local_str();
                nt.archived_at = None;
                nt.order = order;
                Some(nt)
            })
        } else {
            None
        };

        self.persist({
            let id = id.clone();
            let spawn = next_todo.clone();
            move |s| {
                if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                    t.done = done;
                    t.done_at = if done { Some(now_local_str()) } else { None };
                }
                if let Some(nt) = spawn {
                    s.todos.push(nt);
                }
            }
        })?;
        Ok(next_todo)
    }

    pub fn toggle_pin(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.pinned = !t.pinned;
            }
        })
    }

    /// 列表内拖拽排序：把 moved 放到 target 之前/之后
    pub fn reorder_todo(&self, moved_id: String, target_id: Option<String>, after: bool) -> Result<(), String> {
        self.persist(move |s| {
            let list_id = match s.todos.iter().find(|t| t.id == moved_id) {
                Some(t) => t.list_id.clone(),
                None => return,
            };
            let mut seq: Vec<String> = s
                .todos
                .iter()
                .filter(|t| t.list_id == list_id)
                .filter(|t| t.id != moved_id)
                .map(|t| t.id.clone())
                .collect();
            let insert_at = match &target_id {
                Some(tid) => {
                    let pos = seq.iter().position(|x| x == tid);
                    match pos {
                        Some(p) => {
                            if after {
                                p + 1
                            } else {
                                p
                            }
                        }
                        None => seq.len(),
                    }
                }
                None => seq.len(),
            };
            seq.insert(insert_at.min(seq.len()), moved_id.clone());
            for (i, id) in seq.iter().enumerate() {
                if let Some(t) = s.todos.iter_mut().find(|t| t.id == *id) {
                    t.order = i as i32;
                }
            }
        })
    }

    pub fn set_countdown(&self, id: String, cd: Option<crate::domain::Countdown>) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.countdown = cd.clone();
            }
        })
    }

    pub fn set_recurrence(&self, id: String, rc: Option<crate::domain::Recurrence>) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.recurrence = rc.clone();
            }
        })
    }

    /// 设置/清除优先级旗标（None = 清除）
    pub fn set_priority(&self, id: String, p: Option<crate::domain::Priority>) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.priority = p;
            }
        })
    }

    /// 主提醒触发：置 reminded 并记下弹窗时刻（持续提醒节流基准）
    ///
    /// 这里**没有**"只置 reminded 的简化版"（以前有个 `mark_reminded`）：
    /// 它不落 `last_reminded_at`，谁用它，持续提醒的节流基准就停在旧值上 ——
    /// 表现是"重弹节奏全乱"。提醒线程要的就是这个全量版本，留两个入口
    /// 只会让人挑错一个（同类问题：字段级补丁取代整对象回写）。
    pub fn mark_reminded_full(&self, id: &str, at: &str) -> Result<(), String> {
        let id = id.to_string();
        let at = at.to_string();
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.reminded = true;
                t.last_reminded_at = Some(at);
            }
        })
    }

    /// 额外提醒触发：把该时点记进 reminded_keys（各时点独立去重）
    pub fn mark_extra_reminded(&self, id: &str, key: &str, at: &str) -> Result<(), String> {
        let id = id.to_string();
        let key = key.to_string();
        let at = at.to_string();
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                if !t.reminded_keys.contains(&key) {
                    t.reminded_keys.push(key);
                }
                t.last_reminded_at = Some(at);
            }
        })
    }

    /// 持续提醒重弹：只更新弹窗时刻（reminded 早已为 true）
    pub fn mark_constant_reminded(&self, id: &str, at: &str) -> Result<(), String> {
        let id = id.to_string();
        let at = at.to_string();
        self.persist(move |s| {
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == id) {
                t.last_reminded_at = Some(at);
            }
        })
    }

    /// 跨天复位：让"逾期仍未处理"的提醒当天可再响一次
    ///
    /// 必须连 `reminded_keys` 一起清：额外提醒的去重是按**绝对时点**记的，
    /// 只清 `reminded` 会让"昨天没做完"的额外提醒今天永远不再弹
    /// （时点已过期且仍在 reminded_keys 里）。清掉后是否真弹仍由
    /// `remind_hit` 决定，未到点的未来时点不会被误触发。
    /// `last_reminded_at` 同理清零，让持续提醒在新的一天重新计时。
    pub fn reset_reminded(&self) -> Result<(), String> {
        self.persist(move |s| {
            for t in s.todos.iter_mut() {
                if !t.done {
                    t.reminded = false;
                    t.reminded_keys.clear();
                    t.last_reminded_at = None;
                }
            }
        })
    }

    /// 按完成日期归档：done_at 早于 (今天 - archive_after_days)
    pub fn run_archive(&self) -> Result<usize, String> {
        let keep = self.store.read().settings.archive_after_days as i64;
        let cutoff = Local::now().date_naive() - chrono::Duration::days(keep);
        let moving: Vec<Todo> = self
            .store
            .read()
            .todos
            .iter()
            .filter(|t| {
                t.done && done_date(t).map(|d| d < cutoff).unwrap_or(false)
            })
            .cloned()
            .collect();
        if moving.is_empty() {
            return Ok(0);
        }
        let ids: Vec<String> = moving.iter().map(|t| t.id.clone()).collect();
        self.db.append_archive(&moving).map_err(|e| e.to_string())?;
        self.persist(move |s| {
            // 与该待办**绑定**的日程一并移出活动表。
            //
            // 为什么必须一起走（审计实锤，有测试复现）：日历的已完成过滤
            // 只看活动表 store.todos —— 待办归档后 done_todo_ids 查不到它，
            // 于是其关联日程会**重新出现在日历上**（用户视角："我做完并归档
            // 了，日历上又冒出来了"）。而且这些日程的存在意义就是那条待办。
            // 手工日程（todo_id=None）不受影响。
            s.events.retain(|e| match e.todo_id.as_deref() {
                Some(tid) => !ids.contains(&tid.to_string()),
                None => true,
            });
            s.todos.retain(|t| !ids.contains(&t.id));
        })?;
        Ok(moving.len())
    }

    // ── 写入：日程 ──────────────────────────────────────────

    pub fn add_event(&self, mut ev: Event) -> Result<Event, String> {
        if ev.id.is_empty() {
            ev.id = ids::new();
        }
        if ev.color.is_empty() {
            ev.color = "#4c8dff".into();
        }
        // 跨天数/时长夹范围，运行时字段清零（见 sanitize_event_for_store）
        sanitize_event_for_store(&mut ev);
        let out = ev.clone();
        self.persist(move |s| s.events.push(ev))?;
        Ok(out)
    }

    pub fn update_event(&self, ev: Event) -> Result<(), String> {
        // 日历上双击派生条目 → 改的是待办本身。不处理就会静默无反应
        // （events 里没有这个 id），用户以为改成功了。
        if let Some(tid) = ev.id.strip_prefix("todo:") {
            let tid = tid.to_string();
            return self.persist(move |s| {
                if let Some(t) = s.todos.iter_mut().find(|t| t.id == tid) {
                    apply_event_to_todo(t, &ev);
                }
            });
        }
        let mut ev = ev;
        sanitize_event_for_store(&mut ev);
        self.persist(move |s| {
            if let Some(slot) = s.events.iter_mut().find(|e| e.id == ev.id) {
                *slot = ev.clone();
            }
        })
    }

    pub fn remove_event(&self, id: String) -> Result<(), String> {
        // 派生条目的 ✕ = "这天不安排"，所以清掉到期日而非删待办：
        // 可逆、无破坏性，待办本身仍在清单里。
        if let Some(tid) = id.strip_prefix("todo:") {
            return self.persist(move |s| {
                if let Some(t) = s.todos.iter_mut().find(|t| t.id == tid) {
                    t.due = None;
                }
            });
        }
        self.persist(move |s| s.events.retain(|e| e.id != id))
    }

    /// 拖待办到日历格 → 生成日程；待办 due 同步为落点日期
    ///
    /// **已经排过日历的待办 = 改期，不是再造一条**（见下面 linked 分支）。
    pub fn todo_to_event(&self, todo_id: String, date: String) -> Result<Event, String> {
        let (todo, color, linked) = {
            let g = self.store.read();
            let todo = g
                .todos
                .iter()
                .find(|t| t.id == todo_id)
                .cloned()
                .ok_or_else(|| "待办不存在".to_string())?;
            let color = g
                .lists
                .iter()
                .find(|l| l.id == todo.list_id)
                .map(|l| l.color.clone())
                .unwrap_or_else(|| "#4c8dff".into());
            // 这条待办此前是否已经生成过日程（拖过第二次、或「排到…」又点一次）
            let linked = g
                .events
                .iter()
                .find(|e| e.todo_id.as_deref() == Some(todo_id.as_str()))
                .map(|e| e.id.clone());
            (todo, color, linked)
        };
        // 待办若带提醒时间，拖成日程时一并带上（"明天 9:30 站会" → 9:30 的日程）
        let ev = Event {
            id: ids::new(),
            date: date.clone(),
            time: todo
                .remind_at
                .as_deref()
                .and_then(|r| r.split_once('T'))
                .map(|(_, hhmm)| hhmm.to_string())
                .filter(|s| s.len() == 5),
            duration_min: 0,
            title: todo.title.clone(),
            color,
            source: "todo".into(),
            todo_id: Some(todo_id.clone()),
            span_days: 1,
            span_index: 0,
            span_total: 1,
            // 重复待办拖上日历 → 日程同样按该规则重复，语义一致
            recurrence: todo.recurrence.clone(),
            // **落库时就把清单写进母记录**：此后即便待办被删/软删/移走，
            // 这条日程的归属依然明确（选该清单还能看到）。
            list_id: Some(todo.list_id.clone()),
        };
        let tid = todo_id.clone();
        self.persist(move |s| {
            match linked {
                // 同一待办的第二条日程**不新建**，只把原来那条挪到新日期。
                // 此前的行为是每次调用都 push 一条：把待办先拖到 5 号、再拖到
                // 8 号，日历上就同时挂着两条同一待办的日程（用户视角"我改了
                // 日期，旧的怎么还在"）；「排到…」菜单每点一次同样多留一条。
                // 标题/重复规则不动 —— 那是用户排进去之后可能改过的内容。
                Some(eid) if s.events.iter().any(|e| e.id == eid) => {
                    if let Some(e) = s.events.iter_mut().find(|e| e.id == eid) {
                        e.date = date.clone();
                    }
                }
                // 没排过，或那条日程已被删掉（✕ 只删日程、待办还在）→ 新建
                _ => s.events.push(ev),
            }
            if let Some(t) = s.todos.iter_mut().find(|t| t.id == tid) {
                // 与前两处同口径：提醒跟着落点日期平移、复位触发位。
                // 漏了它会出现"我把待办拖到 5 号，提醒还停在 3 号"。
                crate::domain::recurring::shift_reminders_to(t, Some(&date));
                t.due = Some(date);
            }
        })?;
        // 返回**落库后的真实条目**（改期的那条 / 新建的那条），而不是手边的
        // 候选值 —— 调用方（前端）拿它当"现在的样子"用。
        let g = self.store.read();
        g.events
            .iter()
            .find(|e| e.todo_id.as_deref() == Some(todo_id.as_str()))
            .cloned()
            .ok_or_else(|| "日程写入失败".to_string())
    }

    /// 拖日程到另一格 → 改期。派生条目改的是待办的到期日。
    pub fn move_event(&self, event_id: String, date: String) -> Result<(), String> {
        if let Some(tid) = event_id.strip_prefix("todo:") {
            return self.persist(move |s| {
                if let Some(t) = s.todos.iter_mut().find(|t| t.id == tid) {
                    // 提醒处理走**共享函数**（recurring::shift_reminders_to）：
                    // UI 改到期日 / 拖派生条目 / 拖进日历生成日程 三条路径
                    // 必须同一口径，否则同一条待办会因路径不同而结果不同。
                    crate::domain::recurring::shift_reminders_to(t, Some(&date));
                    t.due = Some(date);
                }
            });
        }
        self.persist(move |s| {
            if let Some(e) = s.events.iter_mut().find(|e| e.id == event_id) {
                e.date = date;
            }
        })
    }

    // ── 写入：便签 ──────────────────────────────────────────

    pub fn add_note(&self, body: String) -> Result<Note, String> {
        // 便签是自由多行文本：标签只"标记"不"剥离"（与待办不同，见 extract_tags）
        let tags = crate::quickparse::extract_tags(&body);
        let note = Note {
            id: ids::new(),
            body,
            color: "#fff8c4".into(),
            font_px: 13,
            opacity: 1.0,
            widget_id: None,
            tags,
            pinned: false,
            order: self.store.read().notes.len() as i32,
            updated_at: now_local_str(),
        };
        let out = note.clone();
        self.persist(move |s| s.notes.push(note))?;
        Ok(out)
    }

    pub fn update_note(&self, note: Note) -> Result<(), String> {
        // 保存时按正文重新归类标签（不剥离正文，见 quickparse::extract_tags）
        let tags = crate::quickparse::extract_tags(&note.body);
        self.persist(move |s| {
            if let Some(slot) = s.notes.iter_mut().find(|n| n.id == note.id) {
                let mut n = note.clone();
                n.tags = tags.clone();
                n.updated_at = now_local_str();
                *slot = n;
            }
        })
    }

    pub fn toggle_note_pin(&self, id: String) -> Result<(), String> {
        self.persist(move |s| {
            if let Some(n) = s.notes.iter_mut().find(|n| n.id == id) {
                n.pinned = !n.pinned;
            }
        })
    }

    /// 便签拖拽排序：把 id 移到 target_id 之前/之后
    pub fn reorder_note(&self, id: String, target_id: Option<String>, after: bool) -> Result<(), String> {
        let mut notes: Vec<Note> = {
            let mut v = self
                .store
                .read()
                .notes
                .iter()
                .filter(|n| !n.pinned)
                .cloned()
                .collect::<Vec<_>>();
            v.sort_by_key(|n| n.order);
            v
        };
        let Some(pos) = notes.iter().position(|n| n.id == id) else {
            return Ok(());
        };
        let item = notes.remove(pos);
        let insert_at = match target_id {
            Some(tid) => notes
                .iter()
                .position(|n| n.id == tid)
                .map(|i| i + if after { 1 } else { 0 })
                .unwrap_or(notes.len()),
            None => notes.len(),
        };
        notes.insert(insert_at.min(notes.len()), item);
        let order: Vec<(String, i32)> = notes.iter().enumerate().map(|(i, n)| (n.id.clone(), i as i32)).collect();
        self.persist(move |s| {
            for (nid, o) in order {
                if let Some(n) = s.notes.iter_mut().find(|n| n.id == nid) {
                    n.order = o;
                }
            }
        })
    }

    pub fn remove_note(&self, id: String) -> Result<(), String> {
        self.persist(move |s| s.notes.retain(|n| n.id != id))
    }

    // ── 写入：挂件与设置 ────────────────────────────────────

    pub fn upsert_widget(&self, w: Widget) -> Result<(), String> {
        // **已存在的挂件绝不通过这里改 visible**：visible 是生命周期字段，
        // 只归 set_widget_visible / show_all_widgets 管。前端 saveGeometry
        // 换档时会发 `upsertWidget({...陈旧快照})`，若允许它覆盖 visible，
        // 就会把用户刚点的"收起"写回 true（lost update，✕/显隐失效的根因）。
        // 新挂件（id 不存在）才用传入的 visible（默认 true）。
        self.persist(move |s| {
            match s.widgets.iter_mut().find(|x| x.id == w.id) {
                Some(slot) => {
                    let keep = slot.visible;
                    *slot = w.clone();
                    slot.visible = keep;
                }
                None => s.widgets.push(w.clone()),
            }
        })
    }

    /// 字段级补丁挂件（架构修正的核心）：只改 patch 里给出的字段，
    /// 其余保持**服务端当前值**。这样并发的写者（显隐、尺寸、筛选、贴边）
    /// 各改各的字段，不会互相抹掉 —— 取代此前"整对象快照覆盖"。
    pub fn patch_widget(&self, id: &str, p: crate::domain::WidgetPatch) -> Result<(), String> {
        let id = id.to_string();
        self.persist(move |s| {
            let Some(w) = s.widgets.iter_mut().find(|x| x.id == id) else {
                return; // 挂件已删：静默忽略（前端可能正持有旧引用）
            };
            if let Some(v) = p.list_id {
                w.list_id = v;
            }
            if let Some(v) = p.view {
                w.view = v;
            }
            if let Some(v) = p.filter {
                w.filter = v;
            }
            if let Some(v) = p.tag {
                w.tag = v;
            }
            if let Some(v) = p.size {
                w.size = v;
            }
            if let Some(v) = p.rect {
                w.rect = v;
            }
            if let Some(v) = p.visible {
                w.visible = v;
            }
            if let Some(v) = p.auto_dock {
                w.auto_dock = v;
            }
            if let Some(v) = p.edge_hide {
                w.edge_hide = v;
            }
            if let Some(v) = p.free {
                w.free = v;
            }
        })
    }

    /// 字段级改 visible（等价于 patch_widget 只带 visible 字段）
    pub fn set_widget_visible(&self, id: &str, visible: bool) -> Result<(), String> {
        self.patch_widget(
            id,
            crate::domain::WidgetPatch {
                visible: Some(visible),
                ..Default::default()
            },
        )
    }

    pub fn remove_widget(&self, id: String) -> Result<(), String> {
        self.persist(move |s| s.widgets.retain(|w| w.id != id))
    }

    pub fn set_settings(&self, patch: SettingsPatch) -> Result<crate::domain::Settings, String> {
        self.persist(move |s| {
            if let Some(v) = patch.theme {
                s.settings.theme = v;
            }
            if let Some(v) = patch.attach_mode {
                s.settings.attach_mode = v;
            }
            if let Some(v) = patch.opacity {
                s.settings.opacity = v.clamp(0.2, 1.0);
            }
            if let Some(v) = patch.font_px {
                s.settings.font_px = v.clamp(10, 28);
            }
            if let Some(v) = patch.autostart {
                s.settings.autostart = v;
            }
            if let Some(v) = patch.archive_after_days {
                // 夹范围：这两个数字有设置页输入框，但 `type=number` 的
                // min/max 只管步进，手打一个 99999 照样进来。
                // 归档阈值失控 = 完成项永远不归档（主表越涨越大）；
                // 负值更糟：`as i64` 之后 cutoff 跑到未来，所有已完成项
                // 立刻被搬进归档分片，界面上"勾一下就没了"。
                s.settings.archive_after_days = v.clamp(0, 365);
            }
            if let Some(v) = patch.backup_keep {
                // 与 db::keep_count 读盘时的 clamp 同一区间，两处口径一致
                s.settings.backup_keep = v.clamp(1, 100);
            }
            if let Some(v) = patch.snap_gap {
                s.settings.snap_gap = v.clamp(0.0, 200.0);
            }
            if let Some(v) = patch.edge_hide_threshold {
                s.settings.edge_hide_threshold = v.clamp(0.0, 200.0);
            }
            if let Some(v) = patch.auto_postpone {
                s.settings.auto_postpone = v;
            }
            if let Some(v) = patch.ai {
                s.settings.ai = v;
            }
        })?;
        Ok(self.store.read().settings.clone())
    }

    // ── 导入导出 ────────────────────────────────────────────

    pub fn export(&self) -> String {
        serde_json::to_string_pretty(&*self.store.read()).unwrap_or_default()
    }

    /// 周/月总结：本地统计 + 摘要文本。归档分片也要读 —— 月总结若漏掉
    /// 已归档的完成项，数字就会明显偏小。
    /// `list` = Some 时只统计该清单（主窗口选中清单时总结跟着变）。
    pub fn period_summary(&self, kind: &str, list: Option<&str>) -> crate::stats::Summary {
        let store = self.snapshot();
        let archived = self.db.archive_todos();
        let today = chrono::Local::now().date_naive();
        crate::stats::summary_list(&store, &archived, today, kind, list)
    }

    /// 近 n 天完成趋势（分析页）。复用 compute，口径与总结一致。
    /// list 必须与同页分布同值，否则一屏内自相矛盾。
    pub fn trend(&self, n: i64, list: Option<&str>) -> Vec<(String, usize)> {
        let store = self.snapshot();
        let archived = self.db.archive_todos();
        let today = chrono::Local::now().date_naive();
        crate::stats::trend(&store, &archived, today, n, list)
    }

    /// AI 用的摘要 + 配置（命令层要分两步：先本地算，再决定是否发请求）
    pub fn ai_config(&self) -> crate::domain::AiSettings {
        self.store.read().settings.ai.clone()
    }

    pub fn import(&self, json: &str) -> Result<(), String> {
        let raw: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("JSON 解析失败：{e}"))?;
        let from = raw
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if from > Store::CURRENT_VERSION {
            return Err(format!(
                "数据版本 {from} 高于本程序支持的 {}",
                Store::CURRENT_VERSION
            ));
        }
        let store = crate::db::migrate::upgrade(raw, from).map_err(|e| e.to_string())?;
        self.db.replace_with(&store).map_err(|e| e.to_string())?;
        *self.store.write() = store;
        Ok(())
    }

    /// 导入 Markdown 待办（Kite 等第三方导出、或本程序自己导出的那份）。
    ///
    /// **合并语义，与 `import`(JSON 整库替换) 刻意不同**：
    ///   · 分组名命中现有清单 → 并进去；没有 → 新建清单（日期分组用 `fallback_list`）；
    ///   · 条目按「标题 + 到期日」去重，重复的跳过而不是加第二遍；
    ///   · 全程一次 `persist` → 要么整份落盘要么不落，不会导入到一半崩成半份。
    ///
    /// 返回导入统计（见 `ImportStat`）。
    pub fn import_markdown(&self, md: &str, fallback_list: &str) -> Result<ImportStat, String> {
        let parsed = crate::mdimport::parse(md);
        let groups = parsed.groups;
        if groups.iter().all(|g| g.todos.is_empty()) {
            return Err(if parsed.empty_dropped > 0 {
                format!("没解析出任何有内容的待办（另有 {} 条是空条目）", parsed.empty_dropped)
            } else {
                "没解析出任何待办：需要 `- [ ] 标题` 这种任务清单格式".into()
            });
        }
        let fb = fallback_list.trim().to_string();
        let now = now_local_str();
        let empty = parsed.empty_dropped;
        // 计数器必须活在闭包**里面**并作为返回值带出来（见 `persist` 的注释）
        let mut stat = self.persist(move |s| {
            let mut stat = ImportStat::default();
            for g in &groups {
                let name = g.list.clone().unwrap_or_else(|| fb.clone());
                if name.is_empty() {
                    continue;
                }
                let list_id = match s.lists.iter().find(|l| l.name.trim() == name).map(|l| l.id.clone()) {
                    Some(id) => id,
                    None => {
                        let l = TodoList {
                            id: ids::new(),
                            name: name.clone(),
                            color: LIST_COLORS[stat.lists % LIST_COLORS.len()].to_string(),
                            order: s.lists.iter().map(|l| l.order).max().unwrap_or(-1) + 1,
                            collapsed: false,
                            archived: false,
                            exclude_from_stats: false,
                        };
                        let id = l.id.clone();
                        s.lists.push(l);
                        stat.lists += 1;
                        id
                    }
                };
                for t in &g.todos {
                    let due = t.due.clone().or_else(|| g.date.clone());
                    if s.todos.iter().any(|x| x.title == t.title && x.due == due) {
                        stat.dup += 1;
                        continue;
                    }
                    // 已完成条目没有完成时间可用（Kite 的导出里就没有），拿分组日期兜底
                    // —— 这是**推断**，不写就永远进不了"按完成日归档"与趋势图。
                    let done_at = t
                        .done_at
                        .clone()
                        .or_else(|| t.done.then(|| due.as_ref().map(|d| format!("{d}T12:00"))).flatten());
                    // 提醒时刻**确实已过**的条目才标"已提醒"：否则一次导入几十条历史
                    // 会把通知点燃。**没有提醒的条目保持 reminded=false** —— 给它们
                    // 预先打上 true 是撒谎（什么都没弹过），而且以后设提醒时若哪条路径
                    // 忘了复位，就变成"设了提醒永不响"。
                    let fired = t
                        .remind_at
                        .as_ref()
                        .is_some_and(|r| r.as_str() <= now.as_str());
                    let order = s
                        .todos
                        .iter()
                        .filter(|x| x.list_id == list_id)
                        .map(|x| x.order)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    s.todos.push(Todo {
                        id: ids::new(),
                        list_id: list_id.clone(),
                        title: t.title.clone(),
                        note: t.note.clone(),
                        done: t.done,
                        done_at,
                        pinned: false,
                        order,
                        tags: t.tags.clone(),
                        due,
                        remind_at: t.remind_at.clone(),
                        reminded: t.done || fired,
                        extra_reminds: vec![],
                        reminded_keys: vec![],
                        constant_remind: false,
                        last_reminded_at: None,
                        countdown: None,
                        priority: None,
                        recurrence: None,
                        created_at: now.clone(),
                        archived_at: None,
                    });
                    stat.added += 1;
                }
            }
            stat
        })?;
        stat.empty = empty;
        Ok(stat)
    }
}

/// 导入时给新建清单配的颜色（循环取用）
const LIST_COLORS: [&str; 6] = ["#4c8dff", "#30a46c", "#f5c518", "#e5484d", "#a06cf5", "#ff8a4c"];

/// `import_markdown` 的统计。四个字段都要显示给用户 —— 尤其 `empty`：
/// 第三方导出里混着空条目是常态，不报数就成了"导入少了几条却没处问"。
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStat {
    /// 新建的清单数
    pub lists: usize,
    /// 新增的待办数
    pub added: usize,
    /// 因「标题+到期日」重复而跳过的条数
    pub dup: usize,
    /// 解析时丢掉的空条目数（`- [ ]` 后面没字）
    pub empty: usize,
}

// ── 辅助 ──────────────────────────────────────────────────

/// `HH:MM` 合法性：小时 0..=23、分钟 0..=59，且必须是两位:两位。
/// 不合法返回 None（调用方按"没改"处理，绝不落库一个解析不了的时点）。
fn valid_hhmm(s: &str) -> Option<&str> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else {
        return None;
    };
    if h < 24 && m < 60 {
        Some(s)
    } else {
        None
    }
}

/// 把"日历上对一条派生条目的编辑"落到待办身上。
///
/// 派生条目是待办的镜像，三个可编辑字段在待办上各有归属（镜像规则见
/// `todo_derived_events`）：
///   · 标题 → `Todo.title`
///   · 时刻 → `Todo.remind_at` 的 HH:MM（日历上显示的时间本来就是从它取的）
///   · 重复 → `Todo.recurrence`
///
/// 此前只处理标题，而后端弹窗的标题写的是「改时间/标题」、行上的 tooltip
/// 写的是「双击改时间/标题」—— 时间那一半**静默丢弃**：用户把 09:30 改成
/// 10:00，回车后日历上还是 09:30。这是"承诺了但没实现"，不是"没做功能"。
///
/// 改时刻要复位 `reminded` / `last_reminded_at`（与前端 applyRemindTime 同一
/// 规则），否则新时点会因"已提醒过"的判据永不触发。但**只在值真的变了时**
/// 复位：日视图的 ↻ 面板也是整对象回传（时间原样带着），若无条件复位就会
/// 把已经弹过的提醒再弹一次。
fn apply_event_to_todo(t: &mut Todo, ev: &Event) {
    let title = ev.title.trim();
    if !title.is_empty() {
        t.title = title.to_string();
    }

    // 落点日期以**服务端**的 due 为准（派生条目只在 due 那天出现）。
    // 用 ev.date 的话，前端拿着陈旧快照编辑时会把提醒写到错误的日子上。
    let date = t
        .due
        .clone()
        .or_else(|| crate::domain::recurring::parse_date(&ev.date).map(|_| ev.date.clone()));

    match ev.time.as_deref() {
        Some(raw) => {
            // 输入框是自由文本，`shared/event-line.ts` 刻意只做格式识别
            // （"25:00" 也算通过）。日程那边存字符串无所谓，**待办这边不行**：
            // remind_at 写成非法时刻后 `reminder::parse_min` 解析失败，
            // 提醒静默不响 —— 等于把用户原本能响的提醒改没了。
            // 所以非法值按"这次没改时间"处理。
            if let Some(hhmm) = valid_hhmm(raw) {
                if let Some(d) = date {
                    let next = format!("{d}T{hhmm}");
                    if t.remind_at.as_deref() != Some(next.as_str()) {
                        t.remind_at = Some(next);
                        t.reminded = false;
                        t.last_reminded_at = None;
                    }
                }
            }
        }
        // 没带时间前缀 = 用户要"全天"。对派生条目来说，"时刻"就是定点提醒，
        // 所以这里清掉 remind_at（编辑框初值本来就带着原时间，用户删掉它
        // 才是这个意思）。纯日期形态的 remind_at 不动 —— 那不是"时刻"。
        None => {
            if t.remind_at.as_deref().is_some_and(|r| r.contains('T')) {
                t.remind_at = None;
                t.reminded = false;
                t.last_reminded_at = None;
            }
        }
    }

    // 派生条目现在带着待办自己的规则（见 todo_derived_events），所以
    // "没改"与"改成不重复"分得开；不比较就直接写会让只改标题的编辑
    // 把重复规则清空。
    if ev.recurrence != t.recurrence {
        t.recurrence = ev.recurrence.clone();
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub attach_mode: Option<crate::domain::AttachMode>,
    pub opacity: Option<f32>,
    pub font_px: Option<u16>,
    pub autostart: Option<bool>,
    pub archive_after_days: Option<u16>,
    pub backup_keep: Option<u8>,
    pub snap_gap: Option<f64>,
    pub edge_hide_threshold: Option<f64>,
    pub auto_postpone: Option<bool>,
    pub ai: Option<crate::domain::AiSettings>,
}

/// 回收站上限：防止无限增长。超出后丢最老的（列表按插入序，头部最老）。
/// 抽成纯函数是为了能直接测裁剪边界，不必写 500 次盘去触发它。
const TRASH_CAP: usize = 500;

pub fn trim_trash(v: &mut Vec<crate::domain::TrashedTodo>) {
    if v.len() > TRASH_CAP {
        let cut = v.len() - TRASH_CAP;
        v.drain(0..cut);
    }
}

fn s_to_date(s: Option<&str>) -> Option<NaiveDate> {
    let s = s?;
    NaiveDate::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn done_date(t: &Todo) -> Option<NaiveDate> {
    t.done_at
        .as_deref()
        .and_then(|s| s.get(..10))
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

/// 排序键：无到期 = 大值（沉底）；今天/逾期优先
fn due_key(due: &Option<String>, today: &str) -> String {
    match due.as_deref() {
        Some(d) if d <= today => format!("0000-{d}"),
        Some(d) => format!("1111-{d}"),
        None => "9999".into(),
    }
}


