> 2026-09-29 真实 Codex ACP 验收产物。输入是公开合成 Profiler fixture 和公开 Unity 样例工程，不代表真实游戏基准。仅将工程绝对路径替换为 `<public-project>`，正文建议仍需人工验证。

# Unity 工程性能定位报告

- 录制：public-project-peak.json
- Unity：6000.3.23f1
- 分析帧数：21
- 覆盖率：CPU 21/21（available）；GC 21/21（available）；Draw Call 0/21（unavailable）；SetPass 0/21（unavailable）；Batches 0/21（unavailable）；Triangles 0/21（unavailable）；Vertices 0/21（unavailable）
- Agent：codex
- 时间：2026-09-29T04:42:28.002608600+00:00
- 状态：completed

我会先核对录制与工程摘要，再检查帧 160 的调用树和相关代码、资源引用链；全程只使用允许的 performance_* 与 project_* 工具。
帧 160 的观测与首轮报告一致：`AllocationWork.Update` 为 30 ms，主线程分配 8,388,600 字节。当前源码在每次 `Update` 中无条件创建大数组，而录制只有一帧出现分配；这处差异尚未解释，不能认定当前代码完整重现了录制行为。我会继续核对组件、Prefab 和场景关联。
已确认静态引用链为 `PublicScene → Public.prefab → AllocationWork.cs`，当前 Editor 也确认了 Prefab 中的脚本和材质绑定。源码的 `Update` 没有使用该材质，录制又缺少渲染计数，因此暂不据此提出渲染优化。建议将集中在数组分配候选，并以复现录制行为、确认源码对应关系为前提。
**Unity 工程性能定位报告**

本次选出 **1 个优先调查热点**：帧 160 的 `AllocationWork.Update`。已找到同名源码及静态资源链，并核对当前 Editor 的资源绑定；但源码行为与录制存在差异，尚不能确认具体分配对象或解释全部 30 ms。未修改源码、资产或生成补丁。

**录制与工程核对**

- 录制：`public-project-peak.json`，来源 `unity-editor-dump`。21 个导出帧为原始帧 100、103……160；仅帧 160 主线程 40 ms、GC 分配 8,388,608 字节，其余 20 帧均为 1 ms、0 字节。CPU P95=1 ms。
- CPU、GC 质量均为 `available`，覆盖 21/21，无会话警告。Draw Call、SetPass、Batches、Triangles、Vertices 及渲染事件均 `unavailable`，覆盖 0/21；GPU 时间缺失。
- 录制、离线工程、Editor 均报告 Unity **6000.3.23f1**，未发现版本号不匹配。录制平台未知；Editor 当前目标为 `StandaloneWindows64`、质量级别 5、Built-in，不能代替录制时配置。
- 工程范围为 Assets、ProjectSettings、工程内嵌 Packages，共 42 文件；跳过/无法读取 0 项。工程身份短值 `cc989a2c`，范围身份 `bf900133`，复核未变。

**热点 P1：大数组分配候选**

**原始观测。** `performance_frame`、`performance_cpu_hierarchy` 确认帧 **160 / Main Thread #0**：

`Main Thread → AllocationWork.Update → GC.Alloc → GC.Alloc`

`AllocationWork.Update`（markerId 113）调用 1 次，**30 ms inclusive**；两个分配样本分别为 **8,388,596 字节、4 字节**，归属该路径共 **8,388,600 字节**。同级子样本 `Unmapped.Native` 为 1 ms，已经包含在父耗时内，不能相加。Worker #1 分配 8 字节，与主线程合计匹配帧总量。

帧 160 两线程调用树均无 `nextStart`、深度截断或查询警告。帧 100 主线程树仅有 1 ms 根样本。完整读取导出树不代表全部运行工作均有采样。

**实际代码。** 按类型 `AllocationWork`、方法 `Update(` 搜索后读取 `Assets/AllocationWork.cs`；文件哈希短值 `869776fd`，末次复读一致。L6 为 `private byte[] lastFrame;`，L7–11 原文：

```csharp
    private void Update()
    {
        lastFrame = new byte[8 * 1024 * 1024];
        lastFrame[0] = 1;
    }
```

这是每次执行时创建大数组的明确代码候选，但存在两处未解释差异：当前方法无条件分配，录制却只有一帧出现分配；数组请求长度为 8,388,608，亦不能与最大的 8,388,596 字节样本逐项对应。名称和规模相近不足以证明录制调用执行了此版本代码。

**关键资源链。** 以下由离线 `project_read`、`project_references` 与当前 `project_asset` 共同核对：

- `Assets/PublicScene.unity` L166、226：PrefabInstance **2023790152** 的 `m_SourcePrefab` 指向 `Assets/Public.prefab`，GUID 短值 **153ad8fc…**，fileID **100100000**。该 fileID 离线对象未解析，资源路径已解析。
- `Assets/Public.prefab` L95、104、106：组件 **4838056683082871320** 的 `m_Script` 指向 `Assets/AllocationWork.cs`，GUID **c5c0075c…**、fileID **11500000**，类标识为 `Assembly-CSharp::AllocationWork`。Editor 当前对象摘要确认组件类型及脚本绑定。
- 同组件 L107 的 `effectsMaterial` 指向 `Assets/PublicMaterial.mat`（GUID **4af2d389…**、fileID **2100000**）。材质 L11 的 Shader 引用 fileID **10752** 离线未解析，Editor 当前解析为 `Unlit/Texture`；L26–27 的 `_MainTex` 指向 `Assets/PublicTexture.png`，fileID **2800000**。
- Prefab L45 的 `MeshFilter.m_Mesh` 指向 `Assets/PublicMesh.asset`，fileID **4300000**，Editor 当前摘要也确认该绑定。

这条链仅建立组件和资源关联。已读 `Update` 不使用 L5 声明的 `effectsMaterial`，没有证据将材质、Shader、纹理或 Mesh 归为此次 CPU/GC 尖峰原因。场景 L214–216 声明 `m_CastShadows=0`，而 Prefab 当前摘要为 On；两者分别属于实例声明与源 Prefab 状态，不能混作有效实例值。

离线短哈希：Prefab `b346af47`、场景 `ba4a9aa5`、材质 `ed5c4a88`。Editor 来源均为 `project-asset`、状态 `ready`；Prefab 指纹 `f0f9b4dd`，跨页一致；材质指纹 `08fd3273`。完整指纹以工具后端证据附录为准。

**事实、假设与建议。** 事实是该 marker 内存在大额分配；关联候选是 L9 的数组创建。可能存在可避免的重复存储创建，但尚未证明该行对应录制分配，也不能把全部 30 ms 归因于它；没有 GC 回收停顿证据。

建议先在与录制对应的源码版本、场景和触发操作中复现，确认分配调用栈、组件实例、启用状态及执行频率。确认后再选择：

- **若数组仅需保存当前状态，可取消每帧创建，改为按生命周期一次初始化或按实际需求分配。** 适用条件是允许复用同一数组，且消费者不依赖独立快照或新数组的全零状态。代价是保留 8,388,608 字节逻辑容量及维护清理、释放时机。
- **若完整读取证实业务只需 L10 所示的一字节状态，可缩小存储或移除无用途缓冲。** 条件是确认录制版本及外部使用者没有完整数组需求；代价是数据契约变化，需要功能回归。

当前方法没有可拆分的大循环，不支持直接建议分帧、Jobs/Burst、对象池或修改 GC 模式。

**同场景 A/B。** 固定实际目标平台、Unity 版本、质量配置、实例数量及触发操作，先取得可重复基线，再逐项验证上述改动。比较目标 marker 的 CPU、调用次数、GC 字节与分配样本次数、尖峰频率，同时检查数据正确性、完成时机和常驻/峰值内存。不预估收益比例。

**未定位与覆盖边界**

- **未定位热点：**帧 160 的 `Unmapped.Native`（主线程，1 ms）缺少具体原生调用信息；Worker 根样本 12 ms 缺少工作子路径和同步关系，不能确认主线程关键路径影响。其 8 字节分配不值得优先优化。
- **未解析资源：**脚本 fileID、源 Prefab fileID 在离线对象索引中未解析；Shader 已由当前 Editor 解析名称，但未读取实现；纹理只有路径关联，二进制内部未解析。Mesh、纹理导入配置及 `PublicVariant.prefab` 未专项调查，未扩展为渲染审计。
- **分页与质量：**相关引用页已续读至 `nextStart=null`，但 incoming/outgoing 覆盖始终 `partial`。源码、Prefab、材质读取无 `lineTruncated`；场景仅读 L160–232。Prefab Editor 页已读完，覆盖仍为 `partial`；材质仅读首 50 项，尚有 `nextStart=50`，其余属性未专项调查。
- **Editor 缺失信息：**没有录制当帧的有效实例覆盖、组件启用状态、分配调用栈或运行时加载证据。当前资源核对不等于录制当帧因果验证。
- **时间与范围：**首轮报告时间为 2026-09-29 04:40:39 UTC，实际录制采集时间未提供；Editor 概况采样于 04:42:26 UTC，资源摘要于 04:43:56 UTC。录制平台、目标帧率及帧 160 触发操作仍未知。动态加载、二进制内部、外部包及 Prefab 有效覆盖可能不在索引中；静态 Mesh 数量、材质 pass 数均不能转换为当帧绘制次数、SetPass 或 GPU 耗时。

## 工程采集范围与证据状态

```json
{
  "editor": {
    "details": {
      "loadedScenes": [
        {
          "dirty": false,
          "loaded": true,
          "path": "Assets/PublicScene.unity"
        }
      ],
      "projectRoot": "<public-project>",
      "protocolVersion": 1,
      "qualityLevel": 5,
      "reason": "编译/导入/Play Mode 时不采集；当前 Editor 状态不代表录制当帧",
      "renderPipeline": "Built-in",
      "sampledAt": "2026-09-29T04:42:26.8802309Z",
      "status": "ready",
      "targetPlatform": "StandaloneWindows64",
      "unityVersion": "6000.3.23f1"
    },
    "reason": null,
    "sampledAt": "2026-09-29T04:42:26.8802309Z",
    "status": "ready",
    "targetPlatform": "StandaloneWindows64",
    "unityVersion": "6000.3.23f1"
  },
  "editorAssetsRead": 2,
  "editorEvidence": {
    "Assets/Public.prefab": {
      "coverage": "partial",
      "fingerprint": "f0f9b4dd4d4b19f5e396eeabef44258e",
      "origin": "project-asset",
      "sampledAt": "2026-09-29T04:43:56.1776939Z",
      "status": "ready",
      "targetPlatform": "StandaloneWindows64",
      "unityVersion": "6000.3.23f1"
    },
    "Assets/PublicMaterial.mat": {
      "coverage": "partial",
      "fingerprint": "08fd3273bedfdcece193776944040aad",
      "origin": "project-asset",
      "sampledAt": "2026-09-29T04:43:56.1716942Z",
      "status": "ready",
      "targetPlatform": "StandaloneWindows64",
      "unityVersion": "6000.3.23f1"
    }
  },
  "project": {
    "editor": {
      "details": {
        "loadedScenes": [
          {
            "dirty": false,
            "loaded": true,
            "path": "Assets/PublicScene.unity"
          }
        ],
        "projectRoot": "<public-project>",
        "protocolVersion": 1,
        "qualityLevel": 5,
        "reason": "编译/导入/Play Mode 时不采集；当前 Editor 状态不代表录制当帧",
        "renderPipeline": "Built-in",
        "sampledAt": "2026-09-29T04:42:26.8802309Z",
        "status": "ready",
        "targetPlatform": "StandaloneWindows64",
        "unityVersion": "6000.3.23f1"
      },
      "reason": null,
      "sampledAt": "2026-09-29T04:42:26.8802309Z",
      "status": "ready",
      "targetPlatform": "StandaloneWindows64",
      "unityVersion": "6000.3.23f1"
    },
    "fileCount": 42,
    "fileId": "cc989a2c-34a8-4e66-83f8-a3a183290d26",
    "root": "<public-project>",
    "scopeId": "bf900133-4652-4b27-b139-8e4764d0134e",
    "unityVersion": "6000.3.23f1",
    "warnings": [
      "跳过/无法读取 0 项；最多展示 100 条明细。引用索引仅覆盖已解析的 Unity 文本 PPtr，不含运行时动态加载、二进制内部、外部包；Prefab 覆盖为原始声明，不代表已合成的有效值。"
    ]
  },
  "scope": "Assets、ProjectSettings、工程内嵌 Packages；名称/引用/Editor 当前状态不是录制当帧的因果证据"
}
```

---

证据边界：AI 建议需结合原始性能数据和实际源码核对；inclusive 耗时不可相加为总 CPU，源码可能与录制版本不一致。
