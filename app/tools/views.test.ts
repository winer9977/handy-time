/** 主窗口视图纯逻辑的测试。跑法见 tools/run-frontend-tests.ps1
 *  锚定 today=2026-10-17（周六）→ 本周 10-12~10-18、下周一 10-19 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { boardCtx, classify, columns, dropPatch, dropToDate, bucket, timelineDays, groupByDue } from '../src/shared/views'

const TODAY = '2026-10-17'
const ctx = boardCtx(TODAY)
// 只要运行时用到的字段；views 只读 .done / .due
const mk = (id: string, due: string | null, done = false): any => ({ id, due, done })

test('boardCtx 锚定周六的正确周边界', () => {
  assert.equal(ctx.today, '2026-10-17')
  assert.equal(ctx.weekEnd, '2026-10-18', '周一起算，本周日是 10-18')
  assert.equal(ctx.nextWeek, '2026-10-19', '下周一')
})

test('classify 六个桶', () => {
  assert.equal(classify(mk('a', '2026-10-16'), ctx), 'over')
  assert.equal(classify(mk('b', '2026-10-17'), ctx), 'today')
  assert.equal(classify(mk('c', '2026-10-18'), ctx), 'week')
  assert.equal(classify(mk('d', '2026-10-19'), ctx), 'later')
  assert.equal(classify(mk('e', '2026-11-01'), ctx), 'later')
  assert.equal(classify(mk('f', null), ctx), 'someday')
  // done 优先于一切日期判断
  assert.equal(classify(mk('g', '2026-10-17', true), ctx), 'done')
})

test('classify 用字典序即等价日期序（跨月不误判）', () => {
  assert.equal(classify(mk('x', '2026-09-30'), ctx), 'over')
  assert.equal(classify(mk('y', '2027-01-05'), ctx), 'later')
})

test('columns：逾期不可投放、完成列带标记', () => {
  const cols = columns(ctx)
  const over = cols.find((c) => c.key === 'over')!
  assert.equal(over.due, undefined, '逾期列没有"改成某天"的语义')
  assert.notEqual(over.done, true)
  assert.equal(cols.find((c) => c.key === 'today')!.due, TODAY)
  assert.equal(cols.find((c) => c.key === 'someday')!.due, null)
  assert.equal(cols.find((c) => c.key === 'done')!.done, true)
})

test('dropPatch：逾期列拒绝一切投放', () => {
  const over = columns(ctx).find((c) => c.key === 'over')!
  assert.equal(dropPatch(mk('a', '2026-10-16'), over), null)
})

test('dropPatch：拖到"今天"改写到期日', () => {
  const today = columns(ctx).find((c) => c.key === 'today')!
  assert.deepEqual(dropPatch(mk('a', '2026-10-16'), today), { due: TODAY })
})

test('dropPatch：已在该列则不动（返回 null，避免无谓写盘）', () => {
  const today = columns(ctx).find((c) => c.key === 'today')!
  assert.equal(dropPatch(mk('a', TODAY), today), null)
})

test('dropPatch：拖到"未排期"清除到期日', () => {
  const someday = columns(ctx).find((c) => c.key === 'someday')!
  assert.deepEqual(dropPatch(mk('a', '2026-10-16'), someday), { due: null })
  assert.equal(dropPatch(mk('b', null), someday), null)
})

test('dropPatch：拖到"已完成"置 done', () => {
  const done = columns(ctx).find((c) => c.key === 'done')!
  assert.deepEqual(dropPatch(mk('a', TODAY), done), { done: true })
  assert.equal(dropPatch(mk('b', TODAY, true), done), null, '已完成再拖回完成列不动')
})

test('dropPatch：从"已完成"拖回未列要重新打开（否则弹回，看着像拖不动）', () => {
  const today = columns(ctx).find((c) => c.key === 'today')!
  const p = dropPatch(mk('a', '2026-12-01', true), today)
  assert.equal(p!.done, false, '必须重新打开')
  assert.equal(p!.due, TODAY, '同时改到今天')
})

test('dropPatch：完成态拖到"未排期"也要重新打开且清日期', () => {
  const someday = columns(ctx).find((c) => c.key === 'someday')!
  const p = dropPatch(mk('a', TODAY, true), someday)
  assert.deepEqual(p, { done: false, due: null })
})

test('dropToDate：拖到时间线某天 = 改期到该日', () => {
  assert.deepEqual(dropToDate(mk('a', '2026-10-16'), '2026-10-22'), { due: '2026-10-22' })
})

test('dropToDate：同一天不动（避免无谓写盘）', () => {
  assert.equal(dropToDate(mk('a', TODAY), TODAY), null)
})

test('dropToDate：从已完成拖过来要重新打开（否则弹回完成列，看着像拖不动）', () => {
  assert.deepEqual(dropToDate(mk('a', '2026-12-01', true), '2026-10-22'), {
    done: false,
    due: '2026-10-22',
  })
  // 日期已经是那天，仍必须重新打开
  assert.deepEqual(dropToDate(mk('b', '2026-10-22', true), '2026-10-22'), { done: false })
})

test('dropToDate：无到期日的任务拖上时间轴', () => {
  assert.deepEqual(dropToDate(mk('a', null), '2026-10-25'), { due: '2026-10-25' })
})

test('bucket：分桶正确且同桶内保持输入顺序', () => {
  const todos = [mk('1', '2026-10-16'), mk('2', TODAY), mk('3', TODAY), mk('4', null), mk('5', TODAY, true)]
  const b = bucket(todos, ctx)
  assert.deepEqual(b.over.map((t) => t.id), ['1'])
  assert.deepEqual(b.today.map((t) => t.id), ['2', '3'], '保持输入顺序')
  assert.deepEqual(b.someday.map((t) => t.id), ['4'])
  assert.deepEqual(b.done.map((t) => t.id), ['5'])
  assert.equal(b.week.length, 0)
  assert.equal(b.later.length, 0)
})

test('timelineDays 连续铺日', () => {
  assert.deepEqual(timelineDays(TODAY, 5), [
    '2026-10-17',
    '2026-10-18',
    '2026-10-19',
    '2026-10-20',
    '2026-10-21',
  ])
})

test('timelineDays 跨月正确', () => {
  const days = timelineDays('2026-10-30', 4)
  assert.deepEqual(days, ['2026-10-30', '2026-10-31', '2026-11-01', '2026-11-02'])
})

test('timelineDays 夹紧到 1..90（防失控渲染）', () => {
  assert.equal(timelineDays(TODAY, 0).length, 1)
  assert.equal(timelineDays(TODAY, -5).length, 1)
  assert.equal(timelineDays(TODAY, 1000).length, 90)
})

test('groupByDue 丢弃无到期日、同日聚堆', () => {
  const m = groupByDue([mk('a', TODAY), mk('b', TODAY), mk('c', '2026-10-18'), mk('d', null)])
  assert.equal(m.size, 2, '无 due 的不进时间轴')
  assert.equal(m.get(TODAY)!.length, 2)
  assert.equal(m.get('2026-10-18')!.length, 1)
})
