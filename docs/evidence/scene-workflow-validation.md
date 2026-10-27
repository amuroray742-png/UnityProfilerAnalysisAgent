# 场景分层验收（2026-10-27）

范围：Windows，本地公开夹具；项目 → 用户命名场景 → 独立优化轮次。未对真实业务工程执行修改。

## 自动回归

- `npm test`：97 项通过，包括按场景过滤、场景管理、录制确认、过时基线禁用诊断、完成/阻塞状态和跨场景迟到进度隔离。
- `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline`：189 项通过，23 项依赖外部录制、真实 Agent 或 Editor 的测试忽略；忽略不算通过。
- 新增后端覆盖 X/Y 独立轮次与预算、场景切换重开、名称限制、跨场景录制归属、未解决修改锁、修改和回退后的基线失效、导入失败保留、全项目执行顺序与回退依赖、v1/v2/v3 迁移、重复打开、空项目迁移及迁移保存失败保留旧索引。
- 原有回归明确修正为：已接受或历史轮次只读，不能重新修改或回退；未结束轮次仍可重试。
- `python -X utf8 -B -m unittest discover -s tools -p 'test_*.py'`：8 项通过。
- `npm run tauri:build -- --no-bundle`：前端生产构建和 Windows Release EXE；不生成安装包。

日志：忽略目录 `.cache/scene-frontend-tests.log`、`.cache/scene-rust-tests.log`、`.cache/scene-release-build.log`。

## Windows 桌面验收

使用仓库 Tauri WebDriver 验收入口和与本机 WebView2 匹配的 Microsoft Edge WebDriver 155.0.4283.62。驱动下载到 `.cache/webdriver/edge155/`，未升级或修改系统浏览器。

```powershell
python -X utf8 tools/desktop-smoke.py --scenes --native-driver .cache/webdriver/edge155/msedgedriver.exe
# 使用上一步 result.json 中的 sceneRecords，在全新应用进程中恢复。
python -X utf8 tools/desktop-smoke.py --scene-reopen <sceneRecords> --native-driver .cache/webdriver/edge155/msedgedriver.exe
```

创建、命名、说明、场景切换、完成/重新开启与历史选择使用实际桌面控件。录制绑定走真实生产 IPC，显式确认来自当前代码；没有打开原生文件选择框。

公开流程：X 导入首轮 → 无修改轮次回退结束 → 下一轮 → 标记场景完成 → 创建 Y → Y 独立首轮 → 切回 X 查看两轮历史 → 重新开启 → 关闭并打开工作区 → 全新应用进程恢复。额外验证缺少录制确认、跨场景轮次身份会被后端拒绝，拒绝发生在 Agent 启动前。

结果、产物哈希和截图保存在 `.cache/desktop-scenes/result.json`、`.cache/desktop-scenes/scenes.png`、`.cache/desktop-scenes-reopen/result.json`、`.cache/desktop-scenes-reopen/scenes-reopened.png`。

最终 Release EXE SHA-256：`aca4461d52cb55c6c233e91c7822bd24eef95c83cf4ccea44114c6c5dd6a22b4`。上述两次桌面验收均通过，使用同一最终产物；第二次为全新应用进程。

## 边界

- 本轮没有调用真实 AI、运行 Unity 游戏或执行输入回放，不证明性能收益、玩法正确性或回放一致性。
- 工程修订号只追踪本工具可能修改过的文件操作，写入前持久化；失败/中断可能导致保守的重录要求。外部工程修改仍需人工确认及原有指纹检查。
- 旧存档迁移不自动拆分历史场景；旧录制修订号未知，继续优化前须导入当前录制。
- 原生文件选择、安装包、macOS 原生窗口没有在本轮重新验收。操作录制和自动重录保留为后续功能。
