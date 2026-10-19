/** 前端日期工具：一律用本地日 `YYYY-MM-DD`，不碰 UTC */

export const WD_CN = ['一', '二', '三', '四', '五', '六', '日']

export function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`
}

export function ymd(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

export function todayStr(): string {
  return ymd(new Date())
}

export function hm(d: Date): string {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function parseYmd(s: string): Date {
  const [y, m, d] = s.slice(0, 10).split('-').map(Number)
  return new Date(y, (m || 1) - 1, d || 1)
}

export function addDays(s: string, n: number): string {
  const d = parseYmd(s)
  d.setDate(d.getDate() + n)
  return ymd(d)
}

export function diffDays(a: string, b: string): number {
  const ms = parseYmd(a).getTime() - parseYmd(b).getTime()
  return Math.round(ms / 86400000)
}

/** 周一为一周起点 */
export function startOfWeek(s: string): string {
  const d = parseYmd(s)
  const wd = (d.getDay() + 6) % 7 // 0=周一
  return addDays(s, -wd)
}

export function daysInMonth(year: number, month0: number): number {
  return new Date(year, month0 + 1, 0).getDate()
}

export interface MonthCell {
  date: string
  day: number
  inMonth: boolean
}

/** 6×7 月历网格，周一起 */
export function monthGrid(year: number, month0: number): MonthCell[] {
  const first = new Date(year, month0, 1)
  const lead = (first.getDay() + 6) % 7
  const start = new Date(year, month0, 1 - lead)
  const cells: MonthCell[] = []
  for (let i = 0; i < 42; i++) {
    const d = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)
    cells.push({
      date: ymd(d),
      day: d.getDate(),
      inMonth: d.getMonth() === month0,
    })
  }
  return cells
}

export function monthLabel(year: number, month0: number): string {
  return `${year} 年 ${month0 + 1} 月`
}

/** 后端 remind_at 组装：due 日 + HH:MM */
export function remindAt(date: string, time: string): string {
  return `${date}T${time}`
}

/** 到期文案 */
export function dueLabel(due: string | null): { text: string; tone: 'over' | 'today' | 'soon' | 'later' } | null {
  if (!due) return null
  const n = diffDays(due, todayStr())
  if (n < 0) return { text: `逾期 ${-n} 天`, tone: 'over' }
  if (n === 0) return { text: '今天', tone: 'today' }
  if (n === 1) return { text: '明天', tone: 'soon' }
  if (n <= 3) return { text: `${n} 天后`, tone: 'soon' }
  const d = parseYmd(due)
  return { text: `${d.getMonth() + 1}/${d.getDate()}`, tone: 'later' }
}

/** 倒数文案：还有 N 天 / 已经 N 天
 *
 *  `keepCountingUp`（对照数据模型里的 `Countdown.keepCountingUp`）：
 *  到达目标日之后是否继续累计 —— 开启时"还有 3 天"到点后自然接上
 *  "已经 1 天 / 2 天…"，适合纪念日、戒烟第 N 天这类**持续计数**；
 *  关闭时到点后就停在同一句，不再变化。
 *  此前该字段只存在于数据模型与注释里（前端写死 false、后端不读），
 *  是个"承诺了但没实现"的死字段 —— 这里补上。
 */
export function countdownText(
  date: string,
  mode: 'until' | 'since',
  keepCountingUp = false,
): string {
  const n = diffDays(date, todayStr())
  if (mode === 'until') {
    if (n >= 0) return `还有 ${n} 天`
    // 已过目标日：开关开则继续累计"已经 N 天"，否则停在"已过 N 天"
    return keepCountingUp ? `已经 ${-n} 天` : `已过 ${-n} 天`
  }
  if (n <= 0) return `已经 ${-n} 天`
  return keepCountingUp ? `还有 ${n} 天` : `${n} 天后`
}
