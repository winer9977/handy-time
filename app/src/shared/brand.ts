/** 产品显示名（Vue 侧）。
 *
 *  同一个名字有三处独立定义，改名字**必须三处一起改**：
 *    · 这里（界面里显示）
 *    · `src-tauri/src/lib.rs` 的 `BRAND`（窗口标题、托盘提示、通知、导出文件头）
 *    · `src-tauri/tauri.conf.json` 的 `productName`（exe 名与安装目录；那里用
 *      ASCII `Shoubian`，因为非 ASCII 路径在这条工具链上会出事，见 README）
 *
 *  为什么不做成"从后端要一次"：挂件是各自独立的 webview，启动时多一次 IPC
 *  换一个字符串不值得，换来的是"后端没起来时界面没名字"的新故障面。
 *  跨进程边界只能靠注释 + 测试对齐（本项目里 `attachMode`/`lunarMonthly`
 *  这些契约也是同一套办法）。
 */
export const BRAND = '手边'
