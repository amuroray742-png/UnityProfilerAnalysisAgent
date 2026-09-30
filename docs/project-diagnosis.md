# Unity 工程联合定位与最终报告导出

目标范围：Windows、Unity 6000.3。把录制中的热点与代码、原始序列化引用、Editor 当前属性关联，生成可核对的优化建议。关联并不等于根因，静态资产大小、Mesh 索引数、材质 Pass 数也不是当帧 CPU/GPU 开销。

## 使用

1. 新建／打开优化项目，使用项目绑定的 Unity 工程目录；新建时选择含 `Assets`、`Packages`、`ProjectSettings` 的工程根目录。完整操作见[简单使用说明](optimization-loop.md)。
2. 导入 A，选择分析 AI，点击“一键诊断并定位”。后端先完成性能诊断，再准备工程并定位；首轮失败、取消或超限时不会进入定位。应用校验 Unity 版本并在后台建立索引；代码片段、资源字段会交给所选 Agent。
3. 应用自动检查与该路径匹配的 Editor、插件协议与工程身份。没有 CLI、未安装插件、Editor 未运行/编译/导入/Play Mode 或请求失败时，明确降级为离线定位，不会切换到其他已打开工程。
4. 单次独立 ACP 会话接收完整首轮报告，并按热点查询性能数据、代码、引用和资源。可以停止；诊断完成但定位失败时，“继续诊断定位”只重试定位。重试使用新会话，旧报告版本保留，不自动改绑工程目录。
5. 在项目“历史轮次”查看完整诊断、定位版本及修改记录，并按轮次导出 Markdown / HTML。兼容录制页面另保留 **导出定位报告** 和 **合并导出**。系统保存对话框取消不报错，写入失败可以重试。HTML 禁止正文 HTML、脚本与远程资源，提供打印样式。报告包含实际工程/Editor 采集范围、时间、平台、资源指纹和缺失原因，不附带完整工程或通信日志。

报告正文由后端保存，上限 2 MiB UTF-8，超限停止并明确标记不完整；有正文的取消/失败报告仍可导出。项目主流程按轮次在本地存档，关闭或重新打开项目不会清除已保存报告。旧 `fileId` 兼容接口的内存报告随录制释放，两者生命周期不同。已导出的文件保留。

报告还会按需提供[具体 Marker 补点方案](marker-guidance.md#第二阶段确认具体补点方案)：依据实际读取的代码说明文件、行号、同步范围、固定名称和重录指标，并修正或撤回首轮不合适的方向。诊断和定位阶段只提建议；用户可手动添加，也可点击“开始优化”后由独立修改会话在允许的代码范围内补点；代码位置能确认不代表热点因果已证实。

## 首次安装 Editor 插件

在项目页面“Unity 插件”区域点击“重新检查”，关闭目标工程的 Unity Editor 后点击“安装到工程”或“迁移到工程内”。插件随应用安装到 `Packages/com.upaa.inspector`，不依赖本工具仓库的个人目录；安装完成后重新打开 Unity 等待依赖解析与编译。将插件目录、变更后的 manifest 和 Unity 更新的 lock 文件一起提交。Unity CLI 仍需单独安装。

文件安装状态与 Editor 连接状态分别显示；不安装也可以离线定位。仅用户点击安装才写入插件；AI 和只读诊断没有安装权限。已有不同内容不会被覆盖，旧外部路径的迁移与恢复边界见[插件说明](../unity/Packages/com.upaa.inspector/README.md)。

插件提供固定 `upaa_context`、`upaa_asset`、`upaa_cancel` 命令。后端调用显式工程路径并检查协议 1 和规范化路径；Agent 无权调用 Unity CLI、`eval`、终端、保存、写入、重导入或构建。取消只针对本次采集请求，不退出 Editor。工程 AI 会话整体上限为 900 秒（普通首轮仍为 300 秒）；超时报告标为失败/不完整，已有正文仍可导出。

Editor API 在主线程执行，层级遍历按对象分批让出到下一次 Editor update。插件请求有 10 秒批次检查，CLI/后端分别 15/20 秒超时；取消在批次边界生效，**不能中断一个已经进入的 Unity 同步 API**。编译/域重载/断连可能使当前请求失败，余下诊断可继续使用已验证的离线证据。

只读取已经加载的场景，不打开其他场景、不实例化 Prefab、不进入 Play Mode、不主动保存或修改资源。读取资源可能触发用户工程自己的 Unity 回调；这不是不可信 Editor 插件的操作系统沙箱。

## 范围与限制

- 离线范围：Assets、ProjectSettings 和工程内嵌 Packages。C#、Shader/HLSL、场景、Prefab、材质、Unity 文本资源、meta 与配置；二进制只列出文件元数据，内部属性需要 Editor。UTF-8 和带 BOM 的 UTF-16 可读。
- C# 最大 2 MiB，其他允许文本最大 64 MiB，单行最大 1 MiB；流式索引。最多 200,000 个文件记录、1,000,000 个对象和引用。跳过、编码/权限问题与索引缺口显示警告；警告明细最多 100 条。
- GUID 来自 `.meta`，fileID 来自 Unity 文档头，引用来自原始 inline PPtr。不是完整 YAML/Prefab 合成器：嵌套 Prefab 和 override 保留原始对象/行号/属性关系，不自动计算有效覆盖值。重复 GUID/fileID、解析失败、未解析引用明确报告；外部包 GUID、内置资源和动态加载不保证离线解析。
- Editor 补充 Mesh 顶点/索引/子网格、纹理和平台导入覆盖、Renderer、材质/Shader、LOD、序列化属性、依赖及当前渲染配置。每对象最多 256 属性、层级约 4,096 条记录、直接依赖最多 1,000；平台覆盖为 Default/Standalone/Android/iPhone/WebGL。返回 partial 和分页信息，不承诺所有资源专项诊断。
- 外部/缓存 Packages 仅可读取当前工程已解析包的 Editor 资源摘要；不会因此开放外部磁盘代码读取。符号链接/junction 不遍历，读取时重新校验路径、句柄与哈希。
- 工程摘要至多展示 12 条警告明细（每条 600 字节）和 8 个已采集资源摘要，明确标记截断；完整已保存采集范围保留在导出附录。列表每页最多 100 项、文本最多 400 行，返回含 nextStart 和行截断标记；响应按编码后预算控制，过大时要求减小 limit。不能把部分搜索或反向引用表述成完整结果。
- 一次会话最多采集 128 个 Editor 资源。指纹包括依赖、已观察字段、Unity 版本及平台；后续指纹变化拒绝混用，要求重新准备。离线索引是准备时的快照，新文件不会自动加入；文本内容变化会使读取失败。
- Editor 状态是查询时状态，不能证明录制帧使用了同一资源、配置或代码版本。报告须区分观测事实、关联候选和原因假设，并给出代价、适用条件与 A/B 验证方法。

## 接口与验证

项目主流程由 `workflow_command` 组织两阶段，并使用持久化项目／轮次身份恢复上下文。兼容 Tauri `prepare_project(fileId, root)`、`project_editor_status(fileId, scopeId)`、`diagnose_project(fileId, agentId, parentReportId, scopeId)`。报告阶段新增 `project` 和 `projectContext`。工程会话注册 `project_summary/files/search/read/asset/references` 六个 MCP 工具；普通诊断不开放。旧 `prepare_source/diagnose_source` 与 `source_*` 仍只授权 C#。

公开资源在 [`tests/fixtures/unity-project`](../src-tauri/tests/fixtures/unity-project)，由 [Editor 生成器](../tools/unity-project-fixture/README.md) 生成；性能数据是公开合成 fixture，不声称真实采集关联。默认回归包含索引、安全边界、原始 Prefab override、权限隔离、取消与报告生命周期；真实 Editor 验收使用 `UPAA_PUBLIC_UNITY_PROJECT` 显式运行。当前验证结果及尚未覆盖项集中记录在[状态台账](project-status.md)。
