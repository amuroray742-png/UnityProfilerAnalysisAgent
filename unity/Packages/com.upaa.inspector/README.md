# UPAA 采集与固定检查插件

要求 Unity 6000.3、Unity CLI，以及 `com.unity.pipeline` 0.6.0-exp.1。本包只编译进入 Editor。

在目标工程的 Unity Package Manager 中选择 **Add package from disk**，选择本目录的 `package.json`。Unity 会解析依赖并编译；这是用户主动的安装操作，分析应用不会自动改 manifest 或升级 Editor。

编译后，在分析应用选择同一工程根目录，检查 Editor 状态。诊断和定位只调用 `upaa_context`、`upaa_asset` 和用于取消该次读取的 `upaa_cancel`，不给 AI 注册 Unity 的通用执行或写入工具。

优化修改阶段另提供固定的 `upaa_check_start/status/cancel`（`checkProtocolVersion: 1`）：编译已有 C#、导入批准的 Shader/HLSL 相关文件、读取错误并运行用户明确选定的 EditMode 测试。不会自动打开场景、保存、进入 Play Mode 或构建。测试适配器仅在工程已安装 `com.unity.test-framework` 时编译；不自动安装依赖。旧只读协议仍兼容，没有检查命令时标为“检查未完成”。

检查记录保存到当前工程 `Library/UPAA-optimization-check.json`，带检查 ID、阶段和终态；域重载后继续，断连/超时不算成功。检查先刷新 AssetDatabase 识别新增或回退移除的代码，再执行用户工程的导入/编译回调和选定测试，因此不是只读采集；测试自身副作用不受插件沙箱隔离，应用检查业务文件清单变化并暂停异常推进。

只读采集命令不保存、不重导入、不打开新场景、不实例化 Prefab、不切换 Play Mode。Unity 自身的资源加载回调可能来自用户工程，插件不能充当操作系统沙箱。已加载场景中的未保存内容与磁盘不同，报告会标记 dirty，并对采集结果生成指纹。每次查询只采集一个资源并分页返回；大型层级/依赖/属性列表可能截断，需查看覆盖警告。

Shader 检查仅覆盖导入时可取得的错误，不证明所有平台/变体通过。HLSL、CGINC、Compute 当前不能完整检查依赖 Shader，明确返回“无法完成”，不以导入成功冒充检查通过。
