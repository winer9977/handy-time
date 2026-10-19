<script setup lang="ts">
/** 分析视图（对照滴答统计页）：完成趋势 + 清单分布 + 标签分布 + 优先级分布
 *
 *  与「总结」的分工：总结是"这一周/这一月怎么样"（周期性回顾 + AI 润色），
 *  分析是"跨周期的分布与趋势"（看习惯、看重心）。两者数字都来自后端
 *  stats::compute 同一份口径，绝不会互相打架。
 *
 *  图表一律 CSS 画（不引图表库）：桌面小工具里多一个 200KB 依赖不值当，
 *  而且柱状/条状用 flex + 百分比高度就够，还能吃主题变量。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '../lib/api'
import { useDataStore } from '../shared/store'
import { excludedStatsSig, statsDataSig } from '../shared/scope'
import type { PeriodStats } from '../lib/types'

const props = defineProps<{ listId?: string | null }>()
const data = useDataStore()

const range = ref<7 | 30 | 90>(30)
const trend = ref<[string, number][]>([])
const sum = ref<PeriodStats | null>(null)
const loading = ref(false)
const err = ref('')

/** 统计口径签名：交给 shared/scope.ts 的纯函数（两面板共用，且有单测） */
const excludedSig = () => excludedStatsSig(data.store.lists)

async function load() {
  loading.value = true
  err.value = ''
  try {
    const [tr, s] = await Promise.all([
      // list 与下面 periodSummary 同值：漏了会出现趋势柱"全局 2 项"、
      // 下方分布"本清单 0 项"的同屏自相矛盾（实机截图抓到过）
      api.doneTrend(range.value, props.listId),
      // 分布用"本月"口径：够长能看出结构，又不像 90 天那样把陈旧数据混进来
      api.periodSummary('month', props.listId),
    ])
    trend.value = tr
    sum.value = s.stats
  } catch (e) {
    err.value = String(e)
  } finally {
    loading.value = false
  }
}
onMounted(load)
// 影响统计口径的三个维度：区间、选中清单、**哪些清单不计入总结**
// （最后一条漏了就会出现"勾选了不计入总结，分析页数字没反应"——
//   与 SummaryPanel 同一类 bug，用户报过总结页的那次）。
watch([range, () => props.listId, () => excludedSig()], load)
// 数据变了要重算（在别的窗口完成一项，这里的柱不该停在旧值）。
// ★ 依赖必须用 shared/scope.ts 的 statsDataSig，不能是 todos.length：
//   勾选完成**不改条数**（只有重复待办会 +1），拿长度当依赖等于没有依赖 ——
//   上面这句注释想要的效果此前从未真的生效过（"完成态变了但签名没变"）。
watch(() => statsDataSig(data.store), () => void load())

const trendMax = computed(() => Math.max(1, ...trend.value.map(([, n]) => n)))
const trendTotal = computed(() => trend.value.reduce((a, [, n]) => a + n, 0))
/** 趋势柱太多会糊成一片：超过 45 根就不标日期，只标首尾 */
const sparse = computed(() => trend.value.length > 45)
function dayShort(d: string) {
  return `${Number(d.slice(5, 7))}/${Number(d.slice(8, 10))}`
}
function barH(n: number) {
  return `${Math.round((n / trendMax.value) * 100)}%`
}

/** 分布条：按最大值归一 */
function distRows(rows: [string, number][] | undefined, colorOf?: (k: string) => string) {
  const list = rows ?? []
  const max = Math.max(1, ...list.map(([, n]) => n))
  return list.map(([k, n]) => ({
    k,
    n,
    w: `${Math.round((n / max) * 100)}%`,
    color: colorOf?.(k),
  }))
}
const PR_LABEL: Record<string, string> = { high: '高 !!!', mid: '中 !!', low: '低 !' }
const PR_COLOR: Record<string, string> = { high: '#e5484d', mid: '#f5a524', low: '#4c8dff' }
const listRows = computed(() => distRows(sum.value?.byList))
const tagRows = computed(() => distRows(sum.value?.byTag))
const prRows = computed(() =>
  distRows(sum.value?.byPriority, (k) => PR_COLOR[k] ?? 'var(--xh-ink-2)'),
)
</script>

<template>
  <div class="analysis">
    <header class="ah">
      <div class="segr">
        <button v-for="r in ([7, 30, 90] as const)" :key="r" :class="{ on: range === r }" @click="range = r">
          近{{ r }}天
        </button>
      </div>
      <span class="spacer" />
      <span class="tot">共完成 <b>{{ trendTotal }}</b> 项</span>
    </header>

    <div v-if="err" class="err">{{ err }}</div>
    <div v-else-if="loading && !sum" class="empty">载入中…</div>

    <div v-else class="body">
      <!-- 完成趋势 -->
      <section class="card">
        <h4>完成趋势</h4>
        <div class="chart" v-if="trend.length">
          <div class="bars">
            <div
              v-for="(row, i) in trend"
              :key="row[0]"
              class="bar"
              :title="`${row[0]}：完成 ${row[1]} 项`"
            >
              <div class="fill" :style="{ height: barH(row[1]) }" />
              <span class="xlab" v-if="!sparse || i === 0 || i === trend.length - 1 || i % 7 === 0">
                {{ dayShort(row[0]) }}
              </span>
            </div>
          </div>
          <div class="peak" v-if="sum?.bestDay">
            产出最高：{{ sum.bestDay[0] }}（{{ sum.bestDay[1] }} 项）
          </div>
        </div>
        <div v-else class="none">这段时间还没有完成记录</div>
      </section>

      <!-- 三个分布并排 -->
      <div class="grid">
        <section class="card">
          <h4>清单分布<span class="sub">本月完成</span></h4>
          <div v-if="listRows.length" class="rows">
            <div v-for="r in listRows" :key="r.k" class="row">
              <span class="rk">{{ r.k }}</span>
              <span class="track"><i :style="{ width: r.w }" /></span>
              <b class="rn">{{ r.n }}</b>
            </div>
          </div>
          <div v-else class="none">本月还没有完成的待办</div>
        </section>

        <section class="card">
          <h4>标签分布<span class="sub">本月完成</span></h4>
          <div v-if="tagRows.length" class="rows">
            <div v-for="r in tagRows" :key="r.k" class="row">
              <span class="rk">#{{ r.k }}</span>
              <span class="track"><i :style="{ width: r.w }" /></span>
              <b class="rn">{{ r.n }}</b>
            </div>
          </div>
          <div v-else class="none">没有带标签的完成项</div>
        </section>

        <section class="card">
          <h4>优先级分布<span class="sub">本月完成</span></h4>
          <div v-if="prRows.length" class="rows">
            <div v-for="r in prRows" :key="r.k" class="row">
              <span class="rk">{{ PR_LABEL[r.k] ?? r.k }}</span>
              <span class="track"><i :style="{ width: r.w, background: r.color }" /></span>
              <b class="rn">{{ r.n }}</b>
            </div>
          </div>
          <div v-else class="none">没有设优先级的完成项</div>
        </section>
      </div>

      <!-- 关键指标 -->
      <section class="card" v-if="sum">
        <h4>本月概览</h4>
        <div class="kpis">
          <div class="kpi"><b>{{ sum.done }}</b><span>完成</span></div>
          <div class="kpi"><b>{{ Math.round(sum.completionRate * 100) }}%</b><span>完成率</span></div>
          <div class="kpi"><b>{{ sum.created }}</b><span>新建</span></div>
          <div class="kpi"><b>{{ sum.overdueLeft }}</b><span>遗留</span></div>
          <div class="kpi"><b>{{ sum.eventCount }}</b><span>日程</span></div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.analysis {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
}
.ah {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  border-bottom: 1px solid var(--xh-line);
  flex: none;
}
.segr {
  display: inline-flex;
  gap: 4px;
}
.segr button {
  border: 1px solid var(--xh-line);
  background: transparent;
  color: var(--xh-ink-2);
  border-radius: 6px;
  padding: 3px 10px;
  font-size: 12px;
  cursor: pointer;
}
.segr button.on {
  background: #f5c518;
  border-color: #f5c518;
  color: #1b1b1d;
  font-weight: 600;
}
.spacer {
  flex: 1;
}
.tot {
  font-size: 12px;
  color: var(--xh-ink-2);
}
.body {
  flex: 1;
  overflow-y: auto;
  padding: 12px 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.card {
  border: 1px solid var(--xh-line);
  border-radius: 10px;
  padding: 12px 14px;
  background: rgba(255, 255, 255, 0.5);
}
:global([data-theme='dark']) .card {
  background: rgba(255, 255, 255, 0.03);
}
.card h4 {
  margin: 0 0 10px;
  font-size: 13px;
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.sub {
  font-size: 10px;
  font-weight: 400;
  color: var(--xh-ink-2);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 12px;
}
/* 趋势柱 */
.bars {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 130px;
}
.bar {
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  align-items: center;
  gap: 3px;
}
.fill {
  width: 100%;
  min-height: 2px;
  background: linear-gradient(180deg, #f7d154, #f5c518);
  border-radius: 2px 2px 0 0;
}
.xlab {
  font-size: 9px;
  color: var(--xh-ink-2);
  white-space: nowrap;
}
.peak {
  margin-top: 8px;
  font-size: 11px;
  color: var(--xh-ink-2);
}
/* 分布条 */
.rows {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.row {
  display: grid;
  grid-template-columns: 84px 1fr 28px;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}
.rk {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--xh-ink);
}
.track {
  height: 8px;
  background: var(--xh-line);
  border-radius: 999px;
  overflow: hidden;
}
.track i {
  display: block;
  height: 100%;
  background: #f5c518;
  border-radius: 999px;
}
.rn {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
/* KPI */
.kpis {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.kpi {
  flex: 1;
  min-width: 78px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 8px 4px;
  border: 1px solid var(--xh-line);
  border-radius: 8px;
}
.kpi b {
  font-size: 18px;
}
.kpi span {
  font-size: 11px;
  color: var(--xh-ink-2);
}
.none,
.empty {
  font-size: 12px;
  color: var(--xh-ink-2);
  padding: 10px 0;
}
.err {
  padding: 14px 16px;
  font-size: 12px;
  color: var(--xh-danger);
}
</style>
