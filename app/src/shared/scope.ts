/** 主窗口侧栏的"作用域"模型（对照滴答：智能清单 / 清单 / 标签 / 已完成 / 垃圾桶）
 *
 *  为什么抽成纯函数：作用域决定列表/看板/时间线/日历/总结五处看到什么，
 *  散在模板里必然各写一遍且口径不一（用户报的"选空清单却看到收集箱内容"
 *  就是日历/总结压根没这个维度）。规则收在这里，一处改处处生效。
 */
import type { Store, Todo } from '../lib/types'

/** 优先级排序权重，与 Rust Priority::rank() 同口径（改一处要改两处） */
const PR_RANK: Record<string, number> = { high: 0, mid: 1, low: 2, none: 3 }

/** "哪些清单不计入总结"的签名。
 *
 *  用途：总结页与分析页用它做 `watch` 依赖 —— 这个签名一变就说明**统计口径
 *  变了**，必须重新请求。
 *
 *  为什么需要它（用户报的真 bug）：两个面板原先只监听 `kind`/`listId`，
 *  而"不计入总结"改的是 `lists` 里的字段，两者都不变 → 用户勾选后数字
 *  停在旧值，表现为"点了没反应"。
 *
 *  为什么不直接把整个 lists 数组当依赖：那样任何改名/换色/排序都会触发
 *  一次无谓的后端请求。只摘要"排除与否"这一位信息，精准且省请求。
 */
export function excludedStatsSig(lists: { excludeFromStats?: boolean }[]): string {
  return lists.map((l) => (l.excludeFromStats ? '1' : '0')).join('')
}

/** 置顶→优先级→order→到期（逾期在前）。
 *  从 store.todosOf 抽到这里：作用域视图也要同一排序，再抄一遍就是
 *  第三份实现（Rust + store + 这里），必然漂移。 */
export function sortTodos(list: Todo[], today: string): Todo[] {
  return [...list].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
    const pr = PR_RANK[a.priority ?? 'none'] - PR_RANK[b.priority ?? 'none']
    if (pr !== 0) return pr
    if (a.order !== b.order) return a.order - b.order
    const ka = a.due ?? '9999', kb = b.due ?? '9999'
    if (ka !== kb) {
      const oa = ka <= today ? `0${ka}` : `1${ka}`
      const ob = kb <= today ? `0${kb}` : `1${kb}`
      return oa < ob ? -1 : 1
    }
    return 0
  })
}

export type Scope =
  | { k: 'all' }
  /** 智能清单：口径与挂件 filter 完全一致（同一套语义不能两套实现） */
  | { k: 'smart'; f: 'today' | 'week' | 'flagged' | 'someday' }
  | { k: 'list'; id: string }
  | { k: 'tag'; name: string }
  | { k: 'done' }
  | { k: 'trash' }

/** 到期在今天或已逾期 */
function dueToday(t: Todo, today: string): boolean {
  return !!t.due && t.due <= today
}
/** 到期在本周内（weekEnd = 本周日） */
function dueThisWeek(t: Todo, weekEnd: string): boolean {
  return !!t.due && t.due <= weekEnd
}

/** 未完成待办在作用域内的归属。trash 不看这里（数据在另一个数组）。 */
export function matchScope(t: Todo, s: Scope, today: string, weekEnd: string): boolean {
  switch (s.k) {
    case 'all':
      return true
    case 'list':
      return t.listId === s.id
    case 'tag':
      return t.tags.includes(s.name)
    case 'done':
      return false // 已完成走 doneTodos
    case 'trash':
      return false // 回收站走 store.trashed
    case 'smart':
      switch (s.f) {
        case 'today':
          return dueToday(t, today)
        case 'week':
          return dueThisWeek(t, weekEnd)
        case 'flagged':
          return !!t.priority
        case 'someday':
          return !t.due
      }
  }
}

/** 该待办是否属于某个已归档的清单 */
export function inArchivedList(store: Store, t: Todo): boolean {
  return store.lists.some((l) => l.id === t.listId && l.archived)
}

/** 清单维度：这条待办在该作用域下是否落在"可见范围"里。
 *
 *  归档清单的处理只在这里定义一次：
 *    · `{k:'list', id}` 是**显式点名**（用户从「已归档」区点进来就是要看它）
 *      → 即便清单已归档也照样显示，且只显示它的；
 *    · 其余都是全局类作用域（全部 / 智能清单 / 标签 / 已完成）→ 一律排除
 *      归档清单的待办，否则归档形同虚设。
 *
 *  ★ 列表视图、看板、时间线、侧栏徽标数**必须**共用这条判据。漏一处就会出现
 *  "顶栏写 3 项未完成、看板却摆着 5 张卡"这类同屏自相矛盾（本项目同类 bug
 *  已修过多次：日历/总结串清单、分析页趋势柱与分布不同口径）。
 */
export function scopeAllowsList(store: Store, t: Todo, s: Scope): boolean {
  if (s.k === 'list') return t.listId === s.id
  return !inArchivedList(store, t)
}

/** 作用域内的未完成待办（排序与 store.todosOf 同一实现）。
 *
 *  归档清单的处理在 `scopeAllowsList`（唯一一份定义）。 */
export function scopeTodos(store: Store, s: Scope, today: string, weekEnd: string): Todo[] {
  if (s.k === 'trash') return []
  return sortTodos(
    store.todos.filter(
      (t) => scopeAllowsList(store, t, s) && !t.done && matchScope(t, s, today, weekEnd),
    ),
    today,
  )
}

/** 作用域内的已完成待办（done 视图与"已完成"计数用）*/
export function scopeDone(store: Store, s: Scope, today: string, weekEnd: string): Todo[] {
  return store.todos
    .filter((t) => {
      if (!scopeAllowsList(store, t, s)) return false
      return t.done && (s.k === 'done' ? true : matchScope(t, s, today, weekEnd))
    })
    .sort((a, b) => (b.doneAt ?? '').localeCompare(a.doneAt ?? ''))
}

/** 侧栏徽标数：各作用域显示什么数（滴答在智能清单/清单后都带未完成数）
 *
 *  ★ done 走 `scopeDone` 而不是裸 `todos.filter(done)`：徽标数必须等于点进去
 *  看到的条数。原先这里算的是"含已归档清单的全部已完成"，而列表视图按作用域
 *  规则排除了归档清单的 → 徽标 12、列表 9，用户以为搜索/筛选坏了。 */
export function scopeCount(store: Store, s: Scope, today: string, weekEnd: string): number {
  switch (s.k) {
    case 'done':
      return scopeDone(store, s, today, weekEnd).length
    case 'trash':
      return store.trashed.length
    default:
      return scopeTodos(store, s, today, weekEnd).length
  }
}

/** 垃圾桶条目的作用域过滤（按待办原清单/标签归位）*/
export function scopeTrashed(store: Store, s: Scope, today: string, weekEnd: string) {
  if (s.k !== 'trash') return []
  return store.trashed.filter((x) => matchScope(x.todo, { k: 'all' }, today, weekEnd))
}

/** 作用域的标题（列表顶栏用）*/
export function scopeTitle(store: Store, s: Scope): string {
  switch (s.k) {
    case 'all':
      return '全部清单'
    case 'list':
      return store.lists.find((l) => l.id === s.id)?.name ?? '（已删清单）'
    case 'tag':
      return `#${s.name}`
    case 'done':
      return '已完成'
    case 'trash':
      return '垃圾桶'
    case 'smart':
      // 文案必须描述**判据本身**：week 的判据是"到期 ≤ 本周日（含逾期）"
      // （见 matchScope 与测试「smart week：到期≤本周日」），不是滚动 7 天。
      // 原来叫"最近7天"，周一时两者恰好等价、周二起就不等 —— 周二点进去
      // 只看到 5 天的东西却写着 7 天，用户以为漏了。挂件同一筛选项叫"本周"，
      // 两处叫法也不一样，所以统一成"本周"。
      // 「待定」→「未排期」：与看板列同一个判据同一个名字（快速添加现在默认今天，
      // 侧栏与挂件都撤掉了这一档的入口，只剩持久化状态还可能指到它）。
      return { today: '今天', week: '本周', flagged: '旗标', someday: '未排期' }[s.f]
  }
}

/** 统计面板的"该不该重取数"签名（总结页与分析页共用一份）。
 *
 *  为什么需要它：这两个面板的数字来自后端 `stats::compute`，前端只能主动
 *  重取。依赖写成 `todos.length` 是**不够的** —— 勾选完成不改条数（只有重复
 *  待办会 +1），于是"在挂件里勾掉一条"后，工作台分析页的柱子仍停在旧值，
 *  而当初那句注释想要的恰恰是这个场景。
 *
 *  为什么不做整表 hash：挂件每 600ms 存一次便签也会广播变更，而便签根本
 *  不进统计（见 stats.rs 的说明）。只把**影响 stats 的那几个字段**压成
 *  数字，既抓得住完成/改期/换清单，又不会被无关写操作刷屏。
 */
export function statsDataSig(store: Store): string {
  let done = 0
  let acc = 0
  for (const t of store.todos) {
    if (t.done) done++
    // 标题也进签名：摘要里的「遗留：xxx、yyy」显示的是标题，
    // 不改就等于"改了名字，总结还写着旧名"
    acc += charSum(t.due ?? '') + charSum(t.doneAt ?? '') + charSum(t.listId) + charSum(t.title)
  }
  for (const e of store.events) acc += charSum(e.date) + charSum(e.listId ?? '')
  return `${store.todos.length}:${done}:${acc}:${store.events.length}`
}

/** 把字符串压成一个数（够判"变没变"，不需要加密强度） */
function charSum(s: string): number {
  let n = 0
  for (let i = 0; i < s.length; i++) n = (n + s.charCodeAt(i) * (i + 1)) % 1_000_003
  return n
}
