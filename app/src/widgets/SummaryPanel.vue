<script setup lang="ts">
/** 周/月总结（对照滴答清单的"回顾"）
 *
 * 两层结构，顺序不能反：
 *   1. 本地统计 —— 永远算得出、可单测、离线可用。这是主体。
 *   2. AI 润色 —— 可选。没配 key / 断网 / 模型报错都退回第 1 层的摘要文本，
 *      所以按钮失败不是错误，只是"没加成"。
 */
import { onMounted, ref, watch } from 'vue'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { excludedStatsSig, statsDataSig } from '../shared/scope'
import type { Summary } from '../lib/types'

/** listId = 选中清单：统计跟着过滤。可选 —— 快速添加窗独立用时不传 = 全局 */
const props = defineProps<{ listId?: string | null }>()

const kind = ref<'week' | 'month'>('week')
const sum = ref<Summary | null>(null)
const loading = ref(false)
const err = ref('')
/** 清单数据源：用于监听"哪些清单不计入总结"的变化（见下方 watch） */
const data = useDataStore()

/** AI 文本与本地摘要分开存：切周期要清掉，否则会拿上周的 AI 文案配本周数字 */
const aiText = ref('')
const aiBusy = ref(false)
const aiErr = ref('')

async function load() {
  loading.value = true
  err.value = ''
  aiText.value = ''
  aiErr.value = ''
  try {
    sum.value = await api.periodSummary(kind.value, props.listId)
  } catch (e) {
    err.value = String(e)
  } finally {
    loading.value = false
  }
}

async function askAi() {
  if (!sum.value) return
  aiBusy.value = true
  aiErr.value = ''
  try {
    // 与面板同一 list 口径：AI 润色的必须是屏幕上这份数字
    aiText.value = await api.aiSummary(kind.value, props.listId)
  } catch (e) {
    // 降级是预期路径，措辞别写成"故障"
    aiErr.value = String(e).replace(/^Error:\s*/, '')
    aiText.value = ''
  } finally {
    aiBusy.value = false
  }
}

onMounted(load)
// listId 变化也要重算：主窗口切清单，总结数字必须跟着变（用户报过的 bug）。
//
// 还必须监听**会影响统计口径的清单属性**：用户勾选「不计入总结」后，
// 清单数据变了但 kind/listId 都没变，漏了这条依赖数字就停在旧值
// —— 表现为"我明明排除了那个清单，总结没反应"（用户报的 bug，已加测试锚定）。
// 只取这两个字段参与比较：加整个 lists 数组会让任何改名/换色也触发重算。
//
// 再加一条**数据签名**：在挂件里勾掉一项、或改了一条待办的到期日，
// 面板数字不该停在旧值（总结页与"打开工作台先看一眼"的场景是同一个）。
// 用 statsDataSig 而不是监听整个 store：便签每 600ms 防抖保存也会广播，
// 而便签根本不进统计，不该白刷一次后端。
watch(
  [
    kind,
    () => props.listId,
    () => excludedStatsSig(data.store.lists),
    () => statsDataSig(data.store),
  ],
  load,
)

function pct(n: number) {
  return `${Math.round(n * 100)}%`
}
/** 环比箭头：正=比上期好 */
function delta(s: Summary): { txt: string; cls: string } | null {
  if (!s.stats.prev) return null
  const d = Math.round((s.stats.completionRate - s.stats.prev.completionRate) * 100)
  if (d > 0) return { txt: `↑${d} 个百分点`, cls: 'up' }
  if (d < 0) return { txt: `↓${-d} 个百分点`, cls: 'down' }
  return { txt: '与上期持平', cls: 'flat' }
}
/** 柱状图高度（按本期峰值归一） */
function barH(n: number) {
  const peak = Math.max(1, ...(sum.value?.stats.perDay.map(([, v]) => v) ?? [1]))
  return `${Math.round((n / peak) * 100)}%`
}
function dayLabel(d: string) {
  return d.slice(8).replace(/^0/, '')
}
</script>

<template>
  <div class="sum">
    <div class="seg">
      <button class="xh-btn" :class="{ on: kind === 'week' }" @click="kind = 'week'">本周</button>
      <button class="xh-btn" :class="{ on: kind === 'month' }" @click="kind = 'month'">本月</button>
      <span class="range" v-if="sum">{{ sum.stats.from }} ~ {{ sum.stats.to }}</span>
    </div>

    <div v-if="loading" class="hint">统计中…</div>
    <div v-else-if="err" class="hint bad">{{ err }}</div>

    <template v-else-if="sum">
      <div class="cards">
        <div class="card">
          <b>{{ sum.stats.done }}</b>
          <span>完成</span>
        </div>
        <div class="card">
          <b>{{ pct(sum.stats.completionRate) }}</b>
          <span>完成率</span>
          <em v-if="delta(sum)" :class="delta(sum)!.cls">{{ delta(sum)!.txt }}</em>
        </div>
        <div class="card">
          <b>{{ sum.stats.overdueLeft }}</b>
          <span>遗留</span>
        </div>
        <div class="card">
          <b>{{ sum.stats.created }}</b>
          <span>新建</span>
        </div>
      </div>

      <!-- 每日产出：月视图 31 根柱子会很密，但仍是有效信号，不隐藏 -->
      <div class="bars" :class="{ dense: sum.stats.perDay.length > 10 }">
        <div v-for="[d, n] in sum.stats.perDay" :key="d" class="b" :title="`${d}：${n} 项`">
          <i :style="{ height: n ? barH(n) : '2px' }" :class="{ z: !n }"></i>
          <span>{{ dayLabel(d) }}</span>
        </div>
      </div>

      <div class="tags" v-if="sum.stats.byList.length || sum.stats.byTag.length">
        <span class="k">分布</span>
        <span v-for="[l, n] in sum.stats.byList" :key="'l' + l" class="xh-tag">{{ l }} {{ n }}</span>
        <span v-for="[t, n] in sum.stats.byTag" :key="'t' + t" class="xh-tag">#{{ t }} {{ n }}</span>
      </div>

      <div class="left" v-if="sum.stats.leftTitles.length">
        <span class="k">遗留</span>
        <span class="v">{{ sum.stats.leftTitles.join('、') }}</span>
      </div>

      <!-- 展示文本：AI 有就用 AI，否则本地摘要 —— 两者都是能读的总结 -->
      <div class="out">
        <div class="hd">
          <span>{{ aiText ? 'AI 总结' : '统计摘要' }}</span>
          <button class="xh-btn" :disabled="aiBusy" @click="askAi">
            {{ aiBusy ? '生成中…' : aiText ? '重新生成' : 'AI 润色' }}
          </button>
        </div>
        <p class="txt">{{ aiText || sum.digest }}</p>
        <p v-if="aiErr" class="hint">{{ aiErr }}（已显示本地统计）</p>
        <p v-else-if="!aiText" class="hint">在设置里填 API Key 可让大模型把数字写成一段话</p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.sum {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.seg {
  display: flex;
  align-items: center;
  gap: 5px;
}
.range {
  margin-left: auto;
  font-size: 10px;
  color: var(--xh-ink-2);
}
.cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}
.card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  padding: 6px 2px;
  border: 1px solid var(--xh-line);
  border-radius: 7px;
  background: rgba(127, 127, 127, 0.06);
}
.card b {
  font-size: 17px;
  line-height: 1.1;
}
.card span {
  font-size: 10px;
  color: var(--xh-ink-2);
}
.card em {
  font-size: 9px;
  font-style: normal;
}
.card em.up {
  color: #2e9e4f;
}
.card em.down {
  color: #d0453b;
}
.card em.flat {
  color: var(--xh-ink-2);
}
.bars {
  display: flex;
  align-items: flex-end;
  gap: 4px;
  height: 54px;
  padding: 0 2px;
}
.b {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
  height: 100%;
  min-width: 0;
}
.b i {
  width: 100%;
  max-width: 22px;
  background: var(--xh-accent);
  border-radius: 2px 2px 0 0;
  opacity: 0.85;
}
.b i.z {
  background: var(--xh-line);
}
.b span {
  font-size: 9px;
  color: var(--xh-ink-2);
  line-height: 1;
}
/* 月视图 31 根：数字只留个位数会糊，直接不显示 */
.bars.dense .b span {
  visibility: hidden;
}
.tags,
.left {
  display: flex;
  align-items: baseline;
  gap: 5px;
  flex-wrap: wrap;
  font-size: 11px;
}
.k {
  font-size: 10px;
  color: var(--xh-ink-2);
  flex: none;
}
.left .v {
  color: var(--xh-ink-2);
}
.out {
  border-top: 1px solid var(--xh-line);
  padding-top: 6px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.out .hd {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10px;
  color: var(--xh-ink-2);
}
.txt {
  margin: 0;
  font-size: 12px;
  line-height: 1.55;
  white-space: pre-wrap;
  user-select: text;
  -webkit-user-select: text;
}
.hint {
  font-size: 10px;
  color: var(--xh-ink-2);
}
.hint.bad {
  color: #d0453b;
}
</style>
