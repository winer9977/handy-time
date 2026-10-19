/** 标签编辑的纯规则 —— 与 quickparse 的 `[标签]` 语法保持同一套语义
 *
 *  抽出来的理由：标签的"去重 / 去空白 / 去 # 前缀 / 建议排序"散在 UI 里
 *  会各写一遍且行为不一致（新建时能加的标签，编辑时却加不进去）。
 */
import type { Todo } from '../lib/types'

/** 归一化用户输入：去首尾空白、去 `#` 前缀（滴答用 # 打标签，用户会习惯性带）*/
export function normalizeTag(raw: string): string {
  return raw.trim().replace(/^#+/, '').trim()
}

/** 加一个标签。返回 null 表示无效（空 / 已存在），调用方据此不动数据。 */
export function addTag(tags: string[], raw: string): string[] | null {
  const t = normalizeTag(raw)
  if (!t || tags.includes(t)) return null
  return [...tags, t]
}

export function removeTag(tags: string[], t: string): string[] {
  return tags.filter((x) => x !== t)
}

/** 从已有待办里聚合标签建议：按使用次数降序，同频按字面升序（稳定可预期）。
 *  排除当前待办已有的，避免建议列表里出现"你已经有了"的噪音。 */
export function suggestTags(todos: Todo[], current: string[], limit = 8): { tag: string; n: number }[] {
  const m = new Map<string, number>()
  for (const t of todos) {
    for (const g of t.tags) m.set(g, (m.get(g) ?? 0) + 1)
  }
  return [...m.entries()]
    .filter(([g]) => !current.includes(g))
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, limit)
    .map(([tag, n]) => ({ tag, n }))
}
