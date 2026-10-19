/** 「HH:MM 标题」这一行的解析规则（日程改名/改时共用）
 *
 *  为什么抽出来：挂件日历（CalendarWidget）与主窗口日历（CalendarPane）都要
 *  双击日程改名/改时，两处如果各写一遍解析，必然漂移 —— 本项目栽过太多次
 *  "双实现漂移"。抽到共享模块，测试也能 import 出货代码本身。
 */
import type { Event } from '../lib/types'

export interface ParsedEventLine {
  /** `HH:MM`；null = 全天 */
  time: string | null
  title: string
}

/** 解析用户输入的「HH:MM 标题」。
 *
 *  规则（对照挂件原有行为，逐字保留）：
 *    · 先整串 `trim()` 再匹配 —— 时间前缀必须认得出**带前导空格**的输入
 *      （从微信/邮件里复制一行" 09:30 评审"贴进来是最常见的输入方式）。
 *      不 trim 时 `^(\d...)`  anchored 在空格上直接匹配失败，结果是
 *      "时间留在标题里 + 日程变成全天"，用户以为识别失败了；
 *      测试「首尾空白被去掉，标题内空白保留」锁住这条。
 *    · 以 `HH:MM` 或 `HH：MM`（全角冒号）开头 → 取出时间，其余为标题；
 *    · 小时不补前导零（`9:30` → `09:30`）；
 *    · 不带时间前缀 → 全天（`time = null`），整行都是标题；
 *    · 只写了时间没写标题 → **沿用原标题**（不能把标题清空：用户只想改时间）。
 */
export function parseEventLine(raw: string, fallbackTitle: string): ParsedEventLine {
  const text = raw.trim()
  const m = text.match(/^(\d{1,2}[:：]\d{2})\s*(.*)$/)
  if (!m) {
    return { time: null, title: text }
  }
  const [hh, mm] = m[1]!.replace('：', ':').split(':')
  return {
    time: `${String(Number(hh)).padStart(2, '0')}:${mm}`,
    title: (m[2] ?? '').trim() || fallbackTitle,
  }
}

/** 把日程现有内容拼成"编辑框里的初值"：`09:30 评审` / `全天` → 空 */
export function eventLineOf(e: Pick<Event, 'time' | 'title'>): string {
  return (e.time ? e.time + ' ' : '') + (e.title || '')
}
