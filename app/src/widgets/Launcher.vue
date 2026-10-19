<script setup lang="ts">
/** 快速添加窗（热键 Alt+Shift+N 唤出）："记一笔"入口 + 设置面板
 *
 * 输入实时预览后端解析结果（标题 / 标签 / 日期），所见即所得。
 */
import { nextTick, onMounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { LogicalSize } from '@tauri-apps/api/dpi'
import { api } from '../lib/api'
import { BRAND } from '../shared/brand'
import type { SettingsPatch } from '../lib/types'
import { useDataStore } from '../shared/store'
import { useHeadDrag } from '../shared/winctx'
import { enterShouldSubmit } from '../shared/ime'
import SummaryPanel from './SummaryPanel.vue'

const data = useDataStore()
const win = getCurrentWindow()
// 带按钮守卫的拖拽：裸 startDragging 会吞掉头部按钮的点击
const { onDown } = useHeadDrag()

const tab = ref<'add' | 'sum' | 'settings'>('add')
const text = ref('')
const listId = ref<string>('')
const box = ref<HTMLInputElement>()

/** 各 tab 的窗口高度（逻辑像素）。建窗时是 132，只够"记一笔"；
 *  总结/设置内容多，切过去要撑开，切回来要收回 —— 否则设置面板一直是
 *  在 132px 里滚动，很难用。 */
const TAB_SIZE: Record<typeof tab.value, [number, number]> = {
  add: [460, 132],
  sum: [460, 430],
  settings: [460, 430],
}
watch(tab, async (t) => {
  const [w, h] = TAB_SIZE[t]
  await win.setSize(new LogicalSize(w, h)).catch(() => {})
})

onMounted(async () => {
  await data.load()
  await data.bind()
  // 默认落点必须是**活动**清单：lists[0] 可能是已归档的，
  // 那样快速添加会把待办送进一个侧栏看不见的清单
  listId.value = data.activeLists[0]?.id ?? ''
  await nextTick()
  box.value?.focus()
})

/** 预览：直接调后端 quick_parse，规则与入库完全一致 */
// 从 api 派生，避免此处再写一份内联类型而后端加了字段就悄悄漂移
type ParsePreview = Awaited<ReturnType<typeof api.quickParse>>
const preview = ref<ParsePreview | null>(null)
let pvSeq = 0
async function parseNow() {
  const raw = text.value.trim()
  if (!raw) {
    preview.value = null
    return
  }
  const seq = ++pvSeq
  try {
    const p = await api.quickParse(raw)
    if (seq === pvSeq) preview.value = p // 丢弃过期响应，避免乱序
  } catch {
    if (seq === pvSeq) preview.value = null
  }
}
// commit 校验在 submit 内做（raw 为空即 return），无需额外 computed

async function submit(keepOpen: boolean) {
  const raw = text.value.trim()
  if (!raw) return
  if (!listId.value && data.activeLists.length) listId.value = data.activeLists[0]!.id
  if (!listId.value) {
    const l = await api.addList('收集箱', '#f5c518').catch(() => null)
    if (!l) return
    listId.value = l.id
  }
  try {
    await api.addTodo(listId.value, raw)
    text.value = ''
    preview.value = null
    if (!keepOpen) await win.hide()
    box.value?.focus()
  } catch (e) {
    data.fail(e)
  }
}

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    win.hide()
    return
  }
  // 输入法组合中的回车归 IME（确认候选词），不归提交 —— 见 shared/ime.ts
  if (!enterShouldSubmit(e)) return
  e.preventDefault()
  submit(e.ctrlKey || e.metaKey)
}

// ── 设置 ──────────────────────────────────────────────
async function setTheme(t: string) {
  try {
    await api.setSettings({ theme: t })
  } catch (e) {
    data.fail(e)
  }
}
async function setAttach(m: 'workerw' | 'bottomz') {
  try {
    await api.setSettings({ attachMode: m })
    const msg = await api.reapplyAttach()
    data.lastError = msg // 复用错误条显示提示
    setTimeout(() => (data.lastError = ''), 3000)
  } catch (e) {
    data.fail(e)
  }
}
async function toggleAuto(v: boolean) {
  try {
    await api.setAutostart(v)
  } catch (e) {
    data.fail(e)
  }
}
async function setFontPx(v: number) {
  try {
    await api.setSettings({ fontPx: v })
  } catch (e) {
    data.fail(e)
  }
}
async function setOpacity(v: number) {
  try {
    await api.setSettings({ opacity: v })
  } catch (e) {
    data.fail(e)
  }
}
/** 重新嵌入桌面层：拖窗/切模式后 z-order 复位（不动位置） */
async function reapply() {
  try {
    const msg = await api.reapplyAttach()
    data.lastError = msg
    setTimeout(() => (data.lastError = ''), 3000)
  } catch (e) {
    data.fail(e)
  }
}
/** 自动对齐：把非自由摆放的挂件重排到左右两列（会移动位置） */
async function align() {
  try {
    await api.autoAlign()
  } catch (e) {
    data.fail(e)
  }
}
const dir = ref('')
async function showDir() {
  dir.value = await api.dataDir().catch(() => '')
}

/** AI 配置：后端 AiSettings 是全量结构，patch 会整体替换，所以要先合并现有值 */
async function setAi(e: Event, field: 'apiKey' | 'model' | 'baseUrl') {
  const v = (e.target as HTMLInputElement).value
  const cur = data.settings.ai ?? { baseUrl: '', apiKey: '', model: '' }
  await api.setSettings({ ai: { ...cur, [field]: v } }).catch((x) => data.fail(x))
}

/** 设置项写入：失败一定要让用户看见。
 *
 *  设置页那两个数字框以前直接 `api.setSettings(...)` 不接 reject ——
 *  手打一个解析不出整数的值（NaN）会让后端 u16/u8 反序列化失败，
 *  Promise **静默** reject：界面看着像改成功了，下次打开又变回旧值。
 *  现在统一走这里，失败进同一条错误提示。 */
async function patchSettings(p: SettingsPatch) {
  try {
    await api.setSettings(p)
  } catch (e) {
    data.fail(e)
  }
}
/** 数字框的取值：空串/乱输入都归到"不改"（NaN 传下去后端会报错，但
 *  更稳的做法是压根不发这次写入） */
function num(e: Event): number | null {
  const v = Number((e.target as HTMLInputElement).value)
  return Number.isFinite(v) ? v : null
}
async function setArchive(e: Event) {
  const n = num(e)
  if (n != null) await patchSettings({ archiveAfterDays: n })
}
async function setBackupKeep(e: Event) {
  const n = num(e)
  if (n != null) await patchSettings({ backupKeep: n })
}
</script>

<template>
  <div class="xh-shell launcher">
    <!-- 必须用带守卫的 useHeadDrag：裸 startDragging 会在点按钮时也起拖，
         导致「设置 / 记一笔 / ✕」全部点不动 -->
    <div class="xh-head" @pointerdown="onDown">
      <span class="xh-title">{{ BRAND }}</span>
      <span class="xh-spacer" />
      <button class="xh-btn xh-nodrag" :class="{ on: tab === 'add' }" @click="tab = 'add'">记一笔</button>
      <button class="xh-btn xh-nodrag" :class="{ on: tab === 'sum' }" @click="tab = 'sum'">总结</button>
      <button class="xh-btn xh-nodrag" :class="{ on: tab === 'settings' }" @click="tab = 'settings'">设置</button>
      <button class="xh-btn xh-nodrag" @click="win.hide()">✕</button>
    </div>

    <div class="xh-body pad" v-if="tab === 'add'">
      <input
        ref="box"
        class="xh-input big"
        v-model="text"
        @input="parseNow"
        @keydown="onKey"
        placeholder="说一句话就记下来：[工作] 周五 交周报"
        spellcheck="false"
      />
      <div v-if="preview" class="pv">
        <span class="lbl">将记为</span>
        <b>{{ preview.title || '（空）' }}</b>
        <span v-for="t in preview.tags" :key="t" class="xh-tag">{{ t }}</span>
        <span v-if="preview.due" class="xh-badge soon">{{ preview.due }}</span>
        <span v-else class="xh-badge later">无到期</span>
        <!-- 识别到时间会落成提醒，所以要点出来，别让用户以为丢了 -->
        <span v-if="preview.time" class="xh-badge soon">⏰ {{ preview.time }}</span>
      </div>
      <div class="row">
        <span class="lbl">清单</span>
        <select class="xh-input" v-model="listId">
          <!-- 只列活动清单：归档清单不该出现在"往里加东西"的下拉里，
               否则用户能把待办加进一个侧栏看不见的清单 -->
          <option v-for="l in data.activeLists" :key="l.id" :value="l.id">{{ l.name }}</option>
        </select>
        <button class="xh-btn ok" @click="submit(false)">记（回车）</button>
        <button class="xh-btn" @click="submit(true)">连记（Ctrl+回车）</button>
      </div>
      <div class="hint">
        支持：<code>[标签]</code> 今天/明天/后天 · 周X/下周X · 2026-10-20 / 2026年10月20日 ·
        M月D日 / M-D · 9:30 / 下午3点 / 晚上8点半
      </div>
    </div>

    <div class="xh-body pad scroll" v-else-if="tab === 'sum'">
      <SummaryPanel />
    </div>

    <div class="xh-body pad scroll" v-else-if="tab === 'settings'">
      <div class="grp">外观</div>
      <div class="row">
        <span class="lbl">主题</span>
        <button class="xh-btn" :class="{ on: data.settings.theme === 'light' }" @click="setTheme('light')">浅</button>
        <button class="xh-btn" :class="{ on: data.settings.theme === 'dark' }" @click="setTheme('dark')">深</button>
      </div>
      <div class="row">
        <span class="lbl">字号</span>
        <button class="xh-btn" @click="setFontPx(data.settings.fontPx - 1)">A−</button>
        <span>{{ data.settings.fontPx }}</span>
        <button class="xh-btn" @click="setFontPx(data.settings.fontPx + 1)">A＋</button>
      </div>
      <div class="row">
        <span class="lbl">透明</span>
        <input type="range" min="30" max="100" :value="Math.round(data.settings.opacity * 100)"
          @input="setOpacity(Number(($event.target as HTMLInputElement).value) / 100)" />
        <span>{{ Math.round(data.settings.opacity * 100) }}%</span>
      </div>

      <div class="grp">桌面</div>
      <div class="row">
        <span class="lbl">嵌入</span>
        <button class="xh-btn" :class="{ on: data.settings.attachMode === 'workerw' }" @click="setAttach('workerw')">
          壁纸层
        </button>
        <button class="xh-btn" :class="{ on: data.settings.attachMode === 'bottomz' }" @click="setAttach('bottomz')">
          贴最底
        </button>
      </div>
      <div class="row">
        <span class="lbl">自启</span>
        <button class="xh-btn" :class="{ on: data.settings.autostart }" @click="toggleAuto(!data.settings.autostart)">
          {{ data.settings.autostart ? '开' : '关' }}
        </button>
        <span class="hint">开机即用</span>
      </div>
      <div class="row">
        <button class="xh-btn" @click="reapply">重新嵌入桌面层</button>
        <span class="hint">拖窗后 z-order 复位</span>
        <button class="xh-btn" @click="align">自动对齐</button>
        <span class="hint">重排左右两列</span>
      </div>

      <div class="grp">数据</div>
      <div class="row">
        <button class="xh-btn" @click="showDir">数据目录</button>
        <span class="dir" v-if="dir">{{ dir }}</span>
      </div>
      <div class="row">
        <button class="xh-btn" @click="api.saveMarkdown()">导出 Markdown…</button>
        <span class="hint">贴笔记/周报用</span>
      </div>
      <div class="row">
        <button class="xh-btn" @click="api.importMarkdownFile().catch((e) => data.fail(e))">
          导入 Markdown…
        </button>
        <span class="hint">Kite 等导出的 .md，合并进库不覆盖</span>
      </div>
      <div class="row" v-if="data.notice">
        <span class="dir">{{ data.notice }}</span>
      </div>
      <div class="row">
        <span class="lbl">归档</span>
        <input class="xh-input num" type="number" min="0" max="60" :value="data.settings.archiveAfterDays"
          @change="setArchive($event)" />
        <span class="hint">天后归档完成项</span>
      </div>
      <div class="row">
        <span class="lbl">备份</span>
        <input class="xh-input num" type="number" min="1" max="50" :value="data.settings.backupKeep"
          @change="setBackupKeep($event)" />
        <span class="hint">份轮转</span>
      </div>
      <div class="row">
        <span class="lbl">延期</span>
        <button class="xh-btn" :class="{ on: data.settings.autoPostpone }"
          @click="patchSettings({ autoPostpone: !data.settings.autoPostpone })">
          {{ data.settings.autoPostpone ? '开' : '关' }}
        </button>
        <span class="hint">启动时逾期项推到今天</span>
      </div>

      <div class="grp">AI 总结（可选）</div>
      <div class="row">
        <span class="lbl">Key</span>
        <input class="xh-input grow" type="password" :value="data.settings.ai?.apiKey ?? ''"
          placeholder="sk-…（留空则只用本地统计）" @change="setAi($event, 'apiKey')" />
      </div>
      <div class="row">
        <span class="lbl">模型</span>
        <input class="xh-input grow" :value="data.settings.ai?.model ?? ''" placeholder="qwen-plus"
          @change="setAi($event, 'model')" />
      </div>
      <div class="row">
        <span class="lbl">地址</span>
        <input class="xh-input grow" :value="data.settings.ai?.baseUrl ?? ''" placeholder="https://…"
          @change="setAi($event, 'baseUrl')" />
      </div>
      <div class="hint">OpenAI 兼容接口，默认预填阿里云百炼（千问）。Key 明文存在本机数据目录，
        不会上传到别处；不填也能用，总结退回本地统计摘要。</div>
      <div class="hint">热键：Alt+Shift+H 显示/隐藏全部挂件</div>
    </div>
  </div>
</template>

<style scoped>
.pad {
  padding: 8px;
  gap: 7px;
}
.scroll {
  overflow-y: auto;
}
.big {
  font-size: 14px;
  padding: 7px 10px;
}
.pv {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
  font-size: 11px;
  color: var(--xh-ink-2);
  padding: 0 2px;
}
.pv b {
  color: var(--xh-ink);
  font-weight: 600;
}
.row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.lbl {
  font-size: 11px;
  color: var(--xh-ink-2);
  width: 34px;
  flex: none;
}
.hint {
  font-size: 10px;
  color: var(--xh-ink-2);
}
.dir {
  font-size: 10px;
  word-break: break-all;
  color: var(--xh-ink-2);
}
.grp {
  font-size: 11px;
  font-weight: 700;
  color: var(--xh-ink-2);
  border-bottom: 1px solid var(--xh-line);
  padding-bottom: 2px;
  margin-top: 2px;
}
.num {
  width: 62px;
}
button.ok {
  background: var(--xh-accent);
  color: var(--xh-accent-ink);
  border-color: transparent;
  font-weight: 700;
}
code {
  background: rgba(127, 127, 127, 0.16);
  padding: 0 3px;
  border-radius: 3px;
}
select.xh-input {
  flex: 1;
  min-width: 0;
}
</style>
