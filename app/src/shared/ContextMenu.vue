<script setup lang="ts">
/** 轻量右键菜单（对照滴答清单行的 编辑/归档/删除）。
 *
 *  为什么不用原生 contextmenu 弹窗：浏览器默认菜单会挡住我们自己的项，
 *  且无法定制。这里自己画一层，与主题一致。
 *
 *  定位规则：clientX/clientY 直接当 fixed 坐标；贴边时向内收，
 *  保证任何屏幕位置都完整可见（挂件可能贴在屏幕最边缘）。
 */
import { onBeforeUnmount, onMounted } from 'vue'

export interface MenuItem {
  key: string
  label: string
  danger?: boolean
  /** 显示为分隔线之上/之下 */
  divider?: boolean
}

const props = defineProps<{
  x: number
  y: number
  items: MenuItem[]
}>()
const emit = defineEmits<{ (e: 'pick', key: string): void; (e: 'close'): void }>()

const W = 132
const H_PER = 30

/** 贴边收拢：菜单宽 132、每项约 30px 高 */
function style() {
  const maxX = window.innerWidth - W - 4
  const maxY = window.innerHeight - props.items.length * H_PER - 8
  return {
    left: `${Math.max(4, Math.min(maxX, props.x))}px`,
    top: `${Math.max(4, Math.min(maxY, props.y))}px`,
    width: `${W}px`,
  }
}

function onPick(k: string) {
  emit('pick', k)
  emit('close')
}
function onDocDown(e: MouseEvent) {
  const el = e.target as HTMLElement
  if (!el.closest('.ctxmenu')) emit('close')
}
function onEsc(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
onMounted(() => {
  document.addEventListener('pointerdown', onDocDown, true)
  document.addEventListener('keydown', onEsc)
})
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocDown, true)
  document.removeEventListener('keydown', onEsc)
})
</script>

<template>
  <div class="ctxmenu" :style="style()" @pointerdown.stop @contextmenu.prevent>
    <template v-for="it in items" :key="it.key">
      <div v-if="it.divider" class="sep" />
      <button class="mi" :class="{ danger: it.danger }" @click="onPick(it.key)">{{ it.label }}</button>
    </template>
  </div>
</template>

<style scoped>
.ctxmenu {
  position: fixed;
  z-index: 9500;
  background: var(--xh-bg-solid);
  border: 1px solid var(--xh-line);
  border-radius: 8px;
  box-shadow: var(--xh-shadow);
  padding: 4px;
  display: flex;
  flex-direction: column;
}
.mi {
  border: none;
  background: transparent;
  color: var(--xh-ink);
  text-align: left;
  font-size: 13px;
  padding: 6px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.mi:hover {
  background: var(--xh-line);
}
.mi.danger {
  color: var(--xh-danger);
}
.sep {
  height: 1px;
  background: var(--xh-line);
  margin: 4px 2px;
}
</style>
