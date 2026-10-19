/** 当前窗口身份：由 Rust 侧 label 决定（w-<widgetId> 或 launcher） */
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, ref } from 'vue'
import type { Widget, WidgetKind, WidgetSize } from '../lib/types'
import { useDataStore } from './store'
import { api } from '../lib/api'

export type WinKind = 'todo' | 'calendar' | 'note' | 'launcher' | 'unknown'

const label = getCurrentWindow().label

export function useWinId() {
  return label
}

export function isLauncher() {
  return label === 'launcher'
}

/** 该窗口对应的挂件配置 */
export function useWidget() {
  const data = useDataStore()
  const id = label.startsWith('w-') ? label.slice(2) : ''
  return computed<Widget | null>(() => data.store.widgets.find((w) => w.id === id) ?? null)
}

/** 拖窗口：指针按下 → startDragging（透明窗 HTML5 拖不动，只能走原生） */
export function useHeadDrag() {
  const win = getCurrentWindow()
  let saveTimer: number | undefined
  async function onDown(e: PointerEvent) {
    if (e.button !== 0) return
    // 只有点在"空白处"才拖，按钮/输入框不劫持
    const t = e.target as HTMLElement
    if (t.closest('button, input, textarea, select, .xh-nodrag')) return
    e.preventDefault()
    await win.startDragging()
    // 拖动结束后回写几何
    window.clearTimeout(saveTimer)
    saveTimer = window.setTimeout(() => saveGeometry(), 220)
  }
  return { onDown }
}

/** 把窗口当前位置/尺寸写回 store（逻辑像素）
 *
 * 尺寸变化时同步 size 档位：后端 sync_windows 会按 size 覆写 rect.w/h，
 * 不同步的话用户拉伸出来的尺寸会在下次重排时被档位值吃掉。
 */
export async function saveGeometry() {
  if (!label.startsWith('w-')) return
  const id = label.slice(2)
  const win = getCurrentWindow()
  try {
    const pos = await win.outerPosition()
    const size = await win.innerSize()
    const scale = await win.scaleFactor()
    const rect = {
      x: pos.x / scale,
      y: pos.y / scale,
      w: size.width / scale,
      h: size.height / scale,
    }
    await api.saveWidgetRect(id, rect)
    // 拉伸后按面积就近换档，让 size 与真实尺寸一致。
    // 字段级补丁：只发 size，别把 visible 等字段用陈旧快照覆盖回去。
    const data = useDataStore()
    const w = data.store.widgets.find((x) => x.id === id)
    if (w) {
      const area = rect.w * rect.h
      const next = pickSize(w.kind, area)
      if (next !== w.size) {
        await api.patchWidget(id, { size: next })
      }
    }
  } catch {
    /* 窗口已销毁 */
  }
}

/** 各档位的参考面积（与 Rust size_of 对齐），就近归档 */
const AREA_TABLE: Record<string, Array<[WidgetSize, number]>> = {
  todo: [['s', 250 * 290], ['m', 310 * 370], ['l', 390 * 470], ['xl', 500 * 580]],
  calendar: [['s', 250 * 320], ['m', 310 * 400], ['l', 390 * 500], ['xl', 500 * 620]],
  note: [['s', 250 * 210], ['m', 310 * 270], ['l', 390 * 350], ['xl', 500 * 460]],
}

function pickSize(kind: WidgetKind, area: number): WidgetSize {
  const table = AREA_TABLE[kind] ?? AREA_TABLE.todo!
  let best = table[0]!
  let bestGap = Infinity
  for (const row of table) {
    const gap = Math.abs(row[1] - area)
    if (gap < bestGap) {
      bestGap = gap
      best = row
    }
  }
  return best[0]
}

/** 不透明度：Rust 侧广播，此处落到 CSS 变量 */
export function useOpacity() {
  const opacity = ref(1)
  return {
    opacity,
    apply(v: number) {
      opacity.value = v
      document.documentElement.style.setProperty('--xh-bg', hexA(v))
    },
  }
}

function hexA(a: number) {
  const theme = document.documentElement.dataset.theme
  if (theme === 'dark') return `rgba(28,30,34,${a})`
  return `rgba(255,255,255,${a})`
}
