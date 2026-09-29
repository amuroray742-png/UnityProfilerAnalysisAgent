# UPAA 只读采集插件

要求 Unity 6000.3、Unity CLI，以及 `com.unity.pipeline` 0.6.0-exp.1。本包只编译进入 Editor。

在目标工程的 Unity Package Manager 中选择 **Add package from disk**，选择本目录的 `package.json`。Unity 会解析依赖并编译；这是用户主动的安装操作，分析应用不会自动改 manifest 或升级 Editor。

编译后，在分析应用选择同一工程根目录，检查 Editor 状态。应用只调用 `upaa_context`、`upaa_asset` 和用于取消该次读取的 `upaa_cancel`，不给 AI 注册 Unity 的通用执行或写入工具。

不保存、不重导入、不打开新场景、不实例化 Prefab、不切换 Play Mode。Unity 自身的资源加载回调可能来自用户工程，插件不能充当操作系统沙箱。已加载场景中的未保存内容与磁盘不同，报告会标记 dirty，并对采集结果生成指纹。每次查询只采集一个资源并分页返回；大型层级/依赖/属性列表可能截断，需查看覆盖警告。
