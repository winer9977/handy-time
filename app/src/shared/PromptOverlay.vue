<script setup lang="ts">
/** 挂件内输入弹窗（见 shared/prompt.ts） */
import { nextTick, ref, watch } from 'vue'
import { current, settle } from '../shared/prompt'
import { enterShouldSubmit } from '../shared/ime'

const input = ref('')
const el = ref<HTMLInputElement>()

watch(
  current,
  async (p) => {
    input.value = p?.value ?? ''
    if (p) {
      await nextTick()
      el.value?.focus()
      el.value?.select()
    }
  },
  { immediate: true },
)

function ok() {
  const v = input.value.trim()
  settle(v || null)
}
function cancel() {
  settle(null)
}
/** 用 keydown 而不是 keyup：中文输入法按回车"上屏候选词"时组合已结束，
 *  keyup 照样触发，会把只上了半个词的内容当完整输入提交掉。
 *  keydown + isComposing 才能把那次回车挡在 IME 侧。 */
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (e.isComposing) return
    e.preventDefault()
    cancel()
    return
  }
  if (!enterShouldSubmit(e)) return
  e.preventDefault()
  ok()
}
</script>

<template>
  <div v-if="current" class="mask" @pointerdown.self="cancel">
    <div class="dlg">
      <div class="t">{{ current.title }}</div>
      <input
        ref="el"
        class="xh-input"
        v-model="input"
        :placeholder="current.placeholder ?? ''"
        @keydown="onKey"
      />
      <div class="row">
        <button class="xh-btn" @click="cancel">取消</button>
        <button class="xh-btn ok" @click="ok">{{ current.okText ?? '确定' }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask {
  position: fixed;
  inset: 0;
  z-index: 9000;
  background: rgba(0, 0, 0, 0.28);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 10px;
}
.dlg {
  width: 100%;
  max-width: 260px;
  background: var(--xh-bg-solid);
  border: 1px solid var(--xh-line);
  border-radius: 10px;
  box-shadow: var(--xh-shadow);
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.t {
  font-size: 12px;
  font-weight: 600;
}
.row {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}
button.ok {
  background: var(--xh-accent);
  color: var(--xh-accent-ink);
  border-color: transparent;
  font-weight: 700;
}
</style>
