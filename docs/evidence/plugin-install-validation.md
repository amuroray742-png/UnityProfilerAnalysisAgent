# Unity 内嵌插件安装与团队共享验收（2026-09-30）

范围：Windows、Unity 6000.3.23f1、仓库公开工程。未安装到或修改私有游戏工程。插件字节与清单编入程序，不依赖运行时源码目录；这是交付方式变化，不扩大 Agent 权限。

## 自动回归

- 完整 Rust：165 项通过，22 项默认忽略。插件模块新增 16 项，覆盖首次／重复安装、中文目录、原路径缺失、匹配／修改过的外部包、已有内嵌包冲突、Git 来源、项目身份、忙碌状态、Editor 独占、junction、只读 manifest、原字节备份、中断恢复及外部变化。
- manifest 测试核对只删除插件依赖项，保留其他内容；Unity 生成的 `source=embedded`、`file:com.upaa.inspector` 不误报为个人路径，旧 local lock 保留警告且不重写。
- 前端 60 项通过，其中新增 5 项覆盖安装／迁移按钮、不可覆盖冲突、忙碌禁用、失败重试和迟到响应；Python 工具 8 项通过。日志在忽略目录 `.cache/plugin-rust-final.log`、`.cache/plugin-frontend-final.log`、`.cache/plugin-python-final.log`。

## 公开工程与真实 Unity

从实际桌面按钮将随程序插件安装到公开工程，原 manifest 指向不存在的本地目录。核对所有插件文件及 `.meta`，移除原路径引用，重复安装不产生覆盖；跨工程 ID 和额外目标路径参数被拒绝。

Unity 6000.3.23f1 将包识别为 embedded，解析 Pipeline／Newtonsoft 并完成编译；`upaa_context` 返回协议 1、正确工程路径、`ready`。仅复制 Assets、Packages、ProjectSettings 到另一目录（不复制 Library），重新解析与编译后再次返回新工程身份与 `ready`。第一次导入前后插件字节完全一致。证据位于 `.cache/plugin-unity-A-context.json`、`.cache/plugin-unity-B-context.json` 与相应 Editor 日志。

这证明两个本机目录上的可移植加载，不代表所有 Unity 版本、离线依赖下载或另一台机器环境已验证。新同事仍需能解析 Unity 包依赖；Unity CLI 安装与登录不由本功能处理。

## 验证过程中修正

- 桌面拒绝请求按预期失败，但测试脚本最初只捕获一种异常；补齐脚本后复验通过。
- Unity 嵌入式包的 lock 也使用 `file:` 字符串，实际证据促使检测按 `source=embedded` 排除误报。
- 初始测试项目放在 `.cache` 下，启动诊断出现目录命名警告；正式 Unity 验收移到独立临时公开工程，不改私有工程。
- 一次并发构建因桌面测试占用 EXE 而失败；后续构建与桌面验收串行执行。

## 最终程序与安装包

- `npm run tauri:build -- --bundles nsis` 通过，生成本轮 NSIS 包；程序 SHA-256：`8183f2f9015efa07a86606b868dc70f6dcc3ebaa7475141e39b320b5f34f630b`，安装包 SHA-256：`7cceb77614daeb17255150a296e745372c7c20eb8ec98b0f04e2c2e56d8faf2a`。
- `tools/test-windows-install.ps1 -PluginInstall`：独立目录静默安装、安装后的 51 项协议回归通过（9 项默认忽略）、真实桌面插件迁移／重复安装／身份拒绝／记录读取通过，随后卸载并清理该测试安装登记。
- 设置 `UPAA_PLUGIN_CHECK_PROJECT` 为上述已打开的公开迁移工程，安装后的 UI 显示文件已安装与 Editor 已连接；实际 Editor 运行时安装请求被拒绝，内嵌相对 lock 不产生个人路径警告。
- 独立 Release EXE SHA-256：`1bc7ef1e0568e37f8092130ad15634e425551cbb0cebb9d1fe3c1b3e4c8babd6`，另行桌面安装／迁移回归通过。Tauri 打包时向安装版 EXE 写入 bundle 类型标记，因此独立 EXE 与安装后的 EXE 哈希不同，各自记录、不混用。
- 独立 EXE 桌面报告：`.cache/desktop-plugin/result.json`；安装版桌面报告已另存 `.cache/plugin-installed-desktop.json`，连接截图 `connected.png`；安装记录：`.cache/installer-smoke/result.json`，构建日志 `.cache/plugin-nsis-final.log`。`UPAA_PLUGIN_CHECK_PROJECT` 验收脚本只接受系统临时目录中以 `upaa-plugin-` 命名的公开工程。

本轮不重新验收 MSI、交互式安装向导、Agent 诊断内容或游戏性能收益。安装失败保留可核对记录，不提供强制覆盖／升级／自动卸载插件；旧来源内容不同需人工核对，不静默替换。
