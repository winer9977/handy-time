/** 提醒跟随到期日平移的前端规则测试（shared/remind.ts）
 *
 *  背景：共有三条路径会改待办的到期日。后端把规则抽成
 *  `recurring::shift_reminders_to`（覆盖"日历拖派生条目"与"拖待办进日历"），
 *  前端这份 `shared/remind.ts` 覆盖"UI 改到期日"。进程边界让它们无法共用
 *  代码，但**语义必须一致** —— 本项目栽过太多次"双实现漂移"，这个测试就是
 *  钉子。
 *
 *  ★ 必须 import 出货代码本身：测试若自己复制一份算法，那只是"两份实现
 *    互相印证"，永远抓不到漂移（本文件的初版就犯了这个错，还因此漏掉了
 *    纯日期格式的 bug 一段时间）。
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { shiftMainRemind, shiftRemindTo, duePatch, withDueShift, type DueShiftSource } from '../src/shared/remind'

/** DueShiftSource 的最小造数据器（其余字段给默认） */
const src = (o: Partial<DueShiftSource> = {}): DueShiftSource => ({
  remindAt: null,
  extraReminds: [],
  remindedKeys: [],
  reminded: false,
  lastRemindedAt: null,
  ...o,
})

test('带时刻的提醒：只换日期，HH:MM 保留', () => {
  assert.equal(shiftRemindTo('2026-05-03T09:30', '2026-05-05'), '2026-05-05T09:30')
  assert.equal(shiftRemindTo('2026-05-03T18:00', '2026-05-05'), '2026-05-05T18:00')
})

test('★ 纯日期格式也要平移（与后端同口径）', () => {
  // 后端 reminder::parse_min 明确支持纯日期 `YYYY-MM-DD`。
  // 前端初版用 `includes('T')` 自造判断，纯日期时判定为假 → 不平移
  // （与后端不一致，实测被本测试抓出来）。
  assert.equal(shiftRemindTo('2026-05-03', '2026-05-05'), '2026-05-05')
})

test('主提醒平移：优先取用户原本设的时刻', () => {
  // 关键：不能直接用面板当前值。用户可能从没打开过 ⋯ 面板，那时面板里
  // 是默认 09:00 —— 拿它覆盖会把用户设的 18:00 悄悄改掉。
  assert.equal(shiftMainRemind('2026-05-03T18:00', '2026-05-05', '09:00'), '2026-05-05T18:00')
})

test('主提醒平移：纯日期时回落到面板时刻', () => {
  // 旧值是纯日期（没有时刻可保留）→ 用面板里的时刻补上，得到完整时点
  assert.equal(shiftMainRemind('2026-05-03', '2026-05-05', '08:30'), '2026-05-05T08:30')
})

test('主提醒平移：原本没有提醒则保持没有', () => {
  // 没有提醒时不该凭空造一个 —— 用户没设提醒就是不想被提醒
  assert.equal(shiftMainRemind(null, '2026-05-05', '09:00'), null)
})

// ── duePatch：改期时那组字段只有这一个组装口 ──────────────

test('★ duePatch：改到期日要成组带上"提醒平移 + 状态复位"', () => {
  const t = src({
    remindAt: '2026-05-03T18:00',
    extraReminds: ['2026-05-03T09:00'],
    remindedKeys: ['2026-05-03T09:00'],
    reminded: true,
    lastRemindedAt: '2026-05-03T09:01',
  })
  const p = duePatch(t, '2026-05-05')
  assert.equal(p.due, '2026-05-05')
  assert.equal(p.remindAt, '2026-05-05T18:00', '用户原设的 18:00 须保留，不能被面板默认 09:00 覆盖')
  assert.deepEqual(p.extraReminds, ['2026-05-05T09:00'], '额外提醒一起平移')
  assert.deepEqual(p.remindedKeys, [], '旧的绝对 key 必须清空，否则同名新时点永不触发')
  assert.equal(p.reminded, false, '主提醒触发位复位')
  assert.equal(p.lastRemindedAt, null, '持续提醒节流基准重新起算')
})

test('duePatch：清掉到期日只清 due，提醒留着', () => {
  // 提醒是独立设置：删掉到期日不代表想删提醒（与后端 shift_reminders_to
  // 的 new_date=None 分支同口径）
  assert.deepEqual(duePatch(src({ remindAt: '2026-05-03T18:00' }), null), { due: null })
})

test('duePatch：原本没有提醒，不得凭空造出一个提醒', () => {
  const p = duePatch(src(), '2026-05-05')
  assert.equal(p.remindAt, null, '没有提醒就是没有')
  assert.deepEqual(p.extraReminds, [])
})

// ── withDueShift：看板拖列 / 时间线拖日 走同一条规则 ────────

test('★ withDueShift：拖卡片换列改到期日，提醒必须跟着搬', () => {
  // 这两条路径以前直接发 {due}：拖到"本周"后提醒仍指着旧那天。
  // 旧那天已过 → reminded 还是 true → 新到期日**永远不再提醒**；
  // 这是同类 bug 的第 5 个入口（前 4 处分别已修）。
  const t = src({ remindAt: '2026-10-17T09:00', reminded: true })
  const p = withDueShift(t, { due: '2026-10-21' })
  assert.equal(p.due, '2026-10-21')
  assert.equal(p.remindAt, '2026-10-21T09:00')
  assert.equal(p.reminded, false)
})

test('withDueShift：这次没改到期日就原样返回（拖进"已完成"列只改 done）', () => {
  const t = src({ remindAt: '2026-10-17T09:00', reminded: true })
  assert.deepEqual(withDueShift(t, { done: true }), { done: true })
  // due 显式给 undefined 也是"没改"
  assert.deepEqual(withDueShift(t, { due: undefined, done: true }), { due: undefined, done: true })
})

test('withDueShift：拖到"未排期"（due=null）只清 due、不动提醒', () => {
  const t = src({ remindAt: '2026-10-17T09:00' })
  assert.deepEqual(withDueShift(t, { due: null, done: false }), { due: null, done: false })
})
