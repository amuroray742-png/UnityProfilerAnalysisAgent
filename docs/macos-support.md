# macOS 支持与工程梳理

## 工程结构

本工程是 Tauri 2 本地桌面应用，React / TypeScript 负责界面，Rust 负责解析、工程索引、Agent 会话和持久化。主流程为新建优化项目、导入 A、诊断和工程定位、独立修改会话、手动录制 B、对比并接受或回退。详见[架构](architecture.md)和[优化流程](optimization-loop.md)。

| 入口 | 职责 | 平台关系 |
|---|---|---|
| `src/App.tsx`、`components/ProjectWorkflow.tsx` | 首页及项目工作流 | Tauri 原生对话框 |
| `src-tauri/src/lib.rs`、`commands`、`optimization/workflow.rs` | 命令注册与后端编排 | 桌面壳和系统路径 |
| `parser`、`extractor` | 录制解析、CPU / GC / 内存 / 渲染证据 | 格式由录制来源决定，不能按应用运行系统推断 |
| `acp_client`、`mcp` | ACP stdio 会话和会话专属 MCP 桥 | 命令查找、启动环境、进程树回收 |
| `project`、`source` | 只读代码、资源索引和 Unity CLI 固定查询 | 路径、符号链接、CLI 安装位置 |
| `optimization` | 备份、受限修改、插件安装和回退 | 文件锁、原子替换、Editor 占用检查 |
| `unity/Packages/com.upaa.inspector` | Unity Editor 采集和检查插件 | Editor 平台、版本与工程身份校验 |
| `tools`、`.github/workflows`、`docs/evidence` | 验证、CI 和历史证据 | 原桌面脚本主要基于 Windows WebView2 |

## 本次适配

- `platform.rs` 提供共享命令查找。Unix 检查文件执行权限，允许 npm / Homebrew 的可执行符号链接和包含空格的绝对命令路径。Windows 保留 PATHEXT 和 PowerShell / CMD shim 分支。
- 桌面入口在启动 Tauri 工作线程前读取登录交互 shell 的 PATH，最多等待 3 秒、读取 64 KiB；shell 启动提示不进入 PATH。失败时保留原环境，追加 `/opt/homebrew/bin`、`/usr/local/bin`、`~/.unity/bin`、`~/.cargo/bin`。子进程也继承该 PATH，使 npm 的 `env node` 能找到 Node。
- `--mcp-bridge` 分支在环境初始化之前执行，不启动 shell，不向 MCP stdout 混入初始化信息。应用包中的桥由 `.app/Contents/MacOS/` 内的可执行文件提供。
- Unix Agent 启动时创建独立进程组，完成、取消、异常及会话任务释放时回收该组。主动脱离该组的第三方进程不在此机制范围内。
- Unity CLI 可查找 PATH 和 `~/.unity/bin/unity`，保留原 Windows 安装位置回退。CLI 不可用时仍可离线工程定位。
- macOS 插件安装原子创建 `Temp/UnityLockfile`，已有文件时拒绝安装并保留文件，退出安装后删除自己创建的文件。请先关闭目标 Editor，安装期间不要重新打开。崩溃残留不会自动清理，应确认 Editor 已退出后再清理残留 Temp。保留包校验、安装记录、恢复和 manifest 备份。
- 添加 `.icns` 图标及 `macos.yml`，CI 分别使用 ARM64 / Intel runner，运行公开回归，构建 `.app` / `.dmg`，再从实际应用包中的二进制复验 stdio 桥。

Finder 启动不继承 shell 配置的 PATH，见 [Tauri 官方说明](https://v2.tauri.app/distribute/macos-application-bundle/)。CI runner 架构依照 [GitHub 官方列表](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 开发与构建

支持目标为 macOS 13 及以上，Apple Silicon / Intel。构建需要 Node.js >= 20、Rust stable 和 Xcode Command Line Tools。未安装开发工具时运行 `xcode-select --install`；已有 Xcode 无需重复安装。最低系统版本已写入应用包配置，实际本机验证的系统版本另见状态台账。

```bash
npm ci
npm run tauri:dev
```

回归及本机架构打包：

```bash
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
python3 -m unittest discover -s tools -p 'test_*.py'
npm run tauri:build -- --bundles app,dmg
```

应用位于 `src-tauri/target/release/bundle/macos/Unity Profiler Analysis Agent.app`，DMG 位于 `src-tauri/target/release/bundle/dmg/`。默认匹配构建机器架构。通用应用构建方式：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri:build -- --target universal-apple-darwin --bundles app,dmg
```

通用产物位于 `src-tauri/target/universal-apple-darwin/release/bundle/`；本次 CI 分别构建两种架构，通用包另需实际验证。

安装 DMG 中的应用到 Applications 后启动。当前未配置 Developer ID 签名和 Apple 公证，本地及 CI 产物用于开发验证。正式分发需按 [Tauri macOS 签名文档](https://v2.tauri.app/distribute/sign/macos/)配置证书与公证凭据。

## 使用与验收边界

解析无需 Unity Editor 或 Agent。AI 功能需要安装并登录相应 ACP 适配器；应用检测 `claude-code-acp` / `codex-acp` / `gemini`，存在命令不等于协议兼容。安装后重启应用；Node 版本管理器应在登录 shell 中设置 PATH。Unity 联合定位需要 CLI、所选工程的 Editor 和 UPAA 插件。

应用平台和录制目标平台相互独立。Windows 历史对照仅覆盖原文档列出的输入；Mac Player / Mac Editor 新录制仍需独立样本和 Counter 对照。2022.3 / 6000.3.x 的当前边界见[解析契约](data-parser-current.md)。

本机验证和待验收事项维护于[项目状态](project-status.md)。Windows 安装、WebView2 界面自动化、真实 ACP Agent 的历史证据保持原范围。
