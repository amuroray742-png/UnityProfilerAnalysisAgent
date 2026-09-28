# Unity 导出与解析对照操作

本目录有两个用途不同的脚本。ExtractProfilerDump.cs 已在 Unity 6000.3.23f1 中编译并实际重新导出；ProfilerJsonExporter.cs 仍是未验收示例。当前进度见[验证台账](../project-status.md)。

## 两种 JSON 使用独立解析分支

| 脚本 | 用途 | 输出 / 限制 |
|---|---|---|
| [ExtractProfilerDump.cs](ExtractProfilerDump.cs) | 使用 Editor RawFrameDataView 导出离线录制的线程、样本及 metadata，供布局研究 | 根字段 input_file、unity_version、frame_count；数据在 frames[].threads[].samples[]；由应用独立 dump 分支直接导入 |
| [ProfilerJsonExporter.cs](ProfilerJsonExporter.cs) | 示例：构造应用 V1 JSON | meta + frames，含 cpuMs、gcAllocBytes、mainThreadSamples；脚本编译、API 适用性及跨帧采样逻辑待验证 |

应用 V1 示例见 [README](../../README.md#项目约定-json-示例)，解析定义见 [json.rs](../../src-tauri/src/parser/json.rs)。应用现在直接识别研究 dump，无需转成 V1。dump 结构错误会报告，不再回退成空 V1；帧数以实际导出数组为准，根 frame_count 作为录制声明数量另行展示。

## 导出研究用 dump

2026-09-28 已通过连接中的 Windows Unity 6000.3.23f1，使用 Unity CLI 安装为 Editor 脚本并完成编译、重新导出及生产入口对照。验证项目中工具位置为 Assets/Editor/ProfilerAnalysisValidation/ExtractProfilerDump.cs；未修改游戏场景。新启动 batch mode 的完整流程仍未单独验收。

1. 准备安装了对应 Unity 版本的 Editor 项目；优先使用与录制一致的 6000.3.23f1。
2. 将 ExtractProfilerDump.cs 复制到该项目的 Assets/Editor 目录。
3. 等待编译，确认 Console 无错误。若 API 或 namespace 编译失败，记录 Unity 精确版本及错误；此时尚未产生有效对照。
4. 通过菜单 **Tools > Extract Profiler Dump…** 选择 .data 文件与输出 JSON。
5. 检查日志中 loaded 为 true、存在 wrote 输出，并检查 JSON 的 frames / threads / samples 是否非空。脚本现在拒绝加载失败、无效帧范围、空样本及序列化遗漏 frames；仍应运行解析对照，不能只凭退出码认定正确。

也可通过 PowerShell batch mode 调用。先将以下占位路径替换为本机实际路径：

```powershell
$unityEditor = '<Unity Editor 安装目录>\Editor\Unity.exe'
$unityProject = '<已存在且包含脚本的 Unity 项目绝对路径>'
$capturePath = '<录制文件绝对路径>.data'
$dumpPath = '<输出文件绝对路径>.dump.json'

& $unityEditor -batchMode -nographics -projectPath $unityProject -executeMethod ExtractProfilerDump.ExtractProfilerDump.RunAll -inputFile $capturePath -outputFile $dumpPath -startFrame 0 -maxFrames 64 -quit
```

完整 executeMethod 是命名空间、类名和方法名。输出目录应事先存在；首次导入项目可能耗时，本项目尚未验证统一的导出耗时预算。

## 输出范围与检查

脚本默认从第 0 帧导出至多 64 帧。可使用 -startFrame 与 -maxFrames，后者范围为 1–64；接近录制末尾时自动缩短范围。根 frame_count 按 lastFrameIndex + 1 计算，不代表 frames 数组实际导出的长度；使用数组长度确认范围，并逐帧检查数据有效性。负数起点、超过末帧的起点及无效数量直接报错。

已连接 Unity CLI 的 Editor 可直接调用公开入口，例如在已发现 eval 命令后执行：

```powershell
unity command eval 'ExtractProfilerDump.ExtractProfilerDump.RunExtraction(@"<录制路径>", @"<新的输出路径>", 1856, 8); return "exported";' 600000 --project-path '<Unity 项目路径>' --timeout 600 --detach --format json
```

用返回的 jobId 查询 unity job status，直到任务完成，再校验输出。以上范围必须存在于录制中。脚本需要先作为 Editor 资源正常编译；仅在内存中动态编译会导致本机 Unity JsonUtility 遗漏 List 字段，这种输出不作为参考。

结构如下（字段类型示意，不是可导入的实际样本）：

```text
root
  input_file / captured_at / unity_version / frame_count
  frames[]
    frame_index / frame_time_ms / frame_gpu_time_ms
    sample_count_total / gc_alloc_bytes_total
    threads[]
      thread_index / thread_id / thread_name / thread_group_name
      gc_alloc_total_bytes
      markers[]
      samples[]
        sample_index / marker_id / marker_name
        time_ms / start_time_ms / children_count
        category_index / metadata_count / gc_alloc_bytes
```

保存录制版本、来源、帧范围以及导出日志。比较时区分主线程 GC 与全线程 GC，确认帧对应关系和时间单位。私有录制与完整 dump 留在仓库外或已忽略的 samples/private 目录，不提交到 Git。

## 验证正式导入链路

在仓库根目录设置 UNITY_PROFILER_DUMP_PATH 后，执行：

```powershell
$env:UNITY_PROFILER_DUMP_PATH = '<64 帧参考 dump 的绝对路径>'
cargo test --manifest-path src-tauri/Cargo.toml --locked --test editor_dump real_dump_production_path -- --ignored --nocapture
```

该测试对指定参考录制断言 64 帧、声明 2,000 帧、首帧 2,076 个主线程样本和 136 字节 GC，并逐帧核对。显式运行但缺环境变量或文件时失败。通用合成 fixture 测试随默认 cargo test 执行；不要将参考录制固定断言用于其他录制。

## 运行历史研究测试

在仓库根目录设置对照文件路径后执行：

```powershell
$env:UNITY_PROFILER_DATA_PATH = '<录制文件绝对路径>.data'
$env:UNITY_PROFILER_DUMP_PATH = '<对照文件绝对路径>.dump.json'
cargo test --manifest-path src-tauri/Cargo.toml --locked --test unity6_body_verify -- --ignored --nocapture
```

依赖已缓存时可追加 --offline。环境变量只改变文件来源，**不能让固定偏移测试适配任意录制**：

- 主线程测试只对首帧、固定偏移及特定 marker ID 比较；适用于其原始研究样本，不能作为通用解析正确性的证明。
- 全线程测试找不到 sample 也可能通过，必须阅读输出。
- 缺少文件时会直接返回成功，必须确认确实读取并比对了数据。
- [全文件测试](../../src-tauri/tests/unity6_full_parse.rs)的路径仍硬编码，不读取上述环境变量；当前换机需要后续测试改造，不能套用此命令作为通用全文件验证。
- [另一组真实文件测试](../../src-tauri/tests/unity6_real_data.rs)同样属于默认忽略的研究测试，应先阅读输入要求。

## 后续对照流程

将现有固定偏移验证保留为布局证据；二进制后续验证应直接调用生产 parse_path，按明确帧标识和线程逐项比较 CPU、sample 数量与关系、GC metadata 和总量。尤其要验证自动定位结果，而非只从已知偏移读取。

已知生产入口与局部对照的 GC 差异、测试范围和后续门槛统一记录在[项目状态](../project-status.md#历史基线与未解决问题)。遇到差异时保留输入标识、版本和日志，不能因为全文件遍历成功就把数值认定为可信。
