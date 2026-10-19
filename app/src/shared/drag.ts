/** 跨窗口拖拽（前端侧）
 *
 * 真实的光标跟踪、幽灵窗、抬起检测都在 Rust（src-tauri/src/drag.rs），
 * 因为每个挂件是独立 webview，前端拿不到别的窗口的坐标。
 *
 * 本模块职责：
 *   1. `beginDrag` —— 起拖（调用方需自行做"移动超阈值"判定，避免把点击当拖拽）
 *   2. 维护本窗口的落点表；把 Rust 广播的全局物理坐标换算成本窗口 CSS 坐标后命中
 *   3. 悬停高亮状态
 *
 * 坐标换算：窗口物理原点 (wx,wy)、缩放 s，则光标 (cx,cy) 在本窗口的
 * CSS 坐标 = ((cx-wx)/s, (cy-wy)/s)。CSS 像素 == 逻辑像素 == 物理/缩放。
 */
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { ref, shallowRef } from 'vue'

export interface DragPayload {
  kind: 'todo' | 'event' | 'note' | 'list'
  id: string
  /** 幽灵窗显示的文本 */
  label: string
  data?: Record<string, unknown>
}

export interface DropTarget {
  id: string
  /** 命中判定，参数是本窗口 CSS 坐标 */
  hit: (x: number, y: number) => boolean
  onDrop: (p: DragPayload, at: { x: number; y: number }) => void
}

const win = getCurrentWindow()

export const dragging = shallowRef<DragPayload | null>(null)
/** 本窗口当前被悬停的落点 id（供高亮） */
export const hoverTarget = ref<string | null>(null)

const targets = new Map<string, DropTarget>()
/** 命中优先级：后注册者优先（更具体的落点，如"某一天格"覆盖"整个日历"） */
const order: string[] = []

export function registerTarget(t: DropTarget) {
  targets.set(t.id, t)
  const i = order.indexOf(t.id)
  if (i >= 0) order.splice(i, 1)
  order.push(t.id)
}

export function unregisterTarget(id: string) {
  targets.delete(id)
  const i = order.indexOf(id)
  if (i >= 0) order.splice(i, 1)
  if (hoverTarget.value === id) hoverTarget.value = null
}

/** 本窗口原点物理坐标 + 缩放，缓存 400ms 足够（拖拽期间窗口不动） */
let geo: { wx: number; wy: number; s: number; at: number } | null = null
/** 全局物理坐标 → 本窗口 CSS 坐标。纯函数，单独可测。
 *
 * CSS 像素 == 逻辑像素 == 物理像素 / 缩放。窗口原点必须用**物理**像素
 * （outerPosition 返回物理），混用单位是跨窗口投放落偏的常见根因。
 */
export function globalToLocal(
  gx: number,
  gy: number,
  win: { wx: number; wy: number; s: number },
): { x: number; y: number } {
  const s = win.s > 0 ? win.s : 1
  return { x: (gx - win.wx) / s, y: (gy - win.wy) / s }
}

async function geometry(): Promise<typeof geo> {
  const now = performance.now()
  if (geo && now - geo.at < 400) return geo
  try {
    const pos = await win.outerPosition()
    const s = (await win.scaleFactor()) || 1
    geo = { wx: pos.x, wy: pos.y, s, at: now }
  } catch {
    // 拿不到自身位置就返回 null：绝不能拿 (0,0,1) 去猜。
    // 猜错的后果是"静默投到错误落点"，比不投严重得多。
    geo = null
  }
  return geo
}

async function toLocal(cx: number, cy: number) {
  const g = await geometry()
  if (!g) return null
  return globalToLocal(cx, cy, g)
}

function topmostHit(x: number, y: number): string | null {
  for (let i = order.length - 1; i >= 0; i--) {
    const t = targets.get(order[i]!)
    if (t && t.hit(x, y)) return t.id
  }
  return null
}

/** 把前端侧的关键节点写进 xhy.log（跨窗口拖拽的断点可能在前端，
 *  只有一侧日志等于没有日志）。日志本身失败绝不能影响拖拽。 */
function log(msg: string) {
  invoke('log_line', { source: win.label, msg }).catch(() => {})
}

let bound = false
/** 绑定本窗口的拖拽事件。必须在每个窗口启动时调用一次：
 *  落点窗口（如日历）往往不是起拖窗口，懒绑会漏掉跨窗口投放。 */
export async function bindDrag(): Promise<void> {
  if (bound) return
  bound = true
  await listen<{ x: number; y: number }>('xhy://drag-move', (e) => {
    // 刻意不打日志：每次移动都广播给每个窗口，一次拖拽上百行
    void toLocal(e.payload.x, e.payload.y).then((p) => {
      if (!p) return
      hoverTarget.value = topmostHit(p.x, p.y)
    })
  })

  await listen('xhy://drag-end', () => {
    hoverTarget.value = null
    dragging.value = null
  })

  await listen<{ x: number; y: number; payload: DragPayload }>(
    'xhy://drag-drop',
    async (e) => {
      // 载荷取 Rust 广播的那份，而不是本地 dragging：
      // 跨窗口投放时，本窗口并没有起拖，本地 dragging 是 null。
      const payload = e.payload?.payload
      dragging.value = null
      hoverTarget.value = null
      if (!payload) {
        log('drag-drop 收到但 payload 为空，跳过')
        return
      }
      const local = await toLocal(e.payload.x, e.payload.y)
      if (!local) {
        // 位置未知：宁可不投，也不投到猜出来的地方
        log(`drag-drop 放弃：拿不到自身几何 (全局 ${e.payload.x},${e.payload.y})`)
        return
      }
      const id = topmostHit(local.x, local.y)
      log(
        `drag-drop 全局(${e.payload.x},${e.payload.y}) → 本地(${local.x.toFixed(0)},${local.y.toFixed(0)}) ` +
          `命中=${id ?? '无'} 已注册落点=[${order.join(',')}] kind=${payload.kind}`,
      )
      if (id) targets.get(id)?.onDrop(payload, local)
    },
  )
}

/** 起拖。调用方负责"移动超阈值"判定，避免点击被当成拖拽 */
export async function beginDrag(payload: DragPayload) {
  await bindDrag()
  dragging.value = payload
  try {
    await invoke('drag_begin', { label: payload.label, payload })
    log(`beginDrag ok kind=${payload.kind} id=${payload.id}`)
  } catch (e) {
    // 此前这里静默吞错：用户只看到"拖不动"，一点线索都没有
    log(`beginDrag FAILED: ${String(e)}`)
    dragging.value = null
  }
}

export async function cancelDrag() {
  if (!dragging.value) return
  dragging.value = null
  hoverTarget.value = null
  try {
    await invoke('drag_cancel')
  } catch {
    /* 忽略 */
  }
}

/** 元素命中判定工厂（CSS 坐标） */
export function rectHit(el: HTMLElement, pad = 0) {
  return (x: number, y: number) => {
    const r = el.getBoundingClientRect()
    return x >= r.left - pad && x <= r.right + pad && y >= r.top - pad && y <= r.bottom + pad
  }
}

/** 用元素矩形 + 全局落点表，判定某 CSS 点是否落在该元素上 */
export function hitEl(el: HTMLElement, x: number, y: number) {
  const r = el.getBoundingClientRect()
  return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

/**
 * 指针起拖助手：移动超过阈值才真正起拖。
 * 统一给行/标签/日程复用，避免每处重写一遍监听器管理。
 */
export function armDrag(payload: () => DragPayload, threshold = 6) {
  return (e: PointerEvent) => {
    if (e.button !== 0) return
    const t = e.target as HTMLElement
    if (t.closest('button, input, textarea, select, .xh-nodrag')) {
      // 起点落在按钮/输入框上就不起拖。用户如果总是从文字旁边的
      // 标签/徽章上按下，就会表现为"拖不动"，这里必须留痕。
      log(`armDrag 放弃：起点落在可交互元素上 <${t.tagName.toLowerCase()}>`)
      return
    }
    const { clientX: x, clientY: y } = e
    let moved = false
    const cleanup = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    const onMove = (m: PointerEvent) => {
      if (moved) return
      if (Math.abs(m.clientX - x) + Math.abs(m.clientY - y) > threshold) {
        moved = true
        cleanup()
        log(`armDrag 超阈值起拖 (${x},${y})→(${m.clientX},${m.clientY})`)
        void beginDrag(payload())
      }
    }
    const onUp = () => cleanup()
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }
}

/** 幽灵窗（label 为 drag-ghost）读取自己要显示的文本 */
export async function fetchGhostText(): Promise<{ label: string; dragging: boolean }> {
  try {
    return await invoke('ghost_text')
  } catch {
    return { label: '', dragging: false }
  }
}
