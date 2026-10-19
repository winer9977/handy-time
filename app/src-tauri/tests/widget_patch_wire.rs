//! 锁死 WidgetPatch 的 JSON 边界语义：字段缺失=不改，显式 null=清空。
//! 上一版用 Option<Option<String>> 时，serde 把 `{"listId":null}` 解析成
//! **外层 None**（=不改），于是前端「全」按钮（发 listId:null）静默失效。
//! 这个测试就是那次翻车的固化。
use xhy_lib::domain::WidgetPatch;

fn parse(s: &str) -> WidgetPatch {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("解析失败 {s}: {e}"))
}

#[test]
fn json_null_means_clear_not_skip() {
    let p = parse(r#"{"listId":null}"#);
    assert!(
        p.list_id.is_some(),
        "`listId:null` 必须被识别为**显式清空**，不能当成「没提这个字段」"
    );
}

#[test]
fn json_absent_means_skip() {
    let p = parse(r#"{"visible":true}"#);
    assert!(p.list_id.is_none(), "未提及的字段必须保持不动");
    assert!(p.tag.is_none());
    assert!(p.filter.is_none());
    assert!(p.edge_hide.is_none());
    assert_eq!(p.visible, Some(true));
}

#[test]
fn json_value_means_set() {
    let p = parse(r#"{"listId":"inbox","tag":"急","size":"l","free":true}"#);
    assert_eq!(p.list_id.as_ref().and_then(|o| o.as_ref()).map(String::as_str), Some("inbox"));
    assert_eq!(p.tag.as_ref().and_then(|o| o.as_ref()).map(String::as_str), Some("急"));
    assert_eq!(p.free, Some(true));
}

#[test]
fn all_nullable_fields_share_the_semantics() {
    // 前端会发 null 清空的不止 listId：tag（清除筛选）、edgeHide（贴边切换）、
    // filter（回 all）、view。漏一个就是又一个"点了没反应"。
    for key in ["listId", "tag", "filter", "edgeHide", "view"] {
        let p = parse(&format!(r#"{{"{key}":null}}"#));
        let present = match key {
            "listId" => p.list_id.is_some(),
            "tag" => p.tag.is_some(),
            "filter" => p.filter.is_some(),
            "edgeHide" => p.edge_hide.is_some(),
            "view" => p.view.is_some(),
            _ => unreachable!(),
        };
        assert!(present, "{key}:null 必须解析为清空");
    }
}
