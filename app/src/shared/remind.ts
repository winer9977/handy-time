/** 提醒时点跟随到期日平移的规则（前端侧）
 *
 *  为什么单独抽文件：这条规则有**两份实现** —— 后端
 *  `domain::recurring::shift_reminders_to`（Rust，覆盖"日历拖派生条目"、
 *  "拖待办进日历"、"重复生成下一条"三条路径）与前端本模块（覆盖所有
 *  **由界面改到期日**的入口：⋯ 面板的日期框、看板拖列、时间线拖日）。
 *  进程边界让它们无法共用代码，但**语义必须一致**。
 *
 *  ★ 前端入口一律走 `duePatch` / `withDueShift` 组装补丁，不要在调用点
 *  手写那 6 个字段 —— 漏一个就又是一处"改了日期提醒没跟着走"。
 *
 *  抽到这里而不是写在 .vue 里，是为了让测试 import 的就是**出货代码本身**
 *  —— 测试若自己复制一份算法，那只是"两份实现互相印证"，抓不到漂移。
 */
import type { TodoPatch } from '../lib/types'

/** 把一个提醒时点串换到新日期。
 *
 *  语义与后端 `shift_reminders_to` 逐字对应：
 *    · 带 `HH:MM`（`YYYY-MM-DDTHH:MM`）→ 只换日期、**时刻保留**；
 *    · 纯日期（`YYYY-MM-DD`）→ 直接换成新日期。
 *      注意不能用 `includes('T')` 之类自造判断：后端 `reminder::parse_min`
 *      明确支持纯日期格式，两条路径必须同样处理（实测漏过一次）。
 */
export function shiftRemindTo(full: string, newDate: string): string {
  const i = full.indexOf('T')
  return i >= 0 ? `${newDate}${full.slice(i)}` : newDate
}

/** 主提醒平移到新日期时取用的时刻。
 *
 *  优先用**用户原本设的时刻**（从旧 `remindAt` 里取 HH:MM），取不到才退回
 *  面板当前值。为什么不能直接用面板值：用户可能从没打开过 ⋯ 面板，此时面板
 *  的 `remindTime` 是默认 `09:00` —— 拿它去覆盖会把用户设的时间悄悄改掉
 *  （"我只是改了到期日，提醒时间怎么变成 9 点了"）。
 */
export function shiftMainRemind(oldRemindAt: string | null, newDate: string, panelTime: string): string | null {
  if (!oldRemindAt) return null
  const i = oldRemindAt.indexOf('T')
  const hhmm = i >= 0 ? oldRemindAt.slice(i + 1) : panelTime
  return `${newDate}T${hhmm}`
}

/** 待办里与"改到期日"有关的字段（结构化，不绑死 Todo 类型，测试好造数据） */
export interface DueShiftSource {
  remindAt: string | null
  extraReminds: string[]
  remindedKeys: string[]
  reminded: boolean
  lastRemindedAt: string | null
}

/** 改到期日要**一起**提交的字段 —— 前端唯一的组装口。
 *
 *  为什么必须成组发：只发 `{due}` 的话提醒仍指着旧那天 ——
 *    · 旧那天已过 → `reminded` 还是 true，新到期日上**永远不再提醒**；
 *    · 旧那天未到 → 到点仍按**旧日期**响，用户以为改期没生效。
 *  后端那份同名规则是 `recurring::shift_reminders_to`（进程边界所迫，无法共用
 *  代码）。本项目已因"改了到期日却没平移提醒"修过 4 处（UI 日期框 / 拖派生
 *  条目 / 拖进日历格 / 重复生成下一条），每补一处就多暴露一个没覆盖的入口。
 *  所以字段组装收在这一个函数里：任何改期入口（含**看板拖列、时间线拖日**）
 *  都从这里取补丁，不再各写一遍。
 *
 * `newDate = null`（清掉到期日）→ 只清 due、**不动提醒**：提醒是独立设置，
 * 删掉到期日不代表想删提醒（与后端同口径）。
 */
export function duePatch(
  t: DueShiftSource,
  newDate: string | null,
  panelTime = '09:00',
): TodoPatch {
  if (!newDate) return { due: null }
  return {
    due: newDate,
    // 没有提醒时 shiftMainRemind 返回 null：与"清空"同义（本来就是空），安全
    remindAt: shiftMainRemind(t.remindAt, newDate, panelTime),
    extraReminds: t.extraReminds.map((x) => shiftRemindTo(x, newDate)),
    // 时点整体平移后，旧的绝对 key 已指向不存在的时刻。留着会让同名
    // 新时点被误判"已弹过"而永不触发 —— 改日期等于重新排期，清空重来。
    remindedKeys: [],
    // 同理：主提醒触发位与持续提醒节流基准也跟着重置，
    // 否则改完到期日，新时点因为"已提醒过"永远不响
    reminded: false,
    lastRemindedAt: null,
  }
}

/** 给"落点补丁"（看板拖列 / 时间线拖日返回的 `{due?, done?}`）补上提醒平移。
 *
 *  `due` 整个缺省 = 这次没改到期日（例如只拖进"已完成"列）→ 原样返回；
 *  `due = null` = 清到期日 → 只清 due，提醒留着。
 *
 *  返回类型写 `TodoPatch` 而不是泛型 T：合并后对象**确实**多了 remindAt 等
 *  字段，返回 T 会把这些字段对调用方藏起来（TS 侧看不见 ≠ 运行时没有）。
 */
export function withDueShift(t: DueShiftSource, p: TodoPatch, panelTime?: string): TodoPatch {
  if (p.due === undefined) return p
  return { ...p, ...duePatch(t, p.due, panelTime) }
}
