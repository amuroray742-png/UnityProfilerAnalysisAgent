# 第二阶段 Marker 补点：公开工程验证

2026-09-29，真实 Codex，公开 Unity 工程与合成录制，206.41 秒完成，Editor 不可用的离线范围。以下为模型原始输出（仅整理空行），不是已确认的录制因果。人工核对 Assets/AllocationWork.cs 第 9 行、原始代码及同步补点示例；模型撤回首轮整方法包裹建议，未为 Unmapped.Native 编造入口。测试验证真实性能/代码工具读取与源码保持不变。此单次样例不代表任意模型和输入质量。

我会仅使用本次 unity-profiler 的 performance_* 与 project_* 工具，先核对录制和工程证据，再调查帧 160 的尖峰及帧 100 的对照，并按实际读到的代码评估首轮补点方向。帧树确认了帧 160：`AllocationWork.Update` 为 30 ms，其中 Self 为 27 ms；主线程两条嵌套 `GC.Alloc` 共记录 8,388,600 字节，Worker 另有 8 字节。帧 100 没有这些导出路径，只能作为显式对照。录制与离线工程版本一致，Editor 尚未验证；接下来核对源码、资源引用和 Worker 的 Flow 证据。已读源码的 `Update()` 只有分配数组和写入首字节两条语句，Prefab、场景也存在对应的静态引用链；这些证据仍不能证明帧 160 使用了这版代码。材质字段未在该方法中使用，暂不据此提出渲染优化。`Unmapped.Native` 未找到源码入口，Flow 数据 unavailable，Editor 摘要采集也失败；补点方案将只细化数组分配这一同步语句，撤回整个 Update 再包一层的方向。Unity 工程性能定位报告

**结论与证据质量**

优先调查 `AllocationWork.Update` 的大数组分配；`Unmapped.Native` 保留为待定位项。当前只能完成离线候选定位，尚未完成 Editor 联合验证，也未修改代码、资产或补点。

完整首轮报告已核对。`performance_session_summary` 与 `project_summary` 显示：

- 录制 `public-source-peak.json`，来源 `unity-editor-dump`；21 个导出帧、时长指标 90 ms。CPU、GC 指标均 available，覆盖 21/21，录制警告为空。
- 逐帧数据确认：帧 160 主线程 40 ms、GC 8,388,608 字节；其余 20 个导出帧主线程均为 1 ms、GC 为 0。P95 为 1 ms/0 字节，是此次样本中的孤立峰值；默认筛查阈值不代表项目预算。
- 录制与离线工程版本均为 `6000.3.23f1`；这不证明录制使用了当前源码。录制平台、目标帧率、采集时间、源码构建指纹未提供。
- 工程作用域短标识 `0dfc4a62`，18 个文件。Editor 版本、平台、采集时间和指纹缺失；四次相关 `project_asset` 的 Editor 证据均 unavailable，原因为 `COMMAND_FAILED`。
- Draw Call、SetPass、Batches、Triangles、Vertices 均 unavailable。没有 GPU 耗时证据。
- 已读查询无待续 `nextStart`，代码无 `lineTruncated`，调用树无深度截断或查询警告。场景仅专项读取 160–232 行。引用覆盖始终 partial；GC metadata 三条均截断，因为输入未提供通用 payload，不能视为完整 metadata。

帧 100 是明确选择的对照，尚不能称为正常帧。主线程按完整路径比较如下，父子 inclusive 不相加：

| 路径 | 帧160 inclusive / Self | 相对帧100增量 | 调用次数160/100 |
|---|---:|---:|---:|
| `Main Thread` | 40 / 10 ms | +39 / +9 ms | 1/1 |
| `Main Thread → AllocationWork.Update` | 30 / 27 ms | +30 / +27 ms | 1/0 |
| 上述路径 → `GC.Alloc` | 2 / 1.5 ms | +2 / +1.5 ms | 1/0 |
| 上述路径 → `GC.Alloc → GC.Alloc` | 0.5 / 0.5 ms | +0.5 / +0.5 ms | 1/0 |
| 上述路径 → `Unmapped.Native` | 1 / 1 ms | +1 / +1 ms | 1/0 |

这些 Self 均通过工具校验，包含等待与未细分工作，并非纯计算时间。帧时间 50 ms 与主线程 40 ms 的差额不作归因。

**热点一：AllocationWork.Update，优先验证**

观测：帧 160、Main Thread #0、线程 ID `1000`、marker ID `113`，调用一次，inclusive 30 ms、Self 27 ms。其两条嵌套 `GC.Alloc` 分别记录 8,388,596 和 4 字节，最近非 GC 父样本归因合计 **8,388,600 字节**。Worker 另有 8 字节，与帧总量核对一致；分配不等于回收停顿。

实际源码：`Assets/AllocationWork.cs`，`Update()` 第 7–11 行，离线 SHA-256 短指纹 `869776fd`，末次重读一致。原始片段：

```csharp
// 第9行
lastFrame = new byte[8 * 1024 * 1024];
// 第10行
lastFrame[0] = 1;
```

关键静态引用链：

`Assets/PublicScene.unity` 的 PrefabInstance `2023790152`，第226行 `m_SourcePrefab`（fileID `100100000`，GUID `153ad8fca29416348ad0a375e53f5006`）
→ `Assets/Public.prefab`
→ MonoBehaviour fileID `4838056683082871320`
→ 第104行 `m_Script`（fileID `11500000`，GUID `c5c0075c3e6b0ff4cbb0d1fab51c9733`）
→ `Assets/AllocationWork.cs`。

Prefab 第102行声明 `m_Enabled: 1`。场景和 Prefab 短指纹分别为 `ba4a9aa5`、`b346af47`；均来自离线读取。脚本路径已解析，但脚本对象 `objectResolved=false`；Prefab 有效覆盖和录制当帧实例未验证。

同组件第107行 `effectsMaterial` 指向 `Assets/PublicMaterial.mat`，fileID `2100000`，GUID `4af2d389ec5b2f2458be9e7984c89ad1`。源码第5行声明该字段，但已读 `Update()` 未使用它，因此不作为此次分配原因。

事实是当前源码每次执行该方法都会创建新数组。关联候选是它与同名录制 marker、静态组件链对应；假设是第9行分配及初始化参与尖峰。录制没有源码构建指纹、分配调用栈或运行实例证据，不能确认全部 27 ms Self 来自该语句，字节数也不能证明二者精确对应。

建议条件：若真实业务允许复用数组、没有外部持有旧数组或并发消费者，可由程序将分配移至初始化或容量变化处，并按语义重置数据。代价是持续保留内存、管理生命周期；复用不会自动保持“新数组全零”语义。若这是有意构造分配的测试负载，应保留测试行为。

同场景 A/B：先确认录制代码及组件版本，再保持平台、质量、输入、实例数量和预热方式一致，仅改变分配策略；比较该路径调用次数、inclusive/Self、子 GC.Alloc 字节及尖峰频率，同时验证数据内容。暂不承诺收益比例。

**热点二：Unmapped.Native，尚不能给出优化修改**

观测：帧 160、Main Thread #0，完整路径 `Main Thread → AllocationWork.Update → Unmapped.Native`，marker ID `152`，一次，inclusive/Self 均为 1 ms；帧100该导出路径缺席。其下没有导出的 GC.Alloc 子样本。

按完整名称、`Unmapped` 类型词和 `Native` 方法词搜索，未找到业务源码入口；`NativeFormatImporter` 命中仅为资源 meta 文本，不能关联该 marker。没有可引用的源码行、原始业务片段、资源对象或指纹。也不能把它归因到数组初始化、材质或引擎等待。

建议先取得该 marker 的生成来源、符号映射或真实业务调用栈，再定位可控入口；目前不建议缓存、对象池或资源调整。代价是重新采样及符号准备。入口确认后，在同场景仅改变该入口对应行为，比较完整路径次数、CPU 区间、GC 和尖峰频率。

**具体 Marker 补点方案：仅一项**

首轮 `Perf.Update.BusinessScope` **细化并撤回原名及整方法包裹**：已有 `AllocationWork.Update` 能回答方法总耗时，再包第9–10行近乎重复。证据缺口是帧160的 27 ms Self 尚未定位到数组分配同步语句。

候选位置：已读 `Assets/AllocationWork.cs:9`，`Update()`，指纹 `869776fd`；原始代码为：

```csharp
lastFrame = new byte[8 * 1024 * 1024];
```

建议只包裹第9行，固定名称 `Perf.AllocationWork.AllocateBuffer`，验证该同步区间在新录制中的耗时、调用次数及子分配。以下仅为示意，非可应用补丁：

```csharp
// 类级静态字段
static readonly Unity.Profiling.ProfilerMarker AllocateBufferMarker =
    new Unity.Profiling.ProfilerMarker("Perf.AllocationWork.AllocateBuffer");

// Update 中仅包裹原第9行
using (AllocateBufferMarker.Auto())
{
    lastFrame = new byte[8 * 1024 * 1024];
}
```

不逐帧创建 Marker，不动态拼名，不跨 await/yield 或线程迁移。代码位置可确认，旧录制因果仍待验证；其他 `Update` 命中为 `Assets/OtherWork.cs:4` 的旋转方法，指纹 `9e009b96`，不混用。

由程序手动补点、策划同场景复现后重录，对比新 Marker 调用次数、CPU 区间、子 GC.Alloc 字节、尖峰频率；另比较补点开关或等价录制条件以检查采样开销，不用区间差值推算因果等待。文件变化须重新读取；补点不能补回旧数据，也不等于优化。

`Unmapped.Native` 尚无法给出具体补点位置，需要真实业务入口或符号证据；不在资源文件中插点。

**未定位项与覆盖边界**

- 帧160 Main Thread 根样本 Self 10 ms 未定位。Worker #1、ID `1001` 为12 ms inclusive、11 ms Self，子 GC.Alloc 为8字节；帧100缺少该线程，对比工具拒绝匹配，不能当作零耗时对照。
- 目标帧 Flow 查询 `available=false`，不是零事件；没有 flow_id 可追踪，不能判断调度依赖、等待时长或关键路径。
- 材质短指纹 `ed5c4a88`：第11行 `m_Shader` 的 fileID `10752`、GUID `0000000000000000f000000000000000` 未解析；第26–27行 `_MainTex.m_Texture` 指向 `Assets/PublicTexture.png`，fileID `2800000`，对象未解析。
- Prefab 第45行 `m_Mesh` 解析到 `Assets/PublicMesh.asset`，fileID `4300000`，短指纹 `a46be4de`；Mesh、纹理、Shader和配置无影响当前建议的录制证据，未专项扩展审计。静态 Mesh 数据不代表帧绘制次数，材质属性不代表实际 SetPass。
- Editor 当前资源摘要、有效 Prefab 覆盖、运行场景关联、目标平台与采集时间均缺失。incoming 结果不是完整反向依赖；离线索引不覆盖动态加载、二进制内部及外部包。
- 未观察到已读文件指纹变化；Editor 指纹不可得。后续若文件或 Editor 指纹变化，应停止沿用旧证据并重新准备工程。正文短指纹来自实际查询，录制完整指纹未返回。
