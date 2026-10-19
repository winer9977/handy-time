<script setup lang="ts">
/** 待办行：勾选 / 标签 / 到期 / 倒数 / 重复标记 + 行内编辑 + 指针拖拽排序 + 弹出编辑面板
 *
 * 弹层用 fixed 定位在 ⋯ 按钮下方（放不下翻上），避免被 .xh-shell 的 overflow:hidden 裁掉。
 * 行 class 用 trow，与弹层内 prow 区分，样式不打架。
 */
import { computed, nextTick, ref } from 'vue'
import type { Priority as Pri, Recurrence, Todo } from '../lib/types'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { countdownText, dueLabel } from '../shared/date'
import { armDrag } from '../shared/drag'
import { askPrompt } from '../shared/prompt'
import { recurrenceText } from '../shared/rc-text'
import { duePatch } from '../shared/remind'
import { addTag, removeTag, suggestTags } from '../shared/tags'
import { enterShouldSubmit } from '../shared/ime'
import { vClickAway } from '../shared/click-away'

const props = defineProps<{ todo: Todo }>()
const data = useDataStore()
const list = computed(() => data.lists.find((l) => l.id === props.todo.listId))
const editing = ref(false)
const draft = ref('')
const editEl = ref<HTMLInputElement>()
const popOpen = ref(false)
const moreBtn = ref<HTMLElement>()
const popEl = ref<HTMLElement>()
const popPos = ref({ top: 0, left: 0 })

/** 「今天」走 store.today（依赖快照），不要在 setup 里算一次：
 *  挂件行组件在 store 重载后是**原地复用**的（:key=id），常量会一直停在
 *  创建那天。跨零点后在这里给一条没有到期日的待办设提醒，`due ?? today`
 *  会用昨天的日期 → 提醒时间落在过去，用户当场再收到一次旧提醒。 */
const today = computed(() => data.today)
const due = computed(() => dueLabel(props.todo.due))

const cdText = computed(() =>
  props.todo.countdown
    ? countdownText(
        props.todo.countdown.date,
        props.todo.countdown.mode,
        props.todo.countdown.keepCountingUp,
      )
    : null,
)

const rcText = computed(() => recurrenceText(props.todo.recurrence))

// ── 行内改名 ─────────────────────────────────────────
async function startEdit() {
  editing.value = true
  draft.value = props.todo.title
  await nextTick()
  editEl.value?.focus()
  editEl.value?.select()
}
/** enter 触发后紧接 blur 会二次进入，committed 闸门防重复写 */
let committed = false
async function commitEdit() {
  if (!editing.value || committed) return
  committed = true
  editing.value = false
  const t = draft.value.trim()
  if (t && t !== props.todo.title) {
    try {
      await api.patchTodo(props.todo.id, { title: t })
    } catch (e) {
      data.fail(e)
    }
  }
  nextTick(() => (committed = false))
}
/** 行内改名的按键分派
 *
 * 用 keydown 而非 `@keyup.enter`：中文输入法按回车"上屏候选词"时组合已结束，
 * keyup 照样触发，会把只上了半个词的内容当完整标题提交掉。
 * Esc 是放弃编辑（不提交），与 Enter 语义不同。
 */
function onEditKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (e.isComposing) return
    e.preventDefault()
    editing.value = false
    return
  }
  if (!enterShouldSubmit(e)) return
  e.preventDefault()
  commitEdit()
}

async function toggle(done: boolean) {
  try {
    await api.setDone(props.todo.id, done)
  } catch (e) {
    data.fail(e)
  }
}

async function pin() {
  try {
    await api.togglePin(props.todo.id)
  } catch (e) {
    data.fail(e)
  }
}

/** 删除 = 进回收站（对照滴答：日常删除可恢复）。
 *  彻底删除只在垃圾桶视图里做，这里绝不直接 removeTodo。 */
async function remove() {
  popOpen.value = false
  try {
    await api.trashTodo(props.todo.id)
  } catch (e) {
    data.fail(e)
  }
}

// ── 标签编辑（对照滴答：已有待办也该能加标签，不该只能删了重建）──
const tagInput = ref('')
async function commitTag(raw?: string) {
  const next = addTag(props.todo.tags, raw ?? tagInput.value)
  if (!next) return // 空或已存在：不动数据，免得白广播一次
  try {
    await api.patchTodo(props.todo.id, { tags: next })
    tagInput.value = ''
  } catch (e) {
    data.fail(e)
  }
}
async function dropTag(t: string) {
  try {
    await api.patchTodo(props.todo.id, { tags: removeTag(props.todo.tags, t) })
  } catch (e) {
    data.fail(e)
  }
}

/** 备注：失焦/变更即存。空串与原文相同就不写盘，免得白广播一次。 */
async function applyNote(v: string) {
  const note = v.trim() === '' ? '' : v
  if (note === props.todo.note) return
  try {
    await api.patchTodo(props.todo.id, { note })
  } catch (e) {
    data.fail(e)
  }
}
/** 建议来自全局已用标签（不只本清单）：标签的价值就是跨清单归类 */
const tagSuggest = computed(() => suggestTags(data.store.todos, props.todo.tags))

/** 安排到某天：与"拖到日历格"走同一个后端命令。
 *  跨窗口拖拽依赖真实鼠标，这条菜单项保证该能力始终可达（也可键盘操作）。 */
async function scheduleTo() {
  popOpen.value = false
  const date = await askPrompt({
    title: '安排到哪天（YYYY-MM-DD）',
    value: props.todo.due ?? today.value,
    placeholder: '2026-12-25',
  })
  if (!date) return
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    data.fail('日期格式应为 YYYY-MM-DD')
    return
  }
  try {
    await api.todoToEvent(props.todo.id, date)
  } catch (e) {
    data.fail(e)
  }
}

// ── 到期提醒 ─────────────────────────────────────────
const remindTime = ref(
  (props.todo.remindAt ?? '').includes('T')
    ? (props.todo.remindAt ?? '').slice(11)
    : '09:00',
)

async function setRemindOff() {
  try {
    // 关掉提醒也要重置触发位与去重集合：否则下次重设同一时间时，
    // 旧的 reminded/remindedKeys 还在，新提醒不会响。
    await api.patchTodo(props.todo.id, {
      remindAt: null,
      reminded: false,
      remindedKeys: [],
      lastRemindedAt: null,
    })
  } catch (e) {
    data.fail(e)
  }
}

async function applyRemindTime(v: string) {
  remindTime.value = v
  const date = props.todo.due ?? today.value
  try {
    // **必须重置触发位**：改了提醒时间却不把 reminded 置回 false，
    // 新时点会因"已提醒过"判据（plan_fires 里的 `!t.reminded`）永不触发
    // —— 用户视角"我改了提醒时间，它却不响"。这是迁到字段级补丁时
    // 丢掉 `reminded:false` 造成的真实回归，别再删掉这两个字段。
    await api.patchTodo(props.todo.id, {
      remindAt: `${date}T${v}`,
      reminded: false,
      lastRemindedAt: null,
    })
  } catch (e) {
    data.fail(e)
  }
}

// ── 多提醒时间 + 持续提醒（对照滴答第3节）──────────────
/** 额外提醒按 HH:MM 录入，日期取到期日 —— 与主提醒同一套，窄挂件放得下 */
const extraTime = ref('')
const extraList = computed(() =>
  [...props.todo.extraReminds].sort((a, b) => a.localeCompare(b)),
)
/** 已弹过的时点（后端维护）。UI 上标一下，用户才知道哪些不会再响。 */
const firedExtras = computed(() => new Set(props.todo.remindedKeys))

async function addExtra() {
  const v = extraTime.value
  if (!v) return
  const date = props.todo.due ?? today.value
  const full = `${date}T${v}`
  if (props.todo.extraReminds.includes(full)) {
    extraTime.value = ''
    return
  }
  try {
    await api.patchTodo(props.todo.id, { extraReminds: [...props.todo.extraReminds, full] })
    extraTime.value = ''
  } catch (e) {
    data.fail(e)
  }
}

async function removeExtra(full: string) {
  try {
    await api.patchTodo(props.todo.id, {
      extraReminds: props.todo.extraReminds.filter((x) => x !== full),
      // 去重记录也要一起删：否则改回来同一时间会因残留 key 而永不触发
      remindedKeys: props.todo.remindedKeys.filter((k) => k !== full),
    })
  } catch (e) {
    data.fail(e)
  }
}

async function toggleConstant() {
  try {
    await api.patchTodo(props.todo.id, { constantRemind: !props.todo.constantRemind })
  } catch (e) {
    data.fail(e)
  }
}

async function applyDue(v: string) {
  try {
    // 改到期日 = 提醒整体平移 + 复位去重/节流。字段组装收在
    // shared/remind.ts 的 duePatch（与后端 recurring::shift_reminders_to
    // 同口径）；看板拖列、时间线拖日走的是同一个函数 —— 各写一遍就是
    // 本项目已经修过 4 次的那类 bug（"改了日期，提醒还停在旧那天"）。
    // panelTime 传面板当前值：仅当旧 remind_at 是纯日期（没有时刻可保留）
    // 时才用它补全，不会把用户原设的 18:00 悄悄改成 9 点。
    await api.patchTodo(props.todo.id, duePatch(props.todo, v || null, remindTime.value))
  } catch (e) {
    data.fail(e)
  }
}

// ── 日期倒数 ─────────────────────────────────────────
const cdDate = ref(props.todo.countdown?.date ?? '')
const cdMode = ref<'until' | 'since'>(props.todo.countdown?.mode ?? 'until')
/** 到点后是否继续累计（此前写死 false，字段形同虚设 —— 现在接上开关） */
const cdKeep = ref(props.todo.countdown?.keepCountingUp ?? false)
async function applyCd() {
  try {
    await api.setCountdown(
      props.todo.id,
      cdDate.value
        ? { date: cdDate.value, mode: cdMode.value, keepCountingUp: cdKeep.value }
        : null,
    )
  } catch (e) {
    data.fail(e)
  }
}
/** 只翻"继续累计"开关，保留其余倒数设置 */
async function toggleCdKeep() {
  cdKeep.value = !cdKeep.value
  await applyCd()
}
async function clearCd() {
  cdDate.value = ''
  await applyCd()
}

// ── 优先级旗标 ───────────────────────────────────────
const PR_LABEL: Record<string, string> = { high: '!!!', mid: '!!', low: '!' }
async function setPr(p: Pri | null) {
  try {
    await api.setPriority(props.todo.id, p)
  } catch (e) {
    data.fail(e)
  }
}

// ── 重复规则 ─────────────────────────────────────────
type FreqSel = Recurrence['freq']
const rcWd = ref<number[]>((props.todo.recurrence?.byWeekday ?? []).slice())
const rcMd = ref<string>(String(props.todo.recurrence?.byMonthday?.[0] ?? 1))
const rcUntil = ref(props.todo.recurrence?.until ?? '')

async function applyRc(freq: FreqSel | 'none') {
  if (freq === 'none') {
    popOpen.value = false
    try {
      await api.setRecurrence(props.todo.id, null)
    } catch (e) {
      data.fail(e)
    }
    return
  }
  try {
    await api.setRecurrence(props.todo.id, {
      freq,
      interval: 1,
      byWeekday: [],
      byMonthday: [],
      until: rcUntil.value || null,
      autoCreate: true,
    })
  } catch (e) {
    data.fail(e)
  }
}

async function applyRcWeekday(d: number) {
  const set = new Set(rcWd.value)
  if (set.has(d)) set.delete(d)
  else set.add(d)
  rcWd.value = [...set].sort()
  try {
    await api.setRecurrence(props.todo.id, {
      freq: 'weekly',
      interval: 1,
      byWeekday: rcWd.value,
      byMonthday: [],
      until: rcUntil.value || null,
      autoCreate: true,
    })
  } catch (e) {
    data.fail(e)
  }
}

async function applyRcMonthly() {
  const n = Number(rcMd.value)
  const byMonthday =
    Number.isFinite(n) && n !== 0 ? [Math.max(-1, Math.min(31, Math.trunc(n)))] : []
  try {
    await api.setRecurrence(props.todo.id, {
      freq: 'monthly',
      interval: 1,
      byWeekday: [],
      byMonthday,
      until: rcUntil.value || null,
      autoCreate: true,
    })
  } catch (e) {
    data.fail(e)
  }
}

async function applyRcUntil(v: string) {
  rcUntil.value = v
  if (!props.todo.recurrence) return
  try {
    await api.setRecurrence(props.todo.id, { ...props.todo.recurrence, until: v || null })
  } catch (e) {
    data.fail(e)
  }
}

// ── 弹层定位 ─────────────────────────────────────────
async function togglePop() {
  popOpen.value = !popOpen.value
  if (popOpen.value && moreBtn.value) {
    await nextTick()
    const r = moreBtn.value.getBoundingClientRect()
    const w = 248
    // **实测真实高度**，不能再写死 268：面板已加到十几行（标签/加提醒/
    // 持续/重复…），按 268 估算会以为"放得下"，于是底部的改名/删除被裁到
    // 窗口外 —— 用户报的"删除看不到了、也没法改待办"就是这个。
    const h = popEl.value?.offsetHeight || 268
    const left = Math.max(4, Math.min(window.innerWidth - w - 4, r.right - w))
    const below = r.bottom + 4
    // 下方放不下就翻到上方；再放不下（窗口本身很矮）由 CSS 的 max-height
    // 限高滚动兜底，保证每一行都够得着
    const top = below + h > window.innerHeight ? Math.max(4, r.top - h - 4) : below
    popPos.value = { top, left }
  }
}

// ── 行拖拽（移动超阈值才起拖，避免误触；跨窗口由 Rust 接管） ──
const onPointerDown = (e: PointerEvent) => {
  if (editing.value) return
  armDrag(
    () => ({
      kind: 'todo' as const,
      id: props.todo.id,
      label: props.todo.title,
      data: { order: props.todo.order, listId: props.todo.listId },
    }),
    6,
  )(e)
}

// 被别的窗口（日历）拖拽时的悬停高亮由父容器负责
</script>

<template>
  <div
    class="trow"
    :data-todo-id="todo.id"
    :class="{ done: todo.done, pin: todo.pinned }"
    @pointerdown="onPointerDown"
  >
    <span
      class="cb"
      :class="{ on: todo.done }"
      :style="{ borderColor: todo.done ? 'transparent' : list?.color }"
      @click.stop="toggle(!todo.done)"
      :title="todo.done ? '标记未完成' : '完成'"
      >{{ todo.done ? '✓' : '' }}</span
    >

    <div class="main">
      <input
        v-if="editing"
        ref="editEl"
        class="xh-input edit"
        v-model="draft"
        @keydown="onEditKey"
        @blur="commitEdit"
      />
      <div v-else class="title" @dblclick="startEdit" :title="todo.title">
        <span class="txt">{{ todo.title }}</span>
        <span v-for="t in todo.tags" :key="t" class="xh-tag">{{ t }}</span>
      </div>
      <div v-if="!editing" class="meta">
        <span v-if="todo.priority" class="xh-badge pr" :class="todo.priority" title="优先级">{{ PR_LABEL[todo.priority] }}</span>
        <span v-if="due" class="xh-badge" :class="due.tone">{{ due.text }}</span>
        <span v-if="cdText" class="xh-badge cd">{{ cdText }}</span>
        <span v-if="rcText" class="xh-badge later" title="重复规则">↻{{ rcText }}</span>
        <!-- 有备注才显示；点开 ⋯ 可编辑。不显示内容本身：挂件窄，截断没意义 -->
        <span v-if="todo.note.trim()" class="xh-badge later" :title="todo.note">≡</span>
        <span
          v-if="todo.remindAt && !todo.reminded"
          class="xh-badge soon"
          :title="'提醒 ' + todo.remindAt.replace('T', ' ')"
          >🔔</span
        >
        <span v-if="todo.doneAt" class="xh-badge later">完成 {{ todo.doneAt.slice(5, 10) }}</span>
      </div>
    </div>

    <button class="ic" :class="{ on: todo.pinned }" @click.stop="pin" title="置顶">
      {{ todo.pinned ? '📌' : '📍' }}
    </button>
    <button ref="moreBtn" class="ic" :class="{ on: popOpen }" @click.stop="togglePop">⋯</button>
  </div>

  <div
    v-if="popOpen"
    ref="popEl"
    v-click-away="() => (popOpen = false)"
    class="xh-pop xh-nodrag"
    :style="{ position: 'fixed', top: popPos.top + 'px', left: popPos.left + 'px', right: 'auto' }"
    @pointerdown.stop
  >
    <!-- 操作条置顶：面板已 17 行，改名/删除沉在底部时用户"看不见删除"。
         高频动作必须第一眼可见，不依赖滚动。 -->
    <div class="prow">
      <button class="xh-btn" @click="((popOpen = false), startEdit())">改名</button>
      <button class="xh-btn" @click="scheduleTo" title="生成当天日程（等同拖到日历格）">排到…</button>
      <span class="xh-spacer" />
      <button class="xh-btn danger" @click="remove">删除</button>
    </div>
    <div class="sep"></div>
    <!-- 备注：此前 Todo.note 字段存在但前端零入口，等于不能写备注 -->
    <div class="prow col">
      <span class="lbl">备注</span>
      <textarea
        class="xh-input note"
        rows="3"
        :value="todo.note"
        placeholder="补充说明…（失焦自动保存）"
        @change="applyNote(($event.target as HTMLTextAreaElement).value)"
        @blur="applyNote(($event.target as HTMLTextAreaElement).value)"
      />
    </div>
    <div class="prow">
      <span class="lbl">到期日</span>
      <input
        class="xh-input"
        type="date"
        :value="todo.due ?? ''"
        @change="applyDue(($event.target as HTMLInputElement).value)"
      />
    </div>
    <div class="prow">
      <span class="lbl">优先级</span>
      <button class="xh-btn" :class="{ on: todo.priority === 'high' }" @click="setPr('high')" title="高">!!!</button>
      <button class="xh-btn" :class="{ on: todo.priority === 'mid' }" @click="setPr('mid')" title="中">!!</button>
      <button class="xh-btn" :class="{ on: todo.priority === 'low' }" @click="setPr('low')" title="低">!</button>
      <button v-if="todo.priority" class="xh-btn" @click="setPr(null)">清</button>
    </div>
    <!-- 标签：已有待办也能加/删（对照滴答，不必删了重建）-->
    <div class="prow wrap">
      <span class="lbl">标签</span>
      <span v-for="t in todo.tags" :key="t" class="xr">
        {{ t }}
        <button class="xh-del" title="移除该标签" @click="dropTag(t)">✕</button>
      </span>
      <span v-if="!todo.tags.length" class="hint">无</span>
    </div>
    <div class="prow">
      <span class="lbl">打标签</span>
      <input
        class="xh-input"
        v-model="tagInput"
        placeholder="如 工作，回车确认"
        style="width: 110px"
        @keydown.enter.prevent="commitTag()"
      />
      <button class="xh-btn" :disabled="!tagInput.trim()" @click="commitTag()">＋</button>
    </div>
    <div class="prow wrap" v-if="tagSuggest.length">
      <span class="lbl">常用</span>
      <button
        v-for="s in tagSuggest"
        :key="s.tag"
        class="xh-btn"
        :title="`已有 ${s.n} 条用了这个标签`"
        @click="commitTag(s.tag)"
      >
        {{ s.tag }}
      </button>
    </div>
    <div class="prow">
      <span class="lbl">提醒</span>
      <input
        class="xh-input"
        type="time"
        :value="remindTime"
        style="width: 90px"
        @change="applyRemindTime(($event.target as HTMLInputElement).value)"
      />
      <button class="xh-btn" :class="{ on: !todo.remindAt }" @click="setRemindOff">
        {{ todo.remindAt ? '关闭' : '未设' }}
      </button>
    </div>
    <!-- 额外提醒（对照滴答"多个提醒时间"）：按 HH:MM 加，日期取到期日 -->
    <div class="prow">
      <span class="lbl">加提醒</span>
      <input class="xh-input" type="time" v-model="extraTime" style="width: 90px" />
      <button class="xh-btn" :disabled="!extraTime" @click="addExtra">＋</button>
    </div>
    <div class="prow wrap" v-if="extraList.length">
      <span class="lbl">已加</span>
      <span v-for="x in extraList" :key="x" class="xr" :class="{ fired: firedExtras.has(x) }">
        {{ x.slice(11) }}
        <button class="xh-del" title="删除该提醒" @click="removeExtra(x)">✕</button>
      </span>
    </div>
    <div class="prow">
      <span class="lbl">倒数日</span>
      <input class="xh-input" type="date" v-model="cdDate" @change="applyCd" />
      <button v-if="todo.countdown" class="xh-btn" @click="clearCd">清</button>
    </div>
    <div class="prow" v-if="todo.countdown">
      <span class="lbl">方向</span>
      <button class="xh-btn" :class="{ on: cdMode === 'until' }" @click="((cdMode = 'until'), applyCd())">
        还有
      </button>
      <button class="xh-btn" :class="{ on: cdMode === 'since' }" @click="((cdMode = 'since'), applyCd())">
        已经
      </button>
    </div>
    <div class="prow" v-if="todo.countdown">
      <span class="lbl">到点后</span>
      <button class="xh-btn" :class="{ on: cdKeep }" @click="toggleCdKeep">
        {{ cdKeep ? '继续累计' : '停住' }}
      </button>
      <span class="hint">开启则到点后接着数「已经 N 天」</span>
    </div>
    <div class="prow">
      <span class="lbl">持续</span>
      <button class="xh-btn" :class="{ on: todo.constantRemind }" @click="toggleConstant">
        {{ todo.constantRemind ? '响到完成' : '关' }}
      </button>
      <span class="hint">逾期后每 2 分钟重弹</span>
    </div>
    <div class="sep"></div>
    <div class="prow wrap">
      <span class="lbl">重复</span>
      <button
        v-for="f in (['daily', 'weekly', 'monthly', 'yearly', 'workday', 'officialWorkday', 'holiday', 'lunarMonthly', 'lunarYearly'] as FreqSel[])"
        :key="f"
        class="xh-btn"
        :class="{ on: todo.recurrence?.freq === f }"
        @click="applyRc(f)"
      >
        {{
          {
            daily: '天', weekly: '周', monthly: '月', yearly: '年',
            workday: '作日', officialWorkday: '官作', holiday: '假',
            lunarMonthly: '农历月', lunarYearly: '农历年',
          }[f]
        }}
      </button>
      <button class="xh-btn" @click="applyRc('none')">无</button>
    </div>
    <div class="prow wrap" v-if="todo.recurrence?.freq === 'weekly'">
      <button
        v-for="d in [0, 1, 2, 3, 4, 5, 6]"
        :key="d"
        class="xh-btn"
        :class="{ on: rcWd.includes(d) }"
        @click="applyRcWeekday(d)"
      >
        {{ '一二三四五六日'[d] }}
      </button>
    </div>
    <div class="prow" v-if="todo.recurrence?.freq === 'monthly'">
      <span class="lbl">每月</span>
      <input class="xh-input" v-model="rcMd" style="width: 56px" />
      <button class="xh-btn" @click="applyRcMonthly">定</button>
      <span class="hint">-1 = 当月末</span>
    </div>
    <div class="prow">
      <span class="lbl">重复至</span>
      <input
        class="xh-input"
        type="date"
        :value="rcUntil"
        @change="applyRcUntil(($event.target as HTMLInputElement).value)"
      />
    </div>
  </div>
</template>

<style scoped>
.trow {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 5px 8px;
  border-bottom: 1px solid var(--xh-line);
  transition: background 0.12s;
}
.trow:hover {
  background: rgba(127, 127, 127, 0.08);
}
.trow.pin {
  background: rgba(245, 197, 24, 0.12);
}
.trow.done .title .txt {
  text-decoration: line-through;
  opacity: 0.5;
}
.cb {
  flex: none;
  width: 16px;
  height: 16px;
  margin-top: 1px;
  border: 2px solid;
  border-radius: 5px;
  font-size: 11px;
  line-height: 12px;
  text-align: center;
  cursor: pointer;
  color: transparent;
}
.cb.on {
  background: var(--xh-ok);
  color: #fff;
}
.main {
  flex: 1;
  min-width: 0;
}
.title {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  align-items: baseline;
  cursor: text;
}
.txt {
  word-break: break-all;
  line-height: 1.35;
}
.edit {
  margin: -4px 0;
}
.meta {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
  margin-top: 2px;
}
/* 优先级旗标配色：高红 / 中橙 / 低蓝（滴答、Todoist 同色系） */
.xh-badge.pr {
  font-weight: 700;
  letter-spacing: -1px;
}
.xh-badge.pr.high {
  color: #fff;
  background: #d0453b;
}
.xh-badge.pr.mid {
  color: #fff;
  background: #e08b00;
}
.xh-badge.pr.low {
  color: #fff;
  background: #4c8dff;
}
.ic {
  flex: none;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 12px;
  opacity: 0.45;
  padding: 2px;
  line-height: 1;
  color: inherit;
}
.ic:hover,
.ic.on {
  opacity: 1;
}
.prow {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
/* 备注这类"标签在上、输入框占满整行"的行 */
.prow.col {
  flex-direction: column;
  align-items: stretch;
  gap: 3px;
}
.prow.col .lbl {
  width: auto;
}
.note {
  width: 100%;
  box-sizing: border-box;
  resize: vertical;
  min-height: 46px;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
}
.lbl {
  font-size: 11px;
  color: var(--xh-ink-2);
  width: 44px;
  flex: none;
}
.sep {
  height: 1px;
  background: var(--xh-line);
  margin: 2px 0;
}
.hint {
  font-size: 10px;
  color: var(--xh-ink-2);
}
/* 额外提醒时点胶囊 */
.xr {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 11px;
  padding: 1px 4px 1px 7px;
  border-radius: 999px;
  border: 1px solid var(--xh-line);
  color: var(--xh-ink);
}
/* 已弹过的灰掉 + 划线：让用户看出"这条不会再响" */
.xr.fired {
  opacity: 0.45;
  text-decoration: line-through;
}
/* 删除按钮：命中区必须够大。原来 font-size:9px + padding:0 1px 只有约
   9x11 px，鼠标要点四五次才中一次 —— 用户抱怨"入口不明显"，一半原因在这。
   现在撑到 20x20 以上（字形仍小，但可点区域大）。 */
.xh-del {
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  font-size: 11px;
  min-width: 18px;
  height: 18px;
  padding: 0;
  line-height: 1;
  opacity: 0.55;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
}
.xh-del:hover {
  opacity: 1;
  color: var(--xh-danger);
  background: color-mix(in srgb, var(--xh-danger) 12%, transparent);
}
button.danger {
  color: var(--xh-danger);
}
</style>
