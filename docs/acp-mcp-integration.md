# ACP / MCP 集成与验收

当前已接通 Windows 上的 ACP v1 诊断与 MCP stdio 查询。真实 Claude Code ACP 0.16.2 的完成及取消流程通过公开 fixture 验证；release 桌面完成/取消/重试及 NSIS 安装后协议已通过；MSI 安装后协议也已在 Windows CI 通过；其他 Agent 与更广泛诊断内容仍待验收。完成范围见[项目状态](project-status.md)。

## 使用流程

1. 导入支持的录制或 Editor dump，等待分析完成。
2. 选择已安装且已完成自身登录配置的 ACP Agent，点击“开始 AI 诊断”。PATH 检测只代表程序存在，认证或协议错误会显示在界面。
3. Agent 通过 MCP 查询本次录制的摘要、帧和调用树，回答以文本片段显示。日志中 MCP 记录来自本地服务实际执行，不由 Agent 输出文字冒充。
4. “取消”按 sessionId 发送取消；重置或替换输入会同时释放该文件的诊断和数据。同一文件不能重复启动尚未结束的诊断。

| ID | 命令 | 参数 | 验证范围 |
|---|---|---|---|
| claude-code | claude-code-acp | 无 | 0.16.2，Windows，真实完成与取消 |
| gemini | gemini | --experimental-acp | 预设保留，未验证 |
| codex | codex-acp | 无 | @agentclientprotocol/codex-acp 1.13.1，Windows，公开 fixture 的 MCP 查询与完成已验证；见文末 |

## ACP 会话与资源

[诊断入口](../src-tauri/src/acp_client.rs)创建独立临时工作目录、会话 MetricsStore 和 MCP 桥。[协议层](../src-tauri/src/acp_client/protocol.rs)依次执行 initialize → session/new（注入 MCP 配置）→ session/prompt，仅协商 ACP v1；不再将纯文本写入 stdin 后关闭输入。

session/update 中的 agent_message_chunk 才作为回答正文；其他会话的通知被过滤。stderr 是日志，不直接作为失败终态。只有 end_turn 生成 Finished；refusal、max_tokens、max_turn_requests 会说明诊断未完成。协议错误、过早 EOF、非法 JSON 和超时产生一个 Error，之后不再追加 Finished。

前后端事件包含 fileId 与本地 sessionId，camelCase 字段一致。前端缓冲早于 diagnose 命令响应到达的事件，只接收最终返回的会话；重试、重置后的旧事件被忽略。界面保留最近 500 条事件和最近 2 MiB 字符的回答，长会话可能截去早期内容；这不是服务端整体内存上限。

取消先发 session/cancel，等待最多 2 秒取得 prompt 的取消响应，再清理进程、MCP 服务和临时目录。初始化/新会话阶段尚无远端 sessionId 时直接结束进程。Windows 使用 Job Object 管理适配器后代，关闭 Job 时一并终止；不配合取消的 fixture 也通过子进程退出断言。创建进程后立即加入 Job，极早启动阶段及其他平台的完整进程树行为尚需更广验收。

初始化超时 30 秒，新会话 60 秒，prompt 总期限 300 秒，单条 ACP 消息上限 1 MiB。客户端不声明文件或终端能力；仅对当前会话中名称精确匹配本服务的 MCP 只读工具授予 allow_once，其余权限请求回复 cancelled 并记录日志。这不是对任意 Agent 内建能力的操作系统沙箱。

## MCP 数据通道

[MCP 服务](../src-tauri/src/mcp/server.rs)使用锁定的 rmcp 0.5 完成 initialize、tools/list 和 tools/call。每个[桥服务](../src-tauri/src/mcp/bridge.rs)绑定独立的 127.0.0.1 随机端口，持有该会话的数据。应用自身的 `--mcp-bridge <address>` 模式在启动 GUI 前连接父服务并转发 stdio；随机能力通过 `UPAA_MCP_TOKEN` 环境变量交付，不出现在命令行或日志。子进程不重复读取完整录制。

父服务关闭后连接失效，桥进程退出。认证期限 3 秒，MCP 初始化期限 15 秒，每服务最多 8 个连接，单条入站 JSON 上限 64 KiB。stdout 仅输出 MCP JSON-RPC，启动错误写 stderr。在途阻塞查询结束后释放其数据引用；并发内存预算仍待 P2 测量。

没有独立 `unity-profiler-mcp` 二进制或 `mcp:serve` npm script。桥需要正在运行的诊断会话提供配置，不能作为无参数的独立编辑器 MCP 服务。

| 工具 | 参数 | 语义 |
|---|---|---|
| performance_session_summary | 无 | 聚合指标、质量与 metricSemantics（分位数算法、inclusive CPU 和覆盖语义），省略逐帧时间线；不承诺固定字节大小 |
| performance_frames | start、limit | 时间线数组偏移，limit 为 1–500，默认 200；返回原始帧号、主线程/录制帧时间与 gcAllocBytes（缺失为 null） |
| performance_frame | frame_index、start、limit | 原始帧指标及线程分页，最多 128 个线程 |
| performance_cpu_hierarchy | frame_index、thread_index、start、limit、max_depth | 原始前序树，最多 500 个样本、64 层；默认唯一 Main Thread |
| performance_analysis | focus | 只对 available 指标应用规则，其余返回质量与警告 |

未知参数、非法类型与枚举、数量边界均校验。缺失帧或不可用调用树返回工具错误，不伪造空结果。调用树默认 max_depth=3，仅返回 depth < 3 的样本。响应附 queryWarnings：depthTruncated 时要求加深并从 start=0 重查，nextStart 非空时要求续页。即使 nextStart=null，也可能仍有深度截断；可见 GC 样本之和不能替代已校验的线程/帧总量。诊断提示要求包含嵌套 GC.Alloc，按最近非 GC 父节点归因并核对总量，无法解释的差额必须明确呈现。此约束减少遗漏风险，不保证模型遵循，也不等于对自由文本做了自动事实校验。CPU 为 inclusive，GC 为字节；详见[查询契约](frame-queries.md)。

## 验证记录（2026-09-28）

- [MCP 进程回归](../src-tauri/tests/mcp_wire.rs)：3 项，实际桥进程完成握手、工具发现、嵌套 GC 查询、参数错误、双会话隔离、认证失败、超长请求与退出。
- [ACP 进程回归](../src-tauri/tests/acp_stdio_roundtrip.rs)：5 项默认运行；Node 独立 fixture 执行真实 MCP 调用，覆盖消息权限、非当前会话、stderr、错误终态、初始化取消、不配合取消时的后代清理，以及应用状态的重复诊断/释放。
- 前端 hook 回归验证早到事件、会话过滤、失败不被完成覆盖、按 sessionId 取消与重置；不替代完整 Tauri 桌面交互。
- 真实适配器：`@zed-industries/claude-code-acp 0.16.2`。公开 fixture 的诊断共调用 7 次 MCP，返回 241 个正文片段，以 end_turn 结束；整项测试 17.57 秒。另一次在首个 MCP 查询后取消，Agent 返回 cancelled；取消收尾约 64.6 毫秒，整项测试 4.27 秒。没有发送私有录制。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline
$env:UPAA_REAL_AGENT = 'claude-code-acp'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test acp_stdio_roundtrip real_agent_ -- --ignored --nocapture
```

真实 Agent 测试默认忽略；显式执行必须提供可用适配器与登录环境，失败不视为跳过。运行会把人工 fixture 的查询结果交给所选 Agent 的模型。模型结论受模型影响，上述验收只证明协议和数据链路，不保证诊断建议总是正确。

实现依据：[ACP v1 初始化](https://agentclientprotocol.com/protocol/v1/initialization)、[会话配置](https://agentclientprotocol.com/protocol/v1/session-setup)、[prompt 与取消](https://agentclientprotocol.com/protocol/v1/prompt-turn)及[MCP stdio](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)。release 桌面与发布模式进程回归已有证据；原生窗口已由维护者验收；更多 Agent、长期性能预算及发布为后续范围，见[性能记录](performance-and-release.md)。

### 统计解释约束（2026-09-28）

桌面诊断内容复核发现模型将小样本 p50 误判为口径差异，并从 inclusive 样本猜测剩余 CPU。摘要现附 `metricSemantics`：统计使用有效帧（含真实零），排序后取 `round((n-1)*q)`，不插值；例如 [0,32] 的 p50 为 32，不是平均值 16。未提供已验证的 self/exclusive 耗时，不允许用热点列表或父子 inclusive 相减推断未解释 CPU。树完整返回也不代表 instrumentation 覆盖全部运行工作。

该字段解释现有算法，未改变计算结果、查询上限或 Tauri 快照。MCP 真实 stdio 回归核对有效帧为 2、GC p50=32 及语义字段交付；诊断提示要求遵守它。工具数据正确和提示完整仍不等于模型输出必然正确。

## 诊断筛查与帧证据

`performance_analysis` 的 CPU/GC issues 区分 P95 超限与孤立峰值，附 `unit`、`affectedFrames`、`validFrames`、最多 5 项 `evidenceFrames` 和 `thresholdPolicy`。帧证据用原始帧号，需继续查询原始线程/样本解释原因。默认阈值仅用于筛查，不等于项目预算或已确认瓶颈；issues 为空不证明无性能问题。部分、估算或缺失的指标不触发确定性诊断。

## Codex 适配器安装与检测

界面中的 Codex 使用 `codex-acp` 命令，不是 `codex` 命令。仅安装 Codex CLI 或 Codex 桌面应用不会自动安装此适配器；“未检测到 ACP 适配器”表示当前应用进程的 PATH 中没有找到该命令，不代表 Codex CLI 未安装。

按[适配器维护仓库](https://github.com/agentclientprotocol/codex-acp)说明安装：

```powershell
npm install -g @agentclientprotocol/codex-acp
codex-acp --version
```

旧 `@zed-industries/codex-acp` 项目已迁移，新安装使用上述包。适配器包含兼容的 Codex 依赖；本项目不替换用户原有 `codex.exe`，不修改用户认证或默认模型配置。安装后重启分析应用使其重新检测命令；若仍不可用，确认 npm 全局命令目录在启动应用时的 PATH 中。命令存在不等于已登录或协议一定兼容，实际诊断错误会在界面显示。

显式桌面验证可运行 `python tools/desktop-smoke.py --ui --real-agent --agent-id codex`，仅发送公开合成 fixture。默认仍使用 Claude Code；该参数不改变应用默认 Agent。

2026-09-28 本机验证：安装 `@agentclientprotocol/codex-acp 1.13.1` 后，公开 fixture 的实际 ACP/MCP 完成测试通过，65.42 秒、13 次 MCP 调用、end_turn。前端 19 项测试与构建通过，release 构建通过。指定 `--agent-id codex` 的桌面 12 项检查通过，包含选择 Codex、完成、MCP 活动后取消、无迟到正文及重新诊断；仅原生文件选择返回值替代。EXE SHA-256 为 `8d32455ce8d172d5be6156df52c1c0f30fb4427710bcd34a337e530ba5f1673f`，日志在忽略目录 `.cache/codex-desktop-validation.log`。首轮桌面再次启动未确认触发而超时，给测试补充页签与诊断启动状态等待后复验通过；未更改应用会话行为。不将协议完成等同于任意模型正文准确性。
