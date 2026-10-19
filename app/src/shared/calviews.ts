/** 日历多视图的纯逻辑（对照滴答真实界面：视图类型 + 数量下拉「2 周」）
 *
 * 滴答把"周/日/多日/多周"做成 **视图 + 数量** 两个维度（顶栏那个 `2 周 ▾`
 * 就是数量选择器）。这里照同一模型：视图决定"怎么铺"，count 决定"铺几份"。
 *
 * 为什么单独成文件：视图切换最容易错的是"这次到底覆盖哪几天、翻页翻多
 * 少、标题显示什么" —— 这些是纯日期算术，必须可单测，不能埋在模板里。
 * 数据（日程/农历/节假日）一律走已有命令，本模块不碰 IO。
 *
 * 周起点沿用本项目既定的**周一**（date.ts startOfWeek），不跟滴答的周日起
 * —— 与挂件、后端 byWeekday 约定一致，改一处要改三处。
 */
import { addDays, monthGrid, parseYmd, startOfWeek, WD_CN, ymd } from './date'

export type CalView = 'year' | 'month' | 'week' | 'day' | 'days' | 'weeks'

export interface CalRange {
  from: string
  to: string
}

/** 数量上限：再多一屏也放不下，且 eventsInRange 要拉的区间会失控 */
export const MAX_COUNT = 14

/** 视图的中文标签与提示 */
export const CAL_VIEWS: { k: CalView; label: string; tip: string; hasCount: boolean }[] = [
  { k: 'year', label: '年', tip: '12 个月概览，看全年分布', hasCount: false },
  { k: 'month', label: '月', tip: '标准月历（6 行）', hasCount: false },
  { k: 'week', label: '周', tip: '本周 7 天分栏', hasCount: false },
  { k: 'day', label: '日', tip: '只看某一天', hasCount: false },
  { k: 'days', label: '多日', tip: '从当天起连续 N 天分栏', hasCount: true },
  { k: 'weeks', label: '多周', tip: '从本周起连续 N 周（滴答的「2 周」）', hasCount: true },
]

export function clampCount(n: number): number {
  if (!Number.isFinite(n)) return 1
  return Math.max(1, Math.min(Math.round(n), MAX_COUNT))
}

/** 该视图要覆盖的日期闭区间 —— 取日程/农历/节假日都按它来 */
export function rangeOf(view: CalView, focus: string, count = 1): CalRange {
  switch (view) {
    case 'year': {
      const y = focus.slice(0, 4)
      return { from: `${y}-01-01`, to: `${y}-12-31` }
    }
    case 'month': {
      const cs = monthGridOf(focus)
      return { from: cs[0]!.date, to: cs[cs.length - 1]!.date }
    }
    case 'week': {
      const s = startOfWeek(focus)
      return { from: s, to: addDays(s, 6) }
    }
    case 'day':
      return { from: focus, to: focus }
    case 'days': {
      const n = clampCount(count)
      return { from: focus, to: addDays(focus, n - 1) }
    }
    case 'weeks': {
      const s = startOfWeek(focus)
      return { from: s, to: addDays(s, 7 * clampCount(count) - 1) }
    }
  }
}

function monthGridOf(focus: string) {
  const d = parseYmd(focus)
  return monthGrid(d.getFullYear(), d.getMonth())
}

/** 要渲染成"日格"的日期序列。
 *  年视图返回 [] —— 它渲染 12 个小月格，走 yearMonths()，不是日格。 */
export function dayCells(view: CalView, focus: string, count = 1): string[] {
  if (view === 'year') return []
  const { from, to } = rangeOf(view, focus, count)
  const out: string[] = []
  let d = from
  // 区间长度已由 rangeOf 限定（月 42 天、多周 ≤ 98 天），不会失控
  while (d <= to) {
    out.push(d)
    d = addDays(d, 1)
  }
  return out
}

/** 多周视图按"周"分行（每行 7 天）。月视图也能用它画 6 行。 */
export function weeksRows(dates: string[]): string[][] {
  const rows: string[][] = []
  for (let i = 0; i < dates.length; i += 7) rows.push(dates.slice(i, i + 7))
  return rows
}

export interface YearMonth {
  year: number
  month0: number
  label: string
  /** 该月 1~末 的日期串，用于数事件 */
  days: string[]
}

/** 年视图的 12 个月 */
export function yearMonths(focus: string): YearMonth[] {
  const y = parseYmd(focus).getFullYear()
  const out: YearMonth[] = []
  for (let m = 0; m < 12; m++) {
    const last = new Date(y, m + 1, 0).getDate()
    const days: string[] = []
    for (let d = 1; d <= last; d++) days.push(ymd(new Date(y, m, d)))
    out.push({ year: y, month0: m, label: `${m + 1} 月`, days })
  }
  return out
}

/** 翻一页：各视图按自己的自然周期走。
 *  月/年回到 1 号，避免"1 月 31 日翻一月落到 3 月 3 日"这种溢出漂移。 */
export function stepView(view: CalView, focus: string, count: number, delta: number): string {
  switch (view) {
    case 'year': {
      const d = parseYmd(focus)
      return ymd(new Date(d.getFullYear() + delta, 0, 1))
    }
    case 'month': {
      const d = parseYmd(focus)
      return ymd(new Date(d.getFullYear(), d.getMonth() + delta, 1))
    }
    case 'week':
      return addDays(focus, delta * 7)
    case 'day':
      return addDays(focus, delta)
    case 'days':
      return addDays(focus, delta * clampCount(count))
    case 'weeks':
      return addDays(focus, delta * 7 * clampCount(count))
  }
}

function md(date: string): string {
  return `${Number(date.slice(5, 7))}/${Number(date.slice(8, 10))}`
}

/** 顶栏标题。跨月/跨年才带年份，省得窄栏里换行 */
export function labelOf(view: CalView, focus: string, count = 1): string {
  if (view === 'year') return `${parseYmd(focus).getFullYear()} 年`
  if (view === 'month') {
    const d = parseYmd(focus)
    return `${d.getFullYear()} 年 ${d.getMonth() + 1} 月`
  }
  const { from, to } = rangeOf(view, focus, count)
  if (view === 'day') {
    const d = parseYmd(from)
    const wd = WD_CN[(d.getDay() + 6) % 7]
    return `${d.getFullYear()} 年 ${d.getMonth() + 1} 月 ${d.getDate()} 日 · 周${wd}`
  }
  const crossYear = from.slice(0, 4) !== to.slice(0, 4)
  return crossYear ? `${from} – ${to}` : `${md(from)} – ${md(to)}`
}

/** 数量下拉的可选值（滴答给的是 2/3/4 周；这里覆盖常用档）
 *
 * 必须从 2 起：count 的默认值是 2，若候选集不含它，`<select>` 会因
 * "当前值不在选项里"而渲染成**空白**（实机截图里就多日视图出现过）。 */
export function countOptions(view: CalView): number[] {
  if (view === 'weeks') return [2, 3, 4]
  if (view === 'days') return [2, 3, 4, 5, 7, 10, 14]
  return [1]
}

/** 该视图是否需要数量选择器 */
export function hasCount(view: CalView): boolean {
  return CAL_VIEWS.find((v) => v.k === view)?.hasCount ?? false
}

export function isWeekend(date: string): boolean {
  const wd = parseYmd(date).getDay()
  return wd === 0 || wd === 6
}

/** 同一天里日程的排序：**全天在前**，其余按时刻、再按标题。
 *
 *  为什么单独抽出来：挂件月历角标、挂件日视图、主窗口日历各排一次，
 *  以前两处写的是 `(a.time ?? '99') < (b.time ?? '99')` —— 兜底值 '99'
 *  比任何 "HH:MM" 都大，于是**全天日程被排到最后**，而后端
 *  `state::events_on` 按 `Option<time>` 排（None 最小）= 全天在最前。
 *  同一个日子在挂件和工作台里顺序相反，就是"两份实现必然漂移"的教科书样本。
 *
 *  空串 '' 天然小于任何 "HH:MM" → 全天自动置顶，不需要兜底魔数。
 */
export function sortEvents<T extends { time: string | null; title: string }>(list: T[]): T[] {
  return [...list].sort((a, b) => {
    const ta = a.time ?? ''
    const tb = b.time ?? ''
    if (ta !== tb) return ta < tb ? -1 : 1
    return a.title.localeCompare(b.title)
  })
}

// ── 跨天与时长（两处日历共用一份数学）──────────────────────
import { pad } from './date'
import type { Event } from '../lib/types'

/** 时刻 + 分钟 → 结束时刻；跨过零点则 cross=true（显示成 `23:40–00:40+1`） */
export function addMinutes(hhmm: string, minutes: number): { text: string; cross: boolean } {
  const [h, m] = hhmm.split(':').map(Number)
  const start = (h || 0) * 60 + (m || 0)
  const total = start + Math.max(0, minutes)
  return {
    text: `${pad(Math.floor((total % 1440) / 60))}:${pad(total % 60)}`,
    cross: total >= 1440,
  }
}

/** 日程行的时间文案：`全天` / `09:30` / `09:30–10:30` / `23:40–00:40+1`。
 *  挂件日视图与主窗口日视图必须同一份，否则一边显示结束时刻一边不显示。
 *  时刻串不是 `H:MM` 形状（脏数据）时**原样显示**，绝不退化成"全天"。 */
export function eventTimeLabel(e: { time: string | null; durationMin?: number }): string {
  if (!e.time) return '全天'
  const d = e.durationMin ?? 0
  if (d <= 0 || !/^\d{1,2}:\d{2}$/.test(e.time)) return e.time
  const end = addMinutes(e.time, d)
  return `${e.time}–${end.text}${end.cross ? '+1' : ''}`
}

/** 一行（一周 / 一段多日窗口）里的一条跨天长条 */
export interface SpanSegment {
  id: string
  title: string
  color: string
  /** 本行内的列号，闭区间 [from, to]，0 起 */
  from: number
  to: number
  /** 起始日落在本行内（长条从这里起笔、显示标题） */
  startsHere: boolean
  /** 尾巴还往右伸出本行 */
  continuesRight: boolean
  /** 从上一行延续进来（起始日不在本行） */
  fromLeft: boolean
  spanTotal: number
}

/** 把某一行上的跨天日程归并成长条分段。
 *
 *  入参是后端 `events_in_range` 的**展开结果**：跨天日程在被占的每一天都有一条，
 *  并带 `spanIndex`（第几天）/`spanTotal`（共几天）。这里只算"同一 id 在本行
 *  占哪几列"，不重算日期 —— 展开规则后端只有一份，前端再算一遍就是漂移之源。
 *
 *  单日日程一律跳过：它们继续走格子里的条目列表。只有跨天的才值得画长条
 *  （滴答/小黄条同形：长条=跨天全天，格内条目=当天有安排）。
 */
export function rowSpans(dates: string[], events: Event[]): SpanSegment[] {
  const col = new Map(dates.map((d, i) => [d, i] as const))
  const byId = new Map<string, { occ: Event; c: number }[]>()
  for (const e of events) {
    if ((e.spanTotal ?? 1) <= 1) continue
    const c = col.get(e.date)
    if (c === undefined) continue
    const arr = byId.get(e.id)
    if (arr) arr.push({ occ: e, c })
    else byId.set(e.id, [{ occ: e, c }])
  }
  const out: SpanSegment[] = []
  for (const [id, list] of byId) {
    list.sort((a, b) => a.c - b.c)
    const head = list[0]!
    const tail = list[list.length - 1]!
    out.push({
      id,
      title: head.occ.title,
      color: head.occ.color,
      from: head.c,
      to: tail.c,
      startsHere: (head.occ.spanIndex ?? 0) === 0,
      continuesRight: (tail.occ.spanIndex ?? 0) + 1 < (tail.occ.spanTotal ?? 1),
      fromLeft: (head.occ.spanIndex ?? 0) > 0,
      spanTotal: tail.occ.spanTotal ?? 1,
    })
  }
  return out.sort((a, b) => a.from - b.from || a.to - b.to || a.title.localeCompare(b.title))
}

/** 格子里的条目列表：去掉已由长条表示的跨天日程，否则同一条出现两次 */
export function cellEvents<T extends { date: string; spanTotal?: number }>(date: string, events: T[]): T[] {
  return events.filter((e) => e.date === date && (e.spanTotal ?? 1) <= 1)
}

/** 某一天"实际占着"的全部条目（**含**跨天日程的当天 occurrence）。
 *
 *  日视图/单日详情用它，网格用 [cellEvents]：网格里跨天日程画成长条，格子再列一遍
 *  就成了同一条出现两次；而日视图没有长条层，用 cellEvents 会让"出差 3 天"的第一天
 *  显示「这天没安排」—— 真机跑出来过，条目就在这一天却看不见。 */
export function dayEvents<T extends { date: string }>(date: string, events: T[]): T[] {
  return events.filter((e) => e.date === date)
}

/** 跨天长条的标题：起始日显示正文，延续进来的标「续」，伸出右边的加 ▸ */
export function spanBarLabel(s: SpanSegment): string {
  const t = s.title || '（空）'
  const head = s.startsHere ? '' : '续 '
  const tail = s.continuesRight ? ' ▸' : ''
  return `${head}${t}${tail}`
}

/** 跨天数候选 + 上限（与后端 `MAX_SPAN_DAYS` 同值；改了要两边一起改） */
export const SPAN_CHOICES = [1, 2, 3, 5, 7, 14]
export const MAX_SPAN_DAYS = 60
/** 时长候选（分钟）。0 = 不定长，只显示开始时刻 */
export const DURATION_CHOICES = [0, 15, 30, 45, 60, 90, 120]
export const MAX_DURATION_MIN = 1440

/** 'YYYY-MM-DD' → 0..6（0=周一）。与后端 `by_weekday` 的约定一致。
 *  挂件的日程重复编辑器与主窗口日视图都要用它，两处各写一遍已经漂移过一次
 *  （主窗口初版拿"几号 mod 7"当星期，那是日期不是星期）。 */
export function weekdayOf(date: string): number {
  const d = parseYmd(date)
  return (d.getDay() + 6) % 7
}

/** 挂件上的跨天文字标记（310~500px 宽放不下长条层）：
 *  起始日 `⬌3`（共 3 天），续日 `›2/3`（第 2 天）。单日返回空串，不加噪音。 */
export function spanMark(e: { spanDays?: number; spanIndex?: number; spanTotal?: number }): string {
  const total = e.spanTotal ?? e.spanDays ?? 1
  if (total <= 1) return ''
  const idx = e.spanIndex ?? 0
  return idx === 0 ? `⬌${total}` : `›${idx + 1}/${total}`
}
