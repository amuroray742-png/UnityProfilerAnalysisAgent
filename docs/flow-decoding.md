# Flow 解码与查询

2026-09-30 增量以[当前 .data 解析契约](data-parser-current.md)为准，包含版本待验证状态、内存/未知区段查询、缓存与取消；下文真实录制验收保留原限定范围。

适用 Windows、Unity 6000.3.9f1 / 6000.3.23f1 的现有顺序解析入口。Flow 是录制中的关联事件，不是 CPU 耗时或自动生成的任务依赖图。

## 使用

导入 `.data` → CPU / GC →「单帧调用树」→「帧证据与调用路径对比」→「Flow 跨线程事件」。点击「读取 Flow」，可选择结束帧，或点击列表中的 Flow ID 后重新查询，查看同 ID 在窗口内的线程与样本。每次最多连续 8 帧、每页 50 项（界面 20 项），沿下一页继续读取。

保留 Begin (0)、ParallelNext (1)、End (2)、Next (3) 原始类型、无符号 32 位 Flow ID、样本索引、线程 ID、帧号和线程内事件序号。未知类型显示 Unknown，不猜测；样本索引 -1 表示没有绑定样本，其他越界索引使结构解码失败。线程/marker 展示名限制为前 160/240 字符，索引与 ID 不截断。

只在查询时读取原始帧，沿用源文件变化与单帧 128 MiB 限制。JSON/dump 没有 Flow 时返回 unavailable，不伪装成已验证的零事件。二进制空列表表示当前记录没有事件，并不证明运行时不存在依赖。

## 证据边界

- 列表按帧、线程、原始事件顺序排列，不是全局时间顺序。相同 ID 只提供关联观测，不直接构建方向边或关键路径。
- 查询窗口、分页和录制边界都可能截断生命周期；缺 Begin/End、重复 Begin 或 ID 复用不能静默合并成一条完整任务链。Begin/End 数量仅为窗口内计数，不是完整性判定。
- 返回的纳秒时间与耗时属于关联样本，不是 Flow 精确发生时间。事件枚举顺序、时间重叠、样本耗时都不能用于推算等待时长或收益。
- 原始 marker 文本仍是不可信数据。首轮、源码、工程定位均可按需查询 Flow，报告应区分观测与原因假设。现有报告导出无需新格式。

## 布局与接口

线程末尾的 counted 12-byte 记录：`sample_index: i32, flow_id: u32, event_type: u32`（小端）。由 `RawFrameDataView.GetFlowEvents` 逐条对照确认；样本表后的另一段 12 字节辅助记录仍未知，未当成 Flow。参考 [Unity RawFrameDataView API](https://docs.unity3d.com/6000.3/Documentation/ScriptReference/Profiling.RawFrameDataView.html)。

Tauri：`flow_events(fileId, frameIndex, endFrameIndex, flowId, start, limit)`。
MCP：`performance_flow_events` 使用 snake_case 参数，结束帧默认起始帧，flow_id 可省略。普通/源码/工程会话分别 10/13/16 个工具，保留各自授权范围。响应行的 pretty JSON 预算 20 KiB，超限提前分页，兼容 MCP 文本与 structuredContent 双份响应的 64 KiB 限制。

## 复现

在对应版本的隔离测试 Editor 中加载本地录制，停止 Profiler 录制，通过 Unity CLI `eval_file tools/export-flow-reference.cs` 获取参考文件。脚本只查询录制，输出到系统临时目录；它不更改工程、不向 Agent 发送数据。该 JSON 仅供测试，不是应用输入。

```powershell
$env:UNITY_FLOW_DATA_PATH = '<本地录制.data>'
$env:UNITY_FLOW_REFERENCE_PATH = '<脚本返回的参考 JSON>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test flow_events editor_flow -- --ignored --nocapture
```

显式运行缺少文件会失败。测试逐帧、逐线程、逐事件核对所有解码记录，再通过生产导入与分页查询核对帧 0、1、127、511、999、1500、1999；不是仅以总数相等作为验收。私有录制及参考不提交仓库。

2026-09-29 验收结果集中记录在[状态台账](project-status.md)。本轮不增加 GPU 时间、完整调用栈、等待因果自动归因或全录制依赖图。
