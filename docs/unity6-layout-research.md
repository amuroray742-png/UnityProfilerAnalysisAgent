# Unity 6000.3 二进制布局对照记录

更新：2026-09-28。P0 第二阶段已完成两份录制及加载高峰、末帧的无扫描生产入口对照。验收范围限定 Windows Unity 6000.3.23f1；不能外推至未对照帧或其他版本。下文分别保留当前验收与早期研究证据。

## 多录制与 Editor 重新导出验收

仓库脚本原先将 ProfilerDriver 错误指向 UnityEditorInternal.Profiling；本机 API 和已能工作的项目副本均为 UnityEditorInternal.ProfilerDriver，现已修正。脚本通过 Unity CLI 在连接中的 Editor 正常导入、编译（failed=false），再使用 RawFrameDataView 重新导出。增加了 startFrame / maxFrames 范围参数和加载、空样本、序列化遗漏检查；默认 64 帧，范围上限 64。负数起点拒绝，以及从 1,999 开始请求两帧后实际导出一帧，都已在 Editor 中验证。

| 参考范围 | 导出线程实例 | 样本数 | GC 总字节 | dump 大小 |
|---|---:|---:|---:|---:|
| 录制 A，0–63（已有参考） | 2,560 | 240,350 | 687,178 | 146,127,279 |
| 录制 B，0–63（重新导出） | 9,984 | 712,862 | 2,292,252 | 406,640,706 |
| 录制 A，1,856–1,863（加载高峰） | 296 | 350,448 | 15,914,787 | 187,417,304 |
| 录制 A，1,999（末帧） | 37 | 2,366 | 17,792 | 1,432,543 |

合计 137 个参考帧、1,306,026 个样本。四组都显式执行 [real_capture_without_reference_assisted_location](../src-tauri/tests/unity6_structured.rs)，同时核对结构解码、dump 正式导入、data 生产入口的样本、CPU、GC、逐站点父样本与线程归因；136 个非末帧的录制帧时间与 Editor float32 值一致，末帧在二进制路径中明确不可用。两份录制各 2,000 帧完整解析，但只有上述范围有 Editor 指标对照。

最终各组测试耗时约 23.25、67.55、25.22、17.56 秒；包含读取参考 JSON、再次导入 JSON、结构遍历与生产解析，部分并发执行，不是单次解析性能或内存基准。

新增参考文件 SHA-256：

```text
录制 B 0–63: ba68461db15bebc6f67da07cb849e7eb2b90068f712ec9ad75ac834dc88dea14
录制 A 1856–1863: 77cf8652ee5877e6c0ff6823d7c2dce1e99c92e8d952eb79fdd34d8816657686
录制 A 1999: 37d342dc87ec7a6833a7a075ce654ddcc5e6230f9ecccbf80a5c7dedd77037ae
```

仅在内存动态编译脚本的首次尝试产生了不含 frames 的 226 字节输出；该输出未用于验证并已删除。最终使用正常编译的 Editor 资源，所有完整参考文件均保存在仓库外。全新 batch mode 启动仍未单独验收。

## 当前生产路径

[Rust 结构解码器](../src-tauri/src/parser/data/unity6_structured.rs)从帧头连续读取 stats、辅助计数区段、marker 定义、线程头、样本和 metadata。它不接收 dump 信息，不搜索名称或候选字节。仅在文件声明版本为 `6000.3.23f1` 时接入正式导入；其他 Unity 6 版本的帧头 CPU 结果标记为 estimated。

关键修正是 **marker 定义跨帧保留**。参考录制首帧定义 3,430 个 marker，后续帧的空定义表不能清空映射。该录制的 stats / marker 格式与现有 2022 计数布局一致，旧 `unity6_markers` 中推测的另一套格式不用于新解码路径。

正式导入输出唯一 Main Thread 根样本 CPU、inclusive 热点、全部线程 GC 及最近非 GC 父样本站点。GC 的索引记录、marker 名称和通用 metadata payload 必须交叉一致。录制帧时间使用相邻帧 start_ns 差值，转换公式为 f32(interval_ns) × f32(1e-6)，与参考 Editor 值一致；末帧或时间戳倒退时缺失，聚合显示覆盖率。渲染计数继续不可用。

原始帧头、线程 ID、样本 ID、父节点和 metadata 数量保留在解码器结果中；当前应用汇总模型仍未提供完整调用树查询，这是后续 P1 工作。

### 新增验收证据

[Rust 集成测试](../src-tauri/tests/unity6_structured.rs)直接验证无参考输入的解码器，并另行调用生产 `parse_file`：

- 参考录制全部 2,000 帧结构通过；前 64 帧、2,560 个线程、240,350 个样本的 marker 名称、分类、时间、树结构、metadata 数量、GC 字节与 dump 一致。
- 生产入口 2,000 帧通过；前 64 帧的 CPU、主线程样本数量、GC 总量与每条站点的线程、父样本名称、字节数一致。
- 最终一次上述联合验证约 21.88 秒，包含读取参考 JSON、结构解码和再次运行生产入口，不是单次导入性能基准。
- 第二份本机 6000.3.23f1 录制为 1,162,295,388 字节：早期先通过 2,000 帧、27,935,530 个样本结构解析，约 24.40 秒；之后补齐了上表的 Editor dump 与生产指标对照。单独的结构测试逐帧丢弃结果，不能用来证明应用全量缓存的内存上限。
- 5 项公开 Rust 回归覆盖跨帧 marker、所有字节截断边界、marker/时间/树/GC 损坏、非空 post-GC 索引区段、文件与 bytes 生产入口一致性及快照缺失值。

```powershell
$env:UNITY_PROFILER_DATA_PATH = '<参考录制绝对路径>'
$env:UNITY_PROFILER_DUMP_PATH = '<参考 dump 绝对路径>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test unity6_structured real_capture_without_reference_assisted_location -- --ignored --nocapture

$env:UNITY_PROFILER_ADDITIONAL_DATA_PATH = '<第二份 2000 帧录制绝对路径>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test unity6_structured additional_capture_structure_only -- --ignored --nocapture
```

真实数据测试默认忽略；显式执行但变量未设置或文件缺失时失败。第二份测试只检查结构，不替代生产指标对照。

## 初期 dump 辅助定位证据

[严格对照工具](../tools/verify_unity6_layout.py)通过外部提供的 Editor dump 定位线程头，验证全部导出帧及线程。工具不使用固定 marker ID、固定样本表偏移或固定 GC 偏移；候选缺失、候选不唯一、字段不一致、区段越界均直接失败。

本次 Unity 6000.3.23f1 对照结果：

| 项目 | 结果 |
|---|---|
| `.data` 大小 | 445,696,628 字节 |
| dump 大小 | 146,127,279 字节 |
| 声明录制帧数 / 已验证导出帧数 | 2,000 / 64 |
| 已验证线程实例 | 2,560 |
| 已逐字段核对样本 | 240,350 |
| 全部导出线程 GC 合计 | 687,178 字节 |
| Editor 线程 ID 符号扩展表示差异 | 448 个线程实例 |
| 根 marker 哨兵表示差异 | 2,496 个根样本 |

文件 SHA-256：

```text
data: 4db9c146222f165cb057027de486cd7d242900549407c72c9b8418ed235272f0
dump: c560a26fb7192f478cc1bea78174743098261e08f2c8d8005fb404ef250575bd
```

当前 dump 与第一阶段记录的 143,083,720 字节文件大小不同，因此不把历史结果直接当作当前文件验证。重新执行 Rust 生产入口测试通过：64 帧、声明 2,000 帧，首帧 2,076 个主线程样本与 136 字节 GC，全部帧 CPU/GC 对照一致；该次测试约 5.86 秒。

布局研究工具最终一次耗时约 5.69 秒，包括读取 JSON、二进制对照及完整文件哈希；不作为生产性能或内存上限承诺。

## 已验证的线程区段

初期工具的线程区段起点依赖参考 dump；后续 Rust 解码器已从前置计数结构连续到达该起点。以下结构在全部 64 帧中经对照确认，并在两份完整录制中验证了结构边界：

```text
u32 thread_count
repeat thread_count:
  u64 raw_thread_id
  nul_terminated_group_name, padded to 4 bytes
  nul_terminated_thread_name, padded to 4 bytes
  u32 sample_count
  repeat sample_count:
    u32 raw_marker_id
    f32 duration_ns
    u64 start_ns
    i32 direct_child_count
  u32 section_a_count                 # 目前样本中必须为 0；非零拒绝
  u32 indexed_record_count
  repeat indexed_record_count:
    u32 sample_index
    u32 opaque_value
  u32 gc_record_count
  repeat gc_record_count:
    u32 sample_index
    u32 allocation_bytes
  u32 post_gc_sample_index_count
  u32[post_gc_sample_index_count] sample_indices
  u32 metadata_record_count
  repeat metadata_record_count:
    u32 sample_index
    u32 field_count
    repeat field_count:
      u32 type_tag
      u32 payload_byte_count
      bytes payload, padded to 4 bytes
  u32 sample_index_count
  u32[sample_index_count] sample_indices
  u32 opaque_scalar_1
  u32 opaque_scalar_2
  u32 trailing_record_count
  bytes[trailing_record_count * 12] opaque_records
```

校验内容：

- 样本索引、marker ID、耗时、开始时间和直接子节点数量逐项对照；同时验证树闭合和边界。
- GC 记录的数量、样本索引、逐项字节、线程总量及帧总量必须全部相等。通用 metadata 中的 GC 记录也要一致：本录制中其 tag 为 3、payload 为 4 字节。
- 通用 metadata 的有效样本覆盖和 field_count 与 dump 一致。观察到 `(sample_index=0, field_count=0)` 的空记录，允许重复；有字段的记录不可重复。
- 每个线程定位必须唯一，线程不能共享样本表；前一线程的区段终点必须等于下一线程头。
- 初期工具的 marker 名称来自 dump；Rust 解码器独立读取 marker 定义，并在 64 个参考帧中核对了全部名称和分类。

这不是完整格式规范。未命名字段只验证结构长度或索引边界，不宣称已掌握其业务含义；末线程之后的帧尾区段尚未解析。

完整录制的块索引 1,858 首次出现非空 post-GC 区段：两个样本索引之后直接进入通用 metadata 计数。Rust 解码器按计数和样本边界读取，并保留原始索引；其业务含义尚未知。初期 Python 对照工具仍只覆盖前 64 帧的空区段布局，不能用它验证全部 2,000 帧。

## Editor 与原始字段的表示差异

1. 二进制根样本可使用 `0xFFFFFFFF` marker，Editor 导出为 `0`。该映射仅允许出现在根样本，非根样本不套用这个例外。
2. 部分 Editor thread_id 为原始 ID 低 32 位的有符号扩展后再转无符号 64 位。工具只接受精确的原 ID 或该明确转换，不接受任意低 32 位碰撞；转换次数进入报告。未来生产模型应保留原始 ID 和来源，不能直接用此兼容规则重写身份。
3. 样本耗时对照采用 `f32(duration_ns * f32(1e-6))`，与 Editor 的浮点毫秒表示一致；开始时间采用 `f32(start_ns / 1e6)`。不以宽松的毫秒级误差隐藏样本错位。

## 复现

需要 Python 3.11 或更新版本，仅使用标准库。私有文件不进入仓库。

```powershell
python -B -m unittest discover -s tools -p 'test_verify_unity6_layout.py' -v
python -B tools/verify_unity6_layout.py --data '<录制绝对路径>' --dump '<对应 dump 绝对路径>'
```

`--report '<输出路径>'` 可保存逐帧线程偏移及计数报告，不含原始样本内容。命令缺少文件、结构不支持或任何对照失败时返回失败，不会成功跳过。

[公开合成测试](../tools/test_verify_unity6_layout.py)覆盖零 GC、非零 GC、缺失/歧义定位、逐样本字段错误、GC metadata 缺失或不一致、尾部截断、线程 ID 表示差异、根 marker 限定及部分导出。本轮 8 项通过。

## 接下来

1. 将保留的原始帧 ID、线程 ID、样本树接入有界单帧查询，替换全局热点冒充调用树的接口。
2. 在可信查询基础上接通真实 ACP / MCP 协议、取消及会话隔离。
3. 继续扩大版本和输入范围、核对未知区段；不能仅凭结构能遍历就提升支持承诺。
4. 完成完整桌面交互、缓存释放、内存测量与发布验收，再评估可信分析 MVP。
