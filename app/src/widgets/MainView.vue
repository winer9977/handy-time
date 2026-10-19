<script setup lang="ts">
/** 主窗口（对照滴答三栏）：清单侧栏 + 列表 / 看板 / 时间线 三视图
 *
 * 与挂件的分工：挂件是"瞄一眼"的桌面常驻，主窗口是"坐下来整理"。
 * 数据同源（同一个 store + `xhy://changed` 广播），改哪边都实时同步。
 *
 * 看板列是**时间桶**而非状态列 —— 桌面待办工具里用户关心"这事什么时候做"，
 * 所以拖卡片换列 = 改到期日，一次操作同时完成排期与移动。规则在
 * `shared/views.ts`（纯函数，有 16 项测试锁住），本组件只画。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useDataStore } from '../shared/store'
import { api } from '../lib/api'
import { WD_CN, addDays, startOfWeek } from '../shared/date'
import {
  matchScope,
  scopeAllowsList,
  scopeCount,
  scopeDone,
  scopeTitle,
  scopeTodos,
  scopeTrashed,
  type Scope,
} from '../shared/scope'
import { withDueShift } from '../shared/remind'
import { BRAND } from '../shared/brand'
import { boardCtx, bucket, columns, dropPatch, dropToDate, groupByDue, timelineDays } from '../shared/views'
import { hoverTarget, registerTarget, unregisterTarget } from '../shared/drag'
import { enterShouldSubmit } from '../shared/ime'
import { askPrompt } from '../shared/prompt'
import type { TodoList } from '../lib/types'
import TodoRow from './TodoRow.vue'
import CalendarPane from './CalendarPane.vue'
import SummaryPanel from './SummaryPanel.vue'
import AnalysisPane from './AnalysisPane.vue'
import ContextMenu from '../shared/ContextMenu.vue'

type View = 'list' | 'board' | 'timeline' | 'calendar' | 'summary' | 'analysis'
const VIEWS: { k: View; label: string; tip: string }[] = [
  { k: 'list', label: '列表', tip: '按时间分组的传统清单' },
  { k: 'board', label: '看板', tip: '按时间桶分列，拖动卡片即改期' },
  { k: 'timeline', label: '时间线', tip: '横向日期轴，看未来两周排布' },
  { k: 'calendar', label: '日历', tip: '年/月/周/日/多日/多周 六视图（对照滴答）' },
  { k: 'summary', label: '总结', tip: '周/月回顾：完成趋势、清单分布、标签、优先级' },
  { k: 'analysis', label: '分析', tip: '近 7/30/90 天完成趋势 + 清单/标签/优先级分布' },
]

const data = useDataStore()
/** 视图与作用域是"这台机器的查看偏好"，不是业务数据：存 localStorage。
 *  进 store 会广播所有窗口，还要处理多窗口互相覆盖，不值当。 */
const view = ref<View>((localStorage.getItem('xhy.view') as View) || 'list')
watch(view, (v) => localStorage.setItem('xhy.view', v))

/** 侧栏作用域（对照滴答：智能清单/清单/标签/已完成/垃圾桶）。
 *  旧版本存的是 xhy.list（清单 id 或空），这里兼容读一次。 */
function loadScope(): Scope {
  const raw = localStorage.getItem('xhy.scope')
  if (raw) {
    try {
      const s = JSON.parse(raw) as Scope
      // 清单/标签可能已被删除：落到"全部"比渲染空面板好过渲染报错
      if (s.k === 'list' && !data.store.lists.some((l) => l.id === s.id)) return { k: 'all' }
      return s
    } catch {
      /* 坏值走兼容 */
    }
  }
  const old = localStorage.getItem('xhy.list')
  if (old) {
    const l = data.store.lists.find((x) => x.id === old)
    if (l) return { k: 'list', id: l.id }
  }
  return { k: 'all' }
}
const scope = ref<Scope>(loadScope())
watch(scope, (s) => localStorage.setItem('xhy.scope', JSON.stringify(s)), { deep: true })
/** 日历/总结的清单过滤只在"选中具体清单"时生效（与后端 list 参数同语义） */
const scopeList = computed(() => (scope.value.k === 'list' ? scope.value.id : null))

/** 今天 / 本周日 / 看板边界一律走 computed + store.today。
 *  `const today = todayStr()` 会把日期钉在组件创建那一刻：工作台是"开着不走"
 *  的窗口，跨零点后看板仍按昨天分桶（今天的项掉进"逾期"、本周口径少一天），
 *  而用户只会觉得"数字莫名其妙错了"。store.today 依赖快照，后端每日跨点
 *  广播 + 任何一次写操作都会让它重算。 */
const today = computed(() => data.today)
const weekEnd = computed(() => addDays(startOfWeek(today.value), 6))
const ctx = computed(() => boardCtx(today.value))
const cols = computed(() => columns(ctx.value))

/** 本作用域内的全部待办（含已完成，看板分桶时再拆开）。
 *
 *  ★ 清单维度必须过 `scopeAllowsList`：列表视图走 scopeTodos（已排除归档清单），
 *  看板/时间线以前直接用 matchScope，于是归档清单的待办照样出现在卡片里 ——
 *  同一屏顶栏写「3 项未完成」（scopeCount 排了归档）、下面摆 5 张卡，
 *  且"归档"对这两个视图形同虚设。规则只允许有一份，在 shared/scope.ts。 */
const scoped = computed(() => {
  const s = scope.value
  if (s.k === 'done') return data.store.todos.filter((t) => t.done && scopeAllowsList(data.store, t, s))
  if (s.k === 'trash') return []
  return data.store.todos.filter(
    (t) => scopeAllowsList(data.store, t, s) && matchScope(t, s, today.value, weekEnd.value),
  )
})
const buckets = computed(() => bucket(scoped.value, ctx.value))
const boardCols = computed(() =>
  cols.value.map((c) => ({ ...c, items: buckets.value[c.key] })),
)

/** 列表视图的行：未完成作用域项；done 视图给已完成（倒序）；trash 给回收站 */
const listTodos = computed(() => {
  const s = scope.value
  if (s.k === 'done') return scopeDone(data.store, s, today.value, weekEnd.value)
  return scopeTodos(data.store, s, today.value, weekEnd.value)
})
const trashItems = computed(() => scopeTrashed(data.store, scope.value, today.value, weekEnd.value))
const listDoneCount = computed(() =>
  scope.value.k === 'done' || scope.value.k === 'trash' ? 0 : scoped.value.filter((t) => t.done).length,
)
const scopeTitleText = computed(() => scopeTitle(data.store, scope.value))
const scopeCountText = computed(() => scopeCount(data.store, scope.value, today.value, weekEnd.value))
/** 顶栏计数文案：done/trash 不叫"未完成" */
const countLabel = computed(() => {
  const s = scope.value
  if (s.k === 'done') return `${scopeCountText.value} 项已完成`
  if (s.k === 'trash') return `${scopeCountText.value} 项`
  return `${scopeCountText.value} 项未完成`
})
const emptyText = computed(() => {
  const s = scope.value
  if (s.k === 'done') return '还没有完成过事项 ✦'
  if (s.k === 'tag') return `没有带 #${s.name} 的未完成事项 ✦`
  if (s.k === 'smart') return '这个筛选下没有事项 ✦'
  return '这个清单没有未完成的事项 ✦'
})

// ── 时间线：今天起 14 天 ─────────────────────────────
const TL_DAYS = 14
const tlDays = computed(() => {
  const by = groupByDue(scoped.value.filter((t) => !t.done))
  return timelineDays(today.value, TL_DAYS).map((d) => ({
    date: d,
    label: dayLabel(d),
    isToday: d === today.value,
    isWknd: [0, 6].includes(new Date(d + 'T00:00:00').getDay()),
    items: by.get(d) ?? [],
  }))
})
function dayLabel(d: string) {
  if (d === today.value) return '今天'
  if (d === addDays(today.value, 1)) return '明天'
  const dt = new Date(d + 'T00:00:00')
  return `周${WD_CN[(dt.getDay() + 6) % 7]}`
}

// ── 拖拽落点：整区一个 target，按 x 找命中的列/日 ───────
// 复用跨窗口拖拽机制：TodoRow 起拖 payload.kind='todo'，这里接住并改期。
// 只注册一个 target 而非每列一个，避免"后注册者优先"的顺序陷阱。
const boardEl = ref<HTMLElement>()
const tlEl = ref<HTMLElement>()
const colEls = new Map<string, HTMLElement>()
const dayEls = new Map<string, HTMLElement>()

function setColEl(key: string, el: unknown) {
  if (el) colEls.set(key, el as HTMLElement)
  else colEls.delete(key)
}
function setDayEl(date: string, el: unknown) {
  if (el) dayEls.set(date, el as HTMLElement)
  else dayEls.delete(date)
}

/** 指针落在哪个元素上（元素矩形判定，与挂件 computeListIndex 同套路） */
function hitKey(map: Map<string, HTMLElement>, x: number, y: number): string | null {
  for (const [k, el] of map) {
    const r = el.getBoundingClientRect()
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return k
  }
  return null
}

async function applyDrop(targetKey: string | null, id: string) {
  if (!targetKey) return
  const col = cols.value.find((c) => c.key === targetKey)
  if (!col) return
  const t = data.store.todos.find((x) => x.id === id)
  if (!t) return
  // 规则全在 dropPatch 里（含"逾期列不接受"、"完成态拖回未列要重新打开"）
  const p = dropPatch(t, col)
  if (!p) return
  try {
    // 字段级：只落拖拽真正改的字段（due/order/done…），
    // 不用整对象回写，免得把提醒线程刚写的 reminded 抹掉。
    // ★ 但"改了到期日"这件事必须把**提醒一起搬走**（withDueShift，与 ⋯ 面板
    // 改日期、拖进日历格、后端 move_event 同一口径）。以前这里直接发 {due}：
    // 卡片拖到"本周"后提醒仍指着旧那天 —— 旧那天已过时 reminded 仍是 true，
    // 于是新到期日上**永远不再提醒**，用户视角"我改了日期它就不响了"。
    await api.patchTodo(t.id, withDueShift(t, p))
  } catch (e) {
    data.fail(e)
  }
}

function register() {
  if (boardEl.value) {
    const el = boardEl.value
    registerTarget({
      id: 'main-board',
      hit: (x, y) => {
        const r = el.getBoundingClientRect()
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
      },
      onDrop: (p, at) => {
        if (p.kind !== 'todo') return
        void applyDrop(hitKey(colEls, at.x, at.y), p.id)
      },
    })
  } else {
    unregisterTarget('main-board')
  }
  if (tlEl.value) {
    const el = tlEl.value
    registerTarget({
      id: 'main-timeline',
      hit: (x, y) => {
        const r = el.getBoundingClientRect()
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
      },
      onDrop: (p, at) => {
        if (p.kind !== 'todo') return
        const d = hitKey(dayEls, at.x, at.y)
        if (!d) return
        const t = data.store.todos.find((x) => x.id === p.id)
        if (!t) return
        const patch = dropToDate(t, d)
        if (!patch) return
        // 同看板：拖到某一天 = 改期，提醒要跟着搬（withDueShift）
        api
          .patchTodo(t.id, withDueShift(t, patch))
          .catch((e) => data.fail(e))
      },
    })
  } else {
    unregisterTarget('main-timeline')
  }
}
watch([view, boardEl, tlEl], () => register())
onBeforeUnmount(() => {
  unregisterTarget('main-board')
  unregisterTarget('main-timeline')
})

/** 拖拽中整个看板/时间线区高亮即可。
 *  不做逐列高亮：那要在渲染期读 getBoundingClientRect，拖拽过程中
 *  不会重算（没有响应式依赖），而且列宽随内容变化时 rect 可能是脏的。
 *  日历挂件用的是同一个套路。 */
const boardHot = computed(() => hoverTarget.value === 'main-board')
const tlHot = computed(() => hoverTarget.value === 'main-timeline')

// ── 新建 ────────────────────────────────────────────
const input = ref('')
/** 作用域对应的落库清单：选中清单→它；其余（智能/标签/全部/完成/回收站）→
 *  第一个**活动**清单。与滴答一致：智能清单里新建的也进收集箱。
 *  必须用 activeLists：lists[0] 可能是已归档的，那样新建的待办会落进一个
 *  侧栏看不见的清单（归档等于没做）。 */
const targetList = computed(() =>
  scope.value.k === 'list' ? scope.value.id : (data.activeLists[0]?.id ?? null),
)
async function submit() {
  let text = input.value.trim()
  if (!text) return
  const listId = targetList.value
  if (!listId) return
  // 标签作用域下新建：自动带上该标签（quickparse 认 `[标签]` 前缀，零后端改动）。
  // 不带上会出现"在 #工作 里新建，东西却看不见"的困惑。
  if (scope.value.k === 'tag' && !text.includes(`[${scope.value.name}]`)) {
    text = `[${scope.value.name}] ${text}`
  }
  try {
    await api.addTodo(listId, text)
    input.value = ''
  } catch (e) {
    data.fail(e)
  }
}
function onKey(e: KeyboardEvent) {
  if (!enterShouldSubmit(e)) return
  e.preventDefault()
  void submit()
}

// ── 桌面挂件开关（对照滴答侧栏逐项入口，也是 ✕ 的唤回入口）──
const KIND_NAME: Record<string, string> = {
  todo: '待办清单',
  calendar: '日历清单',
  note: '便签',
}
/** 同类多个挂件加序号区分（"待办清单 2"），与后端 title_of 基础名一致 */
const widgetName = (w: { kind: string; id: string }) => {
  const base = KIND_NAME[w.kind] ?? '挂件'
  const same = data.store.widgets.filter((x) => x.kind === w.kind)
  if (same.length <= 1) return base
  return base + ' ' + (same.findIndex((x) => x.id === w.id) + 1)
}
const hiddenCount = computed(
  () => data.store.widgets.filter((w) => !w.visible).length,
)
async function toggleWidget(id: string, visible: boolean) {
  try {
    await api.setWidgetVisible(id, visible)
  } catch (e) {
    data.fail(e)
  }
}
async function showAll() {
  try {
    await api.showAllWidgets()
  } catch (e) {
    data.fail(e)
  }
}

// ── 侧栏：智能清单 / 标签 / 已完成 / 垃圾桶（对照滴答）──
const SMARTS: { f: 'today' | 'week' | 'flagged'; label: string }[] = [
  { f: 'today', label: '今天' },
  // 判据是"到期 ≤ 本周日（含逾期）"，不是滚动 7 天（matchScope 的 week 分支
  // + 测试「smart week：到期≤本周日」）。原先叫"最近7天"：周二到周日之间
  // 点进去看到的都不是 7 天的量，而且挂件同一筛选项写的是"本周"，两处叫法
  // 还不一致 —— 文案必须描述判据本身。
  { f: 'week', label: '本周' },
  { f: 'flagged', label: '旗标' },
]
/** 标签全集（未完成口径 + 排除归档清单，按使用数降序）。
 *  与挂件 allTags 同一语义；徽标数必须等于点进去看到的条数，
 *  所以清单维度和 scopeTodos 共用 scopeAllowsList。 */
const sideTags = computed(() => {
  const m = new Map<string, number>()
  for (const t of data.store.todos) {
    if (t.done || !scopeAllowsList(data.store, t, { k: 'all' })) continue
    for (const g of t.tags) m.set(g, (m.get(g) ?? 0) + 1)
  }
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
})
/** 「已完成」徽标 = 点进去那个视图的行数（同一条作用域规则，含排除归档清单） */
const doneTotal = computed(() =>
  scopeCount(data.store, { k: 'done' }, today.value, weekEnd.value),
)
const trashTotal = computed(() => data.store.trashed.length)

/** 侧栏清单的右键菜单（对照滴答：编辑 / 归档 / 删除）*/
const listCtx = ref<{ x: number; y: number; list: TodoList } | null>(null)

/** 活动清单 / 已归档清单分组（归档的不进主区，收在可折叠区里）*/
const activeLists = computed(() => data.lists.filter((l) => !l.archived))
const archivedLists = computed(() => data.lists.filter((l) => l.archived))
const showArchived = ref(false)

/** 右键菜单项：归档项按当前状态切换文案 */
const ctxItems = computed(() => {
  if (!listCtx.value) return []
  const l = listCtx.value.list
  return [
    { key: 'edit', label: '编辑' },
    { key: 'archive', label: l.archived ? '取消归档' : '归档' },
    // 用户诉求："我有多个清单，其中一两个不想加入总结"。
    // 放在清单自己的菜单里最直观 —— 它就是这个清单的属性。
    {
      key: 'exclude',
      label: l.excludeFromStats ? '✓ 不计入总结' : '不计入总结',
    },
    { key: 'delete', label: '删除', danger: true, divider: true },
  ]
})
async function onCtxPick(key: string) {
  const l = listCtx.value?.list
  listCtx.value = null
  if (!l) return
  if (key === 'edit') await editList(l)
  else if (key === 'archive') await toggleListArchived(l)
  else if (key === 'exclude') await toggleListExcludeStats(l)
  else if (key === 'delete') await deleteList(l)
}

function openListMenu(e: MouseEvent, l: TodoList) {
  e.preventDefault()
  listCtx.value = { x: e.clientX, y: e.clientY, list: l }
}

/** 编辑：改名 + 换色（滴答的「编辑」面板给这两项）*/
async function editList(l: TodoList) {
  const name = await askPrompt({ title: '清单名称', value: l.name, okText: '下一步' })
  if (name == null) return
  const trimmed = name.trim()
  if (!trimmed) return
  // 颜色：输入一个十六进制值（挂件里新建清单也是这么做的，保持一致）
  const color = await askPrompt({
    title: '清单颜色（#RRGGBB）',
    value: l.color,
    okText: '保存',
  })
  if (color == null) return
  const c = color.trim()
  const okColor = /^#[0-9a-fA-F]{6}$/.test(c)
  try {
    // 字段级：只改 name/color，不碰 order/collapsed/archived
    await api.updateList({ ...l, name: trimmed, color: okColor ? c : l.color })
  } catch (e) {
    data.fail(e)
  }
}

/** 归档 / 取消归档（数据保留，只是不再出现在侧栏主区与活动列表）*/
async function toggleListArchived(l: TodoList) {
  try {
    await api.setListArchived(l.id, !l.archived)
    // 当前作用域正选中它 → 归档后落回"全部"，否则选中一个看不见的清单
    if (!l.archived && scope.value.k === 'list' && scope.value.id === l.id) {
      scope.value = { k: 'all' }
    }
  } catch (e) {
    data.fail(e)
  }
}

/** 切换"不计入总结"：周/月总结与分析页会跳过该清单的待办与日程。
 *  只是统计口径开关，不影响清单本身与它的待办在别的视图里的显示。 */
async function toggleListExcludeStats(l: TodoList) {
  try {
    await api.setListExcludeStats(l.id, !l.excludeFromStats)
  } catch (e) {
    data.fail(e)
  }
}

/** 删除清单：连带其待办会被移入第一个剩余清单（后端保证，绝不丢数据）。
 *  必须二次确认 —— 这是破坏性且影响面大的操作。 */
async function deleteList(l: TodoList) {
  const yes = await askPrompt({
    title: `删除清单「${l.name}」？其待办会移入第一个清单，不会丢`,
    value: '确认删除',
    okText: '删除',
  })
  if (yes == null) return
  try {
    await api.removeList(l.id)
    if (scope.value.k === 'list' && scope.value.id === l.id) {
      scope.value = { k: 'all' }
    }
  } catch (e) {
    data.fail(e)
  }
}

/** 双击侧栏清单改名（与右键「编辑」共用一套入口，双击更快捷）*/
async function renameList(l: TodoList) {
  const name = await askPrompt({ title: '清单改名', value: l.name, okText: '改名' })
  if (name == null) return
  const trimmed = name.trim()
  if (!trimmed || trimmed === l.name) return
  try {
    await api.updateList({ ...l, name: trimmed })
  } catch (e) {
    data.fail(e)
  }
}

/** 垃圾桶条目操作：恢复 / 彻底删除 / 清空 */
async function restoreOne(id: string) {
  try {
    await api.restoreTodo(id)
  } catch (e) {
    data.fail(e)
  }
}
async function purgeOne(id: string) {
  try {
    await api.purgeTodo(id)
  } catch (e) {
    data.fail(e)
  }
}
async function emptyTrash() {
  if (!data.store.trashed.length) return
  try {
    await api.emptyTrash()
  } catch (e) {
    data.fail(e)
  }
}
</script>

<template>
  <div class="main-root">
    <!-- 侧栏：智能清单 / 清单 / 标签 / 已完成 / 垃圾桶（对照滴答） -->
    <aside class="side">
      <div class="brand">{{ BRAND }} 工作台</div>
      <button class="sitem" :class="{ on: scope.k === 'all' }" @click="scope = { k: 'all' }">
        全部清单<b>{{ scopeCount(data.store, { k: 'all' }, today, weekEnd) }}</b>
      </button>
      <div class="slabel">智能清单</div>
      <button
        v-for="sm in SMARTS"
        :key="sm.f"
        class="sitem"
        :class="{ on: scope.k === 'smart' && scope.f === sm.f }"
        @click="scope = { k: 'smart', f: sm.f }"
      >
        {{ sm.label }}<b>{{ scopeCount(data.store, { k: 'smart', f: sm.f }, today, weekEnd) }}</b>
      </button>
      <div class="slabel">清单</div>
      <button
        v-for="l in activeLists"
        :key="l.id"
        class="sitem"
        :class="{ on: scope.k === 'list' && scope.id === l.id }"
        :title="
          l.excludeFromStats
            ? '双击改名，右键更多操作（当前：不计入周/月总结）'
            : '双击改名，右键更多操作'
        "
        @click="scope = { k: 'list', id: l.id }"
        @dblclick="renameList(l)"
        @contextmenu="openListMenu($event, l)"
      >
        <i :style="{ background: l.color }" />{{ l.name
        }}<!-- 不计入总结的标记：一眼能看出哪些清单被排除了统计 -->
        <span v-if="l.excludeFromStats" class="exstat" title="不计入周/月总结与分析">⊘</span><b>{{ scopeCount(data.store, { k: 'list', id: l.id }, today, weekEnd) }}</b>
      </button>
      <template v-if="archivedLists.length">
        <button class="sitem sub" @click="showArchived = !showArchived">
          {{ showArchived ? '▾' : '▸' }} 已归档<b>{{ archivedLists.length }}</b>
        </button>
        <template v-if="showArchived">
          <button
            v-for="l in archivedLists"
            :key="l.id"
            class="sitem dim"
            :class="{ on: scope.k === 'list' && scope.id === l.id }"
            title="右键可取消归档"
            @click="scope = { k: 'list', id: l.id }"
            @contextmenu="openListMenu($event, l)"
          >
            <i :style="{ background: l.color }" />{{ l.name }}<b>{{ scopeCount(data.store, { k: 'list', id: l.id }, today, weekEnd) }}</b>
          </button>
        </template>
      </template>
      <div class="slabel" v-if="sideTags.length">标签</div>
      <button
        v-for="[tg, n] in sideTags"
        :key="tg"
        class="sitem"
        :class="{ on: scope.k === 'tag' && scope.name === tg }"
        @click="scope = { k: 'tag', name: tg }"
      >
        #{{ tg }}<b>{{ n }}</b>
      </button>
      <div class="slabel">其他</div>
      <button class="sitem" :class="{ on: scope.k === 'done' }" @click="scope = { k: 'done' }">
        已完成<b>{{ doneTotal }}</b>
      </button>
      <button class="sitem" :class="{ on: scope.k === 'trash' }" @click="scope = { k: 'trash' }">
        垃圾桶<b>{{ trashTotal }}</b>
      </button>

      <!-- 桌面挂件：每个单独开关（对照滴答侧栏逐项入口）。
           写 store.visible，与挂件 ✕ 同一真相源，收起后可从这里唤回。 -->
      <div class="slabel">桌面挂件</div>
      <button
        v-for="w in data.store.widgets"
        :key="w.id"
        class="sitem"
        :class="{ on: w.visible }"
        :title="w.visible ? '点击隐藏此挂件' : '点击显示此挂件'"
        @click="toggleWidget(w.id, !w.visible)"
      >
        <!-- 右侧是**状态**（显示中/已收起），不是动词：写"显示"看起来像
             动作按钮，已显示的挂件点它实际会收起 —— 用户以为"点不动了"。
             动作语义放 tooltip。 -->
        {{ widgetName(w) }}<b>{{ w.visible ? '显示中' : '已收起' }}</b>
      </button>
      <button class="sitem" title="把已收起的挂件全部唤回" @click="showAll">
        全部显示<b>{{ hiddenCount ? `${hiddenCount} 个已收起` : '' }}</b>
      </button>
    </aside>

    <div class="pane">
      <!-- 顶栏：视图切换 -->
      <header class="top">
        <div class="seg">
          <button
            v-for="v in VIEWS"
            :key="v.k"
            :class="{ on: view === v.k }"
            :title="v.tip"
            @click="view = v.k"
          >
            {{ v.label }}
          </button>
        </div>
        <span class="spacer" />
        <span class="count">{{ scopeTitleText }} · {{ countLabel }}</span>
      </header>

      <!-- 列表 -->
      <div v-if="view === 'list'" class="body">
        <!-- 垃圾桶：条目走独立渲染（恢复 / 彻底删除），不是普通 TodoRow -->
        <div v-if="scope.k === 'trash'" class="scroll">
          <div v-for="x in trashItems" :key="x.todo.id" class="trow">
            <div class="tmain">
              <span class="ttitle">{{ x.todo.title }}</span>
              <span class="thint">{{ x.trashedAt.replace('T', ' ') }} 删除</span>
            </div>
            <button class="xh-btn" @click="restoreOne(x.todo.id)">恢复</button>
            <button class="xh-btn danger" @click="purgeOne(x.todo.id)">彻底删</button>
          </div>
          <div v-if="!trashItems.length" class="empty">垃圾桶是空的 ✦</div>
          <div v-else class="donebar">
            <button class="xh-btn danger" @click="emptyTrash">清空垃圾桶</button>
          </div>
        </div>
        <div v-else class="scroll">
          <TodoRow v-for="t in listTodos" :key="t.id" :todo="t" />
          <div v-if="!listTodos.length" class="empty">{{ emptyText }}</div>
          <div v-if="listDoneCount" class="donebar">已完成 {{ listDoneCount }} 项</div>
        </div>
      </div>

      <!-- 看板 -->
      <div v-else-if="view === 'board'" class="body">
        <div class="board" :class="{ hot: boardHot }" ref="boardEl">
          <div
            v-for="c in boardCols"
            :key="c.key"
            class="col"
            :class="{ drop: c.due !== undefined || c.done }"
            :ref="(el) => setColEl(c.key, el)"
          >
            <div class="chead">
              <span>{{ c.label }}</span>
              <b>{{ c.items.length }}</b>
            </div>
            <div class="cbody">
              <div v-for="t in c.items" :key="t.id" class="card">
                <TodoRow :todo="t" />
              </div>
              <div v-if="!c.items.length" class="cempty">—</div>
            </div>
          </div>
        </div>
      </div>

      <!-- 时间线 -->
      <div v-else-if="view === 'timeline'" class="body">
        <div class="tl" :class="{ hot: tlHot }" ref="tlEl">
          <div
            v-for="d in tlDays"
            :key="d.date"
            class="tcol"
            :class="{ today: d.isToday, wknd: d.isWknd }"
            :ref="(el) => setDayEl(d.date, el)"
          >
            <div class="tchead">
              <span class="dnum">{{ Number(d.date.slice(8, 10)) }}</span>
              <span class="dlab">{{ d.label }}</span>
            </div>
            <div class="cbody">
              <div v-for="t in d.items" :key="t.id" class="card">
                <TodoRow :todo="t" />
              </div>
              <div v-if="!d.items.length" class="cempty">—</div>
            </div>
          </div>
        </div>
      </div>

      <!-- 日历：年/月/周/日/多日/多周 -->
      <CalendarPane v-else-if="view === 'calendar'" :list-id="scopeList" />

      <!-- 总结：周/月回顾（对照滴答"回顾"），面板自带内容 -->
      <div v-else-if="view === 'summary'" class="body">
        <div class="scroll summary-wrap"><SummaryPanel :list-id="scopeList" /></div>
      </div>

      <!-- 分析：趋势 + 分布（对照滴答统计页） -->
      <AnalysisPane v-else-if="view === 'analysis'" :list-id="scopeList" />

      <div v-else class="body">
        <div class="empty">未知视图</div>
      </div>

      <!-- 日历/总结/分析自己占满整屏；done/trash 里"新建"没有落点语义，输入框一并藏掉 -->
      <footer class="foot" v-if="view !== 'calendar' && view !== 'summary' && view !== 'analysis' && scope.k !== 'done' && scope.k !== 'trash'">
        <input
          class="addin"
          v-model="input"
          @keydown="onKey"
          :placeholder="`记一笔到「${data.lists.find((l) => l.id === targetList)?.name ?? ''}」…${scope.k === 'tag' ? ` 自动带 #${scope.name}` : ' 支持 [标签] 明天 下午3点 !!!'}`"
          spellcheck="false"
        />
      </footer>
    </div>

    <!-- 清单右键菜单（编辑 / 归档 / 删除）。fixed 定位，贴边自动收拢 -->
    <ContextMenu
      v-if="listCtx"
      :x="listCtx.x"
      :y="listCtx.y"
      :items="ctxItems"
      @pick="onCtxPick"
      @close="listCtx = null"
    />
  </div>
</template>

<style scoped>
.main-root {
  display: flex;
  height: 100vh;
  overflow: hidden;
  font-size: 13px;
  color: var(--xh-ink);
  background: #f6f6f4;
}
:global([data-theme='dark']) .main-root {
  background: #1b1b1d;
}
.side {
  width: 190px;
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 8px;
  overflow-y: auto;
  border-right: 1px solid var(--xh-line);
  background: rgba(127, 127, 127, 0.05);
}
.brand {
  font-weight: 700;
  font-size: 13px;
  padding: 2px 8px 10px;
  opacity: 0.85;
}
.slabel {
  font-size: 10px;
  color: var(--xh-ink-2);
  padding: 8px 8px 3px;
}
.sitem {
  display: flex;
  align-items: center;
  gap: 6px;
  font: inherit;
  font-size: 12px;
  text-align: left;
  border: none;
  background: transparent;
  color: inherit;
  border-radius: 6px;
  padding: 5px 8px;
  cursor: pointer;
}
.sitem:hover {
  background: rgba(127, 127, 127, 0.12);
}
/* 「已归档」折叠开关：次要按钮，字色淡一档 */
.sitem.sub {
  color: var(--xh-ink-2);
  font-size: 11px;
}
/* 归档区里的清单行：整体压暗，与活动清单区分 */
.sitem.dim {
  opacity: 0.62;
}
.sitem.dim:hover {
  opacity: 1;
}
.sitem.on {
  background: var(--xh-accent);
  color: #241d00;
  font-weight: 600;
}
/* 「不计入总结」标记：小圆标，一眼看出该清单被排除了统计 */
.exstat {
  font-size: 10px;
  opacity: 0.55;
  margin-left: 2px;
  flex: none;
}
.sitem i {
  width: 8px;
  height: 8px;
  border-radius: 50%;  flex: none;
}
.sitem b {
  margin-left: auto;
  font-weight: 400;
  font-size: 10px;
  opacity: 0.6;
}
.pane {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.top {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  flex: none;
  border-bottom: 1px solid var(--xh-line);
}
.seg {
  display: flex;
  gap: 2px;
  border: 1px solid var(--xh-line);
  border-radius: 8px;
  padding: 2px;
}
.seg button {
  font: inherit;
  font-size: 12px;
  border: none;
  background: transparent;
  color: var(--xh-ink-2);
  border-radius: 6px;
  padding: 3px 12px;
  cursor: pointer;
}
.seg button.on {
  background: var(--xh-accent);
  color: #241d00;
  font-weight: 600;
}
.spacer {
  flex: 1;
}
.count {
  font-size: 11px;
  color: var(--xh-ink-2);
}
.body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.scroll {
  height: 100%;
  overflow-y: auto;
  padding: 4px 0;
}
.empty {
  padding: 40px;
  text-align: center;
  color: var(--xh-ink-2);
  font-size: 12px;
}
.donebar {
  padding: 8px 14px;
  font-size: 11px;
  color: var(--xh-ink-2);
  border-top: 1px solid var(--xh-line);
  margin-top: 6px;
}
/* 垃圾桶行：标题 + 删除时刻 + 恢复/彻底删 */
.trow {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--xh-line);
}
.tmain {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}
.ttitle {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--xh-ink-2);
  text-decoration: line-through;
}
.thint {
  font-size: 10px;
  color: var(--xh-ink-2);
}
/* 看板 / 时间线共用横向滚动 */
.board,
.tl {
  display: flex;
  gap: 10px;
  height: 100%;
  padding: 10px 12px;
  overflow-x: auto;
  overflow-y: hidden;
  align-items: stretch;
}
.col,
.tcol {
  flex: none;
  width: 210px;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border: 1px solid var(--xh-line);
  border-radius: 10px;
  background: rgba(127, 127, 127, 0.06);
}
.tcol {
  width: 150px;
}
.tcol.today {
  border-color: var(--xh-accent);
  background: rgba(245, 197, 24, 0.1);
}
.tcol.wknd .tchead {
  opacity: 0.6;
}
.chead,
.tchead {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 10px;
  flex: none;
  font-size: 12px;
  font-weight: 600;
  border-bottom: 1px solid var(--xh-line);
}
.chead b {
  margin-left: auto;
  font-weight: 400;
  font-size: 10px;
  opacity: 0.6;
}
.tchead {
  flex-direction: column;
  align-items: flex-start;
  gap: 0;
}
.dnum {
  font-size: 15px;
  font-weight: 700;
}
.dlab {
  font-size: 10px;
  font-weight: 400;
  color: var(--xh-ink-2);
}
.cbody {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.card {
  border-radius: 7px;
  background: var(--xh-card, #fff);
  border: 1px solid var(--xh-line);
  overflow: hidden;
}
:global([data-theme='dark']) .card {
  --xh-card: #26262a;
}
.cempty {
  text-align: center;
  color: var(--xh-ink-2);
  opacity: 0.4;
  font-size: 11px;
  padding: 10px 0;
}
/* 可投放的列在拖拽时给个提示（hot 挂容器，见 script 注释） */
.col.drop,
.tcol {
  transition: box-shadow 0.12s;
}
.board.hot .col.drop,
.tl.hot .tcol {
  box-shadow: inset 0 0 0 2px var(--xh-accent);
}
.foot {
  flex: none;
  padding: 8px 12px;
  border-top: 1px solid var(--xh-line);
  background: rgba(245, 197, 24, 0.08);
}
.addin {
  width: 100%;
  font: inherit;
  font-size: 13px;
  padding: 7px 10px;
  border: 1px solid var(--xh-line);
  border-radius: 8px;
  background: transparent;
  color: inherit;
  outline: none;
}
.addin:focus {
  border-color: var(--xh-accent);
}
</style>
