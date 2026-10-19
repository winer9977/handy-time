<script setup lang="ts">
/** 桌面日历清单插件
 *
 * 月 / 周 / 日 三视图；月历格显示节假日与调休；日程可拖拽改期；
 * 待办从待办挂件拖进来 → 生成日程（todo_to_event）。
 * 视图状态存后端（calendar_view / calendar_focus 全窗口共享）。
 */
import { computed, ref, watch } from 'vue'
import type { Event, Recurrence, Widget } from '../lib/types'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { registerTarget, beginDrag } from '../shared/drag'
import { askPrompt } from '../shared/prompt'
import { eventLineOf, parseEventLine } from '../shared/event-line'
import { recurrenceText } from '../shared/rc-text'
import { vClickAway } from '../shared/click-away'
import {
  addDays,
  monthGrid,
  monthLabel,
  parseYmd,
  startOfWeek,
  ymd,
} from '../shared/date'
import {
  DURATION_CHOICES,
  MAX_DURATION_MIN,
  MAX_SPAN_DAYS,
  SPAN_CHOICES,
  eventTimeLabel,
  sortEvents,
  spanMark,
  weekdayOf,
} from '../shared/calviews'
import WidgetHead from '../shared/WidgetHead.vue'

const props = defineProps<{ widget: Widget }>()
const data = useDataStore()
const view = computed(() => props.widget.view ?? 'month')

const today = computed(() => data.today)
const focus = ref(data.calendarFocus)

/** 拖入落点：把待办 id 落到某一天 */
const gridEl = ref<HTMLElement>()
watch(
  gridEl,
  (el) => {
    if (!el) return
    registerTarget({
      id: `drop-cal-${props.widget.id}`,
      hit: (x, y) => {
        const r = el!.getBoundingClientRect()
        return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
      },
      onDrop: (p, at) => {
        const dayEl = document.elementFromPoint(at.x, at.y)?.closest('[data-date]')
        const date = dayEl?.getAttribute('data-date')
        if (!date) return
        if (p.kind === 'todo') {
          api.todoToEvent(p.id, date).catch((e) => data.fail(e))
        } else if (p.kind === 'event') {
          api.moveEvent(p.id, date).catch((e) => data.fail(e))
        }
      },
    })
  },
  { immediate: true },
)

// ── 节假日缓存（按月加载） ──────────────────────────
const holidays = ref<Record<string, { name: string; off: boolean }>>({})
async function loadHolidays(year: number, month0: number) {
  const first = `${year}-${String(month0 + 1).padStart(2, '0')}-01`
  const last = ymd(new Date(year, month0 + 1, 0))
  try {
    const list = await api.holidaysInRange(first, last)
    const map: Record<string, { name: string; off: boolean }> = {}
    for (const h of list) map[h.date] = { name: h.name, off: h.off }
    holidays.value = map
  } catch {
    holidays.value = {}
  }
}

const cur = computed(() => {
  const d = parseYmd(focus.value)
  return { year: d.getFullYear(), month0: d.getMonth() }
})
watch(cur, (c) => loadHolidays(c.year, c.month0), { immediate: true })

// ── 日程：按可见窗口从后端取（含重复日程的虚拟展开）────────
//
// 不在前端按 date 过滤快照：那样重复规则得在 TS 再实现一遍，两份规则必然漂移。
// 展开只有一份，在 Rust 的 state::expand_events。
const rangeEvents = ref<Event[]>([])

/** 当前视图实际要渲染的日期区间（声明顺序必须在 cur 之后，否则 immediate watch 触发 TDZ） */
const visibleRange = computed(() => {
  if (view.value === 'day') return { from: focus.value, to: focus.value }
  if (view.value === 'week') {
    const s = startOfWeek(focus.value)
    return { from: s, to: addDays(s, 6) }
  }
  const cs = monthGrid(cur.value.year, cur.value.month0)
  return { from: cs[0]!.date, to: cs[cs.length - 1]!.date }
})

async function loadRange() {
  const { from, to } = visibleRange.value
  try {
    rangeEvents.value = await api.eventsInRange(from, to)
  } catch (e) {
    data.fail(e)
  }
}
// 依赖必须同时含 events 与 todos：
//   · events 变化 → 日程增删改/拖拽/重复规则
//   · **todos 变化** → 派生条目（后端 todo_derived_events 把带到期日的待办
//     派生成日历条目）以及"关联待办完成则隐藏其日程"的规则
// 漏掉 todos 的后果：在别的窗口改了待办到期日或勾选完成，日历挂件不刷新，
// 角标停在旧状态（"待办勾掉了、日历还挂着"这类症状的残余）。
watch(
  [visibleRange, () => data.store.events, () => data.store.todos],
  () => void loadRange(),
  { immediate: true },
)

// ── 农历（对照 YYNote）：按可见窗口加载，月视图跨两月 ──
const lunar = ref<Record<string, string>>({})
async function loadLunar() {
  const { from, to } = visibleRange.value
  try {
    const list = await api.lunarInRange(from, to)
    const map: Record<string, string> = {}
    for (const x of list) map[x.date] = x.label
    lunar.value = map
  } catch {
    lunar.value = {}
  }
}
watch(visibleRange, () => void loadLunar(), { immediate: true })

// ── 月视图 ──────────────────────────────────────────
const cells = computed(() => monthGrid(cur.value.year, cur.value.month0))
function eventsOf(date: string): Event[] {
  // 排序共用 calviews.sortEvents：全天在前、按时刻、再按标题。
  // 与主窗口日历、后端 state::events_on 同一份规则（以前这里自己写
  // `'99'` 兜底，把全天排到最后，两个界面顺序相反）。
  return sortEvents(rangeEvents.value.filter((e) => e.date === date))
}

function step(delta: number) {
  const d = parseYmd(focus.value)
  if (view.value === 'month') d.setMonth(d.getMonth() + delta)
  else if (view.value === 'week') focus.value = addDays(focus.value, delta * 7)
  else focus.value = addDays(focus.value, delta)
  if (view.value === 'month') {
    d.setDate(1)
    focus.value = ymd(d)
  }
}

async function setView(v: 'month' | 'week' | 'day') {
  try {
    await api.patchWidget(props.widget.id, { view: v })
  } catch (e) {
    data.fail(e)
  }
}

/** 点某格 → 进日视图看那天清单
 *
 * 必须走 setView（upsertWidget 持久化 widget.view）：渲染读的是
 * props.widget.view，而 data.calendarView 只是快照里的只读镜像，
 * 写它不会改变任何渲染结果 —— 原先正是这么写的，导致点日历不切视图。
 */
async function pick(date: string) {
  focus.value = date
  if (view.value === 'month') await setView('day')
}

/** 双击周视图某格 → 进日视图
 *
 * 使用习惯调研：Google/Outlook/Apple 日历的周视图里，单击=选中该天，
 * 双击=下钻到日视图。周视图列窄（约 38px），日程只能截断显示，
 * 双击看全天清单是自然诉求。月视图维持"单击即下钻"（格子大、误触成本低）。
 */
async function openDay(date: string) {
  focus.value = date
  if (view.value !== 'day') await setView('day')
}

function isWeekend(date: string) {
  const wd = parseYmd(date).getDay()
  return wd === 0 || wd === 6
}

// 日程条目：拖动 = 改期（跨窗口由 Rust 接管）；未移动 = 点选该天
function onEventDown(e: Event, ev: PointerEvent) {
  if (ev.button !== 0) return
  api.logLine('calendar', `eventDown ${e.id} @(${ev.clientX.toFixed(0)},${ev.clientY.toFixed(0)})`)
  const { clientX: x, clientY: y } = ev
  let moved = false
  const cleanup = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
  }
  const onMove = (m: PointerEvent) => {
    if (moved) return
    if (Math.abs(m.clientX - x) + Math.abs(m.clientY - y) > 5) {
      moved = true
      cleanup()
      api.logLine('calendar', `eventDown 超阈值起拖 → (${m.clientX.toFixed(0)},${m.clientY.toFixed(0)})`)
      void beginDrag({ kind: 'event', id: e.id, label: e.title || '（空）' })
    }
  }
  const onUp = () => {
    cleanup()
    if (!moved) pick(e.date)
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

const list = computed(() => {
  if (view.value !== 'day') return [] as Event[]
  // 复用 eventsOf：它读的是后端展开结果，重复日程才会出现在每个周期日
  return eventsOf(focus.value)
})

async function addEventHere() {
  const title = await askPrompt({ title: `在 ${focus.value} 新建日程`, placeholder: '日程标题' })
  if (!title) return
  try {
    await api.addEvent({
      id: '',
      date: focus.value,
      time: null,
      durationMin: 0,
      title,
      color: '#4c8dff',
      source: 'manual',
      todoId: null,
      spanDays: 1,
      recurrence: null,
    })
  } catch (e) {
    data.fail(e)
  }
}

/** 双击日程改名/改时：一次输入拿"HH:MM 标题"，不带前缀=全天。
 *  解析规则与主窗口日历共用 shared/event-line.ts（两处各写一遍必然漂移）。 */
async function pickDateEdit(e: Event) {
  const raw = await askPrompt({
    title: '格式「HH:MM 标题」，不写时间即全天',
    value: eventLineOf(e),
    placeholder: '如 09:30 评审',
  })
  if (raw === null) return
  const p = parseEventLine(raw, e.title)
  try {
    await api.updateEvent({ ...e, time: p.time, title: p.title })
  } catch (err) {
    data.fail(err)
  }
}

// ── 日程编辑器：重复 / 跨天 / 时长（一个弹层三种内容）───────
type EditorKind = 'rc' | 'span' | 'dur'
const menu = ref<{ id: string; kind: EditorKind } | null>(null)

function openMenu(e: Event, kind: EditorKind) {
  const same = menu.value?.id === e.id && menu.value?.kind === kind
  menu.value = same ? null : { id: e.id, kind }
}

/** 弹层里按 id 找回那条日程；列表刚被别的窗口改过时找不到就静默收掉，
 *  绝不解引用空值（这是本项目"选择器对着理想状态写"的老坑）。 */
function menuTarget(): Event | null {
  const id = menu.value?.id
  return id ? list.value.find((x) => x.id === id) ?? null : null
}

async function patchEvent(e: Event, patch: Partial<Event>) {
  menu.value = null
  try {
    await api.updateEvent({ ...e, ...patch })
  } catch (err) {
    data.fail(err)
  }
}

function setEventRc(e: Event, freq: Recurrence['freq'] | 'none') {
  const rc =
    freq === 'none'
      ? null
      : {
          freq,
          interval: 1,
          byWeekday: freq === 'weekly' ? [weekdayOf(e.date)] : [],
          byMonthday: freq === 'monthly' ? [Number(e.date.slice(8, 10)) || 1] : [],
          until: null,
          autoCreate: true,
        }
  return patchEvent(e, { recurrence: rc })
}

function applyRc(freq: Recurrence['freq'] | 'none') {
  const e = menuTarget()
  if (!e) {
    menu.value = null
    return
  }
  void setEventRc(e, freq)
}

/** 跨几天：1 = 不跨。派生条目（todo: 开头）没有这个维度（待办只占到期那天），
 *  所以模板里那两颗按钮对派生条目根本不出现。 */
function applySpan(n: number) {
  const e = menuTarget()
  if (!e) {
    menu.value = null
    return
  }
  void patchEvent(e, { spanDays: n })
}

async function customSpan() {
  const e = menuTarget()
  if (!e) return
  menu.value = null
  const raw = await askPrompt({
    title: `跨几天（1 ~ ${MAX_SPAN_DAYS}）`,
    value: String(e.spanDays || 1),
    placeholder: '如 3',
  })
  if (raw === null) return
  const n = Number(raw)
  if (!Number.isFinite(n) || n < 1) {
    data.fail('天数要是 1 以上的整数')
    return
  }
  void patchEvent(e, { spanDays: Math.min(MAX_SPAN_DAYS, Math.trunc(n)) })
}

function applyDuration(m: number) {
  const e = menuTarget()
  if (!e) {
    menu.value = null
    return
  }
  void patchEvent(e, { durationMin: m })
}

async function customDuration() {
  const e = menuTarget()
  if (!e) return
  menu.value = null
  const raw = await askPrompt({
    title: `时长多少分钟（0 ~ ${MAX_DURATION_MIN}）`,
    value: String(e.durationMin || 0),
    placeholder: '如 90',
  })
  if (raw === null) return
  const n = Number(raw)
  if (!Number.isFinite(n) || n < 0) {
    data.fail('分钟数要是 0 以上的整数')
    return
  }
  void patchEvent(e, { durationMin: Math.min(MAX_DURATION_MIN, Math.trunc(n)) })
}
</script>


<template>
  <div class="xh-shell">
    <WidgetHead :title="monthLabel(cur.year, cur.month0)">
      <button class="xh-btn" @click="step(-1)">‹</button>
      <button class="xh-btn" @click="focus = today">今天</button>
      <button class="xh-btn" @click="step(1)">›</button>
      <button
        class="xh-btn"
        :class="{ on: view === v }"
        v-for="v in (['month', 'week', 'day'] as const)"
        :key="v"
        @click="setView(v)"
      >
        {{ { month: '月', week: '周', day: '日' }[v] }}
      </button>
    </WidgetHead>

    <div class="xh-body" ref="gridEl">
      <!-- 星期表头只对月/周视图有意义，日视图隐藏 -->
      <div class="wd" v-if="view !== 'day'">
        <span v-for="w in ['一', '二', '三', '四', '五', '六', '日']" :key="w" :class="{ wk: w === '六' || w === '日' }">
          {{ w }}
        </span>
      </div>

      <!-- 月视图 -->
      <div v-if="view === 'month'" class="grid">
        <div
          v-for="c in cells"
          :key="c.date"
          class="cell"
          :data-date="c.date"
          :class="{ out: !c.inMonth, today: c.date === today, wknd: isWeekend(c.date), hov: c.date === focus }"
          @click="pick(c.date)"
        >
          <span class="num">{{ c.day }}</span>
          <span v-if="lunar[c.date]" class="lunar" :class="{ mn: lunar[c.date]!.endsWith('月') }">
            {{ lunar[c.date] }}
          </span>
          <span v-if="holidays[c.date]" class="hol" :class="holidays[c.date]!.off ? 'off' : 'work'">
            {{ holidays[c.date]!.name }}
          </span>
          <span class="marks">
            <i v-for="e in eventsOf(c.date).slice(0, 3)" :key="e.id" :style="{ background: e.color }"></i>
            <em v-if="eventsOf(c.date).length > 3">+{{ eventsOf(c.date).length - 3 }}</em>
          </span>
        </div>
      </div>

      <!-- 周视图 -->
      <div v-else-if="view === 'week'" class="grid week">
        <div
          v-for="i in 7"
          :key="i"
          class="cell col"
          :data-date="addDays(startOfWeek(focus), i - 1)"
          :class="{ today: addDays(startOfWeek(focus), i - 1) === today }"
          @click="pick(addDays(startOfWeek(focus), i - 1))"
          @dblclick="openDay(addDays(startOfWeek(focus), i - 1))"
        >
          <span class="num">{{ parseYmd(addDays(startOfWeek(focus), i - 1)).getDate() }}</span>
          <span v-if="lunar[addDays(startOfWeek(focus), i - 1)]" class="lunar">
            {{ lunar[addDays(startOfWeek(focus), i - 1)] }}
          </span>
          <span v-if="holidays[addDays(startOfWeek(focus), i - 1)]" class="hol"
            :class="holidays[addDays(startOfWeek(focus), i - 1)]!.off ? 'off' : 'work'">
            {{ holidays[addDays(startOfWeek(focus), i - 1)]!.name }}
          </span>
          <div class="evs">
            <div
              v-for="e in eventsOf(addDays(startOfWeek(focus), i - 1))"
              :key="e.id"
              class="ev"
              :style="{ '--ec': e.color }"
              @pointerdown="onEventDown(e, $event)"
            >
              <!-- 周视图列宽仅 ~38px：时间前缀会把标题挤成"09:3…"，
                   所以这里只显示标题（时刻在日视图里看）；跨天的加文字标记 -->
              <b v-if="spanMark(e)" class="spm">{{ spanMark(e) }}</b>
              {{ e.title || '（空）' }}
            </div>
          </div>
        </div>
      </div>

      <!-- 日视图：当日日程清单 -->
      <div v-else class="day">
        <div class="day-h">
          {{ focus }} · 周{{ '日一二三四五六'[parseYmd(focus).getDay()] }}
          <span v-if="holidays[focus]" class="hol" :class="holidays[focus]!.off ? 'off' : 'work'">
            {{ holidays[focus]!.name }}{{ holidays[focus]!.off ? '' : '(班)' }}
          </span>
        </div>
        <div class="xh-scroll">
          <div v-for="e in list" :key="e.id" class="dev" :style="{ '--ec': e.color }">
            <span class="dt" :title="e.durationMin ? '含时长' : ''">{{ eventTimeLabel(e) }}</span>
            <span class="tt" @dblclick="pickDateEdit(e)">{{ e.title || '（空）' }}</span>
            <span v-if="spanMark(e)" class="xh-tag span" :title="`这条日程连续占 ${e.spanTotal ?? e.spanDays} 天`">
              {{ spanMark(e) }}
            </span>
            <span v-if="e.recurrence" class="xh-tag rc" :title="recurrenceText(e.recurrence) ?? ''">
              ↻{{ recurrenceText(e.recurrence) }}
            </span>
            <span v-if="e.source === 'todo'" class="xh-tag">待办</span>
            <button
              class="xh-btn x"
              :class="{ on: menu?.id === e.id && menu?.kind === 'rc' }"
              title="重复规则"
              @click.stop="openMenu(e, 'rc')"
            >↻</button>
            <!-- 派生条目（`todo:` 前缀）没有"跨天/时长"这两个维度：待办只占到期那天，
                 durationMin 在它身上没有对应字段。按钮给了就是"点了没反应"（真机测出来的）。 -->
            <button
              v-if="!e.id.startsWith('todo:')"
              class="xh-btn x"
              :class="{ on: menu?.id === e.id && menu?.kind === 'span' }"
              title="跨几天"
              @click.stop="openMenu(e, 'span')"
            >⬌</button>
            <button
              v-if="e.time && !e.id.startsWith('todo:')"
              class="xh-btn x"
              :class="{ on: menu?.id === e.id && menu?.kind === 'dur' }"
              title="时长"
              @click.stop="openMenu(e, 'dur')"
            >⏱</button>
            <button class="xh-btn x" @click.stop="api.removeEvent(e.id).catch((x) => data.fail(x))">✕</button>
          </div>
          <div v-if="!list.length" class="xh-empty">这天没安排，点下面加一条</div>
        </div>
        <!-- 弹层必须放在 .xh-scroll 之外：滚动容器会把它裁掉 -->
        <div v-if="menu" v-click-away="() => (menu = null)" class="xh-pop xh-nodrag" @pointerdown.stop>
          <div v-if="menu.kind === 'rc'" class="row">
            <span class="lbl">重复</span>
            <button class="xh-btn" @click="applyRc('none')">不重复</button>
            <button class="xh-btn" @click="applyRc('daily')">每天</button>
            <button class="xh-btn" @click="applyRc('weekly')">每周</button>
            <button class="xh-btn" @click="applyRc('monthly')">每月</button>
          </div>
          <div v-else-if="menu.kind === 'span'" class="row">
            <span class="lbl">跨天</span>
            <button v-for="n in SPAN_CHOICES" :key="n" class="xh-btn" @click="applySpan(n)">
              {{ n === 1 ? '不跨' : n + '天' }}
            </button>
            <button class="xh-btn" @click="customSpan">自定义…</button>
          </div>
          <div v-else class="row">
            <span class="lbl">时长</span>
            <button v-for="m in DURATION_CHOICES" :key="m" class="xh-btn" @click="applyDuration(m)">
              {{ m === 0 ? '不定长' : m + '分' }}
            </button>
            <button class="xh-btn" @click="customDuration">自定义…</button>
          </div>
          <div class="row tip">
            {{
              menu.kind === 'span'
                ? '全天日程从当天起连续占用 N 天'
                : menu.kind === 'dur'
                  ? '有时长才显示结束时刻，如 09:30–10:30'
                  : '重复只存一条母记录，日历上按规则铺开'
            }}
          </div>
        </div>
        <button class="xh-btn addday" @click="addEventHere">＋ 在 {{ focus }} 加日程</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wd {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  flex: none;
  border-bottom: 1px solid var(--xh-line);
}
.wd span {
  text-align: center;
  font-size: 10px;
  padding: 2px 0;
  color: var(--xh-ink-2);
}
.wd span.wk {
  color: #e08b00;
}
.grid {
  display: grid;
  /* minmax(0,1fr)：1fr 的下限是 min-content，nowrap 的日程条会撑大列宽，
     挤压其它列。这里不是修"第 7 列被裁"（实测过：容器宽度固定时两种写法
     都渲染成 7 等分），而是让日程条能正常省略号截断而不是顶宽列。 */
  grid-template-columns: repeat(7, minmax(0, 1fr));
  flex: 1;
  min-height: 0;
}
.grid.week {
  grid-template-rows: 1fr;
}
.cell {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-right: 1px solid var(--xh-line);
  border-bottom: 1px solid var(--xh-line);
  padding: 2px 3px;
  overflow: hidden;
  cursor: pointer;
  min-height: 0;
  min-width: 0;
}
.cell:hover {
  background: rgba(127, 127, 127, 0.08);
}
.cell.out .num {
  opacity: 0.3;
}
.lunar {
  font-size: 9px;
  line-height: 1.1;
  color: var(--xh-ink-2);
  text-align: center;
  opacity: 0.85;
}
/* 农历初一显示月名（正月/闰二月…），稍加强调 */
.lunar.mn {
  color: var(--xh-accent);
  opacity: 1;
}
.cell.today {
  background: rgba(245, 197, 24, 0.18);
}
.cell.hov {
  outline: 1px solid var(--xh-accent);
  outline-offset: -1px;
}
.cell.wknd .num {
  color: #d48806;
}
.num {
  font-size: 11px;
  font-weight: 600;
  line-height: 1.2;
  flex: none;
}
.hol {
  font-size: 9px;
  line-height: 1.25;
  flex: none;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hol.off {
  color: var(--xh-danger);
}
.hol.work {
  color: var(--xh-ok);
}
.marks {
  display: flex;
  gap: 2px;
  align-items: center;
  flex-wrap: wrap;
}
.marks i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
}
.marks em {
  font-style: normal;
  font-size: 9px;
  color: var(--xh-ink-2);
}
.evs {
  display: flex;
  flex-direction: column;
  gap: 1px;
  overflow: hidden;
  flex: 1;
  min-height: 0;
}
.ev {
  font-size: 9px;
  line-height: 1.25;
  padding: 1px 3px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--ec) 18%, transparent);
  border-left: 2px solid var(--ec);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: grab;
}
.day {
  display: flex;
  flex-direction: column;
  grid-column: 1 / -1;
  padding: 2px 4px;
  min-height: 0;
}
.day-h {
  font-size: 12px;
  font-weight: 600;
  padding: 3px 0;
  display: flex;
  gap: 6px;
  align-items: center;
}
.devs {}
.dev {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 2px;
  border-bottom: 1px solid var(--xh-line);
  font-size: 12px;
}
.dev .dt {
  width: 40px;
  flex: none;
  color: var(--xh-ink-2);
  font-size: 11px;
}
.dev .tt {
  flex: 1;
  min-width: 0;
  word-break: break-all;
  border-left: 3px solid var(--ec);
  padding-left: 6px;
}
.dev .x {
  flex: none;
  opacity: 0.5;
}
.addday {
  flex: none;
  margin: 6px 0;
  text-align: center;
}
/* 跨天标记：周视图列窄，只有几像素的余地，用加粗小字标在标题前 */
.ev .spm {
  font-weight: 700;
  opacity: 0.75;
  margin-right: 2px;
}
.xh-tag.span {
  color: #241d00;
  background: var(--xh-accent);
  border-color: transparent;
}
.xh-pop .row.tip {
  font-size: 10px;
  color: var(--xh-ink-2);
  margin-top: 3px;
  line-height: 1.35;
}
</style>
