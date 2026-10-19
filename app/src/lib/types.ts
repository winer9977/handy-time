/** 与 src-tauri/src/domain/mod.rs 一一对应的类型（camelCase 已由 serde 保证） */

/** 与 Rust Freq 的 serde 名严格一致。农历两项靠显式 rename 保持 camelCase
 *  （见 domain/mod.rs 注释）—— 写成别的拼法后端反序列化会失败。 */
export type Freq =
  | 'daily'
  | 'weekly'
  | 'monthly'
  | 'yearly'
  | 'workday'
  | 'officialWorkday'
  | 'holiday'
  | 'lunarMonthly'
  | 'lunarYearly'
export type CountdownMode = 'until' | 'since'
export type CalendarView = 'month' | 'week' | 'day'
export type WidgetKind = 'todo' | 'calendar' | 'note'
export type WidgetSize = 's' | 'm' | 'l' | 'xl'
/** 优先级旗标。与 Rust `#[serde(rename_all = "lowercase")]` 严格一致 */
export type Priority = 'high' | 'mid' | 'low'
/** 与 Rust `#[serde(rename_all = "lowercase")]` 严格一致：全小写。
 *  写成 camelCase 会让后端反序列化失败 —— 见 attach_mode 解析测试。
 *  bottomz = 贴所有窗口最底（默认，兼容性好）；workerw = 真·嵌入壁纸层。 */
export type AttachMode = 'workerw' | 'bottomz'

export interface Recurrence {
  freq: Freq
  interval: number
  byWeekday: number[]
  byMonthday: number[]
  until: string | null
  autoCreate: boolean
}

export interface Countdown {
  date: string
  mode: CountdownMode
  keepCountingUp: boolean
}

export interface TodoList {
  id: string
  name: string
  color: string
  order: number
  collapsed: boolean
  /** 归档的清单：不在侧栏主区显示，其待办也不进活动列表；数据保留 */
  archived: boolean
  /** 不参与统计：周/月总结与分析页跳过该清单的待办与日程 */
  excludeFromStats: boolean
}

export interface Todo {
  id: string
  listId: string
  title: string
  note: string
  done: boolean
  doneAt: string | null
  pinned: boolean
  order: number
  tags: string[]
  due: string | null
  remindAt: string | null
  reminded: boolean
  /** 额外提醒时点（`YYYY-MM-DDTHH:MM`）。主提醒仍是 remindAt。对照滴答"多个提醒时间" */
  extraReminds: string[]
  /** 已触发的额外提醒时点（后端维护，前端只读） */
  remindedKeys: string[]
  /** 持续提醒：逾期后每隔一段重弹，直到完成。对照滴答"Constant Reminder" */
  constantRemind: boolean
  /** 上次真正弹通知的时刻（后端维护，前端只读） */
  lastRemindedAt: string | null
  countdown: Countdown | null
  priority: Priority | null
  recurrence: Recurrence | null
  createdAt: string
  archivedAt: string | null
}

/** 待办挂件的筛选口径（对照滴答"智能筛选"）。纯前端视图态，不落库。 */
export type TodoFilter =
  | 'all' // 全部
  | 'today' // 今天/逾期
  | 'week' // 本周内
  | 'flagged' // 带旗标
  | 'someday' // 无到期日
  | 'done' // 已完成

export interface Event {
  id: string
  date: string
  time: string | null
  /** 时长（分钟）；0 = 不定长，只显示开始时刻 */
  durationMin: number
  title: string
  color: string
  source: string
  todoId: string | null
  /** 跨几天：1 = 只占当天；N = 从 date 起连续占 N 天（每天都查得到） */
  spanDays: number
  /** **运行时字段**（后端查询展开时算出，库里不落）：这条 occurrence 是跨天
   *  日程的第几天，0 = 起始日。前端靠它起笔画长条 / 打「续」标记 ——
   *  起始日落在查询窗口之外时（上月就开始、本月还在占着），只看返回结果
   *  自己算不出来。 */
  spanIndex?: number
  /** **运行时字段**：这条日程一共占几天（算长条宽度用） */
  spanTotal?: number
  /** 重复日程：库里只存母记录，查询时虚拟展开 */
  recurrence: Recurrence | null
  /** 归属清单（拖上日历时写入当时的清单，此后不随待办移动）。
   *  后端**总会**返回这个字段（手工日程为 null）；前端手工新建的日程不填
   *  即可 = null = "不属于任何清单、恒显示"（与 state::event_list 同规则）。
   *  声明在这里是为了让 updateEvent 这类整对象回传看得见它，别当成未知字段。 */
  listId?: string | null
}

export interface Note {
  id: string
  body: string
  color: string
  fontPx: number
  opacity: number
  widgetId: string | null
  tags: string[]
  pinned: boolean
  order: number
  updatedAt: string
}

export interface RectL {
  x: number
  y: number
  w: number
  h: number
}

export interface Widget {
  id: string
  kind: WidgetKind
  listId: string | null
  view: CalendarView | null
  /** 待办筛选口径（all/today/week/flagged/someday）。视图态但跨重启保留 */
  filter: TodoFilter | null
  /** 按标签筛选（与 filter 叠加）。null = 不限 */
  tag: string | null
  size: WidgetSize
  rect: RectL
  visible: boolean
  /** 是否允许贴屏幕边缘自动隐藏 */
  autoDock: boolean
  /** 当前停靠边：left / right / top / null */
  edgeHide: string | null
  free: boolean
}

/** 待办字段级补丁：只改给出的字段，其余保持服务端当前值。
 *  取代"发整个 Todo 快照"—— 那会把提醒线程 / set_done / 重复规则刚改的
 *  字段抹回旧值（lost update），例如提醒刚弹完又被前端写回未提醒。
 *  与 Rust `domain::TodoPatch` 一一对应（camelCase）。
 *  可空字段传 `null` = 显式清空；整个字段省略 = 不改。 */
export interface TodoPatch {
  title?: string
  note?: string
  done?: boolean
  pinned?: boolean
  tags?: string[]
  due?: string | null
  remindAt?: string | null
  extraReminds?: string[]
  remindedKeys?: string[]
  constantRemind?: boolean
  /** 改提醒时间时必须一起置 false，否则新时点永不触发 */
  reminded?: boolean
  /** 改提醒时间时一并清空，让持续提醒重新计时 */
  lastRemindedAt?: string | null
  countdown?: Countdown | null
  priority?: Priority | null
  recurrence?: Recurrence | null
}

/** 挂件字段级补丁：只改给出的字段，其余保持服务端当前值。
 *  取代"发整个 Widget 快照覆盖"—— 那种写法两个写者并发时后写的会把
 *  先写的抹掉（lost update），是显隐/尺寸/筛选互相打架的根因。
 *  与 Rust `domain::WidgetPatch` 一一对应（camelCase）。 */
export interface WidgetPatch {
  listId?: string | null
  view?: CalendarView | null
  filter?: string | null
  tag?: string | null
  size?: WidgetSize
  rect?: RectL
  visible?: boolean
  autoDock?: boolean
  edgeHide?: string | null
  free?: boolean
}

/** 快捷键不是设置项：后端 `register_hotkeys` 写死 Alt+Shift+H / Alt+Shift+N。
 *  原先的 `Hotkeys` 接口与 `Settings.hotkeys` 字段属于"看起来能改、改了不生效"，
 *  2026-10-18 按用户决定删除；老 store.json 里残留的键由后端忽略未知字段读通。 */
export interface Settings {
  theme: string
  attachMode: AttachMode
  opacity: number
  fontPx: number
  autostart: boolean
  archiveAfterDays: number
  backupKeep: number
  snapGap: number
  edgeHideThreshold: number
  /** 逾期自动延期（默认关，会在启动时改写到期日） */
  autoPostpone: boolean
  /** AI 总结配置（默认无 key = 不发网络请求，退回本地摘要） */
  ai: AiSettings
}

export interface AiSettings {
  baseUrl: string
  apiKey: string
  model: string
}

/** 周/月总结的统计数字（对照滴答清单的"回顾"） */
export interface PeriodStats {
  from: string
  to: string
  done: number
  created: number
  overdueLeft: number
  completionRate: number
  eventCount: number
  leftTitles: string[]
  byList: [string, number][]
  byTag: [string, number][]
  /** (优先级 high/mid/low, 完成数) 固定序。Rust 侧新增字段，这里必须同步 */
  byPriority: [string, number][]
  perDay: [string, number][]
  bestDay: [string, number] | null
  /** 上一周期同长度统计（无数据时 null，别显示"环比 0%"） */
  prev: PeriodStats | null
}

export interface Summary {
  kind: string
  label: string
  stats: PeriodStats
  /** 本地摘要；没配 AI 时这就是最终展示文本 */
  digest: string
}

/** 回收站条目：待办本体 + 删除时刻（滴答垃圾桶显示"何时删的"） */
export interface TrashedTodo {
  todo: Todo
  trashedAt: string
}

export interface Store {
  version: number
  lists: TodoList[]
  todos: Todo[]
  events: Event[]
  notes: Note[]
  widgets: Widget[]
  settings: Settings
  /** 归档分片（archive/*.jsonl）在 TS 侧**没有**对应字段：Rust 的 Store
   *  里那个 `archived` 索引从来不被写入（run_archive 只写分片），是死字段、
   *  已删。这里要是再声明一份"有 archived 数组"，就是对外面承诺了一份
   *  后端永远不会给的数据 —— 谁写 `store.archived.length` 谁踩空。
   *  要读历史完成项，走 period_summary / done_trend（后端自己拼分片）。 */
  /** 软删除的待办。独立数组而非 Todo 上的标志位，所有现存读取路径天然排除 */
  trashed: TrashedTodo[]
}

export interface Snapshot {
  store: Store
  hiddenAll: boolean
  calendarView: CalendarView
  calendarFocus: string
}

/** SettingsPatch：全字段可选 */
export interface SettingsPatch {
  theme?: string
  attachMode?: AttachMode
  opacity?: number
  fontPx?: number
  autostart?: boolean
  archiveAfterDays?: number
  backupKeep?: number
  snapGap?: number
  edgeHideThreshold?: number
  autoPostpone?: boolean
  ai?: AiSettings
}

export const DEFAULT_SETTINGS: Settings = {
  theme: 'light',
  attachMode: 'bottomz',
  opacity: 1,
  fontPx: 13,
  autostart: false,
  archiveAfterDays: 7,
  backupKeep: 10,
  snapGap: 8,
  edgeHideThreshold: 24,
  autoPostpone: false,
  ai: {
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    apiKey: '',
    model: 'qwen-plus',
  },
}

export function emptyStore(): Store {
  return {
    version: 1,
    lists: [],
    todos: [],
    events: [],
    notes: [],
    widgets: [],
    settings: { ...DEFAULT_SETTINGS },
    trashed: [],
  }
}
