/** 日期与倒数文案测试（shared/date.ts）
 *
 *  此前 date.ts 完全没有测试，而 countdownText 的 keepCountingUp 分支
 *  刚接上（该字段长期是"数据模型有、实现没有"的死字段）。
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { addDays, countdownText, diffDays, startOfWeek, todayStr, ymd } from '../src/shared/date'

// 倒数文案依赖"今天"，所以用相对今天的日期构造，避免写死日期后明天就红
const T = todayStr()

test('diffDays：同一天为 0，未来为正', () => {
  assert.equal(diffDays(T, T), 0)
  assert.equal(diffDays(addDays(T, 5), T), 5)
  assert.equal(diffDays(addDays(T, -3), T), -3)
})

test('until：未来显示"还有 N 天"', () => {
  assert.equal(countdownText(addDays(T, 3), 'until'), '还有 3 天')
  assert.equal(countdownText(T, 'until'), '还有 0 天')
})

test('until：已过目标日默认停住（不继续累计）', () => {
  // keepCountingUp 默认 false —— 到点后文案该停住，不再一天天变
  assert.equal(countdownText(addDays(T, -2), 'until'), '已过 2 天')
  assert.equal(countdownText(addDays(T, -2), 'until', false), '已过 2 天')
})

test('until：开启 keepCountingUp 后到点转为"已经 N 天"', () => {
  // 这是数据模型注释承诺、但此前从未实现的行为
  assert.equal(countdownText(addDays(T, -2), 'until', true), '已经 2 天')
  assert.equal(countdownText(addDays(T, -30), 'until', true), '已经 30 天')
  // 未到点时两种开关都显示"还有 N 天"（开关只影响过点之后）
  assert.equal(countdownText(addDays(T, 4), 'until', true), '还有 4 天')
  assert.equal(countdownText(addDays(T, 4), 'until', false), '还有 4 天')
})

test('since：过去显示"已经 N 天"', () => {
  assert.equal(countdownText(addDays(T, -7), 'since'), '已经 7 天')
  assert.equal(countdownText(T, 'since'), '已经 0 天')
})

test('since：未来默认显示"N 天后"，开启后显示"还有 N 天"', () => {
  assert.equal(countdownText(addDays(T, 5), 'since'), '5 天后')
  assert.equal(countdownText(addDays(T, 5), 'since', true), '还有 5 天')
})

test('addDays 跨月与跨年都正确', () => {
  assert.equal(addDays('2026-01-31', 1), '2026-02-01')
  assert.equal(addDays('2026-12-31', 1), '2027-01-01')
  assert.equal(addDays('2026-03-01', -1), '2026-02-28')
  assert.equal(addDays('2024-03-01', -1), '2024-02-29', '闰年二月有 29 日')
})

test('startOfWeek 回到周一', () => {
  // 2026-10-17 是周六 → 本周一是 10-12
  assert.equal(startOfWeek('2026-10-17'), '2026-10-12')
  // 周一自身不变
  assert.equal(startOfWeek('2026-10-12'), '2026-10-12')
  // 周日仍属本周（回退 6 天）
  assert.equal(startOfWeek('2026-10-18'), '2026-10-12')
})

test('ymd 补零到两位', () => {
  assert.equal(ymd(new Date(2026, 0, 5)), '2026-01-05')
  assert.equal(ymd(new Date(2026, 11, 31)), '2026-12-31')
})
