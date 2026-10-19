//! Windows 系统集成：桌面层嵌入、几何/DPI、贴边隐藏
//!
//! edgehide 是纯几何（跨平台可编译、可单测）；desktop 只在 Windows 下编译。

pub mod aumid;
pub mod edgehide;
pub mod geometry;

#[cfg(windows)]
pub mod desktop;
