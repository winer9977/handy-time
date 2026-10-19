/** 「HH:MM 标题」解析规则测试（shared/event-line.ts）
 *
 *  这条规则同时被挂件日历与主窗口日历用于"双击日程改名/改时"。
 *  抽成共享模块就是为了让两处只有一份实现，且测试 import 的是出货代码本身。
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { eventLineOf, parseEventLine } from '../src/shared/event-line'

test('带时间前缀：取出时间，其余为标题', () => {
  assert.deepEqual(parseEventLine('09:30 评审', '旧'), { time: '09:30', title: '评审' })
})

test('小时不补前导零', () => {
  assert.deepEqual(parseEventLine('9:30 站会', '旧'), { time: '09:30', title: '站会' })
})

test('全角冒号也认（中文输入法常打出）', () => {
  assert.deepEqual(parseEventLine('09：30 评审', '旧'), { time: '09:30', title: '评审' })
})

test('不带时间前缀 → 全天，整行是标题', () => {
  assert.deepEqual(parseEventLine('出差', '旧'), { time: null, title: '出差' })
})

test('★ 只写时间不写标题 → 沿用原标题（用户只想改时间）', () => {
  // 关键：不能把标题清空。用户输入"09:30"是想把这条挪到 9:30，
  // 不是想把标题删掉。
  assert.deepEqual(parseEventLine('09:30', '评审'), { time: '09:30', title: '评审' })
  assert.deepEqual(parseEventLine('09:30   ', '评审'), { time: '09:30', title: '评审' })
})

test('首尾空白被去掉，标题内空白保留', () => {
  assert.deepEqual(parseEventLine('  09:30  项目  评审  ', '旧'), {
    time: '09:30',
    title: '项目  评审',
  })
  assert.deepEqual(parseEventLine('  出差  ', '旧'), { time: null, title: '出差' })
})

test('★ 前导空格不能把时间前缀吃掉（从聊天/邮件复制一行是常见输入）', () => {
  // 早先正则 `^(\d...)` 直接锚在空格上 → 匹配失败 → 时间留在标题里、
  // 日程变成全天。用户视角就是"我明明写了 9:30，它当没看见"。
  assert.deepEqual(parseEventLine(' 09:30 评审', '旧'), { time: '09:30', title: '评审' })
  // 全角空格（中文输入法敲出来的）同样是空白
  assert.deepEqual(parseEventLine('　9:30 站会', '旧'), { time: '09:30', title: '站会' })
})

test('不把 "9:5" 或 "1230" 之类误认成时间（必须两位分钟）', () => {
  assert.deepEqual(parseEventLine('9:5 某事', '旧'), { time: null, title: '9:5 某事' })
  assert.deepEqual(parseEventLine('1230 某事', '旧'), { time: null, title: '1230 某事' })
})

test('25:00 这种非法小时仍按字面解析（不校验范围，交给用户）', () => {
  // 与挂件原行为一致：只做格式识别，不做时间合法性校验。
  // 后端 update_event 存的是字符串，非法时点最坏是不触发提醒，不会崩。
  assert.deepEqual(parseEventLine('25:00 某事', '旧'), { time: '25:00', title: '某事' })
})

test('eventLineOf：把现有日程拼成编辑框初值', () => {
  assert.equal(eventLineOf({ time: '09:30', title: '评审' }), '09:30 评审')
  assert.equal(eventLineOf({ time: null, title: '全天的事' }), '全天的事')
  // 标题为空时只留时间（用户接着补标题）
  assert.equal(eventLineOf({ time: '09:30', title: '' }), '09:30 ')
})

test('往返：现有内容解析回自身（编辑框初值不改动就应产出同样内容）', () => {
  const e = { time: '09:30', title: '评审' }
  const p = parseEventLine(eventLineOf(e), e.title)
  assert.deepEqual(p, { time: '09:30', title: '评审' })
  // 全天项往返也一致
  const e2 = { time: null, title: '出差' }
  const p2 = parseEventLine(eventLineOf(e2), e2.title)
  assert.deepEqual(p2, { time: null, title: '出差' })
})
