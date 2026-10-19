/** 标签编辑规则测试（tags.ts）*/
import { addTag, normalizeTag, removeTag, suggestTags } from '../src/shared/tags'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { Todo } from '../src/lib/types'

const mk = (title: string, tags: string[] = []): Todo =>
  ({ id: title, listId: 'l', title, tags, done: false }) as unknown as Todo

test('归一化：去首尾空白', () => {
  assert.equal(normalizeTag('  工作  '), '工作')
})

test('归一化：去掉 # 前缀（滴答用 # 打标，用户会习惯性带上）', () => {
  assert.equal(normalizeTag('#工作'), '工作')
  assert.equal(normalizeTag('  ## 家庭  '), '家庭')
})

test('归一化：纯 # 或全空白 → 空串', () => {
  assert.equal(normalizeTag('###'), '')
  assert.equal(normalizeTag('   '), '')
})

test('加标签：追加到末尾，不改原数组', () => {
  const src = ['a']
  const out = addTag(src, 'b')
  assert.deepEqual(out, ['a', 'b'])
  assert.deepEqual(src, ['a'], '必须不可变，Vue 响应式依赖这个')
})

test('加标签：空输入返回 null（调用方据此不动数据、不白广播）', () => {
  assert.equal(addTag(['a'], ''), null)
  assert.equal(addTag(['a'], '   '), null)
  assert.equal(addTag(['a'], '###'), null)
})

test('加标签：重复返回 null（大小写敏感，中文无此问题但英文有）', () => {
  assert.equal(addTag(['工作'], '工作'), null)
  assert.deepEqual(addTag(['Work'], 'work'), ['Work', 'work'])
})

test('加标签：先归一再判重，"#a" 不该绕过重复检查', () => {
  assert.equal(addTag(['a'], '#a'), null, '带 # 前缀的重复必须被拦下')
})

test('删标签：只删匹配的那一个', () => {
  assert.deepEqual(removeTag(['a', 'b', 'c'], 'b'), ['a', 'c'])
})

test('删标签：不存在时原样返回', () => {
  assert.deepEqual(removeTag(['a'], 'zzz'), ['a'])
})

test('建议：按使用次数降序', () => {
  const todos = [mk('1', ['少']), mk('2', ['多', '多']), mk('3', ['多'])]
  assert.deepEqual(suggestTags(todos, []).map((s) => s.tag), ['多', '少'])
  assert.deepEqual(suggestTags(todos, []).map((s) => s.n), [3, 1])
})

test('建议：同频按字面升序，结果稳定可预期', () => {
  const todos = [mk('1', ['乙']), mk('2', ['甲'])]
  assert.deepEqual(suggestTags(todos, []).map((s) => s.tag), ['甲', '乙'])
})

test('建议：排除当前待办已有的标签（不出现"你已经有了"的噪音）', () => {
  const todos = [mk('1', ['已有', '别的']), mk('2', ['已有'])]
  assert.deepEqual(suggestTags(todos, ['已有']).map((s) => s.tag), ['别的'])
})

test('建议：limit 截断', () => {
  const todos = [mk('1', ['a', 'b', 'c'])]
  assert.equal(suggestTags(todos, [], 2).length, 2)
})

test('建议：空数据不炸', () => {
  assert.deepEqual(suggestTags([], []), [])
})
