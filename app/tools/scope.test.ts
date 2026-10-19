/** 主窗口侧栏作用域规则测试（shared/scope.ts）*/
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  matchScope,
  scopeAllowsList,
  scopeTodos,
  scopeDone,
  scopeCount,
  scopeTitle,
  scopeTrashed,
  excludedStatsSig,
  statsDataSig,
  type Scope,
} from '../src/shared/scope'
import type { Store, Todo, TrashedTodo } from '../src/lib/types'

const T = '2026-10-17' // 周六
const WE = '2026-10-18' // 本周日
const mk = (o: Partial<Todo> = {}): Todo =>
  ({
    id: o.id ?? 'x',
    listId: o.listId ?? 'inbox',
    title: o.title ?? 't',
    tags: o.tags ?? [],
    done: o.done ?? false,
    due: o.due ?? null,
    priority: o.priority ?? null,
    ...o,
  }) as Todo

const store = (todos: Todo[], trashed: TrashedTodo[] = [], archivedIds: string[] = []): Store =>
  ({
    lists: [
      { id: 'inbox', name: '收集箱', color: '#000', order: 0, collapsed: false, archived: false },
      { id: 'work', name: '待办', color: '#00f', order: 1, collapsed: false, archived: false },
      { id: 'old', name: '旧项目', color: '#888', order: 2, collapsed: false, archived: archivedIds.includes('old') },
    ],
    // events 也要给：真实快照里它永远存在（后端 Store 的必填字段），
    // 测试数据缺字段会让"读 events 的规则函数"在这里假崩、在真机上没人发现
    todos,
    events: [],
    trashed,
  }) as unknown as Store

test('all：全部未完成', () => {
  const s = store([mk({ id: '1' }), mk({ id: '2', done: true })])
  assert.deepEqual(scopeTodos(s, { k: 'all' }, T, WE).map((t) => t.id), ['1'])
})

test('归档清单的待办：不出现在「全部」里（否则归档形同虚设）', () => {
  const s = store([mk({ id: 'live', listId: 'inbox' }), mk({ id: 'arch', listId: 'old' })], [], ['old'])
  assert.deepEqual(scopeTodos(s, { k: 'all' }, T, WE).map((t) => t.id), ['live'])
})

test('归档清单的待办：不出现在智能清单与标签里', () => {
  const s = store([mk({ id: 'arch', listId: 'old', due: T, tags: ['工作'] })], [], ['old'])
  assert.deepEqual(scopeTodos(s, { k: 'smart', f: 'today' }, T, WE), [], '智能清单里也不该有')
  assert.deepEqual(scopeTodos(s, { k: 'tag', name: '工作' }, T, WE), [], '标签视图里也不该有')
})

test('归档清单：显式点它仍能看到（用户从「已归档」区点进来就是要看它）', () => {
  const s = store([mk({ id: 'arch', listId: 'old' })], [], ['old'])
  assert.deepEqual(
    scopeTodos(s, { k: 'list', id: 'old' }, T, WE).map((t) => t.id),
    ['arch'],
  )
})

test('归档清单的已完成项：不出现在「已完成」视图', () => {
  const s = store([mk({ id: 'a', listId: 'old', done: true, doneAt: '2026-10-10T00:00' })], [], ['old'])
  assert.deepEqual(scopeDone(s, { k: 'done' }, T, WE), [])
})

test('未归档清单一切照旧（避免误伤）', () => {
  const s = store([mk({ id: '1', listId: 'old' })], [], [])
  assert.deepEqual(scopeTodos(s, { k: 'all' }, T, WE).map((t) => t.id), ['1'])
  assert.equal(scopeCount(s, { k: 'all' }, T, WE), 1)
})

test('list：只认 listId', () => {
  const s = store([mk({ id: '1', listId: 'inbox' }), mk({ id: '2', listId: 'work' })])
  const sc: Scope = { k: 'list', id: 'work' }
  assert.deepEqual(scopeTodos(s, sc, T, WE).map((t) => t.id), ['2'])
})

test('smart today：到期≤今天（含逾期）', () => {
  const s = store([
    mk({ id: 'over', due: '2026-10-10' }),
    mk({ id: 'today', due: T }),
    mk({ id: 'future', due: '2026-10-20' }),
    mk({ id: 'none', due: null }),
  ])
  assert.deepEqual(scopeTodos(s, { k: 'smart', f: 'today' }, T, WE).map((t) => t.id), ['over', 'today'])
})

test('smart week：到期≤本周日', () => {
  const s = store([mk({ id: 'in', due: WE }), mk({ id: 'out', due: '2026-10-19' })])
  assert.deepEqual(scopeTodos(s, { k: 'smart', f: 'week' }, T, WE).map((t) => t.id), ['in'])
})

test('smart flagged：有优先级', () => {
  const s = store([mk({ id: 'hi', priority: 'high' }), mk({ id: 'no', priority: null })])
  assert.deepEqual(scopeTodos(s, { k: 'smart', f: 'flagged' }, T, WE).map((t) => t.id), ['hi'])
})

test('smart someday：无到期', () => {
  const s = store([mk({ id: 'd', due: T }), mk({ id: 'n', due: null })])
  assert.deepEqual(scopeTodos(s, { k: 'smart', f: 'someday' }, T, WE).map((t) => t.id), ['n'])
})

test('tag：按标签命中', () => {
  const s = store([mk({ id: '1', tags: ['工作', '急'] }), mk({ id: '2', tags: ['私'] })])
  assert.deepEqual(scopeTodos(s, { k: 'tag', name: '工作' }, T, WE).map((t) => t.id), ['1'])
})

test('done 视图不混进未完成，未完成不进 done', () => {
  const s = store([mk({ id: 'a', done: true }), mk({ id: 'b', done: false })])
  assert.deepEqual(scopeTodos(s, { k: 'done' }, T, WE).map((t) => t.id), [])
  assert.deepEqual(scopeDone(s, { k: 'done' }, T, WE).map((t) => t.id), ['a'])
})

test('done 按完成时间倒序', () => {
  const s = store([
    mk({ id: 'old', done: true, doneAt: '2026-10-01T00:00' }),
    mk({ id: 'new', done: true, doneAt: '2026-10-17T00:00' }),
  ])
  assert.deepEqual(scopeDone(s, { k: 'done' }, T, WE).map((t) => t.id), ['new', 'old'])
})

test('trash 视图：未完成恒空，条目走 scopeTrashed', () => {
  const trashed: TrashedTodo[] = [
    { todo: mk({ id: 't1' }), trashedAt: '2026-10-17T10:00' },
  ]
  const s = store([mk({ id: 'live' })], trashed)
  assert.deepEqual(scopeTodos(s, { k: 'trash' }, T, WE), [])
  assert.deepEqual(scopeTrashed(s, { k: 'trash' }, T, WE).map((x) => x.todo.id), ['t1'])
  assert.deepEqual(scopeTrashed(s, { k: 'all' }, T, WE), [], '非 trash 视图不该返回回收站条目')
})

test('scopeCount：done=全部完成数，trash=回收站数，其余=未完成数', () => {
  const s = store([mk({ id: '1' }), mk({ id: '2', done: true })], [
    { todo: mk({ id: 't' }), trashedAt: '' },
  ])
  assert.equal(scopeCount(s, { k: 'all' }, T, WE), 1)
  assert.equal(scopeCount(s, { k: 'done' }, T, WE), 1)
  assert.equal(scopeCount(s, { k: 'trash' }, T, WE), 1)
})

test('scopeTitle：清单名/标签/智能清单中文，删掉的清单有兜底', () => {
  const s = store([])
  assert.equal(scopeTitle(s, { k: 'all' }), '全部清单')
  assert.equal(scopeTitle(s, { k: 'list', id: 'work' }), '待办')
  assert.equal(scopeTitle(s, { k: 'list', id: 'gone' }), '（已删清单）')
  assert.equal(scopeTitle(s, { k: 'tag', name: '工作' }), '#工作')
  assert.equal(scopeTitle(s, { k: 'smart', f: 'week' }), '本周')
  assert.equal(scopeTitle(s, { k: 'trash' }), '垃圾桶')
})

test('matchScope 对 done/trash 返回 false（未完成口径下不参与）', () => {
  assert.equal(matchScope(mk(), { k: 'done' }, T, WE), false)
  assert.equal(matchScope(mk(), { k: 'trash' }, T, WE), false)
})

test('excludedStatsSig：签名必须反映"不计入总结"的变化', () => {
  // 这个签名是总结页/分析页的 watch 依赖。用户勾选「不计入总结」后数字没反应，
  // 正因为原来没把它放进依赖（用户报的真 bug）—— 这里锁住语义：
  // 排除状态一变，签名必须跟着变，否则 watch 不会触发重算。
  const none = [{ excludeFromStats: false }, { excludeFromStats: false }]
  const one = [{ excludeFromStats: true }, { excludeFromStats: false }]
  const both = [{ excludeFromStats: true }, { excludeFromStats: true }]
  assert.notEqual(excludedStatsSig(none), excludedStatsSig(one), '排除变化必须改变签名')
  assert.notEqual(excludedStatsSig(one), excludedStatsSig(both))
  assert.equal(excludedStatsSig(none), excludedStatsSig([...none]), '相同状态签名须相同（避免无谓重算）')
  // 签名里 1 的个数 == 被排除的清单数（便于用断言表达"排除了几个"）
  const count1 = (s: string) => s.split('1').length - 1
  assert.equal(count1(excludedStatsSig(none)), 0)
  assert.equal(count1(excludedStatsSig(one)), 1)
  assert.equal(count1(excludedStatsSig(both)), 2)
  assert.equal(excludedStatsSig([]), '', '没有清单时是空签名')
})

test('excludedStatsSig：改名/换色不该改变签名（避免无谓后端请求）', () => {
  const before = [{ excludeFromStats: false, name: '收集箱', color: '#f5c518' }]
  const after = [{ excludeFromStats: false, name: '改过的名字', color: '#000000' }]
  assert.equal(excludedStatsSig(before), excludedStatsSig(after))
})


// ── 归档口径：徽标数必须等于点进去看到的行数 ──────────────

test('★ scopeCount(已完成) == 已完成视图的行数（含归档排除规则）', () => {
  // 以前徽标是裸 `todos.filter(t => t.done).length`（**含**归档清单），
  // 而列表走 scopeDone（排除归档清单）→ 侧栏写「2 项已完成」、点进去只有 1 行。
  // 同一屏两个数字互相矛盾，比数字不准更伤信任。
  const s = store(
    [
      mk({ id: 'd1', done: true, doneAt: '2026-10-10T00:00' }),
      mk({ id: 'd2', listId: 'old', done: true, doneAt: '2026-10-11T00:00' }),
    ],
    [],
    ['old'],
  )
  assert.equal(scopeDone(s, { k: 'done' }, T, WE).length, 1, '列表只给未归档清单的已完成')
  assert.equal(
    scopeCount(s, { k: 'done' }, T, WE),
    scopeDone(s, { k: 'done' }, T, WE).length,
    '徽标数须与列表同口径',
  )
})

test('scopeAllowsList：显式点名归档清单可见，其余作用域一律排除', () => {
  const s = store([mk({ id: 'arch', listId: 'old' })], [], ['old'])
  const t = s.todos[0]!
  assert.equal(scopeAllowsList(s, t, { k: 'list', id: 'old' }), true, '点名要看它就给看')
  assert.equal(scopeAllowsList(s, t, { k: 'all' }), false)
  assert.equal(scopeAllowsList(s, t, { k: 'smart', f: 'today' }), false)
  assert.equal(scopeAllowsList(s, t, { k: 'tag', name: '工作' }), false)
  assert.equal(scopeAllowsList(s, t, { k: 'done' }), false)
  // 看 a 清单时不该把归档清单的东西混进来（归属判断也走同一条）
  assert.equal(scopeAllowsList(s, t, { k: 'list', id: 'inbox' }), false)
})

// ── 统计面板的"该不该重取数"签名 ────────────────────────

test('★ statsDataSig：勾掉一项必须改签名（todos.length 抓不到这件事）', () => {
  // 分析页原先的依赖是 [todos.length, events.length]，而**完成一项不改条数**
  // —— 注释写着"别的窗口完成一项，柱不该停在旧值"，实际从没生效过。
  const a = store([mk({ id: 'a' }), mk({ id: 'b' })])
  const b = store([mk({ id: 'a', done: true }), mk({ id: 'b' })])
  assert.equal(a.todos.length, b.todos.length, '前置：两条的条数相同')
  assert.equal(statsDataSig(a), statsDataSig(a), '同一份数据签名须稳定（避免无谓后端请求）')
  assert.notEqual(statsDataSig(a), statsDataSig(b), '完成态变了 → 签名必须变')
})

test('statsDataSig：改期 / 换清单 / 改标题都变；动别的字段不变', () => {
  const base = store([mk({ id: 'a', due: '2026-10-20' })])
  assert.notEqual(
    statsDataSig(base),
    statsDataSig(store([mk({ id: 'a', due: '2026-10-21' })])),
    '到期日影响"遗留"计数',
  )
  assert.notEqual(
    statsDataSig(base),
    statsDataSig(store([mk({ id: 'a', due: '2026-10-20', listId: 'work' })])),
    '换清单影响"按清单分布"',
  )
  assert.notEqual(
    statsDataSig(base),
    statsDataSig(store([mk({ id: 'a', due: '2026-10-20', title: '改了名字' })])),
    '摘要里"遗留：xxx"显示的是标题，改名得重算',
  )
})

test('statsDataSig：日程的日期/归属参与签名', () => {
  const mkStore = (date: string, listId: string | null): Store => {
    const s = store([]) as Store
    s.events = [{ id: 'e', date, listId } as never]
    return s
  }
  assert.notEqual(statsDataSig(mkStore('2026-10-20', null)), statsDataSig(mkStore('2026-10-21', null)))
  assert.notEqual(statsDataSig(mkStore('2026-10-20', 'work')), statsDataSig(mkStore('2026-10-20', 'inbox')))
  assert.equal(statsDataSig(mkStore('2026-10-20', 'work')), statsDataSig(mkStore('2026-10-20', 'work')))
})
