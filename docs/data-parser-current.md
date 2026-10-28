# 当前 .data 解析契约

更新：2026-10-28。本页描述当前代码；历史布局、性能和安装验收不自动适用于本轮产物。生产解析器位于 `crates/upaa-core/src/parser/data/`，桌面端复用同一实现。

## 版本和可信度

只接受 Unity 2022.3 LTS 与 6000.3.x，且格式 magic 必须匹配。6000.3.9f1 / 6000.3.23f1 保留已有**限定录制、限定指标**对照证据。其他 6000.3 补丁版本允许结构化导入，但 `versionVerified=false`；2022.3 的 counted thread suffix 也是候选适配，尚无本轮真实 Editor 对照，不能声明该版本已经验证支持。结构不匹配直接报帧号、线程或偏移错误。

2022.3 独立入口使用现有 2022 stats/marker/sample 基础及显式 counted suffix 候选，不再扫描 GC 或 Memory 特征，也不回退为帧头 CPU。Flow 的线程尾部业务含义尚未在 2022 验证，所以该版本仅保留未知区段证据，Flow 查询返回 unavailable。真实输入到位后必须对 suffix 逐段校验，再决定是否调整适配或提升验证状态。

`quality.status=unverified` 表示有解码观测但版本未验证；部分覆盖仍可为 partial，须同时看 reasons、单帧 versionVerified 和 meta.parsing.validation。A/B 确定性统计排除版本未验证帧，单独报告 unverifiedExcluded。内存指标独立处于 pending-editor-comparison，即使版本已有 CPU/GC 对照也不自动升级内存验证状态。

## 读取、GC 和时间

文件与 bytes 入口使用同一个有界顺序读取循环。统一拒绝空文件、缺结束标记、结束标记之后的额外字节、捕获内版本变化和越界结构。单帧二进制上限仍为 128 MiB；样本结构、名称复制、metadata 与 Flow 使用每帧 256 MiB 的保守分配预算，不等于进程总内存上限。

全部线程参与 GC，字节以 u64 累加，不经 float KiB。真实分配的 GC 索引、marker 与 typed metadata 必须一致；匹配失败报错，零记录与真实 0 B 保留。Unity 6000.3 的一种帧边界延续例外见下节。损坏树、未知 marker、NaN/负样本耗时不返回部分成功。主线程缺失、重复或根不唯一时 CPU 不可用，不妨碍独立有效的 GC。

### GC 帧边界延续

Unity 6000.3.23f1 实录中存在这样的样本：上一帧末尾的 GC.Alloc 已有索引及 Size metadata；下一帧在帧起点保留一个零耗时 GC.Alloc，重复 Size metadata，但没有新的 GC 索引。Editor Hierarchy 的该样本 GC Alloc 列为 0。直接累加 raw metadata 会重复计算。

当前仅接受已核对的窄条件：原始帧 ID 连续；同一原始线程 ID、组名和线程名；上一帧最后一个样本是有索引、metadata 一致、正耗时的叶子 GC.Alloc，结束时间精确等于当前帧起点；当前样本为哨兵根下的 sample[1]、零耗时叶子、开始于帧起点、相同 marker ID、无 GC 索引，且单字段 UInt32 Size metadata 与上一帧已计入字节数一致。不使用时间容差、不依赖 AndroidLogcat 名称或固定 marker ID。

该样本保留在树和原始证据中，标记 `gcContinuation=true`、`gcAllocBytes=null`；metadata 保留原值，不产生新的 GC 字节或分配次数，UI 显示“延续（不重复计入）”。每帧解码 checkpoint 保留前帧证据，任意顺序查询同样校验。不满足条件（包括缺前帧、帧 ID 跳跃、metadata 缺失/不符）仍失败；普通有索引的零耗时 GC.Alloc 正常计入。2022.3 未启用此例外；这也不是对所有跨帧样本形态的通用支持。

2026-10-28，在隔离 Unity 6000.3.23f1 Editor 中加载维护者提供的录制，以 RawFrameDataView 获取样本，以 HierarchyFrameDataView 的 `columnGcMemory` 独立获取 GC 统计。文件 SHA-256：`caed7b36a41e720c85ab8ae01560304655c567aee69a50d6dfe4fbd05b55c7da`。生产完整导入及逐帧、逐线程对照覆盖 2,000 帧、406,000 个线程实例；全部一致。全线程 GC 为 **736,039,400 B**，raw metadata 累加为 **736,039,784 B**，四个延续样本的 **384 B** 未重复计入。四帧及前帧的边界样本亦已导出，并通过倒序按需查询验证。

| 原始块索引（0 起） | 原始帧 ID | 线程索引 | 重复 metadata | Editor 本帧 GC（全线程） |
|---:|---:|---:|---:|---:|
| 267 | 2122 | 103 | 104 B | 3,625,093 B |
| 1593 | 3448 | 90 | 48 B | 1,968,087 B |
| 1596 | 3451 | 101 | 104 B | 4,083,450 B |
| 1770 | 3625 | 96 | 128 B | 4,375,701 B |

完整 Editor 参考 JSON 的 SHA-256：`824695db27bd1402cdfd900959138ed6ea05503cf446bafa59744f27386284d2`。

复现：在同版本隔离 Editor 已加载录制后，用 Unity CLI `eval_file` 执行 [export-gc-boundary-reference.cs](../tools/export-gc-boundary-reference.cs)，保留其返回的 JSON 路径。该脚本读取全帧全线程的 Hierarchy GC、raw metadata 总量，另保存上述四组边界样本及各自的 Hierarchy GC 列；不修改游戏工程。然后执行：

```powershell
$env:UNITY_GC_DATA_PATH = '<录制绝对路径>'
$env:UNITY_GC_REFERENCE_PATH = '<Editor 返回的 JSON 路径>'
cargo test --manifest-path crates/upaa-core/Cargo.toml --locked --offline --release --test gc_boundary editor_reference_full_capture_and_lazy_queries -- --ignored --nocapture
```

缺少输入、没有延续样本、统计差异或查询失败均使测试失败。公开合成回归在同一 `gc_boundary` 测试文件中，默认执行；私有录制和完整参考不提交。这一对照只新增本录制的 GC 证据，不扩大 GPU、内存或其他版本的验证结论。

帧时间来自相邻原始帧 ID 连续且时间未倒退的起始时间差，沿用 Editor float32 换算；相同起始时间允许真实零间隔。末帧、ID 跳跃或缺帧使对应区间不可用。2022.3 的零 CPU、零 gatheredData、仅头部的空块保留为 skipped 占位；其余不完整帧报错。meta.parsing 保留 rawBlocks、decodedFrames、skippedFrames；成功导入的 failedFrames 为 0，因为结构失败会中止整个导入。跳过帧仍参与覆盖率分母。

## 未知区段

`frame_sections` / `performance_frame_sections` 按原始块号分页：辅助记录、indexed records、post-GC 索引、样本索引表、线程未知标量及 opaque 帧尾。每行提供帧体相对 offset、byteLength、count、线程索引及最多 64 字节 rawHex，截断明确标记。`semanticStatus=unknown` 不代表完整理解，也不能推断为调用栈、GPU 数据或依赖关系。

普通帧详情提供 unknownSectionCount。帧尾可以为 opaque 数据，不要求强行消费成已知业务字段。已有 CPU/GC、Counter、Flow 校验相互独立。UI 在“帧证据与调用路径对比”中提供未知区段读取。

## 内存 Counter

精确匹配八个名称：Total Used Memory、Total Reserved Memory、GC Used Memory、GC Reserved Memory、Gfx Used Memory、Profiler Used Memory、Profiler Reserved Memory、System Used Memory。不猜别名或 ID；要求 Counter 标志、单字段、bytes 单位及非负整数 payload。

同帧同名观测必须完整且一致才能产生值。缺失、冲突、负值、类型或单位不符分别记录原因。保存 threadId、sampleIndex、markerId 的最多 16 个来源及 sourceCount；完整样本证据仍可从 frame_evidence 分页读取。整数、峰值和正负变化量以十进制字符串传输。

快照新增 memory，含摘要和逐帧观测；旧快照缺字段时不可用。`memory_series(fileId,name,start,limit)` 与 `performance_memory(name,start,limit)` 按数组偏移分页，limit 1..50，响应行预算 20 KiB，nextStart 为唯一续页依据。MCP 摘要移除序列，只保留摘要；UI 每页曲线最多 50 帧，缺失点不连线，按钮可跳转原始帧 CPU/GC。首末变化仅针对有效端点，不能当作泄漏证据，不新增默认 A/B 内存预算。

## 查询和取消

FrameStore 返回 Arc 共享不可变结果，完整树 Self 在物化时计算一次。LRU 默认最多 4 帧、64 MiB 容量预算，计入容器容量、字符串和 metadata；超过缓存预算的帧不缓存。同帧并发请求通过临时 single-flight 共享一次解码，包括超预算帧；最后一个参与请求退出后临时结果释放。超预算帧的后续独立请求会重新解码。缓存命中仍读取目标块并校验文件长度与字节指纹，因此同尺寸改写仍会拒绝，优化的是解码成本而非全部 I/O。

取消信号在文件分块、帧和长记录读取中检查；普通导入支持 cancel_parse，release_file 也会取消正在解析的源。项目 A/B 通过既有取消动作停止，失败或取消不替换原 A/B。提交完成是取消边界；已完成的导入不因后续取消另一任务而失效。查询 marker checkpoint 使用独立的取消信号。

## 复现与待验

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline
cargo test --manifest-path crates/upaa-core/Cargo.toml --locked --offline
npm test
npm run build
python -B -m unittest discover -s tools -p 'test_*.py'
```

公开 `data_repairs` 覆盖版本、字节精度、多线程、缺失主线程、严格 EOF、未知帧尾、取消、缓存、内存与有界随机损坏。真实数据由维护者后续提供。

内存对照：在隔离 Editor 已加载录制后执行 `tools/export-memory-reference.cs`，独立获取 Counter API 存在性与值，再设置 UNITY_MEMORY_DATA_PATH、UNITY_MEMORY_REFERENCE_PATH，执行 `cargo test --manifest-path src-tauri/Cargo.toml --test data_repairs editor_memory_counter_reference -- --ignored`。缺输入必须失败；sample metadata 与 Counter API 如不一致应调查，不能修改参考值以迎合解析结果。脚本本轮尚未在 Editor 编译执行。

调用栈研究与采集要求见 [调用栈可行性](callstack-research.md)。GPU 不在本轮范围。
