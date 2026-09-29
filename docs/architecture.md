# 架构与数据契约

本页区分现有实现与目标设计。评估基线、验证结果和优先级统一维护在[项目状态](project-status.md)，使用方法见 [README](../README.md)。

## 现有数据流

```text
React：点击文件选择框
  → upload(filePath)：登记原文件路径与 fileId 到 AppState
  → analyze(fileId)
      → .data：阻塞任务中分块读取，发送 parse-progress
      → JSON：阻塞任务读取，类型化识别 dump / V1 / V2 / 数组
      → RAW / PD3U：格式识别与估算，性能指标不可用
      → ParsedProfile（全部帧的合并摘要 + warnings + 原始结构查询源）
      → extractor（阻塞任务中聚合有效帧，输出 nullable 指标和质量信息）
      → MetricsSnapshot 与查询源缓存在 AppState，快照返回前端
  → frame_details / cpu_hierarchy：阻塞任务中读取目标帧，返回有界分页
  → release_file：移除路径登记、快照和查询源
  → diagnose(fileId, agentId)
      → 原子绑定快照/详情/本地 sessionId，拒绝同文件重复诊断
      → 独立 MCP 服务 + 临时工作目录 + Agent 进程
      → ACP initialize → session/new（MCP 桥配置）→ session/prompt
      → Agent 经 MCP 查询 → session/update → 带会话标识的事件 → React
      → 正常完成/错误/取消 → 撤销 MCP、回收进程树与临时目录

MCP：rmcp 服务 → 会话专属本机认证通道 → 应用 --mcp-bridge → Agent stdio
```

`upload` 不复制文件；文件必须在分析时仍可访问。当前文件入口是点击选择，拖放仅有视觉反馈。解析进度主要报告字节数，中途 `currentFrame` 为零。

## 模块边界

| 模块 | 现有职责与限制 |
|---|---|
| [commands](../src-tauri/src/commands/mod.rs) | 文件、分析、查询、诊断与按 sessionId 取消命令 |
| [parser](../src-tauri/src/parser/mod.rs) | 格式分派及 ParsedProfile / Frame / Sample；不等于所有格式都能完整解析 |
| [data](../src-tauri/src/parser/data/mod.rs) | 分块读取；6000.3.23f1 / 6000.3.9f1 使用顺序结构解码和跨帧 marker 状态，输出 CPU / GC / 五类渲染计数；其他 Unity 6 版本仅提供帧头 CPU 估算，旧启发式扫描不参与指标 |
| [extractor](../src-tauri/src/extractor/mod.rs) | 聚合快照；保留全帧时间线，无固定 20–50KB 上限 |
| [state](../src-tauri/src/state/mod.rs) | 路径、快照、详情源和会话句柄；同文件诊断互斥、释放时取消 |
| [acp_client.rs](../src-tauri/src/acp_client.rs) | ACP v1 会话与 MCP 生命周期；手写 JSON-RPC 消息状态机，尚未验证其他协议版本 |
| [acp_client/client.rs](../src-tauri/src/acp_client/client.rs) | 子进程、Windows shim、协作取消句柄和 Windows Job Object |
| [mcp](../src-tauri/src/mcp/mod.rs) | 会话 MetricsStore、rmcp Server、类型化分派与本机 stdio 桥 |
| [useDiagnose](../src/hooks/useDiagnose.ts) | 页面状态、早到事件缓冲、fileId/sessionId 过滤和取消 |

## 当前数据契约与风险

- dump 独立解析线程/样本树并校验，CPU 取唯一 Main Thread 根样本，GC 统计全部已导出线程。站点归因按线程和最近非 GC.Alloc 父样本；inclusive 热点不可相加。
- 内部 Frame 通过 FrameQuality 保存字段是否可用。旧内部数值占位不能直接消费；extractor 仅聚合有效值，公开快照中不可用数值为 null。
- FrameTimeStats 的 quality 包含 status（available / partial / unavailable / estimated）、source、reasons、validFrames、totalFrames。CPU 热点、GC 站点、渲染事件有独立质量信息；无效帧不参与分位数。估算混入部分数据时 status 为 partial，reasons 仍说明估算。
- meta.frameCount 是实际分析帧数，declaredFrameCount 是来源声明的录制帧数，source 标识输入；durationMs 为有效帧时间之和，并附 durationQuality。CPU 时间线保留源 frameIndex、nullable ms 与独立 frameTimeMs。
- GC 站点 DTO 使用 totalBytes / avgBytes / maxBytes，不再复用 ms 字段。Gen 回收次数和 SRP 节省没有观测数据，返回 null。
- V1 缺 cpuMs 不用 durationMs 替代主线程时间；V2 估算明确标记。6000.3.23f1 / 6000.3.9f1 data CPU 来自唯一主线程根样本，GC 来自全部线程的索引记录并与通用 metadata 交叉核对；录制帧时间来自相邻帧起始纳秒差，按 Editor 的 float32 转换，末帧或时间戳倒退时不可用。data 渲染计数由 Counter marker metadata 读取，按有效帧聚合；缺失与冲突不补零。渲染 CPU marker 不代表 GPU，SRP 收益及 RAW/PD3U 性能指标不可用。详见[渲染契约与验证](rendering-validation.md)。
- 快照之外保留逐帧查询源：data 使用字节索引与 marker 状态，dump 使用自动清理的临时文件；查询保留原始父子关系和线程，不再用全局热点冒充调用树。详见[查询契约](frame-queries.md)。MCP 服务复用查询源，经本机认证桥提供 stdio；绑定桌面诊断会话并随会话撤销。
- dump 与结构化 data 在每帧内合并同名 CPU 摘要，以及同线程、同归因名称的 GC 站点，保留调用次数、总量和最大值。摘要行数不再等于原始样本数；原始树、marker ID、索引与父子关系仍由查询源完整提供，不截断为 top N。
- JSON 类型化反序列化避免完整 Value 树，仍持有输入字节与解析结果；data 累积全部 Frame。UI 重置和替换输入会释放登记、快照和查询源；在途查询结束后释放最后引用。独立解析进程、桌面 IPC、结果页渲染与 DOM 对照测量见[性能记录](performance-and-release.md)，完整桌面应用预算仍待验证，不承诺固定快照大小。
- 文件读取、主要解析与提取已从异步命令隔离到阻塞任务。进度主要为 data 字节数，JSON 导入仍没有分段进度。
- CSP 已配置；没有统一文件大小上限，误导性的 500MB 提示已移除。Windows Job Object、取消和异常终态已有进程回归；release 桌面主流程已有验收，原生窗口及其他平台进程树仍待验证。
- Agent 诊断在独立临时目录运行，只授权当前会话的 Profiler MCP 查询。未声明文件/终端能力不等于操作系统沙箱；其他 Agent 的权限语义需要独立验收。

## 目标数据流与验收边界

```text
录制 + Unity Editor 对照
  → 按版本解析、结构校验、标注缺失/估算/来源
  → 保留原始帧标识、线程、调用树与明确单位的内部模型
  → 摘要与按帧查询视图
      → 前端展示可用性与证据
      → 可运行 MCP Server 提供有界查询
  → ACP initialize / session/new / session/prompt
      → Agent 查询 MCP → session/update → 前端
      → 会话管理统一处理取消、异常、退出与资源释放
```

保留 parser → extractor 分层。P0 数据质量、P1 调用树和 Windows Claude Code ACP 数据查询闭环已有证据，原始目标没有缩减为仅协议 fixture。release 桌面主流程已通过限定范围验收，原生窗口、更多录制/Agent 和发布仍待验收。MCP 数据通过会话专属本机桥访问，Arc 只在父进程内部共享。

性能预算、安装包体积和跨平台能力均属于待验证目标。测量需要同时记录录制规模、硬件、耗时、峰值内存与保留数据量，不能仅凭分块读取作容量承诺。

## 报告与源码定位增量

`state::AppState` 在录制内存生命周期内保存 `reports::Report` 和 `source::SourceScope`。ACP 事件先由后端更新报告，再转发 UI；会话只允许一次终态，正文上限由 ACP 输入和报告存储双重检查。UI 的事件日志可以限长，导出和终态正文从后端报告读取。

源码准备在可取消的阻塞任务中生成范围 ID、相对路径索引和文件哈希；源码 ACP 会话从后端取得完整首轮报告，并使用带该范围的独立 `MetricsStore`。只有这类会话的 MCP 工具列表包含源码查询工具。文件读取重新验证范围及哈希，Agent 临时工作目录不变。

UI 和导出 HTML 使用同一个 `reports::render_markdown`，Markdown 导出保留正文。导出先校验录制和父子报告关系，再在阻塞任务中写同目录临时文件并替换目标。重置/切换录制释放报告、取消准备和源码范围，不删除用户已导出的文件。详细接口与限制见[报告与源码指南](reports-and-source.md)。


## Unity 工程联合定位

`project::ProjectScope` 在 `spawn_blocking` 中流式索引工程内文本，记录 GUID/fileID、原始 PPtr、哈希及覆盖缺口；`project::files` 负责规范路径、Windows 文件句柄和编码/大小校验。C# 旧范围不变。

```mermaid
flowchart LR
  P[完整首轮报告] --> A[新工程 ACP 会话]
  I[工程根目录] --> S[ProjectScope 离线索引]
  A --> M[性能工具与 project_* MCP]
  M --> S
  M --> B[Rust 固定命令白名单]
  B --> C[Unity CLI 显式工程路径]
  C --> E[Editor-only UPAA 插件]
  E --> V[身份 协议 指纹 平台校验]
  V --> M
  A --> R[后端工程报告与实际采集范围]
  R --> X[单独或合并 MD/安全 HTML 导出]
```

只在工程会话注册六个 `project_*` 工具；AI 无权使用 CLI 的通用执行命令。Editor 插件按对象分批读取，取消是协作式批次检查，不可中断同步 Unity API。只采集已加载场景，外部包只返回当前工程解析的资源摘要。Editor 不可用仍可离线定位，报告明确缺少相关证据；不同平台/版本及变化的资源指纹拒绝混用。完整限制见[工程定位指南](project-diagnosis.md)。
