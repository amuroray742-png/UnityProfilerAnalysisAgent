# ExtractProfilerDump.cs — 穷尽导出 .data 所有字段

目的：使用 Unity 6000.3 官方的 `UnityEditor.Profiling.RawFrameDataView` API，
把 `.data` 文件里所有可见的字段（每帧、每线程、每 sample 的 marker_id / 名字 /
耗时 / 子节点 / GC.Alloc 字节 / category index）穷尽导出为 JSON。

把 JSON 当成 ground truth，再用它去比对 .data 二进制字节，反推 Unity 6 wire format。

## 用法 A：Unity Editor GUI（最简单）

1. 打开任意 Unity 6000.3.23f1 项目（哪怕是新建的空项目）
2. 把 `ExtractProfilerDump.cs` 复制到 `Assets/Editor/ExtractProfilerDump.cs`
3. 等 Unity 编译完成
4. 菜单 `Tools > Extract Profiler Dump…`
5. 选择你的 `.data` 文件
6. 选择 JSON 输出路径
7. 等待运行（64 帧约 30-60 秒）
8. 把生成的 JSON 发我

## 用法 B：命令行 batch mode（适合脚本化）

```powershell
& 'C:\Program Files\Unity\Hub\Editor\6000.3.23f1\Editor\Unity.exe' `
  -batchMode `
  -nographics `
  -projectPath 'C:\work\unity_projects\ProjectJinn' `
  -executeMethod ExtractProfilerDump.ExtractProfilerDump.RunAll `
  -inputFile 'C:\work\unity_projects\ProjectJinn\ProfilerCaptures\If Jinn__2026-09-17_14-16-24.data' `
  -outputFile 'C:\Users\wepie\AppData\Local\Temp\jinn_dump.json' `
  -quit
```

> 注意：Unity 在 batchMode 下第一次启动会先打开项目（导入资源），可能几分钟。
> `-projectPath` 必须是一个 **已存在** 的 Unity 项目（可以新建空项目）。

## 输出 JSON 结构（精简）

```json
{
  "input_file": "...",
  "captured_at": "...",
  "unity_version": "6000.3.23f1",
  "frame_count": 2000,
  "frames": [
    {
      "frame_index": 0,
      "frame_time_ms": 36.178,
      "frame_gpu_time_ms": 0,
      "sample_count_total": 1234,
      "gc_alloc_bytes_total": 1268424,
      "threads": [
        {
          "thread_index": 0,
          "thread_id": 1,
          "thread_name": "Main Thread",
          "thread_group_name": "",
          "max_depth": 32,
          "gc_alloc_total_bytes": 1268424,
          "markers": [
            {"id": 102, "name": "PlayerLoop", "category_index": 0, "flags": ""},
            {"id": 103, "name": "BehaviourUpdate", "category_index": 1, "flags": ""}
          ],
          "samples": [
            {
              "sample_index": 0,
              "marker_id": 102,
              "marker_name": "PlayerLoop",
              "time_ms": 36.178,
              "start_time_ms": 0,
              "children_count": 5,
              "category_index": 0,
              "metadata_count": 0,
              "gc_alloc_bytes": 0
            },
            ...
            {
              "sample_index": 339,
              "marker_id": 7,  // GC.Alloc id
              "marker_name": "GC.Alloc",
              "time_ms": 0.001,
              "start_time_ms": 0,
              "children_count": 0,
              "category_index": 17,
              "metadata_count": 1,
              "gc_alloc_bytes": 24   // ← 这就是我要的「真实值」
            }
          ]
        }
      ]
    }
  ]
}
```

## 拿到 JSON 后我会做的事

1. 在你给的 JSON 里 grep `GC.Alloc` 样本的 `(sample_index, gc_alloc_bytes)`
2. 在 `.data` 二进制 frame body 里找到相同 `(sample_index, gc_alloc_bytes)` 对
3. 推算 GC.Alloc metadata 在 .data 中的字节布局
4. 同样地，按时间定位每个 marker 的 `time_ms / start_time_ms / children_count`
5. 反推 main thread samples 表格布局
6. 实现 Unity 6 body 完整解码（不再依赖盲扫）

## 文件限制

- 默认只导前 64 帧。Unity Editor 跑全 2000 帧会卡死 5-10 分钟。
- 64 帧足够推断布局，因为布局是帧间固定的（layout 不变）
- 如果你想跑更多帧，把代码里 `int maxFrames = Math.Min(frameCount, 64);` 改成你想要的数

## 需要你做的

1. 跑一次（GUI 或 batch mode 都行）
2. 把生成的 JSON 路径告诉我
3. 我会用 JSON + .data 二进制做 reverse engineering

如果遇到任何编译错误（API 命名可能在 Unity 6000.3 与 Unity 2022.3 略有差异），
告诉我具体错误信息，我帮你修。