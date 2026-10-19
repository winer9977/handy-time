<script setup lang="ts">
/** 窗口入口：按 label 分派到对应组件
 *
 * label = "w-<widgetId>" → 从 store 查该挂件的 kind 决定渲染什么；
 * label = "launcher"     → 快速添加窗；
 * label = "drag-ghost"   → 拖拽幽灵窗（不读数据，只跟随光标显示标签）。
 * 每个窗口一份 store 实例，靠后端 xhy://changed 广播保持一致。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useDataStore } from './shared/store'
import { isLauncher, useWidget } from './shared/winctx'
import { onEvent } from './lib/api'
import { bindDrag } from './shared/drag'
import { bindInputMode } from './shared/input-mode'
import TodoWidget from './widgets/TodoWidget.vue'
import CalendarWidget from './widgets/CalendarWidget.vue'
import NoteWidget from './widgets/NoteWidget.vue'
import Launcher from './widgets/Launcher.vue'
import MainView from './widgets/MainView.vue'
import GhostView from './widgets/GhostView.vue'
import PromptOverlay from './shared/PromptOverlay.vue'

const data = useDataStore()
const widget = useWidget()
const isGhost = getCurrentWindow().label === 'drag-ghost'
const isMain = getCurrentWindow().label === 'main'
const kind = computed(() => (isLauncher() ? 'launcher' : (widget.value?.kind ?? 'unknown')))

const ready = ref(false)
const attachNote = ref('')

onMounted(async () => {
  // 幽灵窗不需要任何数据，早退避免多余 IPC 与"挂件不存在"提示
  if (isGhost) {
    ready.value = true
    return
  }
  await data.load()
  await data.bind()
  // 落点窗口不一定是起拖窗口，必须在启动就绑好，否则跨窗口投放收不到
  await bindDrag()
  // 关键：不绑这个，挂件里所有文本框都打不了字（窗口 NOACTIVATE 拿不到键盘）。
  // 但**主窗口不能绑** —— 它是普通顶层窗口，set_input_mode 会给它加
  // WS_EX_NOACTIVATE，反而把键盘废掉。挂件才需要那套"点它不抢焦点"的处理。
  if (!isMain) bindInputMode()
  ready.value = true
  if (!isMain) {
    await onEvent<string>('attach', (p) => {
      attachNote.value = p === 'downgraded' ? '桌面嵌入不可用，已降级为贴底模式' : ''
      setTimeout(() => (attachNote.value = ''), 5000)
    })
  }
})

// 主题 / 字号跨窗口同步（watch 具体字段：load() 会整体替换 settings 对象）
watch(
  () => [data.settings.theme, data.settings.fontPx] as const,
  ([theme, fontPx]) => {
    document.documentElement.dataset.theme = theme
    document.documentElement.style.setProperty('--xh-font', `${fontPx}px`)
  },
  { immediate: true },
)
</script>

<template>
  <PromptOverlay />
  <template v-if="ready">
    <GhostView v-if="isGhost" />
    <template v-else>
      <div v-if="attachNote" class="xh-attach-note">{{ attachNote }}</div>
      <div v-if="data.lastError" class="xh-err">{{ data.lastError }}</div>
      <MainView v-if="isMain" />
      <Launcher v-else-if="kind === 'launcher'" />
      <TodoWidget v-else-if="kind === 'todo' && widget" :widget="widget" />
      <CalendarWidget v-else-if="kind === 'calendar' && widget" :widget="widget" />
      <NoteWidget v-else-if="kind === 'note' && widget" :widget="widget" />
      <div v-else class="xh-shell">
        <div class="xh-empty">
          此挂件的配置已不存在
          <button class="xh-btn" @click="data.load()">重新载入</button>
        </div>
      </div>
    </template>
  </template>
  <div v-else class="xh-shell"><div class="xh-empty">载入中…</div></div>
</template>

<style>
.xh-attach-note {
  position: fixed;
  left: 50%;
  bottom: 8px;
  transform: translateX(-50%);
  z-index: 9998;
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 6px;
  background: rgba(229, 72, 77, 0.92);
  color: #fff;
  pointer-events: none;
}
.xh-err {
  position: fixed;
  left: 50%;
  top: 8px;
  transform: translateX(-50%);
  z-index: 9998;
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 6px;
  background: rgba(229, 72, 77, 0.92);
  color: #fff;
  max-width: 92%;
  pointer-events: none;
}
</style>
