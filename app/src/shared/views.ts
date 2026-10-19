/** 主窗口视图的纯逻辑：看板分列、拖入某列该改成什么、时间线铺日
 *
 * 刻意与组件解耦 —— "任务落在哪一列、拖过去变成什么日期"是最容易出错、
 * 也最该被测试锁住的部分。组件只负责画，不持有规则。
 *
 * 看板的列是**时间桶**（对照滴答看板的"按时间分组"），而不是"状态"：
 * 桌面待办工具里用户真正关心的是"这事什么时候做"，不是"进行到哪一步"。
 * 因此拖卡片换列 = 改到期日，一次操作同时完成"排期"和"看板移动"。
 */
import type { Todo } from '../lib/types'
import { addDays, startOfWeek } from './date'

export type BoardKey = 'over' | 'today' | 'week' | 'later' | 'someday' | 'done'

export interface BoardCtx {
  today: string
  /** 本周日（周一为一周之始） */
  weekEnd: string
  /** 下周一，"以后"列的落点 */
  nextWeek: string
}

export interface BoardCol {
  key: BoardKey
  label: string
  /** 拖入时写入的到期日。null = 清除到期日；undefined = 不接受投放 */
  due: string | null | undefined
  /** 该列代表"已完成" */
  done?: boolean
}

/** 锚定三个边界日。today 显式传入（不读时钟），便于单测。 */
export function boardCtx(today: string): BoardCtx {
  const mon = startOfWeek(today)
  const weekEnd = addDays(mon, 6)
  return { today, weekEnd, nextWeek: addDays(weekEnd, 1) }
}

export function classify(t: Todo, c: BoardCtx): BoardKey {
  if (t.done) return 'done'
  if (!t.due) return 'someday'
  if (t.due < c.today) return 'over'
  if (t.due === c.today) return 'today'
  if (t.due <= c.weekEnd) return 'week'
  return 'later'
}

export function columns(c: BoardCtx): BoardCol[] {
  return [
    // 逾期是算出来的，没有"拖回逾期"这种操作
    { key: 'over', label: '逾期', due: undefined },
    { key: 'today', label: '今天', due: c.today },
    { key: 'week', label: '本周', due: c.weekEnd },
    { key: 'later', label: '以后', due: c.nextWeek },
    // 没到期日的待办。原先叫「待定」：快速添加现在默认今天，这一列装的基本是
    // 老数据或手动清了日期的条目，叫「未排期」更贴合判据（无日期 ≠ 打算以后做）。
    { key: 'someday', label: '未排期', due: null },
    { key: 'done', label: '已完成', due: undefined, done: true },
  ]
}

/** 拖入某列要提交的改动；返回 null = 什么都不用做（含不可投放列） */
export interface DropPatch {
  due?: string | null
  done?: boolean
}

export function dropPatch(t: Todo, col: BoardCol): DropPatch | null {
  if (col.due === undefined && !col.done) return null // 逾期列：不接受
  if (col.done) {
    return t.done ? null : { done: true }
  }
  const p: DropPatch = {}
  // 从"已完成"拖回未列 → 重新打开。否则它会立刻弹回完成列，
  // 用户看到的就是"拖不动"。
  if (t.done) p.done = false
  if (col.due !== undefined && t.due !== col.due) p.due = col.due
  return Object.keys(p).length ? p : null
}

/** 拖到时间线某一天 = 改期到该日。与看板列共用一条"重新打开"规则，
 *  否则从"已完成"拖过来会立刻弹回完成列，用户看到的就是"拖不动"。
 *  返回 null = 无需改动（同一天，或本来就在这天且未完成）。 */
export function dropToDate(t: Todo, date: string): DropPatch | null {
  const p: DropPatch = {}
  if (t.done) p.done = false
  if (t.due !== date) p.due = date
  return Object.keys(p).length ? p : null
}

/** 按列分桶。保持 store 原有顺序（置顶/优先级/到期已由后端与 store 排好） */
export function bucket(todos: Todo[], c: BoardCtx): Record<BoardKey, Todo[]> {
  const out = {
    over: [],
    today: [],
    week: [],
    later: [],
    someday: [],
    done: [],
  } as Record<BoardKey, Todo[]>
  for (const t of todos) out[classify(t, c)].push(t)
  return out
}

/** 时间线：从 start 起连续 count 天（含 start） */
export function timelineDays(start: string, count: number): string[] {
  const n = Math.max(1, Math.min(count, 90))
  return Array.from({ length: n }, (_, i) => addDays(start, i))
}

/** 有到期日的任务按日聚堆（时间线用）。无 due 的丢掉 —— 它在时间轴上没有位置。 */
export function groupByDue(todos: Todo[]): Map<string, Todo[]> {
  const m = new Map<string, Todo[]>()
  for (const t of todos) {
    if (!t.due) continue
    const arr = m.get(t.due)
    if (arr) arr.push(t)
    else m.set(t.due, [t])
  }
  return m
}
