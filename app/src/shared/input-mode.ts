/** 让挂件窗口在编辑时可获得键盘焦点
 *
 * 根因：挂件窗口带 `WS_EX_NOACTIVATE`（为了"点它不抢焦点"），
 * 副作用是窗口永远不会成为前台 —— **点击文本框后键盘输入进不去**，
 * 便签、快速添加框、行内改名、日期框全部无法打字。
 *
 * 修法：按需切换。用户碰到可编辑元素时清除该样式并激活窗口，
 * 编辑结束后恢复（并退回桌面层原位）。
 *
 * 触发器选 pointerdown 而不是 focus：NOACTIVATE 窗口能否收到 DOM focus
 * 依赖 Chromium 内部行为，不可靠；而鼠标事件一定送达。focusin 作为兜底。
 */
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from '@tauri-apps/api/core'

const win = getCurrentWindow()
const label = win.label

/** 幽灵窗不需要输入能力，跳过以免白折腾样式 */
const editable = label !== 'drag-ghost'

const EDITABLE_SEL = 'input, textarea, select, [contenteditable="true"], [contenteditable=""], .xh-editable'

function isEditable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  return !!el && typeof el.closest === 'function' && !!el.closest(EDITABLE_SEL)
}

let on = false
let releaseTimer: number | undefined

async function set(next: boolean) {
  if (next === on) return
  on = next
  if (!next) {
    // 交还焦点后稍等再恢复样式，避免 Chromium 还在处理输入
    await new Promise((r) => setTimeout(r, 120))
    if (on) return // 等待期间又被要求进入编辑态
  }
  try {
    await invoke('set_input_mode', { label, on: next })
    // 样式清掉后还要真正把窗口提到前台。必须在窗口自己的线程做，
    // 从 Rust 命令线程远程 AttachThreadInput 实测不生效；
    // Tauri 的 setFocus 正是走窗口线程的 SetForegroundWindow。
    if (next) await win.setFocus()
  } catch {
    /* 窗口已销毁 */
  }
}

function releaseLater() {
  window.clearTimeout(releaseTimer)
  // 元素间跳转（如从标题框 Tab 到日期框）会先 blur 再 focus，
  // 留一个短窗口避免中途恢复样式导致焦点丢失
  releaseTimer = window.setTimeout(() => {
    const a = document.activeElement
    if (a && isEditable(a)) return
    void set(false)
  }, 260)
}

export function bindInputMode(): void {
  if (!editable) return

  const onDown = (e: PointerEvent) => {
    if (!isEditable(e.target)) return
    window.clearTimeout(releaseTimer)
    void set(true)
  }
  const onFocusIn = (e: FocusEvent) => {
    if (!isEditable(e.target)) return
    window.clearTimeout(releaseTimer)
    void set(true)
  }
  const onFocusOut = () => releaseLater()

  // capture：挂件内部常用 @pointerdown.stop 阻断冒泡，必须抢先看到事件
  document.addEventListener('pointerdown', onDown, true)
  document.addEventListener('focusin', onFocusIn, true)
  document.addEventListener('focusout', onFocusOut, true)
}
