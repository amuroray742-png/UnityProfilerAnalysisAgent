# data 原始证据与帧路径对比

更新：2026-09-29。适用现有生产入口：Unity 6000.3.9f1 / 6000.3.23f1。

## 使用

导入录制 → CPU 或 GC → 单帧调用树。表格增加 Self ms；展开「帧证据与调用路径对比」：

- 「读取帧证据」默认列出当前帧所有已导出线程的 Counter，取消「仅 Counter」可查全部有 metadata 的样本。按下一页续查。
- 选择不同的对照帧，点击「比较调用路径」。对当前所选线程，按完整 marker 名路径比较调用次数、inclusive、Self 和直接 GC 字节。默认对照只是另一个帧，不保证正常；应结合帧列表选择代表性基线。
- 同名但不同父路径保持分开，同一路径的多次调用聚合；递归保留路径层次。非主线程按 thread ID 匹配对照帧，缺失时报错，不按数组位置代替。

## 语义与边界

Self 从完整树计算，再做深度/分页过滤：父 inclusive 区间减直属子样本时间。检查子样本数量、包含关系和互不重叠；时间误差容忍 max(20ns, 父时间×1e-5)，仅舍入误差范围内的负余量归零。原始纳秒时间可用时先做整数差；dump 使用已有毫秒时间。无效时 selfMs=null 并提供 selfReason。Self 包含等待与未细分工作，不证明纯计算或 instrumentation 覆盖完整。

对比按 inclusive 增量降序，不把父子路径的增量相加。GC 字节仅计该路径上的直接 GC.Alloc，不含子路径；任一帧 GC 不可信时，两侧和差值均为 null。对照缺席路径是已导出树中的零调用，不证明整个运行时没有该工作。

metadata 保留字段定义（原始 descriptor、名称）、实际 payload 类型/长度、单位、数值和原始字节预览。声明类型与 payload 编码分开，不直接用声明类型解释 payload；例如 GC 的声明与紧凑字节表示不同。整数以十进制字符串传输，避免 JavaScript 精度损失。已处理实例 ID、32/64 位有符号/无符号整数、有限 float/double、短 UTF-8/UTF-16 文本。二进制 Jobs metadata 等不猜语义，显示不可用原因及原始字节。对象 ID 不能直接解析成另一个 Editor 会话的对象。

Counter 是样本观测，不自动转换成帧总量或全录制内存曲线。同名冲突/多次观测不相加、不取最后一个。原有五类渲染计数仍使用原有独立校验和覆盖率。旧启发式 Memory ID 映射不接入生产指标。

每页最多 50 条；行数据 pretty JSON 预算 20 KiB，必要时提前分页（MCP 同时返回文本和 structuredContent）。每样本保留最多 16 字段、每帧 100000 字段，字段名最多 128 字符；字符串 payload 最多 512 字节；原始预览最多 64 字节，明确截断。旧 dump 缺少通用 payload 时仍可查询条目，但标记未完整读取。调用路径对比最多 100000 个不同路径、深度 64、单路径 4096 字节，超限报错，不返回伪完整比较。沿用单帧 128 MiB 和源文件变化校验。

## 接口

保留现有命令。新增 Tauri `frame_evidence(fileId, frameIndex, start, limit, countersOnly)` 和 `compare_frames(fileId, frameIndex, baselineFrameIndex, threadIndex, start, limit)`。

MCP 增加 `performance_frame_evidence`、`performance_compare_frames`，参数使用 snake_case；均受录制会话范围限制。加入 [Flow 查询](flow-decoding.md)后普通诊断现有 10 个性能工具。调用树增加 `selfMs` / `selfReason` / `isCounter`，通用 payload 单独分页读取，避免把树响应膨胀。首轮、源码和工程定位提示词均接入新证据。报告原有导出保持兼容。

## Editor 对照与复现

在对应版本的隔离 Editor 中加载录制、停止录制后，通过 Unity CLI `eval_file` 执行：

- `tools/export-evidence-reference.cs`：导出帧 0、1、127、511、999、1500、1999 的 metadata 原始字节、字段定义及独立数值 API 结果到系统临时目录。此文件只供测试，不是应用输入格式。
- `tools/inspect-recording-features.cs`：遍历完整已加载录制，仅输出计数和 Counter 名称清单。统计完整调用栈与 Flow 存在性，不宣称已经解码相应二进制结构。

```powershell
$env:UNITY_EVIDENCE_DATA_PATH = '<录制.data>'
$env:UNITY_EVIDENCE_REFERENCE_PATH = '<export-evidence-reference 返回的 JSON>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test frame_evidence editor_metadata_matches_production -- --ignored --nocapture
```

显式执行但缺输入会失败。私有录制和参考文件不提交、不交给真实 Agent。

本机两份录制对照：6000.3.9f1（391868208 字节）7 帧、30860 样本、9893 metadata 字段通过（首次约 34.85 秒）；6000.3.23f1（445696628 字节）7 帧、31929 样本、10876 字段通过（约 44.04 秒）。耗时包括生产导入和反复分页解码，不是性能或内存上限承诺。

6000.3.9f1 全文件 Editor 盘点：2000 帧、277215 个帧内线程条目、9800223 样本、3170479 个带 metadata 样本、121 种 Counter、1147984 个 Flow 事件；完整调用栈样本为 0。该结论只针对这份录制。后续已实现 [Flow 解码](flow-decoding.md)，确认位于线程末尾记录，样本表后另一段辅助记录仍未知。GPU 仍未新增可信指标。

最终自动验证：Rust 112 项、前端 31 项、Python 8 项通过；Release 构建和 18 项协议回归通过；公开/私有桌面检查分别 11/7 组通过（替代原生选择框返回值）。真实 Codex 公开样例报告见 [记录](evidence/public-data-evidence-report.md)。
