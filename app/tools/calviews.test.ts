/** 日历多视图纯逻辑测试。锚定 2026-10-17（实机截图确认是周六）
 *  跑法见 tools/run-frontend-tests.ps1 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  rangeOf,
  dayCells,
  weeksRows,
  yearMonths,
  stepView,
  labelOf,
  countOptions,
  hasCount,
  clampCount,
  isWeekend,
  MAX_COUNT,
} from '../src/shared/calviews'

const SAT = '2026-10-17' // 周六
const MON = '2026-10-12' // 它所在周的周一（本项目周一起算）

// ── 区间 ───────────────────────────────────────────
test('day 视图区间就是当天', () => {
  assert.deepEqual(rangeOf('day', SAT), { from: SAT, to: SAT })
})

test('week 视图覆盖周一到周日', () => {
  assert.deepEqual(rangeOf('week', SAT), { from: MON, to: '2026-10-18' })
})

test('month 视图覆盖 6 行网格（含前后补位日）', () => {
  const r = rangeOf('month', SAT)
  assert.equal(r.from, '2026-09-28', '10-01 是周四，网格从上一个周一补起')
  assert.equal(r.to, '2026-11-08')
  assert.equal(dayCells('month', SAT).length, 42)
})

test('多日视图 = 从当天起连续 N 天', () => {
  assert.deepEqual(rangeOf('days', SAT, 3), { from: SAT, to: '2026-10-19' })
  assert.equal(dayCells('days', SAT, 3).length, 3)
})

test('多周视图 = 从本周一起连续 N 周（滴答的「2 周」）', () => {
  assert.deepEqual(rangeOf('weeks', SAT, 2), { from: MON, to: '2026-10-25' })
  assert.equal(dayCells('weeks', SAT, 2).length, 14)
  assert.equal(dayCells('weeks', SAT, 4).length, 28)
})

test('year 视图覆盖全年，但不产出日格', () => {
  assert.deepEqual(rangeOf('year', SAT), { from: '2026-01-01', to: '2026-12-31' })
  assert.deepEqual(dayCells('year', SAT), [], '年视图渲染 12 个月格，不是日格')
})

test('count 夹紧：非法/越界都不失控', () => {
  assert.equal(dayCells('days', SAT, 0).length, 1)
  assert.equal(dayCells('days', SAT, -5).length, 1)
  assert.equal(dayCells('days', SAT, 999).length, MAX_COUNT)
  assert.equal(dayCells('days', SAT, NaN).length, 1)
  assert.equal(clampCount(2.6), 3, '四舍五入而非截断')
})

// ── 分行 ───────────────────────────────────────────
test('weeksRows 按 7 天切行', () => {
  const rows = weeksRows(dayCells('weeks', SAT, 2))
  assert.equal(rows.length, 2)
  assert.deepEqual(rows[0]!.slice(0, 3), ['2026-10-12', '2026-10-13', '2026-10-14'])
  assert.equal(rows[1]!.length, 7)
})

test('weeksRows 末行不满 7 天也保留', () => {
  const rows = weeksRows(['a', 'b', 'c'])
  assert.deepEqual(rows, [['a', 'b', 'c']])
})

// ── 年视图 ─────────────────────────────────────────
test('yearMonths 给 12 个月且天数正确', () => {
  const ms = yearMonths(SAT)
  assert.equal(ms.length, 12)
  assert.equal(ms[0]!.label, '1 月')
  assert.equal(ms[0]!.days.length, 31)
  assert.equal(ms[1]!.days.length, 28, '2026 非闰年')
  assert.equal(ms[11]!.days.length, 31)
  assert.equal(ms[9]!.days[0], '2026-10-01')
})

test('yearMonths 闰年 2 月 29 天', () => {
  assert.equal(yearMonths('2024-06-01')[1]!.days.length, 29)
})

// ── 翻页 ───────────────────────────────────────────
test('月/年翻页回到 1 号，杜绝月末溢出漂移', () => {
  // 10-31 翻一月：若直接 setMonth+1 会变成 12-01（跳过 11 月）
  assert.equal(stepView('month', '2026-10-31', 1, 1), '2026-11-01')
  assert.equal(stepView('month', SAT, 1, -1), '2026-09-01')
  assert.equal(stepView('year', SAT, 1, 1), '2027-01-01')
  assert.equal(stepView('year', SAT, 1, -1), '2025-01-01')
})

test('周/多周/多日按各自周期翻页', () => {
  assert.equal(stepView('week', SAT, 1, 1), '2026-10-24')
  assert.equal(stepView('day', SAT, 1, 1), '2026-10-18')
  assert.equal(stepView('days', SAT, 3, 1), '2026-10-20', '多日 3 天一页')
  assert.equal(stepView('days', SAT, 3, -1), '2026-10-14')
  assert.equal(stepView('weeks', SAT, 2, 1), '2026-10-31', '多周 2 周=14 天一页')
})

test('翻页后区间首尾相接、不重叠不漏天', () => {
  const a = rangeOf('days', SAT, 4)
  const nextFocus = stepView('days', SAT, 4, 1)
  const b = rangeOf('days', nextFocus, 4)
  assert.equal(addOne(a.to), b.from, '下一页应从上一页末尾的次日开始')
  function addOne(d: string) {
    const x = new Date(d + 'T00:00:00')
    x.setDate(x.getDate() + 1)
    return `${x.getFullYear()}-${String(x.getMonth() + 1).padStart(2, '0')}-${String(x.getDate()).padStart(2, '0')}`
  }
})

// ── 标题 ───────────────────────────────────────────
test('标题：月/年带年月，日视图带星期', () => {
  assert.equal(labelOf('month', SAT), '2026 年 10 月')
  assert.equal(labelOf('year', SAT), '2026 年')
  assert.equal(labelOf('day', SAT), '2026 年 10 月 17 日 · 周六')
})

test('标题：周/多周/多日给日期区间，跨年才带年份', () => {
  assert.equal(labelOf('week', SAT), '10/12 – 10/18')
  assert.equal(labelOf('weeks', SAT, 2), '10/12 – 10/25')
  assert.equal(labelOf('days', SAT, 3), '10/17 – 10/19')
  const cy = labelOf('days', '2026-12-30', 5)
  assert.ok(cy.includes('2026-12-30') && cy.includes('2027-01-03'), `跨年应给全日期：${cy}`)
})

// ── 杂项 ───────────────────────────────────────────
test('countOptions 只对需要的视图给值', () => {
  assert.deepEqual(countOptions('weeks'), [2, 3, 4])
  assert.deepEqual(countOptions('days'), [2, 3, 4, 5, 7, 10, 14])
  assert.deepEqual(countOptions('month'), [1], '月视图无数量维度')
  assert.equal(hasCount('weeks'), true)
  assert.equal(hasCount('days'), true)
  assert.equal(hasCount('month'), false)
  assert.equal(hasCount('year'), false)
})

test('count 默认值必须在候选集里（否则 select 渲染成空白）', () => {
  // 实机踩到：count 默认 2，而 days 候选原本从 3 起 → 下拉框显示空白
  for (const v of ['days', 'weeks'] as const) {
    assert.ok(countOptions(v).includes(2), `${v} 的候选必须含默认值 2`)
  }
})

test('isWeekend 按周六日（周一起算的日历里它们是末两列）', () => {
  assert.equal(isWeekend(SAT), true, '10-17 周六')
  assert.equal(isWeekend('2026-10-18'), true, '周日')
  assert.equal(isWeekend(MON), false, '周一')
})

// ── 同日日程排序（两处日历 + 后端共用一份）────────────────
import { sortEvents } from '../src/shared/calviews'

const ev = (time: string | null, title: string) => ({ time, title })

test('★ sortEvents：全天在前（与后端 events_on 同序，不是最后）', () => {
  // 挂件与主窗口日历原先各写一遍 `(a.time ?? '99') < (b.time ?? '99')`，
  // '99' 比任何 "HH:MM" 都大 → 全天日程被排到**最后**；而后端按
  // Option<time> 排（None 最小）= 全天在最前。同一个日子两处顺序相反。
  const got = sortEvents([ev('09:30', '晨会'), ev(null, '出差'), ev('08:00', '体检')])
  assert.deepEqual(got, [ev(null, '出差'), ev('08:00', '体检'), ev('09:30', '晨会')])
})

test('sortEvents：同时刻按标题、不改动入参数组', () => {
  const input = [ev('09:30', '乙'), ev('09:30', '甲'), ev(null, '丙')]
  const out = sortEvents(input)
  assert.deepEqual(out.map((e) => e.title), ['丙', '甲', '乙'])
  assert.deepEqual(input.map((e) => e.title), ['乙', '甲', '丙'], '排序不该原地改动传入数组')
})

test('sortEvents：空数组与全天多条都稳', () => {
  assert.deepEqual(sortEvents([]), [])
  const allDay = sortEvents([ev(null, 'B'), ev(null, 'A')])
  assert.deepEqual(allDay.map((e) => e.title), ['A', 'B'], '都全天时按标题')
})

// ── 跨天长条与结束时刻（两处日历共用的数学）────────────────
import { addMinutes, cellEvents, dayEvents, eventTimeLabel, rowSpans, spanBarLabel, spanMark } from '../src/shared/calviews'
import type { Event } from '../src/lib/types'

const occ = (id: string, date: string, spanIndex: number, spanTotal = 3, title = '出差'): Event =>
  ({
    id,
    date,
    time: null,
    durationMin: 0,
    title,
    color: '#f00',
    source: 'manual',
    todoId: null,
    spanDays: spanTotal,
    spanIndex,
    spanTotal,
    recurrence: null,
    listId: null,
  }) as Event

test('addMinutes：加时长，跨零点标出来', () => {
  assert.deepEqual(addMinutes('09:30', 45), { text: '10:15', cross: false })
  assert.deepEqual(addMinutes('09:00', 150), { text: '11:30', cross: false })
  assert.deepEqual(addMinutes('23:40', 60), { text: '00:40', cross: true })
  assert.deepEqual(addMinutes('09:00', 0), { text: '09:00', cross: false })
})

test('eventTimeLabel：全天 / 只有开始 / 带结束 / 跨零点 +1', () => {
  assert.equal(eventTimeLabel({ time: null, durationMin: 60 }), '全天', '没时刻就谈不上时长')
  assert.equal(eventTimeLabel({ time: '09:30', durationMin: 0 }), '09:30', '不定长只显示开始')
  assert.equal(eventTimeLabel({ time: '09:30', durationMin: 90 }), '09:30–11:00')
  assert.equal(eventTimeLabel({ time: '23:40', durationMin: 60 }), '23:40–00:40+1')
})

test('rowSpans：跨天日程归并成一行里的长条', () => {
  const row = ['2026-10-19', '2026-10-20', '2026-10-21', '2026-10-22']
  // 起始日在 10-20（第 2 天开始），跨到 10-22
  const seg = rowSpans(row, [occ('e1', '2026-10-20', 0), occ('e1', '2026-10-21', 1), occ('e1', '2026-10-22', 2)])
  assert.equal(seg.length, 1, '同一条日程只出一根条')
  assert.deepEqual(
    { from: seg[0]!.from, to: seg[0]!.to, startsHere: seg[0]!.startsHere, fromLeft: seg[0]!.fromLeft, continuesRight: seg[0]!.continuesRight },
    { from: 1, to: 3, startsHere: true, fromLeft: false, continuesRight: false },
  )
})

test('rowSpans：跨行的尾巴与延续进来的头部都标出来', () => {
  const week1 = ['2026-10-19', '2026-10-20', '2026-10-21'] // 只到周一~周三
  // 跨 5 天的日程：在这 3 天里 spanIndex 0/1/2，最后一天后面还有两天
  const t = rowSpans(week1, [occ('e', '2026-10-19', 0, 5), occ('e', '2026-10-20', 1, 5), occ('e', '2026-10-21', 2, 5)])
  assert.equal(t[0]!.continuesRight, true, '还往右伸 → 该画 ▸')
  assert.equal(spanBarLabel(t[0]!), '出差 ▸')
  // 起始日在上一行：本行看到的是第 3、4 天
  const week2 = ['2026-10-22', '2026-10-23']
  const c = rowSpans(week2, [occ('e', '2026-10-22', 3, 5), occ('e', '2026-10-23', 4, 5)])
  assert.equal(c[0]!.fromLeft, true)
  assert.equal(c[0]!.startsHere, false)
  assert.equal(c[0]!.from, 0, '延续进来的那条从第 0 列开始')
  assert.equal(c[0]!.continuesRight, false, '4、5 天已占满，本行右侧不再伸')
  assert.equal(spanBarLabel(c[0]!), '续 出差', '续条也要有标题，否则用户不知道是哪件事')
})

test('rowSpans：单日日程不进长条层；多根条按列序稳定排列', () => {
  const row = ['2026-10-19', '2026-10-20']
  const mixed = [occ('one', '2026-10-19', 0, 1), occ('two', '2026-10-19', 0, 2), occ('three', '2026-10-20', 0, 2, '培训')]
  const t = rowSpans(row, mixed)
  assert.deepEqual(t.map((s) => s.id), ['two', 'three'], '单日被跳过；按本行内的列序排（同列再按标题）')
})

test('cellEvents：格子里不重复出现长条已表示的跨天日程', () => {
  const list = [occ('a', '2026-10-19', 0, 3), occ('b', '2026-10-19', 0, 1, '例会')]
  const got = cellEvents('2026-10-19', list)
  assert.deepEqual(got.map((e) => e.id), ['b'], '跨天的走长条，单日的留在格子里')
})

test('dayEvents：日视图必须看得见跨天日程占的那一天（真机跑出来的 bug）', () => {
  // 主窗口日视图原先复用 cellEvents → 「出差 3 天」的第一天显示「这天没安排」，
  // 行内的 ⬌/⏱ 编辑按钮也就一个都点不到。网格才需要避开长条。
  const list = [occ('a', '2026-10-19', 0, 3), occ('a2', '2026-10-20', 1, 3), occ('b', '2026-10-19', 0, 1, '例会')]
  assert.deepEqual(dayEvents('2026-10-19', list).map((e) => e.id), ['a', 'b'])
  assert.deepEqual(dayEvents('2026-10-20', list).map((e) => e.id), ['a2'], '延续日也算占着这天')
  assert.deepEqual(cellEvents('2026-10-19', list).map((e) => e.id), ['b'], '两者口径正好相反')
  assert.equal(dayEvents('2026-10-31', list).length, 0, '不占的日子还是空，别把整窗口的日程都倒进来')
})

test('spanMark：挂件用文字标记跨天（起始 ⬌N、续日 ›k/N），单日不加噪音', () => {
  assert.equal(spanMark(occ('a', '2026-10-19', 0, 3)), '⬌3')
  assert.equal(spanMark(occ('a', '2026-10-20', 1, 3)), '›2/3')
  assert.equal(spanMark(occ('a', '2026-10-21', 2, 3)), '›3/3')
  assert.equal(spanMark(occ('a', '2026-10-19', 0, 1)), '', '单日不该有任何标记')
})

test('eventTimeLabel：脏时刻原样显示，绝不退化成"全天"', () => {
  // 库里可能是手改过的脏数据。明明有时刻却显示"全天"，比少个结束时刻更糟
  assert.equal(eventTimeLabel({ time: '乱码', durationMin: 30 }), '乱码')
  assert.equal(eventTimeLabel({ time: '9:5', durationMin: 30 }), '9:5')
})
