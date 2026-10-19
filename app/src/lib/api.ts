/** 对 Rust 命令层的类型化封装。所有组件只经这里访问后端。 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  Countdown,
  Event,
  Note,
  Priority,
  Recurrence,
  RectL,
  Settings,
  SettingsPatch,
  Snapshot,
  Summary,
  Todo,
  TodoList,
  TodoPatch,
  Widget,
  WidgetPatch,
} from './types'

export const api = {
  snapshot: () => invoke<Snapshot>('snapshot'),

  addList: (name: string, color: string) =>
    invoke<TodoList>('add_list', { name, color }),
  updateList: (list: TodoList) => invoke<void>('update_list', { list }),
  removeList: (id: string) => invoke<void>('remove_list', { id }),
  /** 归档 / 取消归档清单（归档后不进侧栏主区与活动列表，数据保留） */
  setListArchived: (id: string, archived: boolean) =>
    invoke<void>('set_list_archived', { id, archived }),
  /** 某清单是否**不参与统计**：周/月总结与分析页会跳过它的待办与日程 */
  setListExcludeStats: (id: string, exclude: boolean) =>
    invoke<void>('set_list_exclude_stats', { id, exclude }),
  reorderLists: (ids: string[]) => invoke<void>('reorder_lists', { ids }),

  addTodo: (listId: string, title: string) =>
    invoke<Todo>('add_todo', { listId, title }),
  /** 整对象写入：仅留给导入/迁移。日常改字段用 patchTodo（避免陈旧快照
   *  抹掉并发写者的改动 —— 提醒线程、set_done、重复规则生成都会改同一行） */
  updateTodo: (todo: Todo) => invoke<void>('update_todo', { todo }),
  /** 字段级补丁：只改给出的字段 */
  patchTodo: (id: string, patch: TodoPatch) =>
    invoke<void>('patch_todo', { id, patch }),
  removeTodo: (id: string) => invoke<void>('remove_todo', { id }),
  setDone: (id: string, done: boolean) =>
    invoke<void>('set_done', { id, done }),
  togglePin: (id: string) => invoke<void>('toggle_pin', { id }),
  reorderTodo: (movedId: string, targetId: string | null, after: boolean) =>
    invoke<void>('reorder_todo', { movedId, targetId, after }),
  setCountdown: (id: string, countdown: Countdown | null) =>
    invoke<void>('set_countdown', { id, countdown }),
  setRecurrence: (id: string, recurrence: Recurrence | null) =>
    invoke<void>('set_recurrence', { id, recurrence }),
  setPriority: (id: string, priority: Priority | null) =>
    invoke<void>('set_priority', { id, priority }),

  addEvent: (event: Event) => invoke<Event>('add_event', { event }),
  updateEvent: (event: Event) => invoke<void>('update_event', { event }),
  removeEvent: (id: string) => invoke<void>('remove_event', { id }),
  todoToEvent: (todoId: string, date: string) =>
    invoke<Event>('todo_to_event', { todoId, date }),
  moveEvent: (eventId: string, date: string) =>
    invoke<void>('move_event', { eventId, date }),
  /** 区间日程，含重复日程的虚拟展开（规则只在后端一份，前端不重算） */
  /** list = 选中清单 id：只保留该清单待办派生的条目（手工日程不受影响） */
  eventsInRange: (from: string, to: string, list?: string | null) =>
    invoke<Event[]>('events_in_range', { from, to, list: list ?? null }),

  addNote: (body: string) => invoke<Note>('add_note', { body }),
  updateNote: (note: Note) => invoke<void>('update_note', { note }),
  removeNote: (id: string) => invoke<void>('remove_note', { id }),
  toggleNotePin: (id: string) => invoke<void>('toggle_note_pin', { id }),
  reorderNote: (id: string, targetId: string | null, after: boolean) =>
    invoke<void>('reorder_note', { id, targetId, after }),

  /** 仅用于新建挂件。改已有挂件一律用 patchWidget（字段级，避免互相抹掉） */
  upsertWidget: (widget: Widget) => invoke<void>('upsert_widget', { widget }),
  /** 字段级补丁：只改 patch 里给出的字段 */
  patchWidget: (id: string, patch: WidgetPatch) =>
    invoke<void>('patch_widget', { id, patch }),
  removeWidget: (id: string) => invoke<void>('remove_widget', { id }),
  /** 收起/唤出单个挂件（写 store.visible，唯一真相源） */
  setWidgetVisible: (id: string, visible: boolean) =>
    invoke<void>('set_widget_visible', { id, visible }),
  showAllWidgets: () => invoke<void>('show_all_widgets'),

  // ── 回收站（对照滴答「垃圾桶」）──
  /** 软删除：进回收站，可恢复。日常删除走这个，不是 removeTodo */
  trashTodo: (id: string) => invoke<void>('trash_todo', { id }),
  restoreTodo: (id: string) => invoke<Todo | null>('restore_todo', { id }),
  /** 彻底删除（不可恢复） */
  purgeTodo: (id: string) => invoke<void>('purge_todo', { id }),
  emptyTrash: () => invoke<number>('empty_trash'),
  saveWidgetRect: (id: string, rect: RectL) =>
    invoke<void>('save_widget_rect', { id, rect }),

  setSettings: (patch: SettingsPatch) =>
    invoke<Settings>('set_settings', { patch }),
  reapplyAttach: () => invoke<string>('reapply_desktop_attach'),
  toggleHideAll: () => invoke<boolean>('toggle_hide_all'),
  setAutostart: (enable: boolean) => invoke<void>('set_autostart', { enable }),
  showLauncher: () => invoke<void>('show_launcher'),
  autoAlign: () => invoke<void>('auto_align_command'),

  /** 前端诊断日志：写进数据目录的 xhy.log（跨窗口拖拽这类真机问题靠它定位） */
  logLine: (source: string, msg: string) =>
    invoke<void>('log_line', { source, msg }).catch(() => {}),

  exportData: () => invoke<string>('export_data'),
  /** 弹另存为对话框导出 Markdown */
  saveMarkdown: () => invoke<void>('save_markdown_file'),
  importData: (json: string) => invoke<void>('import_data', { json }),
  /** 导入 Markdown 待办（合并语义）。文件版走系统选文件对话框，结果由
   *  `xhy://md-import` 事件带回界面（对话框回调是异步的，命令拿不到返回值）。 */
  importMarkdown: (md: string, listName?: string) =>
    invoke<string>('import_markdown', { md, listName }),
  importMarkdownFile: () => invoke<void>('import_markdown_file'),
  dataDir: () => invoke<string>('data_dir'),
  quickParse: (text: string) =>
    invoke<{ title: string; tags: string[]; due: string | null; time: string | null }>(
      'quick_parse',
      { text },
    ),
  holidaysInRange: (from: string, to: string) =>
    invoke<HolidayInfo[]>('holidays_in_range', { from, to }),
  /** 区间内每天农历标签（初一显示月名，其余显示日名） */
  lunarInRange: (from: string, to: string) =>
    invoke<LunarInfo[]>('lunar_in_range', { from, to }),
  /** 周/月总结：本地统计 + 摘要（kind: 'week' | 'month'） */
  /** list = 选中清单 id，统计跟着过滤；不传 = 全局 */
  periodSummary: (kind: string, list?: string | null) =>
    invoke<Summary>('period_summary', { kind, list: list ?? null }),
  /** AI 润色版总结；未配置或失败会 reject，调用方降级用 periodSummary.digest */
  aiSummary: (kind: string, list?: string | null) =>
    invoke<string>('ai_summary', { kind, list: list ?? null }),
  /** 近 n 天完成趋势（分析页）。与总结同一口径（后端复用 compute）。
   *  list 必须与同页分布传同一个值，否则一屏内两个数字自相矛盾。 */
  doneTrend: (days: number, list?: string | null) =>
    invoke<[string, number][]>('done_trend', { days, list: list ?? null }),
}

export interface LunarInfo {
  date: string
  label: string
}

export interface HolidayInfo {
  date: string
  name: string
  off: boolean
}

/** 后端变更事件。scope: lists/todos/events/notes/widgets/settings/all/layout/hidden/remind/aligned */
export function onChanged(fn: (scope: string) => void): Promise<UnlistenFn> {
  return listen<string>('xhy://changed', (e) => fn(e.payload))
}

export function onEvent<T>(name: string, fn: (p: T) => void): Promise<UnlistenFn> {
  return listen<T>(`xhy://${name}`, (e) => fn(e.payload as T))
}
