# Windows 桌面验收记录与剩余检查

状态：2026-09-28，下表的应用界面流程已由 release WebDriver 自动验收，并在 NSIS 安装目录中的程序上复验。测试仅替代系统文件选择框的返回值，实际点击界面并使用真实后端；原生选择框本身仍未验证。Windows 控制工具读取窗口成功，但截图报 `FrameArrived timed out`，点击报 `coordinate input geometry is unavailable`，因此不能将替代返回值计为原生选择框验收。证据与产物哈希见[性能与发布记录](performance-and-release.md)。

## 准备

在仓库根目录运行 `python tools/prepare-manual-fixtures.py`，生成 `.cache/manual-fixtures/` 下的 5 个 JSON。它们全部来自公开人工 fixture，不包含私有录制。双击 `src-tauri/target/release/unity-profiler-analysis-agent.exe` 启动应用。需要核对安装后行为时，使用 `src-tauri/target/release/bundle/nsis/` 中的安装包；当前用户模式静默安装和卸载已经通过，交互向导与快捷方式尚未测试。

## 界面检查

线程列表只显示**当前帧**的线程：`normal.json` 中帧 **10** 有 `Main Thread` 和 `Worker #1`，帧 **12** 只有 `Main Thread`。检查 Worker 前请先将“帧”切回 10；两帧的线程列表不同是 fixture 的预期内容。

| 操作 | 预期结果 |
|---|---|
| 点击文件选择区，先取消选择，再选择 `normal.json` | 取消不报错；导入后显示概览，分析 2 / 声明 20 帧 |
| 查看概览 | 主线程 p95 为 12.00 ms，GC 每帧 p95 为 32 B；Draw Call 为“—”且说明不可用 |
| 切到 CPU，帧 10，Main Thread，深度 8 | 共 5 个样本；两个 GC.Alloc 分别为 20 B / 4 B，后一个父索引为 2 |
| 深度改为 3，然后恢复 8 | 深度 3 提示有被隐藏样本；恢复后再次出现深度 3 的分配节点 |
| 线程切到 Worker，再切帧 12 | Worker 有 2 个样本；切帧重置默认线程，帧 12 只有根样本，帧 GC 为 0 B |
| 查看 GC 与渲染页 | GC 总量 32 B；Update 24 B、2 次，Worker 8 B、1 次；Gen0/Gen2、渲染和 SRP 节省均不可用 |
| 重置，导入 `zero-gc.json` | GC 是真实 0 B，状态可用，有效帧 1/1；不是“—” |
| 重置，导入 `partial-gc.json` | CPU 仍可用；GC 部分可用，有效帧 1/2，只保留另一帧的零分配，说明 metadata 缺失，不触发确定性警告色 |
| 重置，导入 `invalid-tree.json` | 显示带帧/线程/样本位置的结构错误；不能显示空白成功结果；随后仍能选择正常文件 |
| 导入 `pagination.json`，CPU 树下一页/上一页 | 共 211 个原始样本，首屏 200 个；下一页 11 个，从样本 200 开始，父索引仍为 0；返回不重复或漏项 |
| 正常文件重复导入、切页、重置 | 旧调用树或诊断内容不会串到新输入；重置后回到选择文件状态 |

## AI 诊断与取消

使用 `normal.json` 和已经登录的 Claude Code ACP。点击“开始 AI 诊断”，观察流式正文、日志和最终状态；通过日志确认工具访问的是当前 fixture。完成后重新诊断并在运行期间取消，确认按钮恢复、没有后续正文追加、再次诊断可用。模型措辞不作为固定断言。

此流程已通过实际 release 界面按钮验证：首次诊断完成，日志确认 MCP 查询及 end_turn；第二次在 MCP 活动后取消，恢复就绪且没有迟到正文；第三次重新诊断完成。只使用公开合成输入，未向 Agent 发送私有录制。模型措辞和建议准确性不作为固定断言。本次第三次回答遗漏了嵌套的 4 B 分配，虽然工具与界面仍正确显示 20 B + 4 B；协议验收通过不代表诊断正文已通过数值一致性验收。

复现：先生成上述输入，运行 `python tools/desktop-smoke.py --ui --real-agent`。普通界面回归可省略 `--real-agent`；安装后界面回归使用 `./tools/test-windows-install.ps1 -DesktopUi`。截图与结果位于忽略目录 `.cache/desktop-ui/`；真实 Agent 本次报告另存为 `real-agent-result.json`。

验收发现底部 sticky 状态栏遮挡长调用树的分页按钮；已改为正常文档流布局，重新构建后实际点击下一页/上一页通过。帧 12 的帧级 GC 为 **0 B**，根样本不是 GC.Alloc，因此该样本行的 GC 列为 **—**，两者含义不同。

## 安装与发布门槛

- NSIS 向导：安装路径选择、开始菜单/桌面快捷方式、启动和卸载；确认默认保留安装路径偏好，是否删除应用数据由用户选择。
- MSI：perMachine 静默安装、安装后 8 项协议回归、卸载及残留检查已在管理员 CI runner 通过；交互向导、GUI 和版本升级仍未覆盖。
- 远端 CI：PR #1 已合并；提交 `3cd15e8` 的 [Windows CI](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/actions/runs/36400361838) 全部通过，MSI 验收增量见[草稿 PR #2](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/pull/2)。安装包本地生成不等于发布版本。
- 发布身份：确认 `com.ray.unity-profiler-analysis-agent`、维护者名称、MIT 许可证版权归属与是否签名；已按维护者授权补充 MIT LICENSE，版权账号为 amuroray742-png。
- 完整应用内存与并发查询预算、更多录制/版本证据仍需扩充，独立解析进程的峰值不等于桌面总占用。

## 反馈格式

请记录程序/安装包 SHA-256、Windows 与 WebView2 版本、输入文件名、通过的行号或操作、失败时可复现的步骤及实际显示。只执行了一部分时明确列出剩余项。无需提供私有录制或完整 dump。
