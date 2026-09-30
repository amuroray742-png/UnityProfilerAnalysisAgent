# 历史轮次、解析进度与实时工作验收（2026-09-30）

默认范围 Windows。只使用仓库公开工程、公开合成录制及此前公开测试的存档，没有修改私有游戏工程。

## 自动回归

- 完整 Rust：149 项通过，22 项默认忽略；忽略不视为通过。
- 前端：55 项通过；Python 工具：8 项通过；生产构建通过。
- 新增覆盖：历史尝试版本、迟到页面、回退差异、未知／已知进度、公开文字合并、工具调用身份、滚动跟随、旧活动日志缺失、日志分页／上限／中断末行／磁盘失败、二进制真实字节回调。
- 修改会话协议测试额外核对真实 `optimization_replace` 的开始和结束活动、公开正文及参数脱敏，并执行精确回退。
- 本机证据：`.cache/history-rust-final.log`、`.cache/history-frontend-final.log`、`.cache/history-python.log`。

首次 Debug 集成测试因核心工作区持有 Tauri AppHandle 而引入 Windows GUI 导入，测试 EXE 缺少 Common Controls v6 激活导致 `STATUS_ENTRYPOINT_NOT_FOUND`。已把观察通知改为可选回调，与 GUI 运行时解耦；完整回归随后通过，不需要改变系统 DLL。

## 真实 Agent 与桌面

`tools/desktop-smoke.py --history-activity --history-records <此前公开两轮存档>`：

- 打开两轮旧存档，查看诊断、定位和回退后的逐文件差异。
- 使用公开 `isolated-peak.json` 和仓库 Unity 工程，Claude／Codex 各完成性能诊断与工程定位；Editor 不连接时离线定位。
- 四个实际 ACP 会话不同；公开文字和工具事件落盘，可按游标完整读取，重开保留两次分析版本，不自动启动 AI。

| Agent／阶段 | 实际会话 | 已提交活动数 | 工具状态事件数 |
|---|---|---:|---:|
| Claude 诊断 | `7ea60f3e-d1a5-41fa-b8d5-9e322dc695fc` | 616 | 24 |
| Claude 定位 | `4ba62d7a-3a9d-4719-9648-93949a5ae9dc` | 852 | 86 |
| Codex 诊断 | `01a0f10d-8694-7071-af6d-88264b3adc93` | 1789 | 38 |
| Codex 定位 | `01a0f10f-309b-7592-b4b6-a8e36913625a` | 2645 | 70 |

工具事件数包含开始与结束，不等于独立调用数。此轮真实桌面流程 EXE SHA-256：`921dab4f723ea999ccbf4e112793f2689f509a1a0a4fe69b721f49d40e32dc21`。证据：`.cache/desktop-history/result.json` 与截图。

显式运行 `real_claude_modification`、`real_codex_modification`，均在临时公开 `Work.cs` 修改后回退；记录 154／282 条活动，2 项通过。未连接 Editor，不声称编译或玩法验收。桥接 EXE：`f75cbf8d7dea26a3715fa87e6f1d261d1b30018c4e1b2ee90c7942ec33cd87b5`；日志 `.cache/history-real-modification.log`。

## 大文件进度与最终交付

`--history-progress` 生成 61,760,004 字节、40,000 帧的公开重复结构 `.data`，验证实际字节进度、完成后保存，以及随后导入损坏文件不替换原 A。它是通知链路回归输入，不代表真实复杂录制的性能或内存基准。

最终 EXE SHA-256：`f104bf50450251a3ba13cf9422309a5026ef56986cb02cbd992eb8ac04ac4cb5`。

- 完整生产构建通过；最终 EXE 协议与优化回归 51 项通过、9 项默认忽略，见 `.cache/history-protocol-final.log`。
- `--history-progress` 最终复验通过：输入 61,760,004 字节、40,000 帧，观察到 3 次数值解析进度通知；解析／保存和故障保护回归约 1.09 秒。快速解析不要求固定数量中间通知；成功时补最后进度，失败不会显示完成。见 `.cache/desktop-progress/result.json`。
- `--history-replay <本次公开存档>` 使用最终 EXE 重开四个真实会话，分页读取活动、查看各版定位报告并导出 Markdown／HTML；没有再次启动 AI。见 `.cache/desktop-history-replay/result.json`。
- `--workflow-saved <此前公开两轮存档>` 在最终 EXE 验证旧两轮恢复和界面导出按钮。见 `.cache/desktop-workflow-saved/result.json`。
- 构建／复验日志：`.cache/history-release-last.log`、`.cache/history-progress-final.log`、`.cache/history-replay-final.log`、`.cache/history-saved-final.log`。

首次大文件桌面断言要求至少三次中间进度，但快速解析在节流下只产生两次含起始通知的事件。修正为校验实际数值通知、单调性和总大小；同时补齐快速解析完成的最终字节通知。没有按时间伪造进度。

## 边界

- 不显示或模拟模型内部思考；只使用公开正文与实际后端工具调用摘要。
- 旧版没有保存活动日志时，显示兼容提示；原有报告、变更备份和回退记录保留。
- 原生文件／保存对话框在既有桌面测试中采用返回值替代；活动测试使用真实 IPC，不计作系统对话框人工验收。
- 不重复验收 MSI／NSIS，不宣称真实游戏重录或性能改善；公开合成数据不替代真实复测。
