<script setup lang="ts">
/** 挂件通用头部：标题 + 插槽按钮 + 贴边开关 + 关闭。整条可拖窗（点按钮除外） */
import { computed } from 'vue'
import { useHeadDrag } from './winctx'
import { useDataStore } from './store'
import { api } from '../lib/api'
import { useWidget } from './winctx'

const props = defineProps<{ title: string }>()
const { onDown } = useHeadDrag()
const data = useDataStore()
const widget = useWidget()

/** 当前停靠的边：left/right/top/null */
const edge = computed(() => widget.value?.edgeHide ?? null)
const autoDock = computed(() => widget.value?.autoDock ?? true)
const label = computed(() =>
  autoDock.value
    ? edge.value
      ? `贴边隐藏：${{ left: '左', right: '右', top: '上' }[edge.value] ?? edge.value}（点击切换）`
      : '贴边隐藏：开（拖到屏幕边即可收起）'
    : '贴边隐藏：关',
)

/** 三态循环：开(未停靠) → 关 → 开 → …；停靠边由后端自动检测 */
async function toggleEdge() {
  const w = widget.value
  if (!w) return
  try {
    // 字段级补丁：整对象覆盖会连带把 visible 等别的字段写回旧值
    await api.patchWidget(w.id, { autoDock: !w.autoDock, edgeHide: null })
    await data.load()
  } catch (e) {
    data.fail(e)
  }
}

/** 收起此挂件：写 store.visible=false，窗口生死由后端 sync 统一决定。
 *  绝不再用 win.hide() —— 那会造出两套可见性真相（Tauri 内部状态 vs
 *  store），托盘"显示全部"后 ✕ 就失效（用户报的 bug）。 */
async function collapse() {
  const w = widget.value
  if (!w) return
  try {
    await api.setWidgetVisible(w.id, false)
  } catch (e) {
    data.fail(e)
  }
}
</script>

<template>
  <div class="xh-head" @pointerdown="onDown">
    <span class="xh-title">{{ title }}</span>
    <span class="xh-spacer" />
    <slot />
    <button
      v-if="widget"
      class="xh-btn xh-nodrag edge"
      :class="{ on: autoDock }"
      :title="label"
      @click.stop="toggleEdge"
    >
      {{ autoDock ? '⇥' : '⇤' }}
    </button>
    <button class="xh-btn xh-nodrag" title="收起此挂件（不退出程序）" @click="collapse">
      ✕
    </button>
  </div>
</template>

<style scoped>
.edge {
  font-size: 11px;
  padding: 2px 5px;
}
</style>

