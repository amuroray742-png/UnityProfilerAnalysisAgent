# 公开优化闭环验证（2026-09-29）

环境：Windows、Unity 6000.3.23f1、Claude Code ACP 0.16.2、Codex ACP 1.13.1。仅使用仓库公开 Unity 风格工程和 `isolated-peak.json` 派生的 21 帧合成输入。未读取或修改私有游戏工程。

首轮性能诊断和工程定位使用 Claude，定位通过实际代码读取生成候选任务。用户确认动作在测试中通过生产 IPC 提交；每次修改使用真实 ACP/MCP、真实文件替换和 Unity 检查。原生目录/保存对话框由固定返回值替代，不计为系统对话框验收。

| 修改任务 | Agent | 实际 ACP 会话 ID | 编译 / 选定 EditMode | 回退 |
|---|---|---|---|---|
| 复用 8 MiB 缓冲 | claude-code | `8190a4a4-6e88-4270-81ac-58c106439e04` | passed | rolled_back，原字节一致 |
| 复用 8 MiB 缓冲 | codex | `01a0ecd9-1857-7483-8df4-ca5b81acaf87` | passed | rolled_back，原字节一致 |
| 纯 Marker | claude-code | `5da2a0b0-8d10-4337-9a7b-8ec9f7a9f794` | passed | rolled_back，原字节一致 |

三次会话 ID 均不相同。前两次分别由相同 Agent 和不同 Agent 完成相同任务；每次回退后下一次从原始代码开始。第三次仅在现有代码中增加 `Public.AllocationWork` 的同步 Marker，保留原有分配和写入行为，任务人工验收仍为待核对，不能据此认定新采样已经有效。

行为检查：`PublicOptimizationTests.AllocationWorkPreservesVisibleResult` 核对缓冲长度为 8 MiB、首字节为 1。固定检查插件另连续执行两次通过和一次显式失败，验证终态跨域重载保存。检查不验证所有玩法，也不是整个游戏的回归测试。

A/B 使用相同的公开合成输入验证零差值、完整名称路径关联和导出，**不是重新运行游戏取得的性能收益证据**。新增/改名 Marker、缺失、部分覆盖、区间和条件差异另由 Rust 回归覆盖。真实游戏录制 B、性能收益及玩法正确性仍待使用者复测。

关闭并重新打开保存记录后，三个运行及回退状态保留。Markdown / HTML 导出包含任务、证据、Agent、实际修改、检查和比较；HTML 在 Edge 阅读和打印通过，无可执行节点或横向溢出。实际样例产物在忽略目录 `.cache/desktop-optimization/`；最终保存记录的界面与导出检查在 `.cache/desktop-optimization-saved/`。

复现入口：`tools/desktop-smoke.py --optimization-loop --agent-id claude-code`；已保存记录可用 `--optimization-saved <公开验收记录目录>` 验证最终 UI 导出。前提是 `.cache/unity-project-public` 为公开隔离工程且已打开，插件和选定测试已编译；需要已登录的两个真实适配器。`tools/unity-optimization-checks.py --unity <Unity CLI 路径>` 单独验证固定检查协议。

HLSL/CGINC/Compute 的依赖 Shader 和全平台变体覆盖不足时返回无法完成；未把这一范围标为全面验证。真实游戏 A/B、完整三次编译错误自动修复场景、更多 Agent/版本和新安装包交互仍需分别验收。
