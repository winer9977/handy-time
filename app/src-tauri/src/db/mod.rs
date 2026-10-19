//! JSON 文件存储：原子写 + 备份轮转 + 版本迁移
//!
//! 数据目录：`%APPDATA%\dev.xhy.desktop\data\`
//!   store.json      当前数据（原子替换）
//!   backups\        轮转备份 store-<ts>.json，保留 settings.backup_keep 份
//!   archive\        归档分片 archive-YYYY-MM.jsonl

pub mod migrate;

use crate::domain::{Store, Todo};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("读写数据文件失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("数据格式解析失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("数据版本 {found} 高于本程序支持的 {supported}，请升级程序")]
    TooNew { found: u32, supported: u32 },
}

pub struct JsonStore {
    root: PathBuf,
    store: PathBuf,
    backups: PathBuf,
    archive: PathBuf,
}

impl JsonStore {
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        let s = Self {
            root: root.clone(),
            store: root.join("store.json"),
            backups: root.join("backups"),
            archive: root.join("archive"),
        };
        fs::create_dir_all(&s.backups)?;
        fs::create_dir_all(&s.archive)?;
        Ok(s)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 备份目录（测试与"打开数据目录"用）
    pub fn backups(&self) -> &Path {
        &self.backups
    }

    /// 读取；文件不存在则落一份带默认清单的种子数据。
    ///
    /// 坏数据不致命：主文件解析失败时按时间倒序回退到备份，
    /// 并把坏文件**改名保留**（绝不删除用户数据）。全部失败才开新档。
    /// 只有"版本比程序新"才硬失败 —— 那是降级安装，回退备份也救不回来，
    /// 静默开新档会让人以为数据丢了。
    pub fn load(&self) -> Result<Store, StoreError> {
        if !self.store.exists() {
            let seed = Store::seeded();
            self.write_raw(&seed)?;
            return Ok(seed);
        }
        let text = fs::read_to_string(&self.store)?;
        match self.parse(&text) {
            Ok(store) => Ok(store),
            // 可恢复的解析失败：留档 + 回退备份
            Err(e) if !matches!(e, StoreError::TooNew { .. }) => {
                let quarantined = self.quarantine()?;
                tracing::error!("store.json 解析失败（{e}），已留档为 {quarantined}，尝试备份回退");
                if let Some(s) = self.load_backups()? {
                    self.write_raw(&s)?;
                    return Ok(s);
                }
                let seed = Store::seeded();
                self.write_raw(&seed)?;
                Ok(seed)
            }
            Err(e) => Err(e),
        }
    }

    /// 解析 + 版本迁移（不写盘）
    fn parse(&self, text: &str) -> Result<Store, StoreError> {
        let raw: serde_json::Value = serde_json::from_str(text)?;
        let found = raw
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if found > Store::CURRENT_VERSION {
            return Err(StoreError::TooNew {
                found,
                supported: Store::CURRENT_VERSION,
            });
        }
        let store = migrate::upgrade(raw, found)?;
        Ok(store)
    }

    /// 把坏文件改名保留，返回新文件名
    fn quarantine(&self) -> Result<String, StoreError> {
        let name = format!("store.corrupt-{}.json", crate::now_local_stamp());
        let dest = self.root.join(&name);
        fs::rename(&self.store, &dest)?;
        Ok(name)
    }

    /// 按**文件修改时间**倒序尝试每个备份（最新的先试）。
    ///
    /// 不用文件名排序：文件名含定长时间戳，但同秒备份会带 `-1`/`-2` 后缀，
    /// 而 ASCII 里 `-`(0x2D) 小于 `.`(0x2E)，"字典序即时间序"的前提就破了
    /// （实测：加到后缀后 corrupt_store_falls_back_to_backup 立刻红，
    ///   恢复出来的是旧那份）。改用 mtime 排序，与文件名格式无关，稳。
    fn load_backups(&self) -> Result<Option<Store>, StoreError> {
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(&self.backups)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let n = p.file_name()?.to_str()?.to_string();
                if !(n.starts_with("store-") && n.ends_with(".json")) {
                    return None;
                }
                let t = e.metadata().ok()?.modified().ok()?;
                Some((t, p))
            })
            .collect();
        // 排序规则（两级，缺一不可）：
        //  ① 修改时间新的在前 —— 主序；
        //  ② mtime 相同时（同毫秒内连续写盘实测会发生），带 `-n` 后缀的更新
        //     （后缀是"同秒撞名时的递增序号"，越大的越晚创建）。
        // 只靠 ① 会平局后按 read_dir 顺序取，恢复到旧那份（实测踩到）；
        // 只靠文件名 ② 又会被 `-` < `.` 破坏。两级合起来才稳。
        files.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| backup_seq(&b.1).cmp(&backup_seq(&a.1)))
        });
        for (_, f) in files {
            let Ok(text) = fs::read_to_string(&f) else {
                continue;
            };
            match self.parse(&text) {
                Ok(s) => {
                    tracing::warn!("已从备份恢复：{}", f.display());
                    return Ok(Some(s));
                }
                Err(e) => {
                    tracing::warn!("备份 {} 不可用：{e}", f.display());
                }
            }
        }
        Ok(None)
    }

    /// 保存：先备份当前磁盘内容，再原子替换。
    ///
    /// 每次写盘都备份（**语义就是"本次写入前的那份"**，有测试锁住：
    /// corrupt_store_falls_back_to_backup 要求坏文件后能恢复出用户数据）。
    /// 曾试过"按分钟节流"省 IO，会让备份变成 1 分钟前的陈旧快照，
    /// 该测试立刻红了 —— 推测性的性能优化不该改既有语义，已回退。
    /// IO 成本记为已知取舍：备份是救命稻草，宁可多写几次。
    ///
    /// `store.json` 本身是原子的（写临时文件 → fsync → rename）。
    pub fn write(&self, store: &Store) -> Result<(), StoreError> {
        if self.store.exists() {
            self.rotate_backup()?;
        }
        self.write_raw(store)
    }

    /// 原子写：写临时文件 → fsync → rename
    fn write_raw(&self, store: &Store) -> Result<(), StoreError> {
        let tmp = self.store.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(store)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.store)?;
        Ok(())
    }

    fn rotate_backup(&self) -> Result<(), StoreError> {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let mut dest = self.backups.join(format!("store-{ts}.json"));
        // 同名即已存在（同秒/同分钟又触发）→ 加后缀避免静默覆盖上一次备份。
        // 备份是"救命稻草"，覆盖掉等于没有。
        if dest.exists() {
            let mut n = 1;
            loop {
                let cand = self
                    .backups
                    .join(format!("store-{ts}-{n}.json"));
                if !cand.exists() {
                    dest = cand;
                    break;
                }
                n += 1;
                if n > 99 {
                    break; // 极端情况就放弃这次备份，别死循环
                }
            }
        }
        fs::copy(&self.store, &dest)?;
        let keep = self
            .keep_count()
            .unwrap_or(10)
            .max(1) as usize;
        let mut entries: Vec<PathBuf> = fs::read_dir(&self.backups)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("store-") && n.ends_with(".json"))
                    .unwrap_or(false)
            })
            .collect();
        // 文件名含定长时间戳，字典序 == 时间序（"-n" 后缀的排在同时刻之后，
        // 也正是它被创建的次序，符合"老的先删"）
        entries.sort();
        while entries.len() > keep {
            let old = entries.remove(0);
            let _ = fs::remove_file(old);
        }
        Ok(())
    }

    /// 备份保留份数：从磁盘上直接读，避免 rotate 时反序列化整个 store
    fn keep_count(&self) -> Option<u8> {
        let text = fs::read_to_string(&self.store).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get("settings")?
            .get("backupKeep")?
            .as_u64()
            .map(|n| n.clamp(1, 100) as u8)
    }

    /// 归档分片里的待办（周/月总结要统计历史完成项，主表已把它们移走）。
    /// 读不动的分行直接跳过 —— 归档是 append-only 日志，坏行不该让统计崩掉。
    pub fn archive_todos(&self) -> Vec<crate::domain::Todo> {
        let Ok(rd) = fs::read_dir(&self.archive) else {
            return vec![];
        };
        let mut out = Vec::new();
        for path in rd.filter_map(|e| e.ok().map(|e| e.path())) {
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                if let Ok(t) = serde_json::from_str::<crate::domain::Todo>(line) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// 追加归档分片（JSON Lines，一年一文件按月分）
    pub fn append_archive(&self, items: &[Todo]) -> Result<(), StoreError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut by_month: std::collections::BTreeMap<String, Vec<&Todo>> = Default::default();
        for t in items {
            let key = t
                .archived_at
                .as_deref()
                .or(t.done_at.as_deref())
                .and_then(|s| s.get(..7))
                .unwrap_or("unknown")
                .replace('-', "");
            by_month.entry(key).or_default().push(t);
        }
        for (month, list) in by_month {
            let path = self.archive.join(format!("archive-{month}.jsonl"));
            let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
            for t in list {
                writeln!(f, "{}", serde_json::to_string(t)?)?;
            }
            f.flush()?;
        }
        Ok(())
    }

    /// 导入：整体替换，替换前强制备份当前数据
    pub fn replace_with(&self, store: &Store) -> Result<(), StoreError> {
        if self.store.exists() {
            self.rotate_backup()?;
        }
        self.write_raw(store)
    }

    /// 导出为格式化 JSON 文本
    pub fn export_json(&self, store: &Store) -> Result<String, StoreError> {
        Ok(serde_json::to_string_pretty(store)?)
    }
}

/// 从备份文件名提取"同秒序号"：`store-20261018-120000.json` → 0，
/// `store-20261018-120000-3.json` → 3（同秒撞名时递增创建，越大越新）。
///
/// 注意不能直接 `rsplit_once('-')` 后 parse：无后缀时会把时间戳里的
/// `120000` 解析成一个巨大的序号，反而被判成"最新"。所以先按 `-` 的
/// 段数判断 —— 文件名固定是 `store-<8位日期>-<6位时间>[-<序号>]`，
/// 段数 > 4 才说明存在序号段。
fn backup_seq(p: &Path) -> u32 {
    let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
        return 0;
    };
    let parts: Vec<&str> = stem.split('-').collect();
    // store / 20261018 / 120000 → 3 段；带序号是 4 段
    if parts.len() <= 3 {
        return 0;
    }
    parts.last().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &Path) -> JsonStore {
        JsonStore::open(dir.to_path_buf()).unwrap()
    }

    #[test]
    fn backup_seq_reads_suffix_not_timestamp() {
        // 关键陷阱：无后缀的文件名里也有 `-`，直接把最后一段 parse 会把
        // 时间戳的 `120000` 当成巨大序号，把最老的备份判成最新。
        assert_eq!(backup_seq(Path::new("store-20261018-120000.json")), 0);
        assert_eq!(backup_seq(Path::new("store-20261018-120000-1.json")), 1);
        assert_eq!(backup_seq(Path::new("store-20261018-120000-12.json")), 12);
        // 倒序比较：序号大的应排在前面（更新）
        assert!(backup_seq(Path::new("store-20261018-120000-2.json"))
            > backup_seq(Path::new("store-20261018-120000.json")));
    }

    #[test]
    fn same_second_backups_do_not_collide() {
        // 秒级时间戳 + 高频写盘会撞同名文件，静默覆盖掉上一次备份。
        // 备份是救命稻草，被覆盖等于没有。
        let d = tempfile::tempdir().unwrap();
        let s = store_in(d.path());
        let mut st = s.load().unwrap();
        // 连写多次：同一秒内必然多次触发备份
        for i in 0..5 {
            st.lists.push(crate::domain::TodoList {
                id: format!("l{i}"),
                name: format!("第{i}个"),
                color: "#000".into(),
                order: i,
                collapsed: false,
                archived: false,
                exclude_from_stats: false,
            });
            s.write(&st).unwrap();
        }
        let n = fs::read_dir(s.backups())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("store-"))
            .count();
        assert!(
            n >= 2,
            "连续 5 次写盘应留下多份备份（同秒也不能互相覆盖），实得 {n} 份"
        );
    }

    #[test]
    fn restore_picks_newest_backup_even_with_suffix() {
        // 带 `-n` 后缀的同秒备份会破坏"文件名即时间序"的前提
        // （ASCII 里 `-` < `.`）。恢复必须按**修改时间**取最新，
        // 否则可能恢复到更旧的那份，丢掉用户最近的改动。
        //
        // 注意备份语义是"**本次写入前的那份**"：想让某个版本进备份，
        // 必须在写入它之后再写一次（第一次写它时它还在主文件里）。
        let d = tempfile::tempdir().unwrap();
        let s = store_in(d.path());
        let mut st = s.load().unwrap();
        st.lists.push(crate::domain::TodoList {
            id: "old".into(),
            name: "旧内容".into(),
            color: "#000".into(),
            order: 0,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s.write(&st).unwrap(); // 备份=种子
        st.lists.push(crate::domain::TodoList {
            id: "new".into(),
            name: "新内容".into(),
            color: "#000".into(),
            order: 1,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s.write(&st).unwrap(); // 备份=旧内容
        // 再写一次，把"旧内容+新内容"这份推进备份
        st.lists.push(crate::domain::TodoList {
            id: "newer".into(),
            name: "更新的".into(),
            color: "#000".into(),
            order: 2,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s.write(&st).unwrap(); // 备份=旧内容+新内容

        // 写坏主文件 → 必须恢复到最新那份备份（含"新内容"）
        fs::write(d.path().join("store.json"), "{ 坏掉了").unwrap();
        let got = store_in(d.path()).load().unwrap();
        assert!(
            got.lists.iter().any(|l| l.name == "新内容"),
            "应恢复到最新备份（含新内容），实得 {:?}",
            got.lists.iter().map(|l| &l.name).collect::<Vec<_>>()
        );
        assert!(
            !got.lists.iter().any(|l| l.name == "更新的"),
            "不该拿到比最新备份更新的内容（它从未进过备份）"
        );
    }

    #[test]
    fn corrupt_store_falls_back_to_backup() {
        let d = tempfile::tempdir().unwrap();
        let s = store_in(d.path());
        // 先正常写一份数据并触发一次备份
        let mut st = s.load().unwrap();
        st.lists.push(crate::domain::TodoList {
            id: "l1".into(),
            name: "重要清单".into(),
            color: "#30a46c".into(),
            order: 0,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s.write(&st).unwrap();
        // 再写一次，使上一次的内容进入 backups/
        let mut st2 = st.clone();
        st2.lists.push(crate::domain::TodoList {
            id: "l2".into(),
            name: "后来的".into(),
            color: "#000".into(),
            order: 1,
            collapsed: false,
            archived: false,
            exclude_from_stats: false,
        });
        s.write(&st2).unwrap();

        // 把主文件写坏
        fs::write(d.path().join("store.json"), "{ 这不是 JSON").unwrap();

        let reopened = store_in(d.path());
        let got = reopened.load().unwrap();
        assert!(
            got.lists.iter().any(|l| l.name == "重要清单" || l.name == "后来的"),
            "应能从备份恢复清单，实得 {:?}",
            got.lists.iter().map(|l| &l.name).collect::<Vec<_>>()
        );
        // 坏文件必须留档，不能删
        let kept = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("store.corrupt-"));
        assert!(kept, "坏数据应改名保留而非删除");
        // 主文件已恢复成合法 JSON
        assert!(store_in(d.path()).load().is_ok());
    }

    #[test]
    fn corrupt_store_with_no_backups_starts_fresh_but_keeps_evidence() {
        let d = tempfile::tempdir().unwrap();
        let s = store_in(d.path());
        s.load().unwrap();
        fs::write(d.path().join("store.json"), "}".to_string()).unwrap();
        // 清掉备份，模拟最坏情况
        for e in fs::read_dir(s.backups()).unwrap().filter_map(|e| e.ok()) {
            let _ = fs::remove_file(e.path());
        }
        let got = store_in(d.path()).load().unwrap();
        assert_eq!(got.lists.len(), 1, "应回落到种子数据");
        assert_eq!(got.lists[0].name, "收集箱");
    }

    #[test]
    fn too_new_version_still_fails_loudly() {
        let d = tempfile::tempdir().unwrap();
        let s = store_in(d.path());
        s.load().unwrap();
        let big = Store::CURRENT_VERSION + 7;
        fs::write(
            d.path().join("store.json"),
            format!(r#"{{"version":{big}}}"#),
        )
        .unwrap();
        let e = store_in(d.path()).load().unwrap_err();
        assert!(
            matches!(e, StoreError::TooNew { found, .. } if found == big),
            "降级安装应硬失败而非静默开新档：{e}"
        );
    }

    #[test]
    fn wire_format_matches_frontend_contract() {
        // 前端 types.ts 依赖这些小写变体名；改了就等于写坏用户 store
        assert_eq!(
            serde_json::to_value(crate::domain::AttachMode::WorkerW).unwrap(),
            "workerw"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::AttachMode::BottomZ).unwrap(),
            "bottomz"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::CalendarView::Month).unwrap(),
            "month"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::WidgetSize::Xl).unwrap(),
            "xl"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::Freq::Workday).unwrap(),
            "workday"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::CountdownMode::Until).unwrap(),
            "until"
        );
        // 农历两项靠**显式 rename** 保持 camelCase：rename_all="lowercase"
        // 会给出 "lunarmmonthly" 这种无分隔名，前端 types.ts 极易写错且
        // 反序列化静默失败（本项目 attach_mode 就栽过大小写）。
        assert_eq!(
            serde_json::to_value(crate::domain::Freq::LunarMonthly).unwrap(),
            "lunarMonthly"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::Freq::LunarYearly).unwrap(),
            "lunarYearly"
        );
        // 官方工作日同样显式 rename（holiday 是单词，lowercase 就够）
        assert_eq!(
            serde_json::to_value(crate::domain::Freq::OfficialWorkday).unwrap(),
            "officialWorkday"
        );
        assert_eq!(
            serde_json::to_value(crate::domain::Freq::Holiday).unwrap(),
            "holiday"
        );
        let ow: crate::domain::Freq = serde_json::from_str(r#""officialWorkday""#).unwrap();
        assert_eq!(ow, crate::domain::Freq::OfficialWorkday);
        let hd: crate::domain::Freq = serde_json::from_str(r#""holiday""#).unwrap();
        assert_eq!(hd, crate::domain::Freq::Holiday);
        let lm: crate::domain::Freq = serde_json::from_str(r#""lunarMonthly""#).unwrap();
        assert_eq!(lm, crate::domain::Freq::LunarMonthly);
        let ly: crate::domain::Freq = serde_json::from_str(r#""lunarYearly""#).unwrap();
        assert_eq!(ly, crate::domain::Freq::LunarYearly);
        // 反方向也要能读回（前端会原样回传）
        let m: crate::domain::AttachMode = serde_json::from_str(r#""workerw""#).unwrap();
        assert_eq!(m, crate::domain::AttachMode::WorkerW);
    }
}
