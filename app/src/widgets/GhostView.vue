<script setup lang="ts">
/** 拖拽幽灵窗内容：跟随光标的标签
 *
 * 文本来源两路，避免"窗口刚建好还没监听上"的竞态：
 *   · 挂载时主动 `ghost_text()` 拉一次
 *   · 之后接收 `xhy://ghost-text` 定向推送
 */
import { onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { fetchGhostText } from '../shared/drag'

const text = ref('')
const visible = ref(false)

onMounted(async () => {
  const t = await fetchGhostText()
  text.value = t.label
  visible.value = t.dragging
  await listen<string>('xhy://ghost-text', (e) => {
    text.value = e.payload
    visible.value = e.payload.length > 0
  })
})
</script>

<template>
  <div v-if="visible && text" class="ghost">{{ text }}</div>
</template>

<style scoped>
.ghost {
  display: inline-block;
  max-width: 250px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 4px 10px;
  border-radius: 8px;
  background: #f5c518;
  color: #241d00;
  font-weight: 700;
  font-size: 12px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
  transform: rotate(-2deg);
}
</style>
