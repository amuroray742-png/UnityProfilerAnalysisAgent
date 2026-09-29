# 渲染计数解析与对照验收

更新：2026-09-28。当前新增 `.data` 的 Draw Call、SetPass、Batches、Triangles、Vertices 及渲染 CPU marker。GPU 时间、SRP Batcher 节省、Overdraw、材质/对象归因仍不可用，不由现有计数推算。

## 来源与边界

- 顺序解码 marker 定义、样本索引及通用 metadata，要求 Counter 标志 `0x80` 与已知 marker 名匹配；不固定 marker ID，不扫描任意偏移。
- 2026-09-29 经 Editor metadata 定义和数值 API 核对，原有 tag 2 / 4 字节和 tag 4 / 8 字节分别应解释为 Int32 / Int64（此前文档误写为无符号类型）。有效计数必须非负；负值保留原始证据，但不转换成巨大正数参与渲染聚合。从全部已导出线程读取，计数可在 Render Thread，不要求在 Main Thread。
- 同帧同名计数的观测必须完整且一致；冲突或缺 payload 时该计数不可用，不相加或猜测最后一个值。尚未验证多次不同值的事件顺序，当前采用保守拒绝策略。
- 真实零计入分位数；缺失不计入。五类计数分别附质量和有效帧数；不生成 SRP 节省估算。Draw Call / SetPass 的旧 u32 字段不截断越界值。
- Unity 6000.3.9f1 存在样本后的非空辅助段：按数量读取 12 字节记录并验证第二个 u32 的样本索引边界；其余字段语义未确认，不用于性能指标。未知布局仍应报错。
- 渲染 CPU marker 来自 category 0 的非 Counter 正耗时样本，按线程及 marker 聚合 inclusive 毫秒，保留调用次数和最大单次值。父子和跨线程不能相加为总耗时，等待不等于纯计算，不能当成 GPU 时间。
- 生产结构入口限定 6000.3.9f1 / 6000.3.23f1。本轮渲染数值对照只覆盖下述 6000.3.9f1 单份录制；6000.3.23f1 CPU/GC 兼容回归通过，未补做该版本的渲染 Editor 对照。其他版本保持原有估算/不可用行为。
- 本轮没有扩展 Editor dump 的渲染格式；旧 dump 不含这些计数时仍显示不可用。项目 V1/V2 显式 Draw Call/SetPass 继续保留。

## 本地录制证据

私有录制使用 Unity 6000.3.9f1，391,868,208 字节、2,000 帧。录制及完整参考 JSON 均不提交仓库、不发送给 Agent。

| Counter | 首帧 | P95 | 最大值 | 有效帧 |
|---|---:|---:|---:|---:|
| Draw Calls Count | 327 | 328 | 328 | 1,998 / 2,000 |
| SetPass Calls Count | 88 | 89 | 89 | 1,998 / 2,000 |
| Batches Count | 327 | 328 | 328 | 1,998 / 2,000 |
| Triangles Count | 236,477 | 236,479 | 236,479 | 1,998 / 2,000 |
| Vertices Count | 455,290 | 455,294 | 455,294 | 1,998 / 2,000 |

[生产入口集成测试](../src-tauri/tests/render_counters.rs)逐帧核对五类计数的存在性和值，再独立按 Editor 参考计算分位数、最大值和覆盖率；两帧确实没有计数，不能补零。首次完整联合验证约 16.31 秒（debug，含读取/解析/聚合与查询），不是内存或单次导入性能承诺。

CPU/GC 兼容验证使用独立 Editor 导出的帧 0、1、127、511、999、1500、1999，共 30,860 个样本；验证所有线程、样本名称/时间/结构、GC、生产查询与聚合，约 26.11 秒。原有 6000.3.23f1 录制的前 64 帧、240,350 个样本再次通过，约 36.81 秒。两者完整遍历均为 2,000 帧，不把未导出帧算作 CPU/GC 数值对照。

## 复现

1. 在对应 Unity Editor 的 Profiler 中加载录制，停止继续录制。通过已连接的 Unity CLI/MCP `eval_file` 运行 [export-render-reference.cs](../tools/export-render-reference.cs)。脚本已在 6000.3.9f1 执行验证，返回临时目录中的 JSON 路径和 2,000 帧数量。依赖 Editor 环境的 Newtonsoft.Json。
2. 脚本对每帧调用 `GetMarkers`、`HasCounterValue`、`GetCounterValueAsLong`。必须检查存在性，因为没有计数时数值 API 也可能返回零。参考文件仅用于测试，不是应用 dump 格式。
3. 显式执行：

```powershell
$env:UNITY_RENDER_DATA_PATH = '<录制绝对路径.data>'
$env:UNITY_RENDER_REFERENCE_PATH = '<Editor 返回的参考 JSON 路径>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test render_counters editor_render_counters_match_production_import -- --ignored --nocapture
```

未显式执行时私有集成测试跳过；显式执行但环境变量或文件缺失必须失败。公开合成二进制测试覆盖不同 marker ID、跨帧状态、Render Thread 计数、零/缺失/冲突、32/64 位 payload、辅助索引和截断边界；另有生产入口 → 快照 → 单帧 → MCP 的联通断言。

桌面验收命令（仅替代原生文件选择返回值，使用真实 release 后端，不调用 Agent）：

```powershell
npm run tauri:build -- --no-bundle
python tools/desktop-smoke.py --render-input '<录制绝对路径.data>' --render-reference '<参考 JSON 路径>'
```

检查五张计数卡的 P95、最大值及覆盖率，渲染 CPU 标注、SRP 不可用和重置；截图及报告保存在忽略目录 `.cache/desktop-render/`。安装包没有在本轮重新验收。

## 本次验收结果

最终完整 Rust 测试 91 项通过、13 项环境测试默认忽略；前端 20 项、Python 研究回归 8 项、前端构建和 Windows release 构建通过。私有渲染测试显式执行通过（约 16.16 秒）；CPU/GC 独立对照范围见上文，不能把默认忽略算作通过。

最终桌面自动化 6 组检查通过，包含原有公开 fixture 的 CPU/GC、释放和真实调用树 IPC，以及私有录制的五张渲染卡、覆盖率、GPU/SRP 边界和重置；实际导入至就绪约 2.58 秒。首轮计数已正确显示，但标题的 CSS 自动大写使脚本断言失败；修正为检查 DOM 原文后通过，并将卡片原因收敛为各自指标。截图已目视复核，报告与截图位于 `.cache/desktop-render/`，不提交私有内容。

最终 release EXE SHA-256：`1e45b5b71dd70a117b569faad4b4598ac1fc7d44b7b65c3cc08d294b121bc3d8`。使用该 EXE 的公开 ACP/MCP 协议回归通过（环境相关 Agent 测试仍默认忽略）。本次未向真实 Agent 提交私有录制，也不扩大先前安装包或诊断正文的验收范围。
