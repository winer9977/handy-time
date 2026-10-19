/** v-click-away：点弹层外面就关掉
 *
 * 三处弹层（便签 ⋯ / 待办行 ⋯ / 日程 ↻）原先只能再点一次锚点按钮关闭，
 * 点别处不消失 —— 违反弹层常识。
 *
 * 为什么监听 **冒泡阶段的 click** 而不是 pointerdown：
 * 锚点按钮本来就写了 `@click.stop`，事件根本到不了 document，
 * 于是"点锚点"天然不会被误判成"点外面"（否则 pointerdown 先关、
 * 按钮 click 又开，越修越关不掉）。弹层内部点击靠 contains 判定放行。
 *
 * 刻意**不**在 window blur 时关闭：待办行弹层里有原生日期/时间选择器，
 * 打开它会触发 blur，把面板连带关掉，用户选到一半就没了。
 * 代价是"点另一个挂件窗口"不会自动收起 —— 可接受。
 */
import type { Directive } from 'vue'

type Entry = { el: HTMLElement; cb: () => void }
const live = new Set<Entry>()
let bound = false

function onClick(ev: MouseEvent) {
  if (!live.size) return
  const t = ev.target as Node | null
  if (!t) return
  for (const e of [...live]) {
    // 弹层内部的点击不关（日期选择、拖滑块等）
    if (e.el.contains(t)) continue
    e.cb()
  }
}

function onKey(ev: KeyboardEvent) {
  if (ev.key !== 'Escape' || !live.size) return
  for (const e of [...live]) e.cb()
}

function ensureBound() {
  if (bound) return
  bound = true
  document.addEventListener('click', onClick)
  document.addEventListener('keydown', onKey, true)
}

function maybeUnbind() {
  if (live.size || !bound) return
  bound = false
  document.removeEventListener('click', onClick)
  document.removeEventListener('keydown', onKey, true)
}

/**
 * 用法：`<div v-if="open" v-click-away="() => (open = false)">…</div>`
 * 指令名按 Vue 的 `v-` 约定自动解析（vClickAway → v-click-away）。
 */
export const vClickAway: Directive<HTMLElement, () => void> = {
  mounted(el, binding) {
    ensureBound()
    // 互斥：待办每行都有自己的弹层，开新的一定要收掉旧的，
    // 否则两行面板会叠在一起（旧面板的锚点用了 @click.stop，
    // 那次点击不会冒到 document，光靠 onClick 兜不住）
    for (const e of [...live]) e.cb()
    live.add({ el, cb: binding.value })
  },
  updated(el, binding) {
    for (const e of live) if (e.el === el) e.cb = binding.value
  },
  unmounted(el) {
    for (const e of [...live]) if (e.el === el) live.delete(e)
    maybeUnbind()
  },
}
