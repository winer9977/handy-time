<script setup lang="ts">
/** 桌面待办清单插件
 *
 * · 多清单标签页（列表内可拖拽排序）
 * · [标签] 自动打标 + 一句话智能识别（后端 quickparse）
 * · 置顶 / 拖拽排序 / 到期提醒 / 日期倒数 / 重复自动生成 / 完成归档
 * · 底部快速添加框，回车即建；⌘/Ctrl+Enter 建多条
 */
import { computed, nextTick, ref, watch } from 'vue'
import type { Todo, TodoFilter, Widget } from '../lib/types'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { saveGeometry } from '../shared/winctx'
import { registerTarget, armDrag } from '../shared/drag'
import { askPrompt } from '../shared/prompt'
import { enterShouldSubmit } from '../shared/ime'
import { addDays, startOfWeek, todayStr } from '../shared/date'
import { inArchivedList } from '../shared/scope'
import WidgetHead from '../shared/WidgetHead.vue'
import TodoRow from './TodoRow.vue'

const props = defineProps<{ widget: Widget }>()
const data = useDataStore()

const listId = computed(() => props.widget.listId ?? null)

// ── 筛选（对照滴答"智能筛选"）───────────────────────────
// 视图态但持久化在 widget 上，与日历 view 同一套做法：跨重启保留，
// 且多个待办挂件各自记住自己的筛选。
const filter = computed<TodoFilter>(() => props.widget.filter ?? 'all')
const tagSel = computed(() => props.widget.tag ?? null)

const FILTERS: { k: TodoFilter; label: string; tip: string }[] = [
  { k: 'today', label: '今天', tip: '今天到期 + 已逾期' },
  { k: 'week', label: '本周', tip: '本周日结束前（含逾期）' },
  { k: 'flagged', label: '旗标', tip: '带优先级的' },
  // 「待定」这一档按用户口径撤掉了：快速添加没写日期现在默认就是今天，
  // 所以"没到期日的待办"不再是常态。`TodoFilter`/`matchScope` 里的 someday
  // **判据保留** —— 挂件选中的 filter 是持久化在库里的，老状态可能存着
  // 'someday'，删判据会让那个挂件变成"什么都不匹配"的静默空视图。
  { k: 'done', label: '完成', tip: '只看已完成' },
]

/** 该待办是否在本挂件的可见范围内。
 *  绑定了清单 → 只看它（显式点名，归档了也照样看）；
 *  聚合（点「全」）→ 排除**已归档**清单。与后端 `todos_of(None)`、
 *  shared/scope.ts 同一判据 —— 归档清单的待办若还混在聚合视图里，归档等于没做。 */
function inScope(t: Todo) {
  return listId.value == null ? !inArchivedList(data.store, t) : t.listId === listId.value
}

/** 标签全集（本挂件范围内，**未完成**口径，按使用次数降序）。
 *  与主窗口侧栏 sideTags 同口径：数字必须等于点进去看到的条数。
 *  以前这里把已完成项也数进去（注释还写着"同一语义"），于是标签上显示
 *  `#工作 5`、点进去只有 3 行 —— 而 `todos` 走的正是未完成口径。 */
const allTags = computed(() => {
  const m = new Map<string, number>()
  for (const t of data.store.todos) {
    if (t.done || !inScope(t)) continue
    for (const g of t.tags) m.set(g, (m.get(g) ?? 0) + 1)
  }
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
})

function matchFilter(t: Todo): boolean {
  const today = todayStr()
  switch (filter.value) {
    case 'today':
      return !!t.due && t.due <= today
    case 'week':
      return !!t.due && t.due <= addDays(startOfWeek(today), 6)
    case 'flagged':
      return !!t.priority
    case 'someday':
      return !t.due
    default:
      return true
  }
}
function matchTag(t: Todo): boolean {
  return !tagSel.value || t.tags.includes(tagSel.value)
}

const todos = computed(() => {
  if (filter.value === 'done') return []
  // todosOf 已按 置顶→优先级→order→到期 排好并滤掉已完成
  return data.todosOf(listId.value).filter((t) => matchFilter(t) && matchTag(t))
})

/** 已完成（本挂件范围内，跟随筛选口径） */
const dones = computed(() =>
  data.store.todos
    .filter((t) => t.done && inScope(t) && matchFilter(t) && matchTag(t))
    .sort((a, b) => (b.doneAt ?? '').localeCompare(a.doneAt ?? ''))
    .slice(0, 40),
)
/** done 口径下主列表直接展示已完成，折叠头就不必了 */
const doneAsMain = computed(() => filter.value === 'done')
const showDone = ref(false)

async function setFilter(k: TodoFilter) {
  const next = filter.value === k ? 'all' : k
  try {
    await api.patchWidget(props.widget.id, { filter: next })
  } catch (e) {
    data.fail(e)
  }
}
async function setTag(t: string | null) {
  try {
    await api.patchWidget(props.widget.id, { tag: tagSel.value === t ? null : t })
  } catch (e) {
    data.fail(e)
  }
}
async function clearFilters() {
  try {
    await api.patchWidget(props.widget.id, { filter: 'all', tag: null })
  } catch (e) {
    data.fail(e)
  }
}

// ── 快速添加 ────────────────────────────────────────
const input = ref('')

async function submit(add: boolean) {
  const text = input.value.trim()
  if (!text || !add) return
  try {
    if (listId.value) {
      await api.addTodo(listId.value, text)
    } else {
      // 聚合视图落到第一个**活动**清单（收集箱）。
      // 用 activeLists 而非 lists：lists[0] 可能是已归档的，那样待办会加进
      // 一个用户在侧栏看不见的清单里（归档等于没做）。
      const first = data.activeLists[0]
      if (!first) {
        await api.addList('收集箱', '#f5c518')
        await nextTick()
        const created = data.activeLists[0]
        if (!created) return
        await api.addTodo(created.id, text)
      } else {
        await api.addTodo(first.id, text)
      }
    }
    input.value = ''
  } catch (e) {
    data.fail(e)
  }
}

function onKey(e: KeyboardEvent) {
  // 输入法组合中的回车归 IME（确认候选词），不归提交 —— 见 shared/ime.ts
  if (!enterShouldSubmit(e)) return
  e.preventDefault()
  submit(true)
}

// ── 清单标签：切换 / 增删 / 排序 ────────────────────
async function selectList(id: string | null) {
  try {
    await api.patchWidget(props.widget.id, { listId: id })
  } catch (e) {
    data.fail(e)
  }
}
async function newList() {
  const name = await askPrompt({ title: '新清单名称', value: '清单', placeholder: '如：工作 / 生活' })
  if (!name) return
  const palette = ['#f5c518', '#4c8dff', '#30a46c', '#e5484d', '#9333ea', '#f97316']
  try {
    await api.addList(name.trim(), palette[data.lists.length % palette.length]!)
  } catch (e) {
    data.fail(e)
  }
}

// ── 拖拽 ──────────────────────────────────────────────
// 起拖：标签 → list 载荷；行 → todo 载荷（见 TodoRow）。
// 落点命中由 shared/drag 的表驱动，坐标已是本窗口 CSS 坐标。
const tabDragStart = (l: { id: string; name: string }) =>
  armDrag(() => ({ kind: 'list' as const, id: l.id, label: l.name }), 5)

/** 落点顺序换算：按当前标签排布，找指针 x 落在哪个缝隙 */
function computeListIndex(el: HTMLElement, x: number): number {
  const tabs = [...el.querySelectorAll('.tab')] as HTMLElement[]
  let idx = tabs.length
  for (let i = 0; i < tabs.length; i++) {
    const r = tabs[i]!.getBoundingClientRect()
    if (x < r.left + r.width / 2) {
      idx = i
      break
    }
  }
  return idx
}

const tabsEl = ref<HTMLElement>()
watch(
  tabsEl,
  (el) => {
    if (!el) return
    registerTarget({
      id: `drop-list-${props.widget.id}`,
      hit: (x, y) => {
        const r = el!.getBoundingClientRect()
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
      },
      onDrop: async (p, at) => {
        if (p.kind !== 'list') return
        // 落点索引必须落在**渲染出来的那一排**上：标签只画 activeLists，
        // 而旧代码用 data.lists（含归档）算 from/序列 —— 两套房子的门牌号。
        // 一旦有归档清单排在中间，指针位置与序列位置就差一位：
        // "拖到第 2 格却排到第 3 个"。
        const act = data.activeLists.map((l) => l.id)
        const from = act.indexOf(p.id)
        // 排除自身后的目标序列位置
        const without = act.filter((id) => id !== p.id)
        let to = computeListIndex(el!, at.x)
        if (from >= 0 && from < to) to -= 1
        if (from < 0 || to < 0 || to === from) return
        without.splice(to, 0, p.id)
        // 归档清单不进标签栏、不参与这次拖拽，但 order 要一起写：按现有相对
        // 顺序接在活动清单之后。只传活动清单的话，归档清单留着旧序号会与新的
        // 撞号（reorder_lists 按传入顺序 0..n 重编），排序结果取决于数组顺序。
        const archived = data.lists.filter((l) => l.archived).map((l) => l.id)
        try {
          await api.reorderLists([...without, ...archived])
        } catch (e) {
          data.fail(e)
        }
      },
    })
  },
  { immediate: true },
)

// ── 行落点：排序 + 生成日程 ─────────────────────────
const rowsEl = ref<HTMLDivElement>()
watch(rowsEl, (el) => {
  if (!el) return
  registerTarget({
    id: `drop-rows-${props.widget.id}`,
    hit: (x, y) => {
      const r = el!.getBoundingClientRect()
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
    },
    onDrop: (p, at) => {
      if (p.kind !== 'todo') return
      // 找到指针下方的行
      const els = [...el!.querySelectorAll('[data-todo-id]')] as HTMLElement[]
      let target: HTMLElement | null = null
      for (const e2 of els) {
        const r = e2.getBoundingClientRect()
        if (at.y < r.bottom) {
          target = e2
          break
        }
      }
      const movedId = p.id
      const targetId = target?.dataset.todoId ?? null
      if (targetId === movedId) return
      const after = (() => {
        if (!target) return false
        const r = target.getBoundingClientRect()
        return at.y > r.top + r.height / 2
      })()
      api.reorderTodo(movedId, targetId, after).catch((e) => data.fail(e))
    },
  })
})

// 尺寸档位切换
const sizes = ['s', 'm', 'l', 'xl'] as const
async function cycleSize() {
  const i = sizes.indexOf(props.widget.size)
  const next = sizes[(i + 1) % sizes.length]!
  try {
    await api.patchWidget(props.widget.id, { size: next })
    await nextTick()
    saveGeometry()
  } catch (e) {
    data.fail(e)
  }
}

const sizeLabel = computed(() => ({ s: '小', m: '中', l: '大', xl: '特大' })[props.widget.size])
</script>

<template>
  <div class="xh-shell">
    <WidgetHead :title="listId ? (data.lists.find((l) => l.id === listId)?.name ?? '待办') : '待办 · 全部'">
      <button class="xh-btn xh-nodrag" @click="cycleSize" :title="'尺寸：' + sizeLabel">{{ sizeLabel }}</button>
      <button class="xh-btn xh-nodrag" @click="newList" title="新建清单">＋清单</button>
      <button
        class="xh-btn xh-nodrag"
        :class="{ on: listId === null }"
        @click="selectList(null)"
        title="看全部清单"
      >
        全
      </button>
    </WidgetHead>

    <div class="tabs" ref="tabsEl">
      <button
        v-for="l in data.activeLists"
        :key="l.id"
        class="tab"
        :class="{ on: l.id === listId }"
        :style="{ '--c': l.color }"
        @click="selectList(l.id)"
        @pointerdown="tabDragStart(l)"
      >
        {{ l.name }}
        <b>{{ data.store.todos.filter((t) => !t.done && t.listId === l.id).length }}</b>
      </button>
    </div>

    <!-- 筛选栏（对照滴答"智能筛选"）：再点一次取消，回到全部 -->
    <div class="filters">
      <button
        v-for="f in FILTERS"
        :key="f.k"
        class="fbtn"
        :class="{ on: filter === f.k }"
        :title="f.tip"
        @click="setFilter(f.k)"
      >
        {{ f.label }}
      </button>
      <span class="fspacer" />
      <button v-if="filter !== 'all' || tagSel" class="fbtn clr" @click="clearFilters" title="清除筛选">✕</button>
    </div>

    <!-- 标签栏：出现过的标签，点击筛选 -->
    <div class="tagbar" v-if="allTags.length">
      <button
        v-for="[tg, n] in allTags.slice(0, 12)"
        :key="tg"
        class="tgbtn"
        :class="{ on: tagSel === tg }"
        @click="setTag(tg)"
      >
        #{{ tg }}<b>{{ n }}</b>
      </button>
    </div>

    <div class="xh-body">
      <div class="xh-scroll" ref="rowsEl">
        <TodoRow v-for="t in todos" :key="t.id" :todo="t" />
        <!-- 空态判据要跟着口径走：done 口径下主列表恒空，若只看 todos 会
             在"其实有已完成项"时也喊"清单清空了"（实测截图里就同时出现了） -->
        <div v-if="todos.length === 0 && !(doneAsMain && dones.length)" class="xh-empty">
          {{
            doneAsMain
              ? '还没有完成的事项'
              : listId === null
                ? '还没有待办，下面输入一句试试'
                : '这个清单清空了 ✦'
          }}
        </div>

        <div v-if="dones.length && !doneAsMain" class="done-head" @click="showDone = !showDone">
          已完成 {{ dones.length }} 条 {{ showDone ? '▴' : '▾' }}
        </div>
        <template v-if="!doneAsMain && showDone">
          <TodoRow v-for="t in dones" :key="t.id" :todo="t" />
        </template>
        <template v-if="doneAsMain">
          <TodoRow v-for="t in dones" :key="'d' + t.id" :todo="t" />
        </template>
      </div>

      <div class="add">
        <input
          class="xh-input"
          v-model="input"
          @keydown="onKey"
          placeholder="记一笔… 支持 [标签] 明天 8月1日 周五 下午3点"
          spellcheck="false"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.tabs {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 5px 8px;
  flex: none;
  border-bottom: 1px solid var(--xh-line);
}
.tab {
  font: inherit;
  font-size: 11px;
  border: 1px solid var(--xh-line);
  background: transparent;
  color: inherit;
  border-radius: 999px;
  padding: 1px 7px 1px 6px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.tab::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--c);
}
.tab b {
  font-weight: 400;
  opacity: 0.6;
  font-size: 10px;
}
.tab.on {
  background: var(--c);
  color: #241d00;
  border-color: transparent;
}
.tab.on b {
  opacity: 0.8;
}
.done-head {
  padding: 6px 10px;
  font-size: 11px;
  color: var(--xh-ink-2);
  cursor: pointer;
  background: rgba(127, 127, 127, 0.06);
}
.filters {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  flex: none;
  border-bottom: 1px solid var(--xh-line);
  background: rgba(127, 127, 127, 0.04);
}
.fbtn {
  font: inherit;
  font-size: 11px;
  border: 1px solid var(--xh-line);
  background: transparent;
  color: var(--xh-ink-2);
  border-radius: 5px;
  padding: 1px 7px;
  cursor: pointer;
}
.fbtn:hover {
  color: var(--xh-ink);
}
.fbtn.on {
  background: var(--xh-accent);
  color: #241d00;
  border-color: transparent;
  font-weight: 600;
}
.fbtn.clr {
  color: #d0453b;
  border-color: transparent;
}
.fspacer {
  flex: 1;
}
.tagbar {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 4px 8px;
  flex: none;
  border-bottom: 1px solid var(--xh-line);
  max-height: 58px;
  overflow-y: auto;
}
.tgbtn {
  font: inherit;
  font-size: 10px;
  border: 1px solid var(--xh-line);
  background: transparent;
  color: var(--xh-ink-2);
  border-radius: 999px;
  padding: 0 6px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 3px;
}
.tgbtn b {
  font-weight: 400;
  opacity: 0.6;
}
.tgbtn.on {
  background: var(--xh-accent);
  color: #241d00;
  border-color: transparent;
}
.add {
  flex: none;
  padding: 6px 8px;
  border-top: 1px solid var(--xh-line);
  background: rgba(245, 197, 24, 0.08);
}
</style>
