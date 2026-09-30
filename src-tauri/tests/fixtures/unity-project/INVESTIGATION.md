# 公开调查样例

这些代码和既有 Profiler JSON 均为人工构造，不代表真实运行成本。

- `BatchSpawnEntry.SpawnWave` / `LoadBeforeSpawn`：从批量 Instantiate、引擎加载热点继续搜索业务入口；静态候选不能证明对应录制帧。
- `PublicTipPresenter.LateUpdate` → `PublicVoiceQueue.IsIdle`：先读取下游 getter 才能发现 Clone，不能仅按上层 marker 定位分配语句。
- `AutomaticCaller.Invoke` → `AutomaticHotspots.Update`：公开真实 Agent 验收入口，可自主读代码、新增缓存辅助类并改原方法；指定返回内容，允许复用对象，不外推成实际游戏优化依据。
- 若只有 SpawnWave 的整体耗时而无法分离加载／实例化成本，只在已读代码中提出或增加同步 Marker；仍需重录验证。
- `Unmapped.Native` 没有对应源码，不创建假调用点，也不声称找到根因。

自动编辑验收只在带 `.upaa-public-fixture` 的隔离工程显式执行。其余样例用于调查语义与人工核对，不自动生成实际游戏录制。
