# 公开 Unity 工程样例

`UPAAFixture.Build` 只允许在含 `.upaa-public-fixture` 标识的测试工程中运行。它创建 64×32 纹理（Android 覆盖为 RGBA32、最大 32）、3 顶点/3 索引 Mesh、材质、带脚本和 LOD 的 Prefab、Prefab variant 与已保存场景。脚本中的每帧 8 MiB 分配仅用于定位验收，禁止用于真实业务。

已由 Unity 6000.3.23f1 生成的公共资源副本位于 `src-tauri/tests/fixtures/unity-project`。这是合成性能 fixture 的关联候选工程，不是产生该录制的实测游戏；不能将静态资产属性等同为录制中的开销。缺失引用、损坏对象和重复 GUID 的回归由 `tests/project_scope.rs` 构造，避免把损坏样例误当成正常工程。

真实 Editor 验收：使用独立测试工程，放入两个业务样例脚本及 `Editor/UPAAFixture.cs`，手动通过 UPM 安装仓库 `unity/Packages/com.upaa.inspector/package.json`，运行 `UPAAFixture.Build`。不要把生成器安装进业务工程。

```powershell
$env:UPAA_PUBLIC_UNITY_PROJECT = 'C:/path/to/public-test-project'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test project_scope live_editor_public_assets_match_known_fixture -- --ignored --nocapture
```

公开测试工程已经连接 Pipeline 时，可显式运行 `unity command eval "UPAAFixture.Build();" --project-path C:/path/to/public-test-project --format json` 生成资源；这是开发者的测试准备命令，不会向诊断 Agent 开放 eval。

缺少环境变量、标识或连接时测试必须失败。运行测试会读取插件摘要，不会重建资源；生成器是独立的显式准备步骤。
