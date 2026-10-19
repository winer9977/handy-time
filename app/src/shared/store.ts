/** 全局 store：后端快照的只读镜像 + 本地 UI 态（拖拽、编辑焦点）
 *
 * 原则：写操作直接调 api 然后等 `xhy://changed` 事件刷新，
 * 不做乐观更新 —— 单进程 IPC 延迟 <1ms，乐观更新带来的双份逻辑不值得。
 */
import { defineStore } from 'pinia'
import { api, onChanged, onEvent } from '../lib/api'
import { emptyStore, type CalendarView, type Snapshot, type Store, type Todo } from '../lib/types'
import { todayStr } from '../shared/date'
import { inArchivedList } from '../shared/scope'

/** 优先级排序权重，与 Rust Priority::rank() 同口径（改一处要改两处） */
const PR_RANK: Record<string, number> = { high: 0, mid: 1, low: 2, none: 3 }

export const useDataStore = defineStore('data', {
  state: () => ({
    store: emptyStore() as Store,
    hiddenAll: false,
    calendarView: 'month' as CalendarView,
    calendarFocus: todayStr(),
    ready: false,
    lastError: '',
    /** 一次性操作的结果文案（导入 Markdown 的摘要）。与 lastError 分开：
     *  那是 2.6 秒就消失的错误条，成功摘要要留久一点才看得见。 */
    notice: '',
    // 这里曾有一个 `flashing: string[]`（注释写"remind 事件触发，2s 后清除"），
    // 但**没有任何地方写过它、也没有任何地方读它** —— 提醒事件走的是
    // load() 重取快照，行上的 🔔 由 `remindAt && !reminded` 判据画（TodoRow）。
    // 留着等于对外承诺一个不存在的"闪烁"能力，后人会照着它写代码。
  }),
  getters: {
    settings: (s) => s.store.settings,
    /** 「今天」（墙上日期 `YYYY-MM-DD`）。
     *
     *  为什么做成 getter 而不是各组件 `const today = todayStr()`：
     *  这套工具是**常驻**的（挂件贴桌面、工作台开着不动），而 setup 里算
     *  出来的常量跨零点就不再变 —— 于是看板的「今天/逾期」两列、智能清单
     *  「今天/本周」、时间线起点、倒数文案全都停在昨天，且没有任何提示。
     *  读 `s.store` 是为了建立依赖：后端跨点会广播一次 `xhy://changed`
     *  （reminder.rs 的 new_day 分支），load() 整体换掉 store 对象 →
     *  这里跟着重算，各视图自动跟上。 */
    today: (s) => {
      void s.store
      return todayStr()
    },
    /** 全部清单（含已归档）——只给需要展示归档区的 UI 用（主窗口侧栏） */
    lists: (s) => [...s.store.lists].sort((a, b) => a.order - b.order),
    /** **活动清单**（不含归档）：面向"用户要往里放东西"的界面用 ——
     *  快速添加的清单下拉、挂件清单标签页、新建待办的落点。
     *  漏了这条会让归档清单仍出现在这些地方，用户能往一个侧栏看不见的
     *  清单里加待办（归档等于没做）。 */
    activeLists: (s) =>
      s.store.lists.filter((l) => !l.archived).sort((a, b) => a.order - b.order),
    widgets: (s) => s.store.widgets,
    /** 未归档、未完成，按置顶→优先级→order→到期排序（后端同款口径的前端兜底）
     *
     *  「未归档」这条以前只是注释，代码里没滤 —— 于是聚合视图（挂件点「全」）
     *  照样混着归档清单的待办，而**后端 `todos_of(None)` 与主窗口 scope.ts
     *  都排除了**。三处口径必须一致，否则同一份数据在挂件/工作台/日历里
     *  各自给出不同结果（归档形同虚设那类 bug 的复发路径）。
     *  判据收在 shared/scope.ts 的 `inArchivedList`，这里只做调用。
     */
    todosOf: (s) => (listId: string | null): Todo[] => {
      const today = todayStr()
      return s.store.todos
        .filter((t) => !t.done)
        .filter((t) =>
          // 显式点名某清单（挂件绑定了它）时照给，哪怕它已归档 ——
          // 与后端 `explicit` 判据同一条规则
          listId == null ? !inArchivedList(s.store, t) : t.listId === listId,
        )
        .sort((a, b) => {
          if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
          // 优先级：高→低→无。与 Rust todos_of 同口径，改一处要改两处
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
    },
    // 注意：这里**不提供** eventsOn(date)。按日期取日程必须走
    // api.eventsInRange（后端展开重复日程）。在前端过滤 store.events
    // 会静默漏掉所有重复日程的 occurrence —— 这个 getter 曾经就埋了这个坑。
    // 置顶便签恒排最前，其余按手动顺序
    notes: (s) =>
      [...s.store.notes].sort((a, b) => {
        if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
        return a.order - b.order
      }),
  },
  actions: {
    async load() {
      try {
        const snap: Snapshot = await api.snapshot()
        this.store = snap.store
        this.hiddenAll = snap.hiddenAll
        this.calendarView = snap.calendarView
        this.calendarFocus = snap.calendarFocus
        this.ready = true
      } catch (e) {
        this.lastError = String(e)
      }
    },
    async bind() {
      await onChanged(() => this.load())
      await onEvent<string>('layout', () => this.load())
      await onEvent<string>('remind', () => this.load())
      await onEvent<boolean>('hidden', (v) => { this.hiddenAll = v })
      // 导入 Markdown 的摘要（成功/失败都走这一条，见 widgets::import_md）
      await onEvent<string>('md-import', (m) => this.notify(m))
    },
    notify(msg: string) {
      this.notice = msg
      setTimeout(() => { if (this.notice === msg) this.notice = '' }, 20_000)
    },
    fail(e: unknown) {
      this.lastError = String(e)
      setTimeout(() => (this.lastError = ''), 2600)
    },
  },
})
