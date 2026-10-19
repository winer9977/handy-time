//! 领域模型：待办 / 日程 / 便签 / 挂件 / 设置
//!
//! 所有日期时间字段用字符串存（ISO-8601 本地日期 `YYYY-MM-DD`，
//! 或本地日期时间 `YYYY-MM-DDTHH:MM`），避免时区在 JSON 里漂移：
//! 桌面待办的"今天"是用户墙上时间，不是 UTC。

use serde::{Deserialize, Serialize};

pub mod recurring;

/// 存储根：一个 JSON 文件整体读写
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub version: u32,
    #[serde(default)]
    pub lists: Vec<TodoList>,
    #[serde(default)]
    pub todos: Vec<Todo>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub settings: Settings,
    /// 回收站：软删除的待办。**独立数组而非 Todo 上的标志位**，
    /// 这样所有现存读取路径（todos.iter()）天然排除它们，不必到处加过滤，
    /// 也就不会漏一处就让用户看到"删了还在"。
    #[serde(default)]
    pub trashed: Vec<TrashedTodo>,
}

/// 回收站条目：待办本体 + 删除时刻（滴答垃圾桶要显示"何时删的"）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedTodo {
    pub todo: Todo,
    pub trashed_at: String,
}

impl Store {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn seeded() -> Self {
        let mut s = Self {
            version: Self::CURRENT_VERSION,
            ..Default::default()
        };
        s.lists.push(TodoList {
            id: crate::ids::new(),
            name: "收集箱".into(),
            color: "#f5c518".into(),
            order: 0,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoList {
    pub id: String,
    pub name: String,
    pub color: String,
    pub order: i32,
    pub collapsed: bool,
    /// 归档的清单：不在侧栏主区显示，改放「已归档」区；其待办仍在库中，
    /// 只是不再出现在活动列表里。对照滴答清单的清单归档。
    /// serde(default) 保证老库直接可读。
    #[serde(default)]
    pub archived: bool,
    /// 不参与统计：周/月总结与分析页会跳过该清单的待办与日程。
    ///
    /// 为什么做成"清单自己的属性"而不是全局的"排除列表"：
    ///   · 属性跟着清单走，删清单不会留下悬空 id 需要清理；
    ///   · 语义直观——用户想的就是"这个清单不计入"。
    /// 典型用途：记录生活琐事/机器人的清单不想混进工作回顾的完成率。
    /// serde(default) 保证老库直接可读（false）。
    #[serde(default)]
    pub exclude_from_stats: bool,
}

/// 重复规则。对齐小黄条"灵活的重复规则，自动为重复事项创建待办"
///
/// `PartialEq` 不只是方便：日历上的派生条目要把"日程带的规则"与"待办现有
/// 规则"做比较，才能区分「这次没改重复」与「改成不重复」（整对象回传的
/// `None` 本身没有这个表达力，见 state::update_event 的 todo: 分支）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recurrence {
    pub freq: Freq,
    /// freq 步进；weekly 时忽略（用 by_weekday）
    #[serde(default = "one")]
    pub interval: u16,
    /// weekly：0=周一 … 6=周日
    #[serde(default)]
    pub by_weekday: Vec<u8>,
    /// monthly：1..=31；负数表示倒数（-1=当月最后一天）
    #[serde(default)]
    pub by_monthday: Vec<i8>,
    /// 结束日期；None = 无限重复
    #[serde(default)]
    pub until: Option<String>,
    /// true：完成时自动生成下一条；false：仅在到期日提醒重复项
    #[serde(default = "yes")]
    pub auto_create: bool,
}

fn one() -> u16 {
    1
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
    /// 工作日（周一至周五，不看节假日）
    Workday,
    /// 官方工作日（对照滴答 "official working days"）：周一至周五**且扣除法定
    /// 假日**，调休上班的周末也算。数据来自 holiday-cn。
    #[serde(rename = "officialWorkday")]
    OfficialWorkday,
    /// 节假日（对照滴答 "holidays"）：法定假日 + 普通周末（被调休上班占用的除外）
    Holiday,
    /// 农历每月（对照滴答 "lunar monthly cycle"，如每月初一）。
    /// 显式 rename：`rename_all="lowercase"` 会给出 "lunarmmonthly" 这种
    /// 无分隔名，前端契约极易写错（本项目 attach_mode 就栽过大小写）。
    #[serde(rename = "lunarMonthly")]
    LunarMonthly,
    /// 农历每年（对照滴答 "lunar yearly cycle"，如农历生日 / 端午）
    #[serde(rename = "lunarYearly")]
    LunarYearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CountdownMode {
    /// 还有 N 天（目标日在未来）
    Until,
    /// 已经 N 天（目标日在过去）
    Since,
}

/// 优先级旗标（对照滴答清单/Todoist —— 调研中使用率最高的任务字段之一）
///
/// 只有三级：滴答的"高/中/低/无"四级里，"中"和"低"用户几乎不分。
/// 三档 + 无 足够表达"这事急不急"，且排序键好定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Mid,
    Low,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::High => "高",
            Priority::Mid => "中",
            Priority::Low => "低",
        }
    }
    /// 排序权重：小的排前面。无优先级排在所有有优先级之后。
    pub fn rank(self) -> u8 {
        match self {
            Priority::High => 0,
            Priority::Mid => 1,
            Priority::Low => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Countdown {
    pub date: String,
    pub mode: CountdownMode,
    /// 到达目标日后是否自动转为"已经 N 天"继续显示
    #[serde(default)]
    pub keep_counting_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub list_id: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub done_at: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// 同清单内的手动排序键；小者在前
    pub order: i32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub remind_at: Option<String>,
    /// 已触发过的提醒分钟，防止重启后重复弹窗
    #[serde(default)]
    pub reminded: bool,
    /// 额外提醒时点（`YYYY-MM-DDTHH:MM`）。主提醒仍是 `remind_at`。
    /// 对照滴答 "Choose multiple alert times"。
    #[serde(default)]
    pub extra_reminds: Vec<String>,
    /// 已触发的**额外**提醒时点集合（主提醒的已触发仍用 `reminded`，
    /// 保持对老数据的向后兼容 —— 老库没有本字段，判据也不会误重弹）。
    #[serde(default)]
    pub reminded_keys: Vec<String>,
    /// 持续提醒：逾期后每隔一段重弹，直到完成。对照滴答 "Constant Reminder"。
    #[serde(default)]
    pub constant_remind: bool,
    /// 上次真正弹通知的时刻（`YYYY-MM-DDTHH:MM`），持续提醒节流用。
    /// 没有它，20s 一轮的轮询会把"持续提醒"变成"疯狂弹窗"。
    #[serde(default)]
    pub last_reminded_at: Option<String>,
    #[serde(default)]
    pub countdown: Option<Countdown>,
    /// 优先级旗标（! / !! / !!! 或 P1/P2/P3 快捷设置）
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub recurrence: Option<Recurrence>,
    pub created_at: String,
    #[serde(default)]
    pub archived_at: Option<String>,
}

/// **待办字段级补丁**。与 WidgetPatch 同一套路，解决同一类问题：
/// 前端 12 处 `updateTodo({...陈旧快照, 单字段})` 都是**整对象回写**，
/// 会把别的写者刚改的字段抹掉 —— 最典型的是提醒线程刚把 `reminded`
/// 置真、用户同时改备注，前端用旧快照回写又把它抹回 false，提醒重复弹。
///
/// 双层 Option 语义同 WidgetPatch：外层 None = 不改；`Some(None)` = 清空。
/// 可空字段一律配 `de_opt_opt`（serde 会把 null 折叠成外层 None）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TodoPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub due: Option<Option<String>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub remind_at: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_reminds: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminded_keys: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constant_remind: Option<bool>,
    /// 主提醒是否已触发。**改提醒时间时必须一起置回 false**，否则新时点
    /// 永远不触发（依赖 `!reminded` 判据）。此前前端发整对象带
    /// `reminded:false` 是隐式的；迁到字段级补丁后这个字段被漏掉，
    /// 成了真实回归（新设的提醒静默不响）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminded: Option<bool>,
    /// 上次弹通知时刻（持续提醒节流基准）。改提醒时间时一并清空重新计时。
    #[serde(deserialize_with = "de_opt_opt")]
    pub last_reminded_at: Option<Option<String>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub countdown: Option<Option<Countdown>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub priority: Option<Option<Priority>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub recurrence: Option<Option<Recurrence>>,
}

/// 挂件**字段级补丁**。架构性修正：此前所有写者（前端 8 处 + Rust 数处）
/// 都发"整个 Widget 快照"覆盖服务端，谁后写谁就把别人刚改的字段抹掉
/// —— 显隐/尺寸/筛选互相打架、且无法自愈，是"同一错误反复出现"的根因。
///
/// 双层 Option 语义：外层 None = 本次不改这个字段；
/// `Some(None)` = 显式清空（如取消标签筛选）。
///
/// **可空字段必须配 `de_opt_opt` 手写反序列化**：serde 对 `Option<Option<T>>`
/// 会把 JSON `null` 折叠成**外层 None**（=不改），于是前端发 `{listId:null}`
/// 的「全」按钮静默失效（实测踩过，tests/widget_patch_wire.rs 锁死三种状态：
/// 缺失→不改 / null→清空 / 有值→设置）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WidgetPatch {
    #[serde(deserialize_with = "de_opt_opt")]
    pub list_id: Option<Option<String>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub view: Option<Option<CalendarView>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub filter: Option<Option<String>>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub tag: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<WidgetSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<RectL>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_dock: Option<bool>,
    #[serde(deserialize_with = "de_opt_opt")]
    pub edge_hide: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free: Option<bool>,
}

/// 把 JSON 值反序列化成"显式给出"语义：`null` → Some(None)（清空），
/// 有值 → Some(Some(v))（设置）。字段整个缺失时走容器级 `default`，
/// 外层保持 None（不改）—— 三种状态由此可分。
fn de_opt_opt<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    Ok(Some(Option::<T>::deserialize(d)?))
}

/// 日历日程。todo_id 非空表示由待办拖拽生成（可回写来源）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub date: String,
    /// "HH:MM"；None = 全天日程
    #[serde(default)]
    pub time: Option<String>,
    /// 时长（分钟）。**业务字段，落库**：0 = 不定长（只显示开始时刻）。
    /// 定时日程用它算出结束时刻显示成 `09:30–10:30`，与滴答"开始时间+时长"同形。
    /// 此前它只有字段没有功能（前端一律发 0、界面也不显示），本轮接上。
    #[serde(default)]
    pub duration_min: u32,
    pub title: String,
    pub color: String,
    /// 来源标记：todo / note / manual，用于配色区分
    #[serde(default = "manual_source")]
    pub source: String,
    #[serde(default)]
    pub todo_id: Option<String>,
    /// 全天日程在月历上占几行（跨天）。**业务字段，落库**：
    /// 1 = 只占当天；N = 从 `date` 起连续占 N 天（查询时每一天都会出现）。
    #[serde(default = "one_day")]
    pub span_days: u16,
    /// **运行时字段，不落库**（写回时一律清零，见 state::add_event/update_event）：
    /// 这条 occurrence 是这条跨天日程的第几天 —— 0 = 起始日，>0 = 续日。
    /// 前端靠它画长条（起始日起笔）与"续"标记；没有它的话，当**起始日落在查询
    /// 区间之外**（上月就开始、这个月还在占着）时前端无法区分"这就是起始"还是
    /// "尾巴伸进来了"。
    #[serde(default)]
    pub span_index: u16,
    /// **运行时字段，不落库**：这条日程一共占几天（展开时带出，前端算长条宽度）。
    #[serde(default = "one_day")]
    pub span_total: u16,
    /// 重复日程。查询时按规则**虚拟展开**成多个 occurrence，
    /// 库里只存一条母记录（对齐小黄条"灵活的重复日程，自动创建重复日程"，
    /// 也避免为长期重复项堆积行）
    #[serde(default)]
    pub recurrence: Option<Recurrence>,
    /// 归属清单。**拖待办上日历时写入当时的清单**，此后不随待办移动。
    /// 为什么必须落库而不是查时反查待办：待办被删除/软删后反查落空，
    /// 该日程在任何清单下都不显示 —— 实测就是"选收集箱日历全空"的根因
    /// （用户删过一条待办，剩下两条日程的 todo_id 指向已删待办）。
    /// 老数据没有此字段（None）→ 由 `state::event_list` 回落反查。
    #[serde(default)]
    pub list_id: Option<String>,
}

fn manual_source() -> String {
    "manual".into()
}
fn one_day() -> u16 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub body: String,
    pub color: String,
    pub font_px: u16,
    /// 0.0–1.0 窗口不透明度
    pub opacity: f32,
    /// **保留字段，当前无功能**：原设计意图是"多便签共用一个窗口堆叠"，
    /// 但从未实现 —— 只在新建议 `None`，前端也无任何读写。
    /// 不删是为了兼容老库（serde 会忽略未知字段，删了也能读，但保留更保守）；
    /// 标注清楚以免后人误以为有这个能力。
    #[serde(default)]
    pub widget_id: Option<String>,
    /// `[标签]` 自动分类（对齐官网"记录时自动贴标签"）
    #[serde(default)]
    pub tags: Vec<String>,
    /// 置顶便签恒排最前
    #[serde(default)]
    pub pinned: bool,
    pub order: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    Todo,
    Calendar,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CalendarView {
    #[default]
    Month,
    Week,
    Day,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WidgetSize {
    S,
    #[default]
    M,
    L,
    Xl,
}

/// 逻辑像素矩形（左上角屏幕坐标）。物理换算见 win::geometry
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RectL {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Widget {
    pub id: String,
    pub kind: WidgetKind,
    /// todo 挂件绑定的清单；None = 全部清单聚合
    #[serde(default)]
    pub list_id: Option<String>,
    #[serde(default)]
    pub view: Option<CalendarView>,
    /// 待办挂件的筛选口径（all/today/week/flagged/someday）。与日历 `view`
    /// 同理：视图态但跨重启保留。None = 全部。
    #[serde(default)]
    pub filter: Option<String>,
    /// 按标签筛选（None = 不限）。与 filter 叠加。
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub size: WidgetSize,
    pub rect: RectL,
    pub visible: bool,
    /// 是否允许"贴屏幕边缘自动隐藏"。用户可用头部 ⇥ 关闭。
    #[serde(default = "yes")]
    pub auto_dock: bool,
    /// 当前停靠的边：left / right / top / None。由贴边状态机维护，
    /// 重启后据此恢复停靠态。
    #[serde(default)]
    pub edge_hide: Option<String>,
    /// 用户手动摆放位置后为 true；false 时参与自动对齐布局
    #[serde(default)]
    pub free: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachMode {
    /// 嵌入壁纸层 WorkerW（真·桌面挂件）
    WorkerW,
    /// 常驻最底层 Z-order（兜底，兼容性最好）
    BottomZ,
}

/// 快捷键。**刻意不是设置项**：两个热键在 `widgets::register_hotkeys` 里写死
/// （Alt+Shift+H 显示/隐藏全部挂件、Alt+Shift+N 快速添加）。
/// 这里原先有一个 `Settings.hotkeys` 结构 + patch 通道，但从来没有 UI 写它、
/// 也从没有代码读它去注册热键 —— 一个"看起来能改、改了永远不生效"的字段
/// 比没有更糟，2026-10-18 按用户决定删掉。老 store.json 里残留的 `hotkeys`
/// 键由 serde 忽略未知字段自然读通。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    /// 桌面嵌入模式。**默认 BottomZ**（保守：兼容所有 Win10/11 版本，
    /// 且输入行为已实机验证）。WorkerW 是"真·嵌入壁纸层"，在本机实测可用，
    /// 但属未公开手法，故作为设置项由用户显式开启；探测失败会自动降级。
    pub attach_mode: AttachMode,
    pub opacity: f32,
    pub font_px: u16,
    /// 开机自启。**默认关**：不自作主张往 Run 键里写东西，
    /// 由用户在设置里明确打开（打开后 sync_autostart 保证注册表与之一致）。
    pub autostart: bool,
    /// 归档阈值：完成日早于 N 天前则归档
    pub archive_after_days: u16,
    /// 备份保留份数
    pub backup_keep: u8,
    /// 自动对齐网格边距（逻辑像素）
    pub snap_gap: f64,
    /// 贴边隐藏触发距离（逻辑像素）
    pub edge_hide_threshold: f64,
    /// 逾期自动延期（对照 Kite）：启动时把已逾期未完成待办的到期日推到今天。
    /// **默认关** —— 它会改写用户数据，必须由用户显式开启。
    #[serde(default)]
    pub auto_postpone: bool,
    /// AI 总结配置（周/月回顾的润色层）。**默认空 = 不发任何网络请求**，
    /// 总结退回本地统计摘要。
    #[serde(default)]
    pub ai: AiSettings,
}

/// OpenAI 兼容接口的三件套。默认预填阿里云百炼（千问）的兼容端点，
/// 但 **api_key 一律留空**：程序绝不擅自读取系统里其它工具的凭据，
/// 也不凭空造一个"看起来能用"的 key —— 空 key 时 ready() 直接不发请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    #[serde(default = "default_ai_base")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_ai_model")]
    pub model: String,
}

fn default_ai_base() -> String {
    "https://dashscope.aliyuncs.com/compatible-mode/v1".into()
}
fn default_ai_model() -> String {
    "qwen-plus".into()
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            base_url: default_ai_base(),
            api_key: String::new(),
            model: default_ai_model(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "light".into(),
            attach_mode: AttachMode::BottomZ,
            opacity: 1.0,
            font_px: 13,
            autostart: false,
            archive_after_days: 7,
            backup_keep: 10,
            snap_gap: 8.0,
            edge_hide_threshold: 24.0,
            auto_postpone: false,
            ai: AiSettings::default(),
        }
    }
}
