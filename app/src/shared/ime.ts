/** 回车是否应该触发提交 —— 中文输入法安全
 *
 * 坑：用 IME 打中文时，回车的第一下是"把候选词上屏"，不是"提交"。
 * 而 Vue 的 v-model 在组合期间（compositionstart…compositionend）**不更新**，
 * 此时 input.value 还是空的。
 *
 * 若像原先那样无条件 `preventDefault() + submit()`：
 *   - preventDefault 掐断了 IME 上屏 → 文字卡在候选窗
 *   - submit 读到空串直接 return → 什么都没发生
 * 用户看到的就是"回车没用了"。
 *
 * 正确做法：组合中一律放行，交给输入法处理。用户上屏后再按一次回车即提交。
 * `keyCode === 229` 是老版 Chromium/Safari 上报组合态的方式，一并挡掉。
 */
export function enterShouldSubmit(e: {
  key: string
  shiftKey?: boolean
  isComposing?: boolean
  keyCode?: number
}): boolean {
  if (e.key !== 'Enter' || e.shiftKey) return false
  return !e.isComposing && e.keyCode !== 229
}
