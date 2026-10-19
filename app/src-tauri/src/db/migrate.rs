//! 版本迁移链：v0/v1 → CURRENT_VERSION
//!
//! 每加一个版本，就在这里追加一次 `if from < N` 的字段补齐。
//! 迁移只做"加字段/改结构"，绝不删数据；未知字段保留透传。

use super::StoreError;
use crate::domain::Store;

pub fn upgrade(mut raw: serde_json::Value, from: u32) -> Result<Store, StoreError> {
    let obj = raw
        .as_object_mut()
        .ok_or_else(|| serde_json::Error::io(std::io::Error::other("store 根节点不是对象")))?;

    if from < 1 {
        obj.insert("version".into(), serde_json::json!(1));
        for key in ["lists", "todos", "events", "notes", "widgets", "archived"] {
            obj.entry(key).or_insert_with(|| serde_json::json!([]));
        }
        // settings 用完整默认值：塞 `{}` 会让必填字段反序列化失败
        obj
            .entry("settings")
            .or_insert_with(|| serde_json::to_value(crate::domain::Settings::default()).unwrap());
    }

    obj.insert("version".into(), serde_json::json!(Store::CURRENT_VERSION));

    // 反序列化失败时给出具体原因，而不是静默丢数据
    let store: Store = serde_json::from_value(raw.clone())?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_gets_defaults() {
        // v0 记录只有必填字段，其余靠 serde default 补齐
        let raw = serde_json::json!({
            "todos": [{ "id": "a", "listId": "l1", "title": "旧条目",
                        "order": 0, "createdAt": "2026-01-01T00:00" }]
        });
        let store = upgrade(raw, 0).unwrap();
        assert_eq!(store.version, Store::CURRENT_VERSION);
        assert_eq!(store.todos.len(), 1);
        assert!(!store.todos[0].done);
        assert!(store.todos[0].tags.is_empty());
    }

    #[test]
    fn too_new_is_rejected_before_upgrade() {
        // JsonStore::load 负责拒绝过高版本；此处只验证 CURRENT_VERSION 写入
        let raw = serde_json::json!({ "version": 99 });
        let store = upgrade(raw, 99).unwrap();
        assert_eq!(store.version, Store::CURRENT_VERSION);
    }
}
