//! 状态层集成测试：端到端跑通"数据 → 规则 → 落盘 → 重载"
//!
//! 覆盖目标里最难自动验证的几条：重复规则自动生成、按完成日归档、
//! 拖待办生成日程并回写 due、导入导出往返、备份轮转。

use xhy_lib::domain::{CalendarView, Countdown, CountdownMode, Event, Freq, Recurrence, RectL, Todo, TodoPatch, Widget, WidgetKind, WidgetSize};
use xhy_lib::state::AppState;

fn tmp_app() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::open(dir.path().to_path_buf()).unwrap();
    (st, dir)
}

fn first_list(st: &AppState) -> String {
    st.snapshot().lists[0].id.clone()
}

#[test]
fn seeds_default_list_and_persists() {
    let (st, dir) = tmp_app();
    assert_eq!(st.snapshot().lists.len(), 1);
    assert_eq!(st.snapshot().lists[0].name, "收集箱");
    // 落盘了
    assert!(dir.path().join("store.json").exists());
}

#[test]
fn quickparse_fills_tags_and_due_on_add() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "[工作] 8月1日 交周报".into()).unwrap();
    assert_eq!(t.title, "交周报");
    assert_eq!(t.tags, vec!["工作"]);
    assert!(t.due.is_some());
    assert_eq!(&t.due.as_deref().unwrap()[5..], "08-01");
}

#[test]
fn bare_title_defaults_to_due_today() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let today = xhy_lib::now_local_date_str();
    // 没写日期 → 今天到期。快速添加不该产出"无到期日"的待办：那种条目既不在
    // 今天也不在任何时间桶里，用户记完的感觉是"东西不见了"。
    let t = st.add_todo(l.clone(), "买杯咖啡".into()).unwrap();
    assert_eq!(t.due.as_deref(), Some(today.as_str()), "无日期默认今天");
    assert!(t.remind_at.is_none(), "只默认到期日，不擅自造提醒（否则每条都响）");
    // 写了日期就用给的日期，不被默认值覆盖
    let t2 = st.add_todo(l.clone(), "2031-10-20 续费".into()).unwrap();
    assert_eq!(t2.due.as_deref(), Some("2031-10-20"));
    // 只有时刻没有日期 → 提醒落在默认的今天那天
    let t3 = st.add_todo(l, "下午3点 面试".into()).unwrap();
    assert_eq!(t3.due.as_deref(), Some(today.as_str()));
    assert_eq!(
        t3.remind_at.as_deref(),
        Some(format!("{today}T15:00").as_str()),
        "提醒日期跟着默认到期日一起走"
    );
}

#[test]
fn recognized_time_becomes_a_reminder() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    // 日期 + 时间 → remind_at = "日期T时间"，正是提醒调度器的格式
    let t = st.add_todo(l.clone(), "明天 9:30 站会".into()).unwrap();
    assert_eq!(t.title, "站会");
    let r = t.remind_at.clone().expect("识别到时间应落成提醒");
    assert!(r.ends_with("T09:30"), "got {r}");
    assert_eq!(&r[..10], t.due.as_deref().unwrap());
    // 中文式时间同样生效
    let t2 = st.add_todo(l.clone(), "下午3点半 面试".into()).unwrap();
    assert_eq!(t2.remind_at.as_deref().map(|s| &s[11..]), Some("15:30"));
    // 只有日期没有时间 → 不擅自造提醒
    let t3 = st.add_todo(l, "8月1日 体检".into()).unwrap();
    assert!(t3.remind_at.is_none());
}

#[test]
fn todo_time_carries_into_event_when_scheduled() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l.clone(), "周五 14:00 评审".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-05-08".into()).unwrap();
    assert_eq!(ev.time.as_deref(), Some("14:00"));
    // 无时间的待办拖成日程仍是全天
    let t2 = st.add_todo(l, "买咖啡".into()).unwrap();
    assert_eq!(st.todo_to_event(t2.id, "2026-05-08".into()).unwrap().time, None);
}

#[test]
fn completing_recurring_todo_spawns_next() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "喝水".into()).unwrap();
    t.due = Some("2026-01-05".into());
    t.recurrence = Some(Recurrence {
        freq: Freq::Daily,
        interval: 1,
        by_weekday: vec![],
        by_monthday: vec![],
        until: None,
        auto_create: true,
    });
    st.update_todo(t.clone()).unwrap();

    let spawned = st.set_done(t.id.clone(), true).unwrap();
    assert!(spawned.is_some(), "应自动生成下一条");
    let s = spawned.unwrap();
    assert_eq!(s.due.as_deref(), Some("2026-01-06"));
    assert!(!s.done);
    assert_ne!(s.id, t.id);

    // 原条目已完成，活动列表只剩新的
    let act = st.todos_of(None);
    assert_eq!(act.len(), 1);
    assert_eq!(act[0].id, s.id);
}

#[test]
fn spawned_recurrence_carries_reminders_to_new_date() {
    // 审计第 4 处同类问题：set_done 生成重复项时改了 due，却只把 reminded
    // 置 false，**没搬 remind_at / extra_reminds**。
    // 后果（"每天 9 点站会"这种最常见用法）：新条目的提醒仍指向**旧那一天**
    // —— 那个时点早已过期，下一轮轮询就会立刻误报。用户视角：
    // "我按时完成了今天的，怎么马上又提醒我一遍？"
    use xhy_lib::domain::{Freq, Recurrence};
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "每日站会".into()).unwrap();
    t.due = Some("2026-05-03".into());
    t.remind_at = Some("2026-05-03T09:00".into());
    t.extra_reminds = vec!["2026-05-03T18:00".into()];
    t.recurrence = Some(Recurrence {
        freq: Freq::Daily,
        interval: 1,
        by_weekday: vec![],
        by_monthday: vec![],
        until: None,
        auto_create: true,
    });
    st.update_todo(t.clone()).unwrap();

    let nt = st.set_done(t.id.clone(), true).unwrap().expect("应生成下一条");
    assert_eq!(nt.due.as_deref(), Some("2026-05-04"), "下一条应在新日期");
    assert_eq!(
        nt.remind_at.as_deref(),
        Some("2026-05-04T09:00"),
        "★ 提醒必须跟着新日期走（否则旧时点已过期、立刻误报）"
    );
    assert_eq!(
        nt.extra_reminds,
        vec!["2026-05-04T18:00".to_string()],
        "★ 额外提醒也要跟着搬"
    );
    assert!(!nt.reminded);
    assert!(nt.reminded_keys.is_empty(), "新条目的去重记录必须干净");
    assert!(nt.last_reminded_at.is_none());
}

#[test]
fn non_recurring_completion_spawns_nothing() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "一次性".into()).unwrap();
    assert!(st.set_done(t.id, true).unwrap().is_none());
    assert!(st.todos_of(None).is_empty());
}

#[test]
fn archive_moves_old_done_out_of_main_store() {
    let (st, dir) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "很久以前完成".into()).unwrap();
    t.done = true;
    // 伪造完成日为 60 天前
    t.done_at = Some("2020-01-01T10:00".into());
    st.update_todo(t.clone()).unwrap();

    let n = st.run_archive().unwrap();
    assert_eq!(n, 1, "应归档 1 条");
    // 主表不再含它
    assert!(st.snapshot().todos.iter().all(|x| x.id != t.id));
    // 归档分片文件存在且含该行
    let shard = dir.path().join("archive").join("archive-202001.jsonl");
    assert!(shard.exists(), "应写归档分片");
    let body = std::fs::read_to_string(&shard).unwrap();
    assert!(body.contains("很久以前完成"));
}

#[test]
fn todo_order_semantics_are_consistent_across_lists() {
    // 审计疑点：order 的编号体系是否前后一致？
    //   · add_todo 用**清单内** max+1
    //   · reorder_todo 给同清单的项写 0,1,2…（清单内相对位次）
    //   · set_done 生成重复项时却用**全局** max+1
    // 若三者混用，"新条目该排在哪"就会因来源不同而不一致。
    let (st, _d) = tmp_app();
    let a = first_list(&st);
    let b = st.add_list("另一个清单".into(), "#fff".into()).unwrap();
    let bid = b.id.clone();

    let a1 = st.add_todo(a.clone(), "A1".into()).unwrap();
    let a2 = st.add_todo(a.clone(), "A2".into()).unwrap();
    let b1 = st.add_todo(bid.clone(), "B1".into()).unwrap();
    eprintln!(
        "ORDER| A1={} A2={} B1={}",
        a1.order, a2.order, b1.order
    );

    // 同清单内必须严格递增（这是 order 存在的意义）
    assert!(a2.order > a1.order, "同清单内新建应递增：A1={} A2={}", a1.order, a2.order);

    // 重复项生成后的 order 必须仍能与同清单其他项正确排序
    let mut r = st.add_todo(a.clone(), "重复的".into()).unwrap();
    r.recurrence = Some(Recurrence {
        freq: Freq::Daily,
        interval: 1,
        by_weekday: vec![],
        by_monthday: vec![],
        until: None,
        auto_create: true,
    });
    r.due = Some("2026-03-19".into());
    st.update_todo(r.clone()).unwrap();
    let next = st.set_done(r.id.clone(), true).unwrap().expect("应生成下一条");
    eprintln!("ORDER| 重复生成的下一条 order={}", next.order);
    // 关键：新生成的这条 order 必须大于同清单内所有现存项（排在最后），
    // 否则它会插到 A1/A2 之间，用户看到"新重复项跑到列表中间"
    let max_in_a = st
        .snapshot()
        .todos
        .iter()
        .filter(|t| t.list_id == a && t.id != next.id)
        .map(|t| t.order)
        .max()
        .unwrap_or(0);
    assert!(
        next.order > max_in_a,
        "重复生成的新项应排在所属清单末尾：新 order={} 清单内最大={}",
        next.order,
        max_in_a
    );
    let _ = b1;
    let _ = a2;
}

#[test]
fn events_in_range_survives_absurd_span() {
    // 审计疑点：events_in_range 展开重复日程，区间长度来自 IPC。
    // 若前端传来超长区间（如年视图被改成 50 年），会不会产生巨量数据？
    // 这条测试**量化**它的实际行为，作为是否需要限长的依据。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    st.add_event(Event {
        id: String::new(),
        date: "2026-01-01".into(),
        time: None,
        duration_min: 0,
        title: "每日站会".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        recurrence: Some(Recurrence {
            freq: Freq::Daily,
            interval: 1,
            by_weekday: vec![],
            by_monthday: vec![],
            until: None,
            auto_create: true,
        }),
        list_id: Some(l),
    })
    .unwrap();

    // 正常一年：应产生约 365 条，不该爆
    let one_year = st.events_in_range("2026-01-01", "2026-12-31", None).len();
    assert!(
        (360..=370).contains(&one_year),
        "一年区间应展开约 365 条，实得 {one_year}"
    );

    // 极端 50 年：量化它到底产生多少（这是性能风险的实测依据）
    let huge = st.events_in_range("2026-01-01", "2075-12-31", None).len();
    eprintln!("SPAN| 50 年区间展开 {huge} 条");
    assert!(
        huge <= 20_000,
        "50 年区间不该产生失控数量（实得 {huge}），否则前端渲染会卡死"
    );
}

#[test]
fn patch_todo_can_reset_reminded_flag() {
    // 回归护栏：改提醒时间后必须能把主提醒触发位置回 false，否则新时点
    // 永远不会触发（plan_fires 的判据是 `!t.reminded`）。
    //
    // 这里锁的是**后端能力**：TodoPatch 必须支持 reminded / lastRemindedAt。
    // 曾把前端从整对象回写迁到字段级补丁时漏掉 reminded，于是"改了提醒时间
    // 却不响"——用户视角完全看不出原因。
    use xhy_lib::domain::TodoPatch;
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "改提醒时间".into()).unwrap();
    t.remind_at = Some("2026-10-18T09:00".into());
    st.patch_todo(
        &t.id,
        TodoPatch {
            remind_at: Some(Some("2026-10-18T09:00".into())),
            ..Default::default()
        },
    )
    .unwrap();
    // 模拟提醒已触发
    st.mark_reminded_full(&t.id, "2026-10-18T09:00").unwrap();
    let now = st.snapshot();
    let cur = now.todos.iter().find(|x| x.id == t.id).unwrap();
    assert!(cur.reminded, "前置：应已标记为提醒过");

    // 用户改了提醒时间 → 触发位必须能重置
    let r = st.patch_todo(
        &t.id,
        TodoPatch {
            remind_at: Some(Some("2026-10-18T15:00".into())),
            reminded: Some(false),
            last_reminded_at: Some(None),
            ..Default::default()
        },
    );
    assert!(r.is_ok(), "TodoPatch 必须支持 reminded / lastRemindedAt：{r:?}");
    let after = st.snapshot();
    let got = after.todos.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(got.remind_at.as_deref(), Some("2026-10-18T15:00"));
    assert!(!got.reminded, "★ 改提醒时间必须能重置触发位（否则新时点不响）");
    assert!(got.last_reminded_at.is_none(), "持续提醒计时应重算");
    // 重置后再到点必须能触发（端到端确认判据真的生效）
    let fired = xhy_lib::reminder::plan_fires(&after.todos, "2026-10-18T15:00");
    assert!(
        fired.iter().any(|(id, _, _)| id == &t.id),
        "重置后新时点应能触发，实得 {fired:?}"
    );
}

#[test]
fn drag_todo_to_calendar_shifts_reminder_too() {
    // 与 move_derived_entry_shift_reminder 同一类：拖待办到日历格会同时
    // 生成日程并改写 due，但提醒**没跟着走** —— 用户视角"我把它拖到 5 号，
    // 提醒还停在 3 号"。UI 改到期日（TodoRow.applyDue）是跟着平移的，
    // 两条路径必须同口径。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "拖到日历的事".into()).unwrap();
    t.due = Some("2026-05-03".into());
    t.remind_at = Some("2026-05-03T09:00".into());
    t.extra_reminds = vec!["2026-05-03T18:00".into()];
    st.update_todo(t.clone()).unwrap();

    st.todo_to_event(t.id.clone(), "2026-05-05".into()).unwrap();

    let g = st.snapshot();
    let got = g.todos.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(got.due.as_deref(), Some("2026-05-05"));
    assert_eq!(
        got.remind_at.as_deref(),
        Some("2026-05-05T09:00"),
        "★ 拖到日历后主提醒应随之平移，实得 {:?}",
        got.remind_at
    );
    assert_eq!(
        got.extra_reminds,
        vec!["2026-05-05T18:00".to_string()],
        "★ 额外提醒也要平移"
    );
    assert!(!got.reminded, "改期后触发位应复位");
}

#[test]
fn move_derived_entry_shift_reminder_consistently_with_ui() {
    // 审计疑点：日历上拖"派生条目"（todo: 前缀）改期时，只改了 due，
    // 没像 UI 那样把提醒一起平移。两条路径（UI 改到期日 vs 日历拖拽）
    // 对同一条待办会产生不同结果 —— 用户视角：
    // "我把待办拖到 5 号了，可提醒时间还停在 3 号。"
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l, "带提醒的事".into()).unwrap();
    t.due = Some("2026-05-03".into());
    t.remind_at = Some("2026-05-03T09:00".into());
    st.update_todo(t.clone()).unwrap();

    // 拖派生条目到 2026-05-05
    st.move_event(format!("todo:{}", t.id), "2026-05-05".into()).unwrap();

    let g = st.snapshot();
    let got = g.todos.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(got.due.as_deref(), Some("2026-05-05"), "到期日应改到新日期");
    assert_eq!(
        got.remind_at.as_deref(),
        Some("2026-05-05T09:00"),
        "★ 提醒必须跟着新日期平移（与 UI 改到期日同一口径），实得 {:?}",
        got.remind_at
    );
    // 提醒触发位要复位，否则新时点不会响（与 applyDue 同规则）
    assert!(!got.reminded, "改期后提醒触发位应复位");
}

#[test]
fn removal_and_moves_keep_list_refs_consistent() {
    // 审计疑点：remove_list 只迁移了 todos，没处理**日程的 list_id** 与
    // **挂件的 list_id** —— 两者都可能指向被删清单，留下悬空引用。
    // 用户可见后果：
    //   ① 挂件绑定被删清单 → 列表恒为空（inScope 过滤掉一切），
    //      用户看到"我删了个清单，挂件就空了"，且不知道点「全」能恢复；
    //   ② 待办已被迁到收集箱，但它拖出来的日程仍带旧 list_id →
    //      在主窗口选「收集箱」看日历时，这些日程**不显示**（过滤落空）。
    let (st, _d) = tmp_app();
    let inbox = first_list(&st);
    let doomed = st.add_list("要删的".into(), "#888".into()).unwrap();

    // ① 挂件绑定到 doomed
    let mut w = widget("wBind", true);
    w.list_id = Some(doomed.id.clone());
    st.upsert_widget(w).unwrap();

    // ② doomed 里一条待办 + 由它拖出的日程
    let t = st.add_todo(doomed.id.clone(), "带日程的".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-05-01".into()).unwrap();
    assert_eq!(ev.list_id.as_deref(), Some(doomed.id.as_str()), "前置：日程应带该清单");

    st.remove_list(doomed.id.clone()).unwrap();

    // 待办已迁入收集箱（既有行为）
    let g = st.snapshot();
    assert_eq!(
        g.todos.iter().find(|x| x.id == t.id).unwrap().list_id,
        inbox,
        "待办应迁入第一个剩余清单"
    );

    // ① 挂件不应再指向已删清单
    let w2 = g.widgets.iter().find(|x| x.id == "wBind").unwrap();
    assert_ne!(
        w2.list_id.as_deref(),
        Some(doomed.id.as_str()),
        "★ 挂件不该继续绑定已删清单（否则挂件恒空）"
    );

    // ② 日程归属应跟着待办走（否则选收集箱时日历漏掉它）
    let ev2 = g.events.iter().find(|x| x.id == ev.id).unwrap();
    assert_ne!(
        ev2.list_id.as_deref(),
        Some(doomed.id.as_str()),
        "★ 日程不该留着已删清单的归属"
    );
    // 用查询验证：选收集箱时这条日程必须出现
    let on_inbox = st.events_in_range("2026-05-01", "2026-05-01", Some(&inbox));
    assert!(
        on_inbox.iter().any(|e| e.id == ev.id),
        "★ 选收集箱时该日程必须可见（它已归属收集箱），实得 {:?}",
        on_inbox.iter().map(|e| (&e.id, &e.title)).collect::<Vec<_>>()
    );
}

#[test]
fn excluded_list_is_skipped_by_stats() {
    // 用户诉求："我有多个清单，其中一两个不想加入总结"。
    // 这是清单自身的属性（exclude_from_stats），不是临时视图选择。
    let (st, _d) = tmp_app();
    let inbox = first_list(&st);
    let work = st.add_list("工作".into(), "#00f".into()).unwrap();
    let private = st.add_list("私事".into(), "#888".into()).unwrap();
    let _ = &work; // 这个清单下面没用到，显式忽略以免未用变量警告
    let today = xhy_lib::now_local_date_str();

    // 两个清单各放一条"今天到期且未完成"（会进"遗留"）
    let a = st.add_todo(inbox.clone(), "收集箱的事".into()).unwrap();
    st.patch_todo(
        &a.id,
        TodoPatch { due: Some(Some(today.clone())), ..Default::default() },
    )
    .unwrap();
    let b = st.add_todo(private.id.clone(), "私事一桩".into()).unwrap();
    st.patch_todo(
        &b.id,
        TodoPatch { due: Some(Some(today.clone())), ..Default::default() },
    )
    .unwrap();
    // 私事清单里再排一条日程，验证日程也被排除
    st.todo_to_event(b.id.clone(), today.clone()).unwrap();

    let before = st.period_summary("week", None).stats;
    assert!(before.overdue_left >= 2, "前置：两条遗留都该在，实得 {}", before.overdue_left);
    assert!(before.left_titles.iter().any(|t| t == "私事一桩"));

    // 把"私事"标记为不参与统计
    st.set_list_exclude_stats(&private.id, true).unwrap();

    let after = st.period_summary("week", None).stats;
    assert_eq!(
        after.overdue_left,
        before.overdue_left - 1,
        "排除一个清单后遗留应少 1"
    );
    assert!(
        !after.left_titles.iter().any(|t| t == "私事一桩"),
        "被排除清单的待办不该出现在遗留清单里，实得 {:?}",
        after.left_titles
    );
    assert!(
        after.left_titles.iter().any(|t| t == "收集箱的事"),
        "未排除清单的待办必须照旧统计"
    );
    // 日程也要跟着排除（否则"排除了那个清单，总结里还有它的日程"）
    assert_eq!(
        after.event_count,
        before.event_count - 1,
        "被排除清单的日程不该计入统计"
    );
    // 分析页趋势与总结走同一 compute，口径必须一致 —— 用**同一区间**比对：
    // 本周完成数 = 趋势里落在本周那几天的和
    let (from, to) = xhy_lib::stats::week_bounds(chrono::Local::now().date_naive());
    let week_days = (to - from).num_days() + 1;
    let trend_week: usize = st
        .trend(week_days, None)
        .iter()
        .map(|(_, n)| *n)
        .sum();
    assert_eq!(
        trend_week, before.done,
        "同区间的趋势之和必须等于总结的完成数（同口径）；排除清单后也要一致"
    );

    // 取消排除后应完全恢复
    st.set_list_exclude_stats(&private.id, false).unwrap();
    let back = st.period_summary("week", None).stats;
    assert_eq!(back.overdue_left, before.overdue_left, "取消排除后应恢复原值");
    assert_eq!(back.event_count, before.event_count);
}

#[test]
fn excluded_list_combines_with_single_list_selection() {
    // list（临时只看某个）与 exclude（永久排除）是两个正交维度，叠加时取交集
    let (st, _d) = tmp_app();
    let inbox = first_list(&st);
    let priv2 = st.add_list("私事2".into(), "#888".into()).unwrap();
    let today = xhy_lib::now_local_date_str();
    let t = st.add_todo(priv2.id.clone(), "私事2的活".into()).unwrap();
    st.patch_todo(
        &t.id,
        TodoPatch { due: Some(Some(today.clone())), ..Default::default() },
    )
    .unwrap();

    // 显式点这个清单：能统计到
    let s1 = st.period_summary("week", Some(&priv2.id)).stats;
    assert_eq!(s1.overdue_left, 1, "显式选它时应能统计");

    // 但它被排除后，即便显式选它也应是 0（交集语义）
    st.set_list_exclude_stats(&priv2.id, true).unwrap();
    let s2 = st.period_summary("week", Some(&priv2.id)).stats;
    assert_eq!(s2.overdue_left, 0, "被排除的清单显式选它也不该统计");

    // 其他清单不受影响
    let s3 = st.period_summary("week", Some(&inbox)).stats;
    assert_eq!(s3.overdue_left, 0, "收集箱本来就没东西");
}

#[test]
fn set_list_exclude_stats_is_idempotent_and_safe() {
    let (st, _d) = tmp_app();
    let l = st.add_list("测试".into(), "#000".into()).unwrap();
    st.set_list_exclude_stats(&l.id, true).unwrap();
    st.set_list_exclude_stats(&l.id, true).unwrap();
    let g = st.snapshot();
    assert!(g.lists.iter().find(|x| x.id == l.id).unwrap().exclude_from_stats);
    // 不存在的 id 静默（前端可能持有已删清单的引用）
    st.set_list_exclude_stats("不存在", true).unwrap();
    assert_eq!(st.snapshot().lists.len(), g.lists.len(), "不该凭空造清单");
}

#[test]
fn archived_done_todo_still_hides_its_linked_event() {
    // 审计疑点：calendar_entries 的已完成过滤只看**活动表**（store.todos）。
    // 待办完成满 N 天会被 run_archive 移出活动表 —— 此后 done_todo_ids
    // 查不到它，若其关联日程还在，日历上就会出现一条"已完成事项的日程"。
    // 这条测试验明该场景是否真的漏。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "带日程的完成项".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-05-01".into()).unwrap();
    // 标完成（关联日程应立即从日历隐藏）
    st.set_done(t.id.clone(), true).unwrap();
    assert!(
        st.events_on("2026-05-01").iter().all(|e| e.id != ev.id),
        "完成当天就该隐藏其关联日程"
    );
    // 再把它做成"很久前完成"并归档
    let mut t2 = st.snapshot().todos.iter().find(|x| x.id == t.id).unwrap().clone();
    t2.done_at = Some("2020-01-01T10:00".into());
    st.update_todo(t2).unwrap();
    assert_eq!(st.run_archive().unwrap(), 1, "应被归档");
    assert!(st.snapshot().todos.iter().all(|x| x.id != t.id), "已移出活动表");
    // 关键断言：归档后日历仍不该显示这条已完成待办的日程
    let on_date = st.events_on("2026-05-01");
    assert!(
        on_date.iter().all(|e| e.id != ev.id),
        "归档的已完成待办，其关联日程不该重新出现在日历上（实得 {:?}）",
        on_date.iter().map(|e| (&e.id, &e.title)).collect::<Vec<_>>()
    );
}

#[test]
fn todo_to_event_links_and_moves_due() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "见客户".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-03-09".into()).unwrap();
    assert_eq!(ev.date, "2026-03-09");
    assert_eq!(ev.source, "todo");
    assert_eq!(ev.todo_id.as_deref(), Some(t.id.as_str()));
    // 待办 due 同步到落点日
    let after = st.snapshot().todos.into_iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(after.due.as_deref(), Some("2026-03-09"));
    // 日程可查
    assert_eq!(st.events_on("2026-03-09").len(), 1);
}

#[test]
fn move_event_redates() {
    let (st, _d) = tmp_app();
    let ev = st
        .add_event(Event {
            id: String::new(),
            date: "2026-05-01".into(),
            time: Some("09:30".into()),
            duration_min: 60,
            title: "评审".into(),
            color: String::new(),
            source: "manual".into(),
            todo_id: None,
            span_days: 1,
            span_index: 0,
            span_total: 1,
            list_id: None,
            recurrence: None,
        })
        .unwrap();
    st.move_event(ev.id.clone(), "2026-05-04".into()).unwrap();
    assert!(st.events_on("2026-05-04").iter().any(|e| e.id == ev.id));
    assert!(st.events_on("2026-05-01").is_empty());
}

#[test]
fn deleting_list_rehomes_its_todos() {
    let (st, _d) = tmp_app();
    let a = first_list(&st);
    let b = st.add_list("第二清单".into(), "#4c8dff".into()).unwrap();
    st.add_todo(b.id.clone(), "属于B".into()).unwrap();
    st.remove_list(b.id.clone()).unwrap();
    let left = st.snapshot().todos;
    assert_eq!(left.len(), 1, "删清单不应连带丢待办");
    assert_eq!(left[0].list_id, a, "应迁回剩余清单");
}

#[test]
fn reorder_respects_pin_then_order() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t1 = st.add_todo(l.clone(), "一".into()).unwrap();
    let t2 = st.add_todo(l.clone(), "二".into()).unwrap();
    let t3 = st.add_todo(l.clone(), "三".into()).unwrap();

    // 把 t3 拖到 t1 之前
    st.reorder_todo(t3.id.clone(), Some(t1.id.clone()), false)
        .unwrap();
    let seq: Vec<String> = st
        .todos_of(Some(&l))
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(seq, vec!["三", "一", "二"]);

    // 置顶优先于 order
    st.toggle_pin(t2.id.clone()).unwrap();
    let seq2: Vec<String> = st
        .todos_of(Some(&l))
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(seq2[0], "二", "置顶项应排最前");
}

#[test]
fn countdown_roundtrip() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "生日".into()).unwrap();
    st.set_countdown(
        t.id.clone(),
        Some(Countdown {
            date: "2026-12-31".into(),
            mode: CountdownMode::Until,
            keep_counting_up: true,
        }),
    )
    .unwrap();
    let got = st.snapshot().todos.into_iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(got.countdown.unwrap().date, "2026-12-31");
}

#[test]
fn defaults_are_conservative_and_match_frontend() {
    // 锁死默认值：Rust 与前端 types.ts 必须一致，否则新装用户首启就会
    // 看到"设置说 A、实际是 B"的错乱（本项目真实踩过 attach_mode 漂移）。
    let (st, _d) = tmp_app();
    let s = &st.snapshot().settings;
    assert_eq!(
        s.attach_mode,
        xhy_lib::domain::AttachMode::BottomZ,
        "默认应为 BottomZ（保守、已验证）；WorkerW 由用户显式开启"
    );
    assert!(!s.autostart, "默认不自启");
    assert_eq!(s.theme, "light");
    assert_eq!(s.font_px, 13);
    // 播种时三件套都未停靠
    assert!(st.snapshot().widgets.iter().all(|w| w.edge_hide.is_none()));
}

#[test]
fn autostart_defaults_off() {
    // 开机自启必须默认关：不擅自往 HKCU\Run 写东西。
    // 这条测试锁死默认值，防止有人图省事又改回 true。
    let (st, _d) = tmp_app();
    assert!(
        !st.snapshot().settings.autostart,
        "autostart 默认应为 false，实得 {}",
        st.snapshot().settings.autostart
    );
}

#[test]
fn settings_patch_clamps_and_persists() {
    let (st, _d) = tmp_app();
    use xhy_lib::state::SettingsPatch;
    let s = st
        .set_settings(SettingsPatch {
            opacity: Some(9.0),  // 越界 → 应夹到 1.0
            font_px: Some(2),    // 越界 → 应夹到 10
            ..Default::default()
        })
        .unwrap();
    assert_eq!(s.opacity, 1.0);
    assert_eq!(s.font_px, 10);

    // 归档阈值 / 备份份数：设置页那两个 number 输入框只有 min/max 管步进，
    // 手打 99999 照样进得来。归档阈值跑偏 = 完成项永远不归档（主表越涨越大）
    // 或（负数经 `as i64` 后）cutoff 落到未来 → 所有已完成项立刻被搬走。
    let s = st
        .set_settings(SettingsPatch {
            archive_after_days: Some(u16::MAX),
            backup_keep: Some(0),
            snap_gap: Some(-8.0),
            edge_hide_threshold: Some(9_999.0),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(s.archive_after_days, 365, "归档阈值上限 365 天");
    assert_eq!(s.backup_keep, 1, "至少留一份备份（与 db::keep_count 同区间）");
    assert_eq!(s.snap_gap, 0.0, "吸附间隙不能是负的");
    assert_eq!(s.edge_hide_threshold, 200.0, "贴边触发距离要有上限");
}

#[test]
fn export_import_roundtrip_survives_reload() {
    let (st, dir) = tmp_app();
    let l = first_list(&st);
    st.add_todo(l.clone(), "要带走".into()).unwrap();
    st.add_note("便签内容".into()).unwrap();
    st.upsert_widget(Widget {
        id: "w1".into(),
        kind: WidgetKind::Calendar,
        list_id: None,
        view: Some(CalendarView::Week),
        filter: None,
        tag: None,
        size: WidgetSize::L,
        rect: RectL { x: 10.0, y: 20.0, w: 300.0, h: 400.0 },
        visible: true,
        auto_dock: true,
        edge_hide: Some("right".into()),
        free: true,
    })
    .unwrap();
    let json = st.export();

    // 新实例导入同一份数据
    let st2 = AppState::open(dir.path().join("second")).unwrap();
    st2.import(&json).unwrap();
    let g = st2.snapshot();
    assert!(g.todos.iter().any(|t| t.title == "要带走"));
    assert!(g.notes.iter().any(|n| n.body == "便签内容"));
    let w = g.widgets.iter().find(|w| w.id == "w1").unwrap();
    assert_eq!(w.view, Some(CalendarView::Week));
    assert_eq!(w.edge_hide.as_deref(), Some("right"));
}

#[test]
fn import_rejects_newer_version() {
    let (st, _d) = tmp_app();
    let bad = r#"{"version":99999,"lists":[],"todos":[],"events":[],"notes":[],"widgets":[],"settings":{"theme":"light","attachMode":"bottomZ","opacity":1,"fontPx":13,"autostart":true,"hotkeys":{"toggleAll":"Alt+Shift+H"},"archiveAfterDays":7,"backupKeep":10,"snapGap":8,"edgeHideThreshold":24},"archived":[]}"#;
    let e = st.import(bad).unwrap_err();
    assert!(e.contains("高于"), "错误信息应说明版本过高：{e}");
}

#[test]
fn backups_rotate_and_keep_limit() {
    let (st, dir) = tmp_app();
    let l = first_list(&st);
    st.set_settings(xhy_lib::state::SettingsPatch {
        backup_keep: Some(3),
        ..Default::default()
    })
    .unwrap();
    for i in 0..8 {
        st.add_todo(l.clone(), format!("第{i}条")).unwrap();
    }
    let backups = dir.path().join("backups");
    let n = std::fs::read_dir(&backups)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
            .to_string_lossy()
            .starts_with("store-")
        })
        .count();
    assert!(n <= 3, "备份应轮转保留 ≤3 份，实际 {n}");
    assert!(n >= 1, "至少应有一份备份");
}

#[test]
fn reload_reads_back_everything() {
    // 写 → 重新 open → 数据一致（验证原子写与 serde 往返）
    let (st, dir) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "持久化检查".into()).unwrap();
    st.set_done(t.id.clone(), true).unwrap();
    drop(st);
    let st2 = AppState::open(dir.path().to_path_buf()).unwrap();
    let g = st2.snapshot();
    assert_eq!(g.todos.len(), 1);
    assert!(g.todos[0].done);
    assert!(g.todos[0].done_at.is_some());
}

#[test]
fn notes_crud() {
    let (st, _d) = tmp_app();
    let n = st.add_note("第一行\n第二行".into()).unwrap();
    assert_eq!(n.font_px, 13);
    st.update_note(xhy_lib::domain::Note {
        body: "改过了".into(),
        font_px: 20,
        ..n.clone()
    })
    .unwrap();
    let g = st.snapshot();
    let got = g.notes.iter().find(|x| x.id == n.id).unwrap();
    assert_eq!(got.body, "改过了");
    assert_eq!(got.font_px, 20);
    let id = n.id.clone();
    st.remove_note(id.clone()).unwrap();
    assert!(st.snapshot().notes.iter().all(|x| x.id != id));
}

#[test]
fn note_tags_are_marked_not_stripped() {
    let (st, _d) = tmp_app();
    let n = st.add_note("[紧急] 买牛奶".into()).unwrap();
    assert_eq!(n.tags, vec!["紧急"]);
    // 与待办不同：便签正文必须原样保留，不能把用户敲的字符删掉
    assert_eq!(n.body, "[紧急] 买牛奶");
    // 保存时按正文重新归类（用户是"先建空便签再打字"）
    let empty = st.add_note(String::new()).unwrap();
    assert!(empty.tags.is_empty());
    st.update_note(xhy_lib::domain::Note {
        body: "开会 [工作] 带本子".into(),
        ..empty.clone()
    })
    .unwrap();
    let g = st.snapshot();
    let got = g.notes.iter().find(|x| x.id == empty.id).unwrap();
    assert_eq!(got.tags, vec!["工作"]);
    assert_eq!(got.body, "开会 [工作] 带本子");
}

#[test]
fn note_pin_and_reorder() {
    let (st, _d) = tmp_app();
    // 播种自带一张空便签，这里只关心我们新建的三张
    let a = st.add_note("A".into()).unwrap();
    let b = st.add_note("B".into()).unwrap();
    let c = st.add_note("C".into()).unwrap();

    // 置顶可翻转
    st.toggle_note_pin(a.id.clone()).unwrap();
    assert!(note_of(&st, &a.id).pinned);
    st.toggle_note_pin(a.id.clone()).unwrap();
    assert!(!note_of(&st, &a.id).pinned);

    // 把 C 插到 A 之后 → 未置顶序列应为 A, C, B
    st.reorder_note(c.id.clone(), Some(a.id.clone()), true).unwrap();
    assert_eq!(unpinned_bodies(&st), vec!["A", "C", "B"]);

    // 置顶后恒排最前（store 侧排序键 pinned 优先）
    st.toggle_note_pin(b.id.clone()).unwrap();
    let mut all: Vec<(bool, i32, String)> = st
        .snapshot()
        .notes
        .into_iter()
        .filter(|n| matches!(n.body.as_str(), "A" | "B" | "C"))
        .map(|n| (n.pinned, n.order, n.body))
        .collect();
    all.sort_by(|x, y| (y.0.cmp(&x.0)).then(x.1.cmp(&y.1)));
    assert_eq!(all.first().unwrap().2, "B", "置顶项应排最前");
    assert_eq!(unpinned_bodies(&st), vec!["A", "C"]);
}

fn note_of(st: &AppState, id: &str) -> xhy_lib::domain::Note {
    st.snapshot().notes.into_iter().find(|n| n.id == id).unwrap()
}

/// 未置顶便签按 order 排完的正文
fn unpinned_bodies(st: &AppState) -> Vec<String> {
    let mut v: Vec<(i32, String)> = st
        .snapshot()
        .notes
        .into_iter()
        .filter(|n| !n.pinned && matches!(n.body.as_str(), "A" | "B" | "C"))
        .map(|n| (n.order, n.body))
        .collect();
    v.sort_by_key(|(o, _)| *o);
    v.into_iter().map(|(_, b)| b).collect()
}

#[test]
fn recurring_event_expands_across_days() {
    use xhy_lib::domain::{Freq, Recurrence};
    let (st, _d) = tmp_app();
    let ev = st
        .add_event(Event {
            id: String::new(),
            date: "2026-05-04".into(), // 周一
            time: Some("09:00".into()),
            duration_min: 0,
            title: "周会".into(),
            color: String::new(),
            source: "manual".into(),
            todo_id: None,
            span_days: 1,
            span_index: 0,
            span_total: 1,
            list_id: None,
            recurrence: Some(Recurrence {
                freq: Freq::Weekly,
                interval: 1,
                by_weekday: vec![],
                by_monthday: vec![],
                until: None,
                auto_create: true,
            }),
        })
        .unwrap();
    // 同一个母记录，5/4 与 5/11 各命中一次
    assert_eq!(st.events_on("2026-05-04").len(), 1);
    assert_eq!(st.events_on("2026-05-11").len(), 1);
    assert_eq!(st.events_on("2026-05-05").len(), 0, "非重复日不该出现");
    let on11 = &st.events_on("2026-05-11")[0];
    assert_eq!(on11.id, ev.id, "occurrence 应沿用母记录 id");
    assert_eq!(on11.date, "2026-05-11", "date 应换成当天");
    assert_eq!(on11.time.as_deref(), Some("09:00"), "时间应继承");
    // 月视图一次拿到 5 月的 4 个周一
    let month = st.events_in_range("2026-05-01", "2026-05-31", None);
    assert_eq!(month.len(), 4, "5 月的周一为 4、11、18、25");
    let days: Vec<&str> = month.iter().map(|e| e.date.as_str()).collect();
    assert!(days.contains(&"2026-05-04") && days.contains(&"2026-05-25"));
}

#[test]
fn recurring_event_respects_until() {
    use xhy_lib::domain::{Freq, Recurrence};
    let (st, _d) = tmp_app();
    st.add_event(Event {
        id: String::new(),
        date: "2026-05-04".into(),
        time: None,
        duration_min: 0,
        title: "限期周会".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        list_id: None,
        recurrence: Some(Recurrence {
            freq: Freq::Weekly,
            interval: 1,
            by_weekday: vec![],
            by_monthday: vec![],
            until: Some("2026-05-18".into()),
            auto_create: true,
        }),
    })
    .unwrap();
    assert_eq!(st.events_on("2026-05-18").len(), 1, "until 当天仍算一次");
    assert_eq!(st.events_on("2026-05-25").len(), 0, "until 之后不该再出现");
    assert_eq!(st.events_in_range("2026-05-01", "2026-05-31", None).len(), 3);
}

#[test]
fn non_recurring_event_unaffected_by_expansion() {
    let (st, _d) = tmp_app();
    st.add_event(Event {
        id: String::new(),
        date: "2026-05-04".into(),
        time: None,
        duration_min: 0,
        title: "一次性".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        list_id: None,
        recurrence: None,
    })
    .unwrap();
    assert_eq!(st.events_on("2026-05-04").len(), 1);
    assert_eq!(st.events_on("2026-05-11").len(), 0);
    assert_eq!(st.events_in_range("2026-05-01", "2026-06-30", None).len(), 1);
}

#[test]
fn recurring_todo_dragged_to_calendar_carries_rule() {
    use xhy_lib::domain::{Freq, Recurrence};
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "写周报".into()).unwrap();
    st.set_recurrence(
        t.id.clone(),
        Some(Recurrence {
            freq: Freq::Weekly,
            interval: 1,
            by_weekday: vec![4], // 周五
            by_monthday: vec![],
            until: None,
            auto_create: true,
        }),
    )
    .unwrap();
    let ev = st.todo_to_event(t.id, "2026-05-08".into()).unwrap();
    assert!(ev.recurrence.is_some(), "重复待办拖上日历应保留规则");
    assert_eq!(st.events_on("2026-05-08").len(), 1);
    assert_eq!(st.events_on("2026-05-15").len(), 1, "下一个周五也该有");
}

#[test]
fn todo_with_due_shows_on_calendar_without_event() {
    // 用户诉求：写完"明天 交周报"，日历上就该看到，不必手动拖
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "8月1日 交周报".into()).unwrap();
    let due = t.due.clone().expect("应识别出到期日");
    let on = st.events_on(&due);
    assert_eq!(on.len(), 1, "待办应派生出日历条目");
    assert_eq!(on[0].id, format!("todo:{}", t.id));
    assert_eq!(on[0].source, "todo");
    assert_eq!(on[0].todo_id.as_deref(), Some(t.id.as_str()));
    // 月视图区间同样能看到。注意 8月1日 已过会被解析器顺延到明年，
    // 所以区间必须围绕**实际解析出的** due 构造，不能硬编码年份。
    let prev = {
        let d = due.as_str();
        let y: i32 = d[..4].parse().unwrap();
        let m: u32 = d[5..7].parse().unwrap();
        format!("{y}-{:02}-01", if m == 1 { 12 } else { m - 1 })
    };
    assert!(st
        .events_in_range(&prev, &due, None)
        .iter()
        .any(|e| e.id == format!("todo:{}", t.id)));
    // 完成的待办不再上日历
    st.set_done(t.id.clone(), true).unwrap();
    assert_eq!(st.events_on(&due).len(), 0, "已完成不该再占日历");
}

#[test]
fn scheduled_todo_is_not_double_counted_on_calendar() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "8月1日 交周报".into()).unwrap();
    let due = t.due.clone().unwrap();
    assert_eq!(st.events_on(&due).len(), 1, "派生一条");
    // 显式排进日历（拖拽/「排到…」）后，应只剩那条真实日程，不是两条
    st.todo_to_event(t.id.clone(), due.clone()).unwrap();
    let after = st.events_on(&due);
    assert_eq!(after.len(), 1, "已排进日历的待办不该再派生一份");
    assert!(!after[0].id.starts_with("todo:"), "留下的应是真实日程");
}

#[test]
fn derived_calendar_entry_edits_the_todo_itself() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "8月1日 交周报".into()).unwrap();
    let due = t.due.clone().unwrap();
    let derived = st.events_on(&due).remove(0);
    assert!(derived.id.starts_with("todo:"));

    // 双击改标题 → 改的是待办
    st.update_event(xhy_lib::domain::Event {
        title: "改过的标题".into(),
        ..derived.clone()
    })
    .unwrap();
    assert_eq!(
        st.snapshot().todos.iter().find(|x| x.id == t.id).unwrap().title,
        "改过的标题"
    );

    // 拖到别的日子 → 改待办 due
    st.move_event(derived.id.clone(), "2026-08-05".into()).unwrap();
    assert_eq!(
        st.snapshot().todos.iter().find(|x| x.id == t.id).unwrap().due.as_deref(),
        Some("2026-08-05")
    );

    // ✕ 只清到期日，不删待办（可逆、无破坏性）
    st.remove_event(derived.id).unwrap();
    let g = st.snapshot();
    assert!(g.todos.iter().any(|x| x.id == t.id), "待办必须还在");
    assert!(g.todos.iter().find(|x| x.id == t.id).unwrap().due.is_none());
    assert_eq!(g.events.len(), 0, "不该凭空造出日程");
}

#[test]
fn completing_todo_hides_its_linked_event_too() {
    // 用户报的"有的还在有的没了"：派生条目完成即消失，但拖进过日历的
    // 待办其日程（todo_id 关联）原先不看完成态 → 行为不一致。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "见客户".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-03-09".into()).unwrap();
    assert_eq!(st.events_on("2026-03-09").len(), 1, "未完成时日程可见");
    // 完成待办 → 关联日程一并从日历消失
    st.set_done(t.id.clone(), true).unwrap();
    assert_eq!(st.events_on("2026-03-09").len(), 0, "待办完成后关联日程不该再显示");
    assert_eq!(
        st.events_in_range("2026-03-01", "2026-03-31", None)
            .iter()
            .filter(|e| e.id == ev.id)
            .count(),
        0,
        "月视图同样不该出现"
    );
    // 取消完成 → 恢复显示（数据没删，只是不显示）
    st.set_done(t.id.clone(), false).unwrap();
    assert_eq!(st.events_on("2026-03-09").len(), 1, "取消完成后应恢复");
    // 日程本身还在库里（没被连带删除）
    assert!(st.snapshot().events.iter().any(|e| e.id == ev.id));
}

#[test]
fn manual_events_unaffected_by_todo_completion() {
    // 手工建的日程（无 todo_id）不受任何待办完成影响
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "无关待办".into()).unwrap();
    st.add_event(Event {
        id: String::new(),
        date: "2026-05-01".into(),
        time: None,
        duration_min: 0,
        title: "手工日程".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        list_id: None,
        recurrence: None,
    })
    .unwrap();
    st.set_done(t.id, true).unwrap();
    assert_eq!(st.events_on("2026-05-01").len(), 1, "手工日程不该消失");
}

#[test]
fn auto_postpone_moves_overdue_todos_to_today() {
    use xhy_lib::domain::Settings;
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    // 造一条"昨天到期"的（直接写 due 为远古日期，保证必然逾期）
    let t = st.add_todo(l.clone(), "逾期任务".into()).unwrap();
    st.update_todo(xhy_lib::domain::Todo {
        due: Some("2020-01-01".into()),
        remind_at: Some("2020-01-01T09:00".into()),
        reminded: true,
        ..t.clone()
    })
    .unwrap();
    // 默认关：不该动数据
    assert!(st.snapshot().settings.auto_postpone == false);
    // 显式调用延期
    let n = st.auto_postpone().unwrap();
    assert_eq!(n, 1, "应延期 1 条");
    let g = st.snapshot();
    let got = g.todos.iter().find(|x| x.id == t.id).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(got.due.as_deref(), Some(today.as_str()), "到期日应推到今天");
    assert!(
        got.remind_at.as_deref().unwrap().starts_with(&today),
        "提醒时间应同步平移"
    );
    assert_eq!(&got.remind_at.as_deref().unwrap()[11..], "09:00", "提醒时刻应保留");
    assert!(!got.reminded, "提醒位应重置，今天还能提醒");
    // 再跑一次：没有逾期项了
    assert_eq!(st.auto_postpone().unwrap(), 0);
    // 未逾期的不该被动
    let t2 = st.add_todo(l.clone(), "未来的事".into()).unwrap();
    st.update_todo(xhy_lib::domain::Todo {
        due: Some("2099-01-01".into()),
        ..t2.clone()
    })
    .unwrap();
    st.auto_postpone().unwrap();
    assert_eq!(
        st.snapshot().todos.iter().find(|x| x.id == t2.id).unwrap().due.as_deref(),
        Some("2099-01-01"),
        "未来到期日不该被改"
    );
    // 已完成的不该被动
    let t3 = st.add_todo(l.clone(), "完成的旧事".into()).unwrap();
    st.update_todo(xhy_lib::domain::Todo {
        due: Some("2020-01-01".into()),
        ..t3.clone()
    })
    .unwrap();
    st.set_done(t3.id, true).unwrap();
    let before = st.snapshot().todos.iter().filter(|x| x.due.as_deref() == Some("2020-01-01")).count();
    st.auto_postpone().unwrap();
    let after = st.snapshot().todos.iter().filter(|x| x.due.as_deref() == Some("2020-01-01")).count();
    assert_eq!(before, after, "已完成项不该被延期");
    let _ = Settings::default();
}

#[test]
fn export_markdown_contains_everything() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st
        .add_todo(l, "[工作] 8月1日 交周报".into())
        .unwrap();
    st.update_todo(xhy_lib::domain::Todo {
        note: "带备注\n第二行".into(),
        ..t.clone()
    })
    .unwrap();
    st.add_note("[灵感] 记一笔".into()).unwrap();
    st.add_event(xhy_lib::domain::Event {
        id: String::new(),
        date: "2026-05-01".into(),
        time: Some("09:30".into()),
        duration_min: 0,
        title: "评审".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        list_id: None,
        recurrence: None,
    })
    .unwrap();
    let md = xhy_lib::export::to_markdown(&st.snapshot());
    // 待办：清单名 / 勾选框 / 标签 / 日期 / 备注（多行引用）
    assert!(md.contains("## 📋 收集箱"), "缺清单节:\n{md}");
    assert!(md.contains("- [ ] 交周报"), "缺待办行:\n{md}");
    assert!(md.contains("`工作`"), "缺标签:\n{md}");
    assert!(md.contains("📅 202"), "缺到期日:\n{md}");
    assert!(md.contains("> 带备注"), "缺备注:\n{md}");
    // 日程
    assert!(md.contains("## 🗓️ 日程"), "缺日程节:\n{md}");
    assert!(md.contains("**2026-05-01** 09:30 · 评审"), "缺日程行:\n{md}");
    // 便签
    assert!(md.contains("## 📝 便签"), "缺便签节:\n{md}");
    assert!(md.contains("[灵感] 记一笔"), "缺便签正文:\n{md}");
    // 完成态：勾掉的用 [x] 并带完成日期
    st.set_done(t.id, true).unwrap();
    let md2 = xhy_lib::export::to_markdown(&st.snapshot());
    assert!(md2.contains("- [x] 交周报"), "完成项应为 [x]:\n{md2}");
    assert!(md2.contains("✅ 202"), "完成项应带完成日期:\n{md2}");
}

#[test]
fn markdown_export_note_order_matches_ui() {
    // 审计实锤：导出曾用 `(pinned, Reverse(order))`，bool 升序把**置顶的排到
    // 最后**、order 方向也反了 —— 导出顺序与界面完全颠倒。这里锁死一致性。
    let (st, _d) = tmp_app();
    st.add_note("普通A".into()).unwrap();
    st.add_note("普通B".into()).unwrap();
    let p = st.add_note("置顶的".into()).unwrap();
    st.toggle_note_pin(p.id.clone()).unwrap();

    let md = xhy_lib::export::to_markdown(&st.snapshot());
    let pos = |name: &str| md.find(name).unwrap_or_else(|| panic!("导出里应有 {name}:\n{md}"));
    let (ip, ia, ib) = (pos("置顶的"), pos("普通A"), pos("普通B"));
    assert!(ip < ia && ip < ib, "置顶便签必须排最前（与界面一致）:\n{md}");
    assert!(ia < ib, "未置顶的按 order 升序（A 先建，应在前）:\n{md}");
}

#[test]
fn removing_todo_unlinks_event_but_keeps_it() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "关联测试".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-06-06".into()).unwrap();
    st.remove_todo(t.id).unwrap();
    let kept = st.events_on("2026-06-06");
    assert_eq!(kept.len(), 1, "删待办不应连带删日程");
    assert_eq!(kept[0].id, ev.id);
    assert!(kept[0].todo_id.is_none());
    assert_eq!(kept[0].source, "manual");
}

#[test]
fn reset_reminded_clears_extra_and_constant_state() {
    // 跨天复位必须连 reminded_keys / last_reminded_at 一起清：
    // 额外提醒按**绝对时点**去重，只清 reminded 会让昨天没做完的
    // 额外提醒今天永远不再弹。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mut t = st.add_todo(l.clone(), "多提醒任务".into()).unwrap();
    t.remind_at = Some("2026-10-16T09:00".into());
    t.reminded = true;
    t.extra_reminds = vec!["2026-10-16T09:30".into(), "2026-10-16T10:00".into()];
    t.reminded_keys = vec!["2026-10-16T09:30".into()];
    t.constant_remind = true;
    t.last_reminded_at = Some("2026-10-16T23:00".into());
    st.update_todo(t.clone()).unwrap();

    let mut done = st.add_todo(l, "已完成不该被动".into()).unwrap();
    done.reminded = true;
    done.reminded_keys = vec!["x".into()];
    done.last_reminded_at = Some("y".into());
    st.update_todo(done.clone()).unwrap();
    st.set_done(done.id.clone(), true).unwrap();

    st.reset_reminded().unwrap();
    let g = st.snapshot();
    let got = g.todos.iter().find(|x| x.id == t.id).unwrap();
    assert!(!got.reminded, "主提醒位应复位");
    assert!(got.reminded_keys.is_empty(), "额外提醒去重应清空");
    assert!(got.last_reminded_at.is_none(), "持续提醒计时应重新起算");
    // 提醒配置本身不能被清掉（那是用户设的）
    assert_eq!(got.extra_reminds.len(), 2, "额外提醒时点须保留");
    assert!(got.constant_remind, "持续提醒开关须保留");

    let kept = g.todos.iter().find(|x| x.id == done.id).unwrap();
    assert!(kept.reminded, "已完成项不该被复位");
    assert_eq!(kept.reminded_keys, vec!["x".to_string()]);
}

#[test]
fn trash_restore_roundtrip_keeps_event_link() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l.clone(), "要扔进垃圾桶".into()).unwrap();
    let ev = st.todo_to_event(t.id.clone(), "2026-06-06".into()).unwrap();

    st.trash_todo(t.id.clone()).unwrap();
    let g = st.snapshot();
    assert!(g.todos.iter().all(|x| x.id != t.id), "软删除后不该出现在活动列表");
    assert_eq!(g.trashed.len(), 1, "应进回收站");
    assert_eq!(g.trashed[0].todo.id, t.id);
    assert!(!g.trashed[0].trashed_at.is_empty(), "应记下删除时刻");
    // 关键语义差异：trash **不**解绑日程（与 remove_todo 不同），
    // 否则恢复后用户要重连。
    assert_eq!(
        st.events_on("2026-06-06")[0].todo_id.as_deref(),
        Some(t.id.as_str()),
        "回收站期间应保持关联"
    );
    assert_eq!(st.events_on("2026-06-06")[0].id, ev.id);

    let back = st.restore_todo(t.id.clone()).unwrap().expect("应恢复");
    assert_eq!(back.list_id, l, "应回到原清单");
    let g2 = st.snapshot();
    assert!(g2.todos.iter().any(|x| x.id == t.id), "恢复后回到活动列表");
    assert!(g2.trashed.is_empty(), "回收站应已移出");
    assert_eq!(st.events_on("2026-06-06")[0].todo_id.as_deref(), Some(t.id.as_str()));
}

#[test]
fn trash_is_idempotent_and_purge_works() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "重复删除测试".into()).unwrap();
    st.trash_todo(t.id.clone()).unwrap();
    st.trash_todo(t.id.clone()).unwrap();
    assert_eq!(st.snapshot().trashed.len(), 1, "重复 trash 不该塞两条");

    st.purge_todo(t.id.clone()).unwrap();
    assert!(st.snapshot().trashed.is_empty(), "彻底删除应移出回收站");
    assert!(st.snapshot().todos.iter().all(|x| x.id != t.id));
    // 对不存在的 id 操作不该报错（幂等）
    st.purge_todo("不存在".into()).unwrap();
    st.trash_todo("不存在".into()).unwrap();
    assert!(st.restore_todo("不存在".into()).unwrap().is_none(), "不在回收站应返回 None");
}

#[test]
fn empty_trash_clears_all() {
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    for i in 0..3 {
        let t = st.add_todo(l.clone(), format!("待扔 {i}")).unwrap();
        st.trash_todo(t.id).unwrap();
    }
    assert_eq!(st.snapshot().trashed.len(), 3);
    assert_eq!(st.empty_trash().unwrap(), 3, "应报告清掉几条");
    assert!(st.snapshot().trashed.is_empty());
    assert_eq!(st.empty_trash().unwrap(), 0, "空了再清应为 0");
}

#[test]
fn restore_falls_back_when_list_deleted() {
    // 待办的清单被删后恢复：不该造出指向不存在清单的孤儿，落回第一个清单
    let (st, _d) = tmp_app();
    let l1 = first_list(&st);
    let l2 = st.add_list("第二个".into(), "#ffffff".into()).unwrap();
    let t = st.add_todo(l2.id.clone(), "在第二个清单".into()).unwrap();
    st.trash_todo(t.id.clone()).unwrap();
    // remove_list 只迁移 s.todos 里的，回收站那条的 list_id 会指向已删清单
    st.remove_list(l2.id.clone()).unwrap();

    let back = st.restore_todo(t.id).unwrap().expect("应能恢复");
    assert_eq!(back.list_id, l1, "清单没了应落回第一个清单");
    assert!(st.snapshot().lists.iter().any(|x| x.id == back.list_id));
}

#[test]
fn trim_trash_evicts_oldest_at_cap() {
    // 纯函数直接测裁剪边界。模板只建一次再 clone —— 若每个条目都开一个
    // tmp_app，1500 次落盘会把一个毫秒级测试拖成几十秒。
    use xhy_lib::domain::TrashedTodo;
    use xhy_lib::state::trim_trash;
    let (st, _d) = tmp_app();
    let tpl = st.add_todo(first_list(&st), "模板".into()).unwrap();
    let mk = |i: usize| TrashedTodo {
        todo: Todo { title: format!("t{i}"), ..tpl.clone() },
        trashed_at: "2026-10-17T10:00".into(),
    };
    // 未超限：不动
    let mut v: Vec<_> = (0..499).map(mk).collect();
    trim_trash(&mut v);
    assert_eq!(v.len(), 499, "未超限不该裁剪");
    // 正好等于上限：不动
    let mut at_cap: Vec<_> = (0..500).map(mk).collect();
    trim_trash(&mut at_cap);
    assert_eq!(at_cap.len(), 500, "等于上限不该裁剪");
    // 超限：丢最老的，保新的
    let mut over: Vec<_> = (0..505).map(mk).collect();
    trim_trash(&mut over);
    assert_eq!(over.len(), 500, "应截在上限");
    assert_eq!(over[0].todo.title, "t5", "最老的 5 条应被丢弃");
    assert_eq!(over[499].todo.title, "t504", "最新的应保留");
}

#[test]
fn calendar_and_summary_follow_selected_list() {
    // 用户报的 bug：新建空清单后在主窗口选中它，日历和总结显示的内容与
    // 选"收集箱"完全一致 —— 两个查询压根没有清单维度。
    let (st, _d) = tmp_app();
    let inbox = first_list(&st);
    let empty = st.add_list("待办".into(), "#4c8dff".into()).unwrap();
    let today = xhy_lib::now_local_date_str();
    let (wfrom, wto) = {
        use chrono::Datelike;
        let d = chrono::Local::now().date_naive();
        let mon = d - chrono::Duration::days(d.weekday().number_from_monday() as i64 - 1);
        (
            mon.format("%Y-%m-%d").to_string(),
            (mon + chrono::Duration::days(6)).format("%Y-%m-%d").to_string(),
        )
    };

    // 收集箱里一条带到期日的待办
    let t = st.add_todo(inbox.clone(), "写季度汇报".into()).unwrap();
    st.update_todo(Todo { due: Some(today.clone()), ..t.clone() }).unwrap();

    // ① 日历：选空清单不该看到收集箱的派生项
    let all = st.events_in_range(&wfrom, &wto, None);
    let sel_empty = st.events_in_range(&wfrom, &wto, Some(&empty.id));
    let sel_inbox = st.events_in_range(&wfrom, &wto, Some(&inbox));
    assert!(all.iter().any(|e| e.title == "写季度汇报"), "全局应含派生项");
    assert!(sel_inbox.iter().any(|e| e.title == "写季度汇报"), "选收集箱应含");
    assert!(
        sel_empty.iter().all(|e| e.title != "写季度汇报"),
        "选空清单不该混进别的清单的待办"
    );

    // ② 手工日程不属于任何清单：切清单它恒在（与日历同一规则）
    st.add_event(Event {
        id: String::new(),
        date: today.clone(),
        time: Some("09:00".into()),
        duration_min: 0,
        title: "站会".into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days: 1,
        span_index: 0,
        span_total: 1,
        list_id: None,
        recurrence: None,
    })
    .unwrap();
    let sel_empty2 = st.events_in_range(&wfrom, &wto, Some(&empty.id));
    assert!(sel_empty2.iter().any(|e| e.title == "站会"), "手工日程应恒显示");

    // ③ 拖上日历的待办：过滤跟着关联待办的清单走，否则"过滤时灵时不灵"
    let t2 = st.add_todo(inbox.clone(), "拖上去的".into()).unwrap();
    st.todo_to_event(t2.id.clone(), today.clone()).unwrap();
    let sel_empty3 = st.events_in_range(&wfrom, &wto, Some(&empty.id));
    assert!(sel_empty3.iter().all(|e| e.title != "拖上去的"), "拖成的日程按关联待办过滤");
    let sel_inbox3 = st.events_in_range(&wfrom, &wto, Some(&inbox));
    assert!(sel_inbox3.iter().any(|e| e.title == "拖上去的"));

    // ④ 总结同一口径：空清单的新建/遗留都应是 0
    let s_all = st.period_summary("week", None);
    let s_empty = st.period_summary("week", Some(&empty.id));
    let s_inbox = st.period_summary("week", Some(&inbox));
    assert!(s_all.stats.created > 0, "全局应有新建");
    assert_eq!(s_empty.stats.created, 0, "空清单不该有新建");
    assert_eq!(s_empty.stats.overdue_left, 0, "空清单不该有遗留");
    assert!(s_inbox.stats.overdue_left >= 1, "收集箱应有遗留（那条到期待办）");
    // 日程数：手工"站会"不属于清单、两边都算；拖上去的只算收集箱
    assert_eq!(s_empty.stats.event_count, 1, "手工日程恒计入");
    assert_eq!(s_inbox.stats.event_count, 2, "手工 + 拖上去的");
}

#[test]
fn lunar_recurrence_spawns_next_on_complete() {
    // recurring::next_due 的农历推进是纯函数测过的；这里测**接线**：
    // 完成一条农历重复待办，自动生成的下一条要落在正确的农历日，
    // 且规则要带着走（否则第二次完成就不再重复）。
    use xhy_lib::domain::{Freq, Recurrence};
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let mk_rc = |freq| Recurrence {
        freq,
        interval: 1,
        by_weekday: vec![],
        by_monthday: vec![],
        until: None,
        auto_create: true,
    };

    // 农历每月：2026-03-19 = 农历二月初一 → 三月初一 = 2026-04-17
    let mut t = st.add_todo(l.clone(), "给妈打电话".into()).unwrap();
    t.due = Some("2026-03-19".into());
    t.recurrence = Some(mk_rc(Freq::LunarMonthly));
    st.update_todo(t.clone()).unwrap();
    let next = st
        .set_done(t.id.clone(), true)
        .unwrap()
        .expect("auto_create 应生成下一条");
    assert_eq!(next.due.as_deref(), Some("2026-04-17"), "三月初一");
    assert!(!next.done);
    assert!(!next.reminded);
    assert_eq!(next.recurrence.as_ref().unwrap().freq, Freq::LunarMonthly);
    assert_ne!(next.id, t.id, "必须是新行，不是原地改");
    // 生成的新行确实进了库
    assert!(st.snapshot().todos.iter().any(|x| x.id == next.id));
    // 原行仍是已完成状态
    assert!(st.snapshot().todos.iter().find(|x| x.id == t.id).unwrap().done);

    // 农历每年：2026-01-01 = 农历十一月十三 → 次年同农历日 = 2026-12-21
    let mut b = st.add_todo(l, "农历生日".into()).unwrap();
    b.due = Some("2026-01-01".into());
    b.recurrence = Some(mk_rc(Freq::LunarYearly));
    st.update_todo(b.clone()).unwrap();
    let nb = st.set_done(b.id, true).unwrap().expect("应生成下一条");
    assert_eq!(nb.due.as_deref(), Some("2026-12-21"), "农历十一月十三的次年");
}

/// 造一个挂件（字段照 Widget 定义，避免猜）
fn widget(id: &str, visible: bool) -> Widget {
    Widget {
        id: id.into(),
        kind: WidgetKind::Todo,
        list_id: None,
        view: None,
        filter: None,
        tag: None,
        size: WidgetSize::M,
        rect: RectL { x: 0.0, y: 0.0, w: 300.0, h: 400.0 },
        visible,
        auto_dock: true,
        edge_hide: None,
        free: false,
    }
}

#[test]
fn stale_widget_snapshot_cannot_revive_hidden_widget() {
    // 用户两次报"✕ / 显隐点了没反应"的架构根因：前端 saveGeometry 换档时会发
    // `upsertWidget({...陈旧快照})`，整对象覆盖把刚设的 visible=false 写回 true
    // （lost update）。现在 upsert_widget 对已存在挂件**保留** visible。
    let (st, _d) = tmp_app();
    st.upsert_widget(widget("wX", true)).unwrap();

    // 收起（走字段级入口，等价于命令 set_widget_visible）
    st.set_widget_visible("wX", false).unwrap();
    assert!(!st.snapshot().widgets.iter().find(|w| w.id == "wX").unwrap().visible);

    // 模拟前端迟到的整对象写回：带 visible=true 的旧快照
    st.upsert_widget(widget("wX", true)).unwrap();
    let g = st.snapshot();
    let w = g.widgets.iter().find(|w| w.id == "wX").unwrap();
    assert!(!w.visible, "陈旧快照不得把已收起的挂件复活（lost update）");

    // 但其他字段必须照常更新（不能因保留 visible 就把 size/rect 也挡住）
    let mut moved = widget("wX", true);
    moved.rect.x = 777.0;
    moved.size = WidgetSize::Xl;
    st.upsert_widget(moved).unwrap();
    let g2 = st.snapshot();
    let w2 = g2.widgets.iter().find(|w| w.id == "wX").unwrap();
    assert_eq!(w2.rect.x, 777.0, "rect 须更新");
    assert_eq!(w2.size, WidgetSize::Xl, "size 须更新");
    assert!(!w2.visible, "更新其他字段时 visible 仍须保持");

    // 显式显示要能生效（走字段级入口）
    st.set_widget_visible("wX", true).unwrap();
    let g3 = st.snapshot();
    assert!(g3.widgets.iter().find(|w| w.id == "wX").unwrap().visible, "显示须生效");
}

#[test]
fn new_widget_keeps_incoming_visible() {
    // 保留 visible 只针对**已存在**挂件；新建时传入的值必须生效，
    // 否则"新建即隐藏"的挂件会变成永远可见。
    let (st, _d) = tmp_app();
    st.upsert_widget(widget("wNew", false)).unwrap();
    let g = st.snapshot();
    let w = g.widgets.iter().find(|w| w.id == "wNew").unwrap();
    assert!(!w.visible, "新挂件的 visible=false 须生效");
}

#[test]
fn patch_widget_only_touches_given_fields() {
    // 架构核心：字段级补丁。每个写者只碰自己那一个字段，
    // 并发时不会把别人刚改的字段抹回旧值（lost update）。
    use xhy_lib::domain::WidgetPatch;
    let (st, _d) = tmp_app();
    st.upsert_widget(widget("wP", true)).unwrap();

    // A：改 filter
    st.patch_widget(
        "wP",
        WidgetPatch { filter: Some(Some("today".into())), ..Default::default() },
    )
    .unwrap();
    // B：改 visible（模拟用户点收起）
    st.patch_widget(
        "wP",
        WidgetPatch { visible: Some(false), ..Default::default() },
    )
    .unwrap();
    // C：改 size（模拟前端换档，它并不知道自己读到的 filter/visible）
    st.patch_widget(
        "wP",
        WidgetPatch { size: Some(WidgetSize::Xl), ..Default::default() },
    )
    .unwrap();

    let g = st.snapshot();
    let w = g.widgets.iter().find(|x| x.id == "wP").unwrap();
    assert_eq!(w.filter.as_deref(), Some("today"), "A 的 filter 不能被 B/C 抹掉");
    assert!(!w.visible, "B 的 visible 不能被 C 抹掉");
    assert_eq!(w.size, WidgetSize::Xl, "C 的 size 须生效");
}

#[test]
fn patch_widget_double_option_distinguishes_unset_from_clear() {
    // `filter: None`（外层）= 不改；`filter: Some(None)` = 显式清空。
    // 这个区分是字段级补丁能替代整对象写入的前提，必须锁死。
    use xhy_lib::domain::WidgetPatch;
    let (st, _d) = tmp_app();
    st.upsert_widget(widget("wD", true)).unwrap();
    st.patch_widget(
        "wD",
        WidgetPatch {
            filter: Some(Some("week".into())),
            tag: Some(Some("急".into())),
            ..Default::default()
        },
    )
    .unwrap();
    let g = st.snapshot();
    let w = g.widgets.iter().find(|x| x.id == "wD").unwrap();
    assert_eq!(w.tag.as_deref(), Some("急"));

    // 只清 tag，filter 必须原样留着
    st.patch_widget("wD", WidgetPatch { tag: Some(None), ..Default::default() }).unwrap();
    let g2 = st.snapshot();
    let w2 = g2.widgets.iter().find(|x| x.id == "wD").unwrap();
    assert!(w2.tag.is_none(), "Some(None) 应清空 tag");
    assert_eq!(w2.filter.as_deref(), Some("week"), "未提及的 filter 不能被清掉");
}

#[test]
fn patch_widget_missing_id_is_silent() {
    // 前端可能持有已删挂件的引用并发补丁：静默忽略而非报错刷屏。
    // 断言"数量不变"而非某个具体数字 —— 播种几个挂件不是本测试的关切，
    // 写死数字会让测试与播种逻辑无谓耦合。
    use xhy_lib::domain::WidgetPatch;
    let (st, _d) = tmp_app();
    let before = st.snapshot().widgets.len();
    st.patch_widget("不存在", WidgetPatch { visible: Some(false), ..Default::default() })
        .unwrap();
    let after = st.snapshot().widgets;
    assert_eq!(after.len(), before, "不该凭空造出挂件");
    assert!(after.iter().all(|w| w.id != "不存在"));
}

#[test]
fn remove_list_moves_todos_instead_of_deleting_them() {
    // 审计实锤：旧代码"先迁移再 retain 删掉"，迁移是死代码，
    // 实际效果是删清单**连带删光其中全部待办**（数据丢失级 bug）。
    let (st, _d) = tmp_app();
    let l1 = first_list(&st);
    let l2 = st.add_list("要被删的".into(), "#fff".into()).unwrap();
    st.add_todo(l2.id.clone(), "会保住吗".into()).unwrap();
    st.add_todo(l2.id.clone(), "第二条".into()).unwrap();

    st.remove_list(l2.id.clone()).unwrap();
    let g = st.snapshot();
    assert!(g.lists.iter().all(|l| l.id != l2.id), "清单应被删除");
    assert_eq!(g.todos.len(), 2, "待办必须保住，不能被连带删掉");
    assert!(
        g.todos.iter().all(|t| t.list_id == l1),
        "待办应移入第一个剩余清单"
    );
}

#[test]
fn remove_list_keeps_at_least_one_list() {
    // 收集箱是数据锚点：恢复待办、新建待办都指向它，删空会让后续操作落空
    let (st, _d) = tmp_app();
    let only = first_list(&st);
    st.remove_list(only.clone()).unwrap();
    assert_eq!(st.snapshot().lists.len(), 1, "最后一个清单不该被删掉");
}

#[test]
fn set_list_archived_toggles_flag() {
    let (st, _d) = tmp_app();
    let l2 = st.add_list("归档候选".into(), "#fff".into()).unwrap();
    assert!(!st.snapshot().lists.iter().find(|l| l.id == l2.id).unwrap().archived);
    st.set_list_archived(&l2.id, true).unwrap();
    assert!(st.snapshot().lists.iter().find(|l| l.id == l2.id).unwrap().archived);
    st.set_list_archived(&l2.id, false).unwrap();
    assert!(!st.snapshot().lists.iter().find(|l| l.id == l2.id).unwrap().archived);
    // 不存在的 id 静默
    st.set_list_archived("不存在", true).unwrap();
}

// ── 日历上编辑"派生条目"：三个字段都得落到待办身上 ──────────

fn rc(freq: Freq) -> Recurrence {
    Recurrence {
        freq,
        interval: 1,
        by_weekday: vec![],
        by_monthday: vec![],
        until: None,
        auto_create: true,
    }
}

fn todo_of(st: &AppState, id: &str) -> Todo {
    st.snapshot().todos.iter().find(|t| t.id == id).unwrap().clone()
}

/// 造一条"带到期日（可选带定时提醒）"的待办。
/// 刻意不靠 quickparse 认全写 ISO 日期（它只支持 M月D日 / M-D），
/// 这些测试要锚定的是"日历编辑派生条目"，不是解析器。
fn dated_todo(st: &AppState, l: &str, title: &str, due: &str, hhmm: Option<&str>) -> Todo {
    let t = st.add_todo(l.into(), title.into()).unwrap();
    let remind_at = hhmm.map(|h| format!("{due}T{h}"));
    st.update_todo(Todo {
        due: Some(due.into()),
        remind_at,
        ..t.clone()
    })
    .unwrap();
    todo_of(st, &t.id)
}

#[test]
fn derived_entry_edit_changes_time_not_only_title() {
    // 弹窗标题写的是「格式 HH:MM 标题」、日视图行的 tooltip 写的是
    // 「双击改时间/标题」。此前 update_event 的 todo: 分支只落标题，时间
    // 那半被**静默丢弃**：用户把 09:30 改成 10:00，回车后日历上还是 09:30。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = dated_todo(&st, &l, "交周报", "2031-08-01", Some("09:30"));
    let derived = st.events_on("2031-08-01").remove(0);
    assert_eq!(derived.time.as_deref(), Some("09:30"), "派生条目显示提醒时刻");

    // 改时间（标题原样带着）
    st.update_event(Event {
        time: Some("10:00".into()),
        ..derived.clone()
    })
    .unwrap();
    let after = todo_of(&st, &t.id);
    assert_eq!(after.remind_at.as_deref(), Some("2031-08-01T10:00"), "时间应落到待办");
    assert!(!after.reminded, "改了提醒时间要复位触发位，否则新时点永不响");
    assert_eq!(after.title, "交周报", "只改时间不该动标题");

    // 删掉时间前缀 = 改成全天 = 取消定点提醒
    let derived2 = st.events_on("2031-08-01").remove(0);
    assert_eq!(derived2.time.as_deref(), Some("10:00"));
    st.update_event(Event {
        time: None,
        ..derived2
    })
    .unwrap();
    assert_eq!(todo_of(&st, &t.id).remind_at, None, "改全天应清掉提醒时间");
}

#[test]
fn derived_entry_edit_rejects_illegal_time_instead_of_silencing_reminder() {
    // 日历那个输入框是自由文本，shared/event-line.ts 刻意只认格式不校验范围
    // （"25:00" 也算通过）。日程存字符串无所谓；待办这边要是照收，
    // reminder::parse_min 解析失败 → 提醒**静默不响**，等于把原本能响的
    // 提醒改没了。非法值按"这次没改时间"处理。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = dated_todo(&st, &l, "交周报", "2031-08-01", Some("09:30"));
    let derived = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        time: Some("25:00".into()),
        ..derived.clone()
    })
    .unwrap();
    assert_eq!(
        todo_of(&st, &t.id).remind_at.as_deref(),
        Some("2031-08-01T09:30"),
        "非法时刻不该覆盖已设的提醒"
    );
    // 分钟越界同样拒绝
    let derived2 = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        time: Some("09:75".into()),
        ..derived2
    })
    .unwrap();
    assert_eq!(todo_of(&st, &t.id).remind_at.as_deref(), Some("2031-08-01T09:30"));
    // 合法值照常生效（别把校验做过头）
    let derived3 = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        time: Some("23:59".into()),
        ..derived3
    })
    .unwrap();
    assert_eq!(todo_of(&st, &t.id).remind_at.as_deref(), Some("2031-08-01T23:59"));
}

#[test]
fn derived_entry_edit_title_does_not_re_fire_reminder() {
    // 整对象回传时时间原样带着 —— 此时 remind_at 没变，就**不该**复位
    // reminded。否则"只改个标题"会把今天已经弹过的提醒再弹一次。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = dated_todo(&st, &l, "周报", "2031-08-01", Some("09:30"));
    st.update_todo(Todo {
        reminded: true,
        ..t.clone()
    })
    .unwrap();
    let derived = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        title: "改过的标题".into(),
        ..derived
    })
    .unwrap();
    let after = todo_of(&st, &t.id);
    assert_eq!(after.title, "改过的标题");
    assert_eq!(after.remind_at.as_deref(), Some("2031-08-01T09:30"));
    assert!(after.reminded, "时间没变就不该复位触发位（否则重复弹窗）");
}

#[test]
fn derived_entry_exposes_and_edits_todo_recurrence() {
    // ① 此前派生条目一律 recurrence: None → 日历上看不出"这条每天重复"；
    // ② 日视图的 ↻ 面板对派生条目也是静默无反应。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = dated_todo(&st, &l, "站会", "2031-08-01", None);
    st.set_recurrence(t.id.clone(), Some(rc(Freq::Daily))).unwrap();
    let derived = st.events_on("2031-08-01").remove(0);
    assert_eq!(
        derived.recurrence.as_ref().map(|r| r.freq),
        Some(Freq::Daily),
        "派生条目该带上待办的规则，日历上才看得见 ↻"
    );

    // 在日历上改成"每周" → 改的是待办的规则
    st.update_event(Event {
        recurrence: Some(rc(Freq::Weekly)),
        ..derived.clone()
    })
    .unwrap();
    assert_eq!(
        todo_of(&st, &t.id).recurrence.as_ref().map(|r| r.freq),
        Some(Freq::Weekly)
    );

    // ★ 反过来：只改标题（recurrence 原样回传）不该把规则清空
    let derived2 = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        title: "早会".into(),
        ..derived2
    })
    .unwrap();
    let after = todo_of(&st, &t.id);
    assert_eq!(after.title, "早会");
    assert!(after.recurrence.is_some(), "只改标题不得顺手清掉重复规则");

    // 显式选"不重复" → 能清掉（这就是 recurrence: None 的真实语义）
    let derived3 = st.events_on("2031-08-01").remove(0);
    st.update_event(Event {
        recurrence: None,
        ..derived3
    })
    .unwrap();
    assert!(todo_of(&st, &t.id).recurrence.is_none(), "选不重复该真的清掉");
}

#[test]
fn rescheduling_same_todo_moves_event_instead_of_duplicating() {
    // 用户动作：把同一条待办先拖到 5 号、再拖到 8 号（或连点两次「排到…」）。
    // 此前每次都 push 一条新日程 → 日历上 5 号和 8 号各挂着同一条待办。
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = st.add_todo(l, "见客户".into()).unwrap();

    let first = st.todo_to_event(t.id.clone(), "2031-03-05".into()).unwrap();
    assert_eq!(first.date, "2031-03-05");
    let second = st.todo_to_event(t.id.clone(), "2031-03-08".into()).unwrap();

    let g = st.snapshot();
    assert_eq!(g.events.len(), 1, "同一条待办第二次排期不该造出第二条日程");
    assert_eq!(second.id, first.id, "改期应作用于原来那条，返回值是落库后的真身");
    assert_eq!(second.date, "2031-03-08");
    assert_eq!(g.todos.iter().find(|x| x.id == t.id).unwrap().due.as_deref(), Some("2031-03-08"));
    // 旧日期上不该再有痕迹，新日期上有且只有一条
    assert_eq!(st.events_on("2031-03-05").len(), 0, "旧格子该清空");
    assert_eq!(st.events_on("2031-03-08").len(), 1);

    // 日程被 ✕ 删掉之后（待办还在），再排一次应该真的新建一条
    st.remove_event(first.id.clone()).unwrap();
    let third = st.todo_to_event(t.id.clone(), "2031-03-20".into()).unwrap();
    assert_eq!(st.snapshot().events.len(), 1);
    assert_ne!(third.id, first.id, "原日程已删：这次应当新建");
    assert_eq!(third.date, "2031-03-20");
}

#[test]
fn add_list_order_does_not_collide_after_delete() {
    // order 拿"条数"当序号会在删过中间清单后与现存序号撞号：
    // 两条 order 相同时排序结果取决于数组顺序，新建的清单可能插到中间。
    let (st, _d) = tmp_app();
    let a = st.add_list("A".into(), "#fff".into()).unwrap();
    let b = st.add_list("B".into(), "#fff".into()).unwrap();
    let c = st.add_list("C".into(), "#fff".into()).unwrap();
    assert_eq!((a.order, b.order, c.order), (1, 2, 3), "收集箱是 0");
    st.remove_list(b.id.clone()).unwrap();
    let d = st.add_list("D".into(), "#fff".into()).unwrap();
    let orders: Vec<i32> = st.snapshot().lists.iter().map(|l| l.order).collect();
    assert_eq!(d.order, 4, "新清单应接在最大序号之后");
    let mut sorted = orders.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), orders.len(), "清单序号不该重复：{orders:?}");
}

// ── 跨天日程（span_days）与时长（duration_min）──────────────

/// 造一条日程（可指定跨几天）
fn ev(date: &str, title: &str, span_days: u16) -> Event {
    Event {
        id: String::new(),
        date: date.into(),
        time: None,
        duration_min: 0,
        title: title.into(),
        color: String::new(),
        source: "manual".into(),
        todo_id: None,
        span_days,
        span_index: 0,
        span_total: 1,
        recurrence: None,
        list_id: None,
    }
}

fn dates_of(v: &[Event]) -> Vec<String> {
    v.iter().map(|e| e.date.clone()).collect()
}

#[test]
fn multi_day_event_covers_every_day_it_occupies() {
    // 以前 span_days 是"有字段没功能"：后端展开只看 date，
    // 跨 3 天的出差只在 1 号出现，2/3 号格子是空的。
    let (st, _d) = tmp_app();
    st.add_event(ev("2026-05-01", "出差", 3)).unwrap();
    for day in ["2026-05-01", "2026-05-02", "2026-05-03"] {
        let on = st.events_on(day);
        assert_eq!(on.len(), 1, "{day} 该看得到这条跨天日程");
        assert_eq!(on[0].title, "出差");
    }
    assert!(st.events_on("2026-05-04").is_empty(), "占满 3 天，第 4 天不该再有");

    // 起始日在查询窗口之前、尾巴伸进来 → 仍然要出现（否则"本月看不到上月就开始的
    // 跨月日程"，而这正是跨天最常见的形态）
    let tail = st.events_in_range("2026-05-02", "2026-05-05", None);
    assert_eq!(dates_of(&tail), vec!["2026-05-02", "2026-05-03"], "只发区间内的天");
}

#[test]
fn span_index_distinguishes_start_from_continuation() {
    // 前端要区分"起始日"（长条起笔 / 挂件标 ⬌N天）与"续日"（标 续）。
    // 窗口里看不到更早天时，前端自己算不出来，所以由展开结果带出。
    let (st, _d) = tmp_app();
    st.add_event(ev("2026-05-01", "出差", 3)).unwrap();
    let v = st.events_in_range("2026-05-01", "2026-05-03", None);
    assert_eq!(
        v.iter().map(|e| e.span_index).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "起始日 0、续日依次 1/2"
    );
    assert!(v.iter().all(|e| e.span_total == 3), "总天数要一起带出（画条宽度用）");

    let mid = st.events_in_range("2026-05-02", "2026-05-02", None);
    assert_eq!(mid[0].span_index, 1, "窗口里没有起始日时仍标得出这是第 2 天");
}

#[test]
fn recurring_and_span_combine_without_duplicates() {
    // 每天重复 + 跨 2 天：每一天仍然只有一条（自重叠去重），
    // 且"既是昨天那条的续日、又是今天新一轮的起始日"时按**起始日**算
    // （否则今天这条长条永远画不出来）。
    let (st, _d) = tmp_app();
    let mut e = ev("2026-06-01", "值班", 2);
    e.recurrence = Some(rc(Freq::Daily));
    st.add_event(e).unwrap();
    let v = st.events_in_range("2026-06-01", "2026-06-05", None);
    assert_eq!(
        dates_of(&v),
        vec!["2026-06-01", "2026-06-02", "2026-06-03", "2026-06-04", "2026-06-05"],
        "每天恰好一条：{:#?}",
        dates_of(&v)
    );
    assert!(
        v.iter().all(|x| x.span_index == 0),
        "起始日优先：{:?}",
        v.iter().map(|x| x.span_index).collect::<Vec<_>>()
    );
}

#[test]
fn event_span_and_duration_are_clamped_and_runtime_fields_not_stored() {
    let (st, _d) = tmp_app();
    let mut d = ev("2026-07-01", "评审", 1);
    d.duration_min = 45;
    assert_eq!(st.add_event(d).unwrap().duration_min, 45, "时长要落库（结束时刻靠它）");

    let mut big = ev("2026-07-02", "马拉松", 1);
    big.duration_min = u32::MAX;
    assert_eq!(st.add_event(big).unwrap().duration_min, 24 * 60, "时长上限一天");

    // 前端 updateEvent 是整对象回传，会把展开时带出的运行时字段一起发回来。
    // 照单全收就把"第 7 天"这种一次性上下文写进库，与 date 从此不一致。
    let mut w = ev("2026-07-03", "工程", 9999);
    w.span_index = 7;
    w.span_total = 4242;
    let saved = st.add_event(w).unwrap();
    assert_eq!(saved.span_days, 60, "跨天数上限 60 天（否则区间展开循环爆掉）");
    assert_eq!(saved.span_index, 0, "运行时字段不该落库");
    let g = st
        .snapshot()
        .events
        .into_iter()
        .find(|x| x.id == saved.id)
        .unwrap();
    assert_eq!((g.span_index, g.span_total), (0, 60), "库里只留业务字段");
}

#[test]
fn derived_todo_entries_never_span_days() {
    // 待办派生条目只占到期那一天（重复由待办本身推进）
    let (st, _d) = tmp_app();
    let l = first_list(&st);
    let t = dated_todo(&st, &l, "交周报", "2031-08-01", None);
    let on = st.events_on("2031-08-01");
    assert_eq!(on[0].id, format!("todo:{}", t.id));
    assert_eq!((on[0].span_days, on[0].span_index, on[0].span_total), (1, 0, 1));
    assert!(st.events_on("2031-08-02").is_empty(), "派生条目不往后占天");
}

// ── Markdown 导入（第三方待办工具导出的常见排版）──────────────────
//
// 样本形状对着真实第三方导出：`## YYYY-MM-DD` 日期分组 + `- [x]` 条目，
// 标题后用半角 `: ` 带备注。条目内容全部是合成的。

const THIRD_PARTY_MD: &str = "## 2026-08-18\n\n- [x] 交季度报表\n- [x] 约客户看方案\n\n## 2026-10-19\n\n- [ ] 出差上海10月19日-10月23日\n- [ ] 整理调研材料: 初稿已收到\n";

#[test]
fn import_markdown_merges_date_groups_into_fallback_list() {
    let (st, _d) = tmp_app();
    let before = st.snapshot().todos.len();
    let stat = st.import_markdown(THIRD_PARTY_MD, "导入待办").unwrap();
    assert_eq!((stat.lists, stat.added, stat.dup), (1, 4, 0), "两个日期分组共用一个兜底清单");
    assert_eq!(stat.empty, 0, "这份样本没有空条目");
    let g = st.snapshot();
    let target = g.lists.iter().find(|l| l.name == "导入待办").expect("兜底清单建出来了");
    let got: Vec<_> = g.todos.iter().filter(|t| t.list_id == target.id).collect();
    assert_eq!(got.len(), 4);
    // 分组日期落成每条的到期日
    assert!(got.iter().any(|t| t.due.as_deref() == Some("2026-08-18") && t.done));
    let open = got.iter().find(|t| t.title == "整理调研材料").unwrap();
    assert_eq!(open.due.as_deref(), Some("2026-10-19"));
    assert_eq!(open.note, "初稿已收到", "半角冒号后的备注要留住");
    assert!(!open.done);
    // 已完成条目按分组日期补 doneAt（导出里没有完成时间，这是推断，但不写就永远进不了归档与趋势）
    let done = got.iter().find(|t| t.title == "交季度报表").unwrap();
    assert_eq!(done.done_at.as_deref(), Some("2026-08-18T12:00"));
    // 原有数据一条没少（合并语义，不是整库替换）
    assert_eq!(st.snapshot().todos.len(), before + 4);
}

#[test]
fn import_markdown_is_idempotent_and_merges_same_named_list() {
    let (st, _d) = tmp_app();
    let seed = st.snapshot().lists[0].name.clone(); // 「收集箱」
    let md = format!("## {seed}\n\n- [ ] 只应出现一次\n\n## 2026-10-20\n\n- [ ] 另一条\n");
    let s1 = st.import_markdown(&md, "导入待办").unwrap();
    assert_eq!((s1.lists, s1.added, s1.dup), (1, 2, 0), "收集箱命中现有清单，只新建兜底那一个");
    let s2 = st.import_markdown(&md, "导入待办").unwrap();
    assert_eq!((s2.lists, s2.added, s2.dup), (0, 0, 2), "再导一次：不新增清单、不重复条目");
    let g = st.snapshot();
    assert_eq!(g.todos.iter().filter(|t| t.title == "只应出现一次").count(), 1);
    assert_eq!(g.lists.iter().filter(|l| l.name == seed).count(), 1, "同名单清单不许建两遍");
}

#[test]
fn import_markdown_does_not_resurface_past_reminders() {
    let (st, _d) = tmp_app();
    let md = "- [ ] 历史提醒 · 🔔 2020-01-02T09:00\n- [ ] 将来提醒 · 🔔 2099-01-02T09:00\n- [ ] 无提醒\n";
    st.import_markdown(md, "导入待办").unwrap();
    let g = st.snapshot();
    let past = g.todos.iter().find(|t| t.title == "历史提醒").unwrap();
    assert!(past.reminded, "提醒时刻已过 → 标成已提醒，否则一次导入几十条会把通知点燃");
    let future = g.todos.iter().find(|t| t.title == "将来提醒").unwrap();
    assert!(!future.reminded, "还没到的提醒要留着能响");
    assert_eq!(future.remind_at.as_deref(), Some("2099-01-02T09:00"));
    // **没有提醒的条目不许预先打上"已提醒"**（Kite 那种只给日期不给时间的就是这情形）
    let plain = g.todos.iter().find(|t| t.title == "无提醒").unwrap();
    assert!(plain.remind_at.is_none());
    assert!(!plain.reminded, "什么都没弹过就不能记成已弹过");
}

#[test]
fn import_markdown_rejects_text_without_any_item() {
    let (st, _d) = tmp_app();
    let before = st.snapshot().todos.len();
    let e = st.import_markdown("# 只有标题\n\n普通段落\n", "导入待办").unwrap_err();
    assert!(e.contains("没解析出"), "got {e}");
    assert_eq!(st.snapshot().todos.len(), before, "解析失败不许留下半个空清单");
}
