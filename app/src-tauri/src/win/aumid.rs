//! AUMID（AppUserModelID）注册：让 Windows 通知上那行"来源"显示产品名
//!
//! **为什么要动注册表**：通知的**标题**是我们写的（`手边 待办提醒`），但 toast 顶部
//! 那行"来源"是 Windows 按 **AUMID 的注册显示名**渲染的 —— 没注册时它退回显示
//! **exe 文件名**。我们的 exe 仍叫 `xhy.exe`（那名字来自 Cargo 的 bin 名，
//! `productName` 改不了它，见 README），所以用户看到的是"xhy"而不是"手边"。
//! 光改代码里的 title 治不了这一行 —— 上一轮就是这么漏的。
//!
//! tauri-plugin-notification 用的 AUMID 就是 `config().identifier`
//! （`dev.xhy.desktop`），且**只在"已安装"路径下**才把它设进 toast
//! （`target/debug|release` 下跑不设），所以开发期直跑看不到这个问题。
//!
//! 只写 `HKCU`（当前用户，免管理员），幂等：每次启动重写同样两个值。
//! 失败不影响功能（提醒照样弹，只是来源名沿用 exe 名），所以调用方只记日志。

/// 要写的键值。`IconUri` 让通知中心里的图标也跟着走，不给就用 exe 自带图标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity<'a> {
    pub aumid: &'a str,
    pub display_name: &'a str,
    pub icon_path: Option<&'a str>,
}

/// 生成要落盘的注册表操作清单（**纯函数**：路径拼接与转义规则要能单测）。
///
/// 返回 `(子键路径, [(值名, 值), ...])`。真正的 Win32 调用留在 `apply` 里，
/// 这样"键名拼错/值名拼错"这类问题不必真开注册表就能抓到。
pub fn plan<'a>(id: &'a Identity<'a>) -> (String, Vec<(&'a str, String)>) {
    let key = format!("Software\\Classes\\AppUserModelId\\{}", id.aumid);
    let mut vals: Vec<(&str, String)> = vec![("DisplayName", id.display_name.to_string())];
    if let Some(icon) = id.icon_path {
        vals.push(("IconUri", icon.to_string()));
    }
    (key, vals)
}

/// 写注册表（Windows）。返回实际写下的键路径，给日志用。
#[cfg(windows)]
pub fn apply(id: &Identity<'_>) -> Result<String, String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
    };

    let (key, vals) = plan(id);
    // 注册表宽字符接口要 NUL 结尾的 UTF-16
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let key_w = wide(&key);

    unsafe {
        let mut hk = HKEY::default();
        // 用 RegCreateKeyW 而不是 RegCreateKeyExW：后者多一个 SECURITY_ATTRIBUTES 参数，
        // 会把这个函数关在 `Win32_Security` feature 后面（为一个键多开一个 feature 不值）。
        RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(key_w.as_ptr()), &mut hk)
            .ok()
            .map_err(|e| format!("打开 {key} 失败：{e}"))?;
        for (name, value) in &vals {
            let name_w = wide(name);
            let data_w = wide(value);
            let bytes = std::slice::from_raw_parts(
                data_w.as_ptr() as *const u8,
                std::mem::size_of_val(data_w.as_slice()),
            );
            RegSetValueExW(hk, PCWSTR(name_w.as_ptr()), 0, REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| format!("写 {name} 失败：{e}"))?;
        }
        let _ = RegCloseKey(hk);
    }
    Ok(key)
}

#[cfg(not(windows))]
pub fn apply(_id: &Identity<'_>) -> Result<String, String> {
    Err("AUMID 注册只在 Windows 上有意义".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_path_uses_the_aumid_and_display_name_is_always_written() {
        let id = Identity {
            aumid: "dev.xhy.desktop",
            display_name: "手边",
            icon_path: None,
        };
        let (key, vals) = plan(&id);
        assert_eq!(key, "Software\\Classes\\AppUserModelId\\dev.xhy.desktop");
        assert_eq!(vals.len(), 1, "没给图标就只写 DisplayName");
        assert_eq!(vals[0], ("DisplayName", "手边".to_string()));
    }

    #[test]
    fn icon_path_is_written_as_icon_uri() {
        let id = Identity {
            aumid: "a.b",
            display_name: "X",
            icon_path: Some("C:\\app\\xhy.exe"),
        };
        let (_, vals) = plan(&id);
        assert!(vals.iter().any(|(n, v)| *n == "IconUri" && v == "C:\\app\\xhy.exe"));
    }
}
