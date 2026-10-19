# 第三方组件与许可

本仓库代码为 MIT（见 [LICENSE](LICENSE)）。这里单独列出**随仓库分发**或**打包进产物**
的第三方内容，避免"MIT 覆盖一切"的误读。

## 随仓库分发的数据

### holiday-cn（中国法定节假日与调休）

- 来源：<https://github.com/NateScarlet/holiday-cn>
- 许可：**MIT**，Copyright (c) 2019 NateScarlet
- 位置：`app/src-tauri/holidays/*.json`（构建期打包进二进制，运行时离线查表）
- 上游的完整 MIT 正文与其仓库一致，如下：

```
MIT License

Copyright (c) 2019 NateScarlet

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 构建时获取的依赖

不随本仓库分发，由包管理器在构建时拉取，各自保留自己的许可证：

| 组件 | 许可 | 用途 |
|---|---|---|
| [Tauri 2](https://tauri.app/) | MIT | 应用框架、窗口 / 托盘 / 通知 / 打包 |
| [Vue 3](https://vuejs.org/) | MIT | 前端视图 |
| [Pinia](https://pinia.vuejs.org/) | MIT | 前端状态 |
| [lunar-lite](https://github.com/JinyangWang27/lunar-lite-rs) 1.3.0 | MIT | 农历换算（节气 / 闰月 / 农历重复规则） |
| [serde](https://github.com/serde-rs) / [chrono](https://github.com/chronotope/chrono) / [tokio](https://github.com/tokio-rs/tokio) 等 | MIT / Apache-2.0 | 序列化、日期时间、异步运行时 |

完整清单以锁文件为准：`app/src-tauri/Cargo.lock`（Rust）与 `app/package-lock.json`（npm）。
需要逐份许可证正文时可本地生成：`cargo install cargo-license && cargo license`（在
`app/src-tauri/` 下）——本项目不预生成，免得清单与锁文件漂移。

## 不是"第三方来源"的东西

- **功能口径**参考了 [小黄条 YYNote](https://www.yynote.cn/) 与
  [滴答清单](https://www.dida365.com/) 的**公开功能页与使用习惯**，但**未使用**它们的
  名称、logo、图标、字体、截图、文案或任何代码。所有实现均为本仓库原创。
- **和风天气 QWeather** 只出现在 `rainmeter/` 的示例配置里，需要使用者自己注册并填
  API Key；本仓库不含任何 key，也不代为分发其数据。
