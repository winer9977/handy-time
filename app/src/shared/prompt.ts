/** 应用内输入弹窗，取代 window.prompt
 *
 * 为什么不能用 window.prompt：挂件是透明、常驻、贴底的窗口，原生模态框会
 *  1) 抢走前台焦点，把用户从别的工作里拽出来；
 *  2) 在贴底窗口上可能被其它窗口盖住，看起来像"卡死"（实测会挂住整条自动化）；
 *  3) 视觉与挂件完全脱节。
 * 所以做成挂件内部的覆盖层，样式跟主题一致，且只挡自己这个窗口。
 */
import { ref } from 'vue'

export interface PromptOpts {
  title: string
  value?: string
  placeholder?: string
  /** 输入框类型；time 用于"HH:MM 标题"这类 */
  multiline?: boolean
  okText?: string
}

interface Pending extends PromptOpts {
  id: number
  resolve: (v: string | null) => void
}

export const current = ref<Pending | null>(null)
let seq = 0

export function askPrompt(opts: PromptOpts): Promise<string | null> {
  return new Promise((resolve) => {
    // 同一窗口一次只问一个；后来者直接取消前一个，避免堆积
    current.value?.resolve(null)
    current.value = { ...opts, id: ++seq, resolve }
  })
}

export function settle(v: string | null) {
  const p = current.value
  current.value = null
  p?.resolve(v)
}
