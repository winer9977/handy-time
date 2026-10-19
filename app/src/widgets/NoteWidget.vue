<script setup lang="ts">
/** 桌面便签插件：一个窗口内多张便签翻页
 *
 * 每张便签 = 一段富文本（纯文本 + 换行），可换颜色/字号/透明度。
 * 内容改动 600ms 防抖落盘，避免每次按键都写文件。
 */
import { computed, onUnmounted, ref, watch } from 'vue'
import type { Note, Widget } from '../lib/types'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { vClickAway } from '../shared/click-away'
import WidgetHead from '../shared/WidgetHead.vue'

const props = defineProps<{ widget: Widget }>()
const data = useDataStore()

const notes = computed(() => data.notes)
/** 当前便签**按 id** 认定，不按下标。
 *
 *  以前是个 `idx` 数字，靠"猜"维护：置顶后 `idx = 0`、删除后 `idx - 1`。
 *  两张都置顶时点第二张的「置顶」，列表重排后 idx=0 指向的是**另一张**
 *  —— 用户视角"我一置顶，便签内容就变了"。列表还会因为别的窗口保存、
 *  归档区重排而变化，下标从来不是稳定身份。 */
const curId = ref<string | null>(null)
const cur = computed<Note | null>(() => {
  const list = notes.value
  if (!list.length) return null
  return list.find((n) => n.id === curId.value) ?? list[0]!
})
/** 顶栏「2/5」的页码：由 id 反查位置，组件自己不维护游标 */
const pos = computed(() => {
  const i = notes.value.findIndex((n) => n.id === cur.value?.id)
  return i < 0 ? 0 : i
})
const draft = ref('')
const popOpen = ref(false)

/** 服务端快照里的正文与 id：用于判断"输入框里的东西是不是就是已保存的那份" */
let syncedId: string | null = null
let syncedBody = ''
/** 防抖窗口里等着落盘的内容，连同**它属于哪张便签**。
 *
 *  ★ 为什么不能等计时器响了再读 cur.value：600ms 之内用户完全可能翻页/新建/
 *  删除。以前回调里读的是"当时那张"，于是甲便签没落盘的尾巴被写进乙便签，
 *  而甲丢了尾巴；关窗口时更是直接丢（onUnmounted 只 clear 不 flush）。
 *  所以起计时的那一刻就把 owner id 与正文钉下来。 */
let pending: { id: string | null; body: string } | null = null

watch(
  cur,
  (n) => {
    const body = n?.body ?? ''
    // ★ 600ms 防抖落盘 → 后端广播 → store 重载 → cur 变化。这里若**无条件**
    // 把输入框拉回服务端内容，用户在"敲下最后一个字"与"那次保存往返"之间
    // 打的字会被直接抹掉（表现为"便签打到一半尾巴不见了"）。
    // 只有两种情况才覆盖：① 确实切到了另一张便签；② 输入框里没有未保存的
    // 改动（draft 就等于上次同步的内容）。
    if (n?.id !== syncedId) {
      // 切便签：把上一张没落盘的字先冲出去，再换输入框
      if (pending) void flush()
      draft.value = body
    } else if (draft.value === syncedBody) {
      draft.value = body
    }
    syncedId = n?.id ?? null
    syncedBody = body
  },
  { immediate: true },
)

let timer: number | undefined

async function flush() {
  window.clearTimeout(timer)
  const p = pending
  pending = null
  if (!p) return
  if (p.id === null) {
    // 一张便签都没有时 textarea 依然可敲，但没有承载对象 ——
    // 不在这里当场建一张，用户打的字就会被静默丢掉（表现为"便签不能输入"）
    if (!p.body.trim()) return
    try {
      const created = await api.addNote(p.body)
      curId.value = created.id
    } catch (e) {
      data.fail(e)
    }
    return
  }
  const n = notes.value.find((x) => x.id === p.id)
  if (!n) return // 那张已被删（含本次删除流程）：不再写回
  if (n.body === p.body) return
  try {
    await api.updateNote({ ...n, body: p.body })
  } catch (e) {
    data.fail(e)
  }
}

function scheduleSave() {
  pending = { id: cur.value?.id ?? null, body: draft.value }
  window.clearTimeout(timer)
  timer = window.setTimeout(() => void flush(), 600)
}
onUnmounted(() => void flush())

async function next() {
  const list = notes.value
  if (!list.length) return
  curId.value = list[(pos.value + 1) % list.length]!.id
}
async function prev() {
  const list = notes.value
  if (!list.length) return
  curId.value = list[(pos.value - 1 + list.length) % list.length]!.id
}
async function add() {
  try {
    const n = await api.addNote('')
    curId.value = n.id
  } catch (e) {
    data.fail(e)
  }
}
async function del() {
  const n = cur.value
  if (!n) return
  popOpen.value = false
  // 先记下"它走后该看哪张"（下一张，没有就看上一张），再删
  const list = notes.value
  const i = list.findIndex((x) => x.id === n.id)
  const successor = list[i + 1] ?? list[i - 1] ?? null
  pending = null // 删掉就是不要了：别让防抖窗口里的字再写回别处
  try {
    await api.removeNote(n.id)
    curId.value = successor?.id ?? null
  } catch (e) {
    data.fail(e)
  }
}

/** 整对象回写（颜色/字号/透明度）必须带上输入框里**还没落盘**的正文。
 *  这三处走的是 `updateNote(整个 Note)`，正文取的是服务端快照 ——
 *  用户在防抖窗口里刚打的字会被旧正文覆盖掉（点一下色板就丢一段）。 */
function bodyOf(n: Note): string {
  return pending && pending.id === n.id ? pending.body : n.body
}

async function togglePin() {
  const n = cur.value
  if (!n) return
  popOpen.value = false
  try {
    // 只翻数据，游标跟着 id 走：置顶会重排列表，按 idx 猜位置必然跳错便签
    await api.toggleNotePin(n.id)
    curId.value = n.id
  } catch (e) {
    data.fail(e)
  }
}

const COLORS = ['#fff8c4', '#ffe4e6', '#dbeafe', '#dcfce7', '#f3e8ff', '#ffffff', '#1c1e22']
async function setColor(c: string) {
  const n = cur.value
  if (!n) return
  try {
    await api.updateNote({ ...n, body: bodyOf(n), color: c })
  } catch (e) {
    data.fail(e)
  }
}
async function bumpFont(d: number) {
  const n = cur.value
  if (!n) return
  try {
    await api.updateNote({
      ...n,
      body: bodyOf(n),
      fontPx: Math.max(11, Math.min(28, n.fontPx + d)),
    })
  } catch (e) {
    data.fail(e)
  }
}
async function bumpOpacity(d: number) {
  const n = cur.value
  if (!n) return
  try {
    await api.updateNote({
      ...n,
      body: bodyOf(n),
      opacity: Math.max(0.25, Math.min(1, n.opacity + d)),
    })
  } catch (e) {
    data.fail(e)
  }
}

/** 深色底用浅色字 */
const inkFor = computed(() => {
  const c = cur.value?.color ?? '#fff8c4'
  return c === '#1c1e22' ? '#e8eaed' : '#23262b'
})

/** 便签背景色附加透明度：#rrggbb + 两位 alpha 十六进制 */
const alphaTail = computed(() => {
  const o = Math.round(Math.max(0.25, Math.min(1, cur.value?.opacity ?? 1)) * 255)
  return o.toString(16).padStart(2, '0')
})
</script>

<template>
  <div class="xh-shell note" :style="{ background: (cur?.color ?? '#fff8c4') + alphaTail, color: inkFor }">
    <WidgetHead :title="notes.length > 1 ? `便签 ${pos + 1}/${notes.length}` : '便签'">
      <template v-if="notes.length > 1">
        <button class="xh-btn" @click="prev">‹</button>
        <button class="xh-btn" @click="next">›</button>
      </template>
      <button class="xh-btn" @click="add" title="新便签">＋</button>
      <!-- .stop 必须有：否则这次点击会同时被 v-click-away 看到，开关判定不确定 -->
      <button class="xh-btn" :class="{ on: popOpen }" @click.stop="popOpen = !popOpen">⋯</button>
    </WidgetHead>

    <!-- 标签 + 置顶标记：记录时写 [标签] 即自动归类 -->
    <div v-if="cur?.pinned || cur?.tags?.length" class="meta">
      <span v-if="cur?.pinned" class="pinmark">📌</span>
      <span v-for="t in cur?.tags ?? []" :key="t" class="xh-tag">{{ t }}</span>
    </div>

    <div class="xh-body">
      <textarea
        class="paper"
        v-model="draft"
        @input="scheduleSave"
        @blur="scheduleSave"
        :style="{ fontSize: (cur?.fontPx ?? 13) + 'px' }"
        placeholder="随手记点啥…"
        spellcheck="false"
      ></textarea>

      <div v-if="popOpen" v-click-away="() => (popOpen = false)" class="xh-pop xh-nodrag" @pointerdown.stop>
        <div class="row">
          <span class="lbl">颜色</span>
          <button
            v-for="c in COLORS"
            :key="c"
            class="sw"
            :class="{ on: cur?.color === c }"
            :style="{ background: c }"
            @click="setColor(c)"
          ></button>
        </div>
        <div class="row">
          <span class="lbl">字号</span>
          <button class="xh-btn" @click="bumpFont(-1)">A−</button>
          <span>{{ cur?.fontPx }}</span>
          <button class="xh-btn" @click="bumpFont(1)">A＋</button>
        </div>
        <div class="row">
          <span class="lbl">不透明</span>
          <button class="xh-btn" @click="bumpOpacity(-0.1)">−</button>
          <span>{{ Math.round((cur?.opacity ?? 1) * 100) }}%</span>
          <button class="xh-btn" @click="bumpOpacity(0.1)">＋</button>
        </div>
        <div class="row">
          <button class="xh-btn" :class="{ on: cur?.pinned }" @click="togglePin">
            {{ cur?.pinned ? '📌 已置顶' : '📌 置顶' }}
          </button>
        </div>
        <div class="row">
          <button class="xh-btn danger" @click="del">删除这张</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.note {
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.16);
}
.meta {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  padding: 2px 10px 0;
  font-size: 11px;
  opacity: 0.85;
}
.pinmark {
  font-size: 11px;
}
.paper {
  flex: 1;
  min-height: 0;
  border: none;
  outline: none;
  resize: none;
  background: transparent;
  color: inherit;
  font-family: inherit;
  padding: 8px 10px;
  line-height: 1.55;
  user-select: text;
  -webkit-user-select: text;
}
.sw {
  width: 18px;
  height: 18px;
  border-radius: 5px;
  border: 1px solid rgba(127, 127, 127, 0.4);
  cursor: pointer;
  padding: 0;
}
.sw.on {
  outline: 2px solid var(--xh-accent);
}
button.danger {
  color: var(--xh-danger);
}
</style>
