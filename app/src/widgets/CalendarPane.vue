<script setup lang="ts">
/** 主窗口的日历面板：年 / 月 / 周 / 日 / 多日 / 多周
 *
 * 对照滴答真实界面（实机截图核对）：顶栏 `‹ 今天 ›` + 数量下拉「2 周 ▾」，
 * 日格里右上是农历、节假日名，今天实心圆。
 *
 * 视图与数量是两个维度（滴答也是），规则全在 shared/calviews.ts（18 项测试
 * 锁住区间/翻页/标题），本组件只画。
 *
 * 数据一律按 rangeOf() 的区间向后端取：重复日程的虚拟展开、已完成待办的
 * 隐藏规则都在 Rust 那一份里，前端绝不重算。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { Event } from '../lib/types'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { todayStr } from '../shared/date'
import {
  CAL_VIEWS,
  DURATION_CHOICES,
  MAX_DURATION_MIN,
  MAX_SPAN_DAYS,
  SPAN_CHOICES,
  cellEvents,
  countOptions,
  dayCells,
  dayEvents,
  eventTimeLabel,
  hasCount,
  isWeekend,
  labelOf,
  rangeOf,
  rowSpans,
  sortEvents,
  spanBarLabel,
  stepView,
  weeksRows,
  weekdayOf,
  yearMonths,
  type CalView,
} from '../shared/calviews'
import { armDrag, hoverTarget, registerTarget, unregisterTarget } from '../shared/drag'
import { askPrompt } from '../shared/prompt'
import { eventLineOf, parseEventLine } from '../shared/event-line'

const data = useDataStore()
/** 选中清单：日历里待办派生的条目跟着它过滤（手工日程不受影响）。
 *  不传（挂件独立用）时 = 全局。 */
const props = defineProps<{ listId?: string | null }>()

/** 子视图/数量/聚焦日都是查看偏好 → localStorage（进 store 会广播所有窗口） */
const view = ref<CalView>((localStorage.getItem('xhy.calview') as CalView) || 'month')
const count = ref(Number(localStorage.getItem('xhy.calcount') || 2) || 2)
const focus = ref(todayStr())
watch(view, (v) => localStorage.setItem('xhy.calview', v))
watch(count, (v) => localStorage.setItem('xhy.calcount', String(v)))

const today = computed(() => data.today)
const range = computed(() => rangeOf(view.value, focus.value, count.value))
const label = computed(() => labelOf(view.value, focus.value, count.value))
const months = computed(() => yearMonths(focus.value))

// ── 数据：区间内日程 + 农历 + 节假日 ─────────────────
const events = ref<Event[]>([])
const lunar = ref<Record<string, string>>({})
const holidays = ref<Record<string, { name: string; off: boolean }>>({})

async function load() {
  const { from, to } = range.value
  try {
    const [ev, lu, ho] = await Promise.all([
      api.eventsInRange(from, to, props.listId),
      api.lunarInRange(from, to),
      api.holidaysInRange(from, to),
    ])
    events.value = ev
    const lm: Record<string, string> = {}
    for (const x of lu) lm[x.date] = x.label
    lunar.value = lm
    const hm: Record<string, { name: string; off: boolean }> = {}
    for (const x of ho) hm[x.date] = { name: x.name, off: x.off }
    holidays.value = hm
  } catch (e) {
    data.fail(e)
  }
}
// listId 必须在依赖里：主窗口切清单要重查，漏了就是"点了没反应"
watch([range, () => props.listId, () => data.store.events, () => data.store.todos], () => void load(), { immediate: true })

/** 格子里的条目：**跨天日程不在格子里重复出现**（它们由长条层表示），
 *  排序与挂件日历、后端 events_on 共用 sortEvents。 */
function eventsOf(date: string): Event[] {
  return sortEvents(cellEvents(date, events.value))
}

/** 日视图（.detail）用这个：网格之外没有长条层，跨天日程也得住这一天里，
 *  否则「出差 3 天」的第一天会显示「这天没安排」（真机测出来的）。 */
function detailOf(date: string): Event[] {
  return sortEvents(dayEvents(date, events.value))
}

/** 某一行（一周 / 一段多日窗口）上的跨天长条 */
function barsOf(row: string[]) {
  return rowSpans(row, events.value)
}

// ── 日视图的行内编辑器（重复 / 跨天 / 时长）────────────────
/** 一次只展开一行的一个编辑器：三个入口共用一个开关，避免叠层。 */
const editorFor = ref<string | null>(null)
const editorKind = ref<'rc' | 'span' | 'dur' | null>(null)

function openEditor(e: Event, kind: 'rc' | 'span' | 'dur') {
  const same = editorFor.value === e.id && editorKind.value === kind
  editorFor.value = same ? null : e.id
  editorKind.value = same ? null : kind
}

/** 日程补丁统一走这里。后端 `update_event` 对派生条目改的是待办本身，
 *  所以 `todo:` 开头的条目只允许改标题/时间/重复，跨天与时长对它没有
 *  语义（待办只占到期那一天）—— 那两个按钮对派生条目直接不给。 */
async function patchEvent(e: Event, patch: Partial<Event>) {
  try {
    await api.updateEvent({ ...e, ...patch })
    editorFor.value = null
    editorKind.value = null
  } catch (err) {
    data.fail(err)
  }
}

/** 自定义跨天数：走同一个应用内弹窗（原生 prompt 在挂件里会卡前台） */
async function customSpan(e: Event) {
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
  await patchEvent(e, { spanDays: Math.min(MAX_SPAN_DAYS, Math.trunc(n)) })
}

async function customDuration(e: Event) {
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
  await patchEvent(e, { durationMin: Math.min(MAX_DURATION_MIN, Math.trunc(n)) })
}

/** 双击日程改名/改时间：输入「HH:MM 标题」，不带时间前缀=全天。
 *  解析规则来自 shared/event-line.ts —— 挂件日历用的是**同一份**，
 *  两处各写一遍必然漂移。后端 update_event 已能处理派生条目（`todo:` 前缀
 *  会改到待办标题上），所以这里统一调它即可。 */
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

// ── 网格：统一"列数 x 行数"，各视图只差参数 ──────────
const cols = computed(() => (view.value === 'days' ? Math.min(count.value, 7) : 7))
const rows = computed(() => {
  const cells = dayCells(view.value, focus.value, count.value)
  if (view.value === 'days') return [cells]
  return weeksRows(cells)
})

// ── 拖拽落点：一个 target，按坐标找命中的日格 ─────────
const gridEl = ref<HTMLElement>()
const dayEls = new Map<string, HTMLElement>()
function setDayEl(d: string, el: unknown) {
  if (el) dayEls.set(d, el as HTMLElement)
  else dayEls.delete(d)
}
function hitDate(x: number, y: number): string | null {
  for (const [d, el] of dayEls) {
    const r = el.getBoundingClientRect()
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return d
  }
  return null
}
watch(gridEl, (el) => {
  if (!el) {
    unregisterTarget('main-cal')
    return
  }
  registerTarget({
    id: 'main-cal',
    hit: (x, y) => {
      const r = el!.getBoundingClientRect()
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
    },
    onDrop: (p, at) => {
      const d = hitDate(at.x, at.y)
      if (!d) return
      if (p.kind === 'todo') api.todoToEvent(p.id, d).catch((e) => data.fail(e))
      else if (p.kind === 'event') api.moveEvent(p.id, d).catch((e) => data.fail(e))
    },
  })
})
onBeforeUnmount(() => unregisterTarget('main-cal'))
const hot = computed(() => hoverTarget.value === 'main-cal')

function go(delta: number) {
  focus.value = stepView(view.value, focus.value, count.value, delta)
}
const gridHot = computed(() => hot.value && view.value !== 'year')

/** 年视图：每月"有日程的日子"预计算成 Map。
 *  模板里若直接调 busyDays()，12 个月 × 每天一次 = 上百次重建 Set，
 *  每次还要遍历全部事件 —— 纯浪费，且首屏会明显卡。 */
const busyByMonth = computed(() => {
  const m = new Map<string, Set<string>>()
  for (const e of events.value) {
    const key = e.date.slice(0, 7)
    let s = m.get(key)
    if (!s) {
      s = new Set()
      m.set(key, s)
    }
    s.add(e.date)
  }
  return m
})
function monthBusy(year: number, month0: number): Set<string> {
  return busyByMonth.value.get(`${year}-${String(month0 + 1).padStart(2, '0')}`) ?? new Set()
}
</script>

<template>
  <div class="cal">
    <!-- 顶栏：视图 + 数量（滴答的「2 周 ▾」）+ 翻页 + 今天 -->
    <div class="chd">
      <div class="seg">
        <button
          v-for="v in CAL_VIEWS"
          :key="v.k"
          :class="{ on: view === v.k }"
          :title="v.tip"
          @click="view = v.k"
        >
          {{ v.label }}
        </button>
      </div>
      <select v-if="hasCount(view)" class="cnt" v-model.number="count" title="显示几份">
        <option v-for="n in countOptions(view)" :key="n" :value="n">{{ n }} {{ view === 'weeks' ? '周' : '天' }}</option>
      </select>
      <span class="ttl">{{ label }}</span>
      <span class="sp" />
      <button class="nav" @click="go(-1)" title="上一页">‹</button>
      <button class="nav today" @click="focus = today">今天</button>
      <button class="nav" @click="go(1)" title="下一页">›</button>
    </div>

    <!-- 年视图：12 个月概览 -->
    <div v-if="view === 'year'" class="ygrid">
      <div v-for="m in months" :key="m.label" class="ymo" @click="focus = `${m.year}-${String(m.month0 + 1).padStart(2, '0')}-01`; view = 'month'">
        <div class="ymh">{{ m.label }}<b>{{ monthBusy(m.year, m.month0).size }}</b></div>
        <div class="ydots">
          <i
            v-for="d in m.days"
            :key="d"
            :class="{ on: monthBusy(m.year, m.month0).has(d), td: d === today }"
          />
        </div>
      </div>
    </div>

    <!-- 日视图：单日详情。双击改名/改时间；行内按钮改重复/跨天/时长 -->
    <div v-else-if="view === 'day'" class="detail">
      <template v-for="e in detailOf(focus)" :key="e.id">
        <div
          class="dev"
          :class="{ editorOpen: editorFor === e.id }"
          :draggable="false"
          title="双击改时间/标题"
          @dblclick="pickDateEdit(e)"
          @pointerdown="armDrag(() => ({ kind: 'event' as const, id: e.id, label: e.title }), 6)"
        >
          <i :style="{ background: e.color }" />
          <span class="tm">{{ eventTimeLabel(e) }}</span>
          <span class="ti">{{ e.title }}</span>
          <!-- 跨天日程的归属说明：起始日标「⬌N天」，续日标「续·第N天」 -->
          <span v-if="e.spanDays > 1" class="tag span">
            ⬌{{ e.spanDays }}天{{ (e.spanIndex ?? 0) > 0 ? ` · 第${(e.spanIndex ?? 0) + 1}天` : '' }}
          </span>
          <span v-if="e.recurrence" class="rc">↻</span>
          <span v-if="e.source === 'todo'" class="tag src">待办</span>
          <button
            class="ib"
            :class="{ on: editorFor === e.id && editorKind === 'rc' }"
            title="重复规则"
            @click.stop="openEditor(e, 'rc')"
          >↻</button>
          <button
            v-if="!e.id.startsWith('todo:')"
            class="ib"
            :class="{ on: editorFor === e.id && editorKind === 'span' }"
            title="跨几天"
            @click.stop="openEditor(e, 'span')"
          >⬌</button>
          <button
            v-if="e.time && !e.id.startsWith('todo:')"
            class="ib"
            :class="{ on: editorFor === e.id && editorKind === 'dur' }"
            title="时长"
            @click.stop="openEditor(e, 'dur')"
          >⏱</button>
        </div>
        <!-- 行内展开的编辑器：不用 fixed 弹层，日视图本来就在滚容器里，
             弹层会被裁（挂件那边踩过同一坑） -->
        <div v-if="editorFor === e.id && editorKind === 'rc'" class="edbox" @click.stop>
          <span class="elb">重复</span>
          <button class="xh-btn" @click="patchEvent(e, { recurrence: null })">不重复</button>
          <button class="xh-btn" @click="patchEvent(e, { recurrence: { freq: 'daily', interval: 1, byWeekday: [], byMonthday: [], until: null, autoCreate: true } })">每天</button>
          <button class="xh-btn" @click="patchEvent(e, { recurrence: { freq: 'weekly', interval: 1, byWeekday: [weekdayOf(e.date)], byMonthday: [], until: null, autoCreate: true } })">每周</button>
          <button class="xh-btn" @click="patchEvent(e, { recurrence: { freq: 'monthly', interval: 1, byWeekday: [], byMonthday: [Number(e.date.slice(8, 10))], until: null, autoCreate: true } })">每月</button>
        </div>
        <div v-else-if="editorFor === e.id && editorKind === 'span'" class="edbox" @click.stop>
          <span class="elb">跨天</span>
          <button
            v-for="n in SPAN_CHOICES"
            :key="n"
            class="xh-btn"
            :class="{ on: e.spanDays === n }"
            @click="patchEvent(e, { spanDays: n })"
          >
            {{ n === 1 ? '不跨' : n + ' 天' }}
          </button>
          <button class="xh-btn" @click="customSpan(e)">自定义…</button>
          <span class="hint">全天日程从当天起连续占用 N 天</span>
        </div>
        <div v-else-if="editorFor === e.id && editorKind === 'dur'" class="edbox" @click.stop>
          <span class="elb">时长</span>
          <button
            v-for="m in DURATION_CHOICES"
            :key="m"
            class="xh-btn"
            :class="{ on: (e.durationMin || 0) === m }"
            @click="patchEvent(e, { durationMin: m })"
          >
            {{ m === 0 ? '不定长' : m + '分' }}
          </button>
          <button class="xh-btn" @click="customDuration(e)">自定义…</button>
        </div>
      </template>
      <div v-if="!detailOf(focus).length" class="empty">这天没安排</div>
    </div>

    <!-- 网格视图：月 / 周 / 多周 / 多日 -->
    <div v-else class="gridwrap" :class="{ hot: gridHot }" ref="gridEl">
      <div class="wdrow" v-if="cols === 7">
        <span v-for="w in ['一', '二', '三', '四', '五', '六', '日']" :key="w" :class="{ wk: w === '六' || w === '日' }">周{{ w }}</span>
      </div>
      <div class="rows">
        <div v-for="(row, ri) in rows" :key="ri" class="rowwrap">
          <!-- 跨天长条层：与下面的格子用同一个列网格（列数、gap 都一样），
               所以 gridColumn 直接对齐到日子列上，不用算百分比。
               滴答/小黄条同形：长条=跨天全天，格内条目=当天有安排。 -->
          <div v-if="barsOf(row).length" class="bars" :style="{ '--cols': cols }">
            <div
              v-for="s in barsOf(row)"
              :key="s.id + ':' + s.from"
              class="bar"
              :class="{ cont: !s.startsHere }"
              :style="{ '--c': s.color, gridColumn: `${s.from + 1} / span ${s.to - s.from + 1}` }"
              :title="`${s.title}（共 ${s.spanTotal} 天）`"
              @click="focus = row[s.from] ?? focus"
            >
              {{ spanBarLabel(s) }}
            </div>
          </div>
          <div class="row" :style="{ '--cols': cols }">
            <div
              v-for="d in row"
              :key="d"
              class="cell"
              :data-date="d"
              :class="{ out: view === 'month' && d.slice(0, 7) !== focus.slice(0, 7), td: d === today, wknd: isWeekend(d) }"
              :ref="(el) => setDayEl(d, el)"
              @click="focus = d"
            >
              <div class="ctop">
                <span class="dn">{{ Number(d.slice(8, 10)) }}</span>
                <span v-if="holidays[d]" class="hol" :class="holidays[d]!.off ? 'off' : 'work'">{{ holidays[d]!.name }}</span>
                <span v-else-if="lunar[d]" class="ln">{{ lunar[d] }}</span>
              </div>
              <div class="cevs">
                <div v-for="e in eventsOf(d).slice(0, 4)" :key="e.id" class="cev" :style="{ '--c': e.color }">
                  {{ e.time ? e.time + ' ' : '' }}{{ e.title }}
                </div>
                <div v-if="eventsOf(d).length > 4" class="more">+{{ eventsOf(d).length - 4 }}</div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cal {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.chd {
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
  padding: 3px 9px;
  cursor: pointer;
}
.seg button.on {
  background: var(--xh-accent);
  color: #241d00;
  font-weight: 600;
}
.cnt {
  font: inherit;
  font-size: 12px;
  border: 1px solid var(--xh-line);
  border-radius: 6px;
  padding: 2px 4px;
  background: transparent;
  color: inherit;
  cursor: pointer;
}
.ttl {
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
}
.sp {
  flex: 1;
}
.nav {
  font: inherit;
  font-size: 12px;
  border: 1px solid var(--xh-line);
  background: transparent;
  color: inherit;
  border-radius: 6px;
  padding: 2px 9px;
  cursor: pointer;
}
.nav.today {
  border-color: var(--xh-accent);
}
/* 年视图 */
.ygrid {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 10px;
  padding: 12px;
}
.ymo {
  border: 1px solid var(--xh-line);
  border-radius: 8px;
  padding: 8px;
  cursor: pointer;
  background: rgba(127, 127, 127, 0.04);
}
.ymo:hover {
  border-color: var(--xh-accent);
}
.ymh {
  display: flex;
  font-size: 12px;
  font-weight: 600;
  margin-bottom: 6px;
}
.ymh b {
  margin-left: auto;
  font-weight: 400;
  font-size: 10px;
  opacity: 0.6;
}
.ydots {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 2px;
}
.ydots i {
  aspect-ratio: 1;
  border-radius: 2px;
  background: rgba(127, 127, 127, 0.12);
}
.ydots i.on {
  background: var(--xh-accent);
}
.ydots i.td {
  outline: 1px solid #4c8dff;
}
/* 网格 */
.gridwrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 0 12px 12px;
}
.gridwrap.hot {
  box-shadow: inset 0 0 0 2px var(--xh-accent);
}
.wdrow {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  flex: none;
  padding: 4px 0;
  font-size: 10px;
  color: var(--xh-ink-2);
}
.wdrow span {
  text-align: center;
}
.wdrow span.wk {
  color: #e08b00;
}
.rows {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
/* 一周 = 长条层 + 那一周的 7 个格子，上下堆叠 */
.rowwrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.row {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
  gap: 4px;
}
/* 跨天长条层：列数与 gap 与 .row 完全一致，故 gridColumn 天然对齐到日子列 */
.bars {
  display: grid;
  grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
  gap: 4px;
  flex: none;
}
.bar {
  font-size: 10px;
  line-height: 1.5;
  padding: 0 5px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--c) 20%, transparent);
  border-left: 3px solid var(--c);
  color: var(--xh-ink);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  min-width: 0;
}
.bar.cont {
  border-left: 3px dotted var(--c);
  opacity: 0.82;
}
.cell {
  border: 1px solid var(--xh-line);
  border-radius: 6px;
  padding: 3px 4px;
  min-height: 0;
  overflow: hidden;
  cursor: pointer;
  background: rgba(255, 255, 255, 0.5);
  display: flex;
  flex-direction: column;
  gap: 2px;
}
:global([data-theme='dark']) .cell {
  background: rgba(38, 38, 42, 0.6);
}
.cell.wknd {
  background: rgba(127, 127, 127, 0.07);
}
.cell.out {
  opacity: 0.38;
}
.cell.td {
  border-color: var(--xh-accent);
  background: rgba(245, 197, 24, 0.14);
}
.ctop {
  display: flex;
  align-items: baseline;
  gap: 3px;
  flex: none;
}
.dn {
  font-size: 12px;
  font-weight: 600;
}
.ln {
  font-size: 9px;
  color: var(--xh-ink-2);
  margin-left: auto;
}
.hol {
  font-size: 9px;
  margin-left: auto;
}
.hol.off {
  color: #d0453b;
}
.hol.work {
  color: #2e9e4f;
}
.cevs {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.cev {
  font-size: 10px;
  line-height: 1.35;
  padding: 1px 3px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--c) 18%, transparent);
  border-left: 2px solid var(--c);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.more {
  font-size: 9px;
  color: var(--xh-ink-2);
  padding-left: 3px;
}
/* 日视图 */
.detail {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.dev {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--xh-line);
  border-radius: 8px;
  background: rgba(127, 127, 127, 0.05);
  cursor: grab;
}
.dev i {
  width: 4px;
  height: 18px;
  border-radius: 2px;
  flex: none;
}
.tm {
  font-size: 11px;
  color: var(--xh-ink-2);
  width: 44px;
  flex: none;
}
.ti {
  font-size: 13px;
}
.rc {
  margin-left: auto;
  color: var(--xh-accent);
}
.empty {
  padding: 30px;
  text-align: center;
  color: var(--xh-ink-2);
  font-size: 12px;
}
/* 日视图行内的小按钮与展开编辑器 */
.dev .ib {
  flex: none;
  border: 1px solid var(--xh-line);
  background: transparent;
  color: var(--xh-ink-2);
  border-radius: 6px;
  font-size: 11px;
  padding: 2px 7px;
  cursor: pointer;
}
.dev .ib:hover,
.dev .ib.on {
  color: #241d00;
  background: var(--xh-accent);
  border-color: transparent;
}
.dev .tag {
  flex: none;
  font-size: 10px;
  color: var(--xh-ink-2);
  border: 1px solid var(--xh-line);
  border-radius: 999px;
  padding: 0 6px;
}
.dev .tag.span {
  color: var(--xh-accent-ink, #241d00);
  background: color-mix(in srgb, var(--xh-accent) 22%, transparent);
  border-color: transparent;
}
.edbox {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
  margin: -2px 0 4px;
  padding: 6px 8px;
  border: 1px dashed var(--xh-line);
  border-radius: 8px;
  background: rgba(127, 127, 127, 0.05);
}
.edbox .elb {
  font-size: 11px;
  color: var(--xh-ink-2);
  width: 34px;
  flex: none;
}
.edbox .hint {
  font-size: 10px;
  color: var(--xh-ink-2);
}
</style>
