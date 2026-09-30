# Unity Profiler Analysis Agent

当前主流程：**新建／打开优化项目 → 导入 A → 一键诊断并定位 → 选择修改 AI → 开始优化 → 手动重录 B → 对比 → 接受或回退 → 下一轮**。每轮报告和修改记录自动存档，重开项目可继续。参见[简单使用说明](docs/optimization-loop.md)。

面向 Unity Profiler 离线录制的本地桌面分析原型，使用 Tauri 2、React / TypeScript 和 Rust。首要目标是 **Windows 上的 Unity 6000.3 可信分析**，通过 ACP Agent 与 MCP 数据查询辅助诊断。

当前 CPU/GC 功能链路已完成本轮限定范围的本地验收：Editor dump 与 Unity 6000.3.23f1 `.data` 导入、指标聚合、慢帧/高分配帧定位、原始线程调用树，以及 Windows Claude Code ACP 的 MCP 查询、流式诊断、取消和重新诊断。两份录制的 137 个参考帧通过样本、CPU/GC 和站点归因对照，包含加载高峰和末帧；该证据不覆盖所有录制或 Unity 版本。缺失指标显示“—”，有效零值仍显示为零。AI 正文是辅助解释，须与原始样本核对。

CPU/GC 与诊断之后，新增 Unity 6000.3.9f1 渲染计数对照与展示，见[渲染验收记录](docs/rendering-validation.md)。GPU 时间、版本升级、签名及长期性能预算仍后续推进。原生文件选择与 EXE/MSI 安装、启动、快捷方式、卸载已由维护者人工确认通过。完整可信分析 MVP 与正式发布仍按[状态台账](docs/project-status.md)分别验收。

新增[工具内优化项目](docs/optimization-loop.md)：独立修改 Agent、新 ACP 会话、自主代码调查、修改与新增、持久化备份与回退，以及跨录制 A/B 复验。诊断和定位仍只读，用户选择修改 AI 并点击“开始优化”即可启动。

## 文档导航

- [手动 Marker 补点建议](docs/marker-guidance.md)：首轮指出采样缺口，第二阶段根据实际代码给出具体补点方案，用户可交给程序手动添加，或在点击“开始优化”后由修改 AI 补点，再重新录制。

- [Flow 解码与跨线程查询](docs/flow-decoding.md)：事件 ID、样本关联、相邻帧查询和证据边界。

诊断正文可导出为 Markdown 或 HTML。新建项目时绑定 Unity 工程，诊断后自动联合分析代码、关联资源和同一工程的 Editor 采集信息。Editor 不可用时明确降级为离线定位。原报告保留，最终报告旁可直接单独或合并导出；诊断和定位阶段不会修改代码、资产或设置。用户另行点击“开始优化”后才进入代码修改阶段。版本匹配与性能因果仍需核对。

- [Unity 工程联合定位与最终报告导出](docs/project-diagnosis.md)：工程索引、Editor 插件、证据边界和验收。
- [报告导出与 C# 源码定位](docs/reports-and-source.md)：Markdown / HTML 导出、只读源码定位、访问范围与公开样例验收。
- [项目状态、验证证据与路线图](docs/project-status.md)：完成判断的唯一详细台账。
- [架构与数据契约](docs/architecture.md)：现有数据流、目标数据流及实现边界。
- [原始帧与调用树查询](docs/frame-queries.md)：线程选择、分页、数据来源和临时存储生命周期。
- [Windows 性能与发布验收](docs/performance-and-release.md)：release 测量方法、基线与安装验收边界。
- [Windows 桌面验收](docs/manual-acceptance.md)：已验证的结果页/诊断流程与剩余原生窗口、安装检查。
- [本地交付与发布门槛](docs/release-readiness.md)：本轮交付范围、检查结果及需要维护者决定的事项。
- [ACP / MCP 集成状态](docs/acp-mcp-integration.md)：分阶段会话、工具权限与适配器验收边界。
- [Unity 导出与对照操作](docs/unity-scripts/ExtractProfilerDump.README.md)：研究用 dump 与应用 JSON 的区别。

## 当前能力

状态定义及完整证据见[项目状态](docs/project-status.md#状态口径)。下表为台账的入口摘要，不代表所有版本和输入均已验证。

| 能力 | 状态 | 边界 |
|---|---|---|
| 文件选择、解析进度、指标和诊断输出界面 | 已验证 | 限定 Windows release 主流程回归通过；自动化替代文件选择返回值，原生选择与取消另由维护者人工确认 |
| Editor dump JSON 导入 | 已验证 | 64 帧参考 dump 与合成样本；CPU/GC 范围与有效帧数明确 |
| 项目约定 JSON 解析与指标聚合 | 部分实现 | 有合成输入单元测试，不是任意 Unity JSON 通用导入器 |
| Unity 2022.3 `.data` | 部分实现 | 有采样树与 GC metadata 解码；Draw Call / SetPass 标为不可用 |
| Unity 6000.3.23f1 `.data` | 部分实现 | 两份录制的指定范围通过 CPU/GC 对照；帧时间来自下一帧起点，末帧不可用；渲染计数已接入，渲染数值对照范围为另一个 6000.3.9f1 录制；更广版本待验证 |
| Unity 6000.3.9f1 `.data` | 已验证（限定范围） | 单录制 2,000 帧五类渲染计数逐帧对照，1,998 帧有效；CPU/GC 对照 7 帧；GPU 与 SRP 收益不可用 |
| `.pd3u` / `.raw` | 占位 | 文件头识别及帧数估算，不具备实质性能分析能力 |
| ACP / MCP | 部分实现 | Windows Claude Code ACP 0.16.2 与 Codex ACP 1.13.1 已有公开样例诊断、定位及受限修改证据；Gemini 与广泛诊断准确性待验收 |
| 跨平台安装包、体积和性能承诺 | 待验证 | 不能从框架支持推导出本项目已验证 |

## 开发环境

- Node.js >= 20 与 npm。
- Rust stable；依赖版本以 `src-tauri/Cargo.lock` 为准。清单声明的最低 Rust 版本尚未单独验证。
- Windows 构建需要 MSVC C++ 构建工具、Windows SDK 与 WebView2，见 [Tauri 前置条件](https://v2.tauri.app/start/prerequisites/)。
- 解析文件不需要 Agent；内置 Agent 命令被 PATH 检测到，也不代表协议兼容。

在仓库根目录执行：

```powershell
npm ci
npm run tauri:dev
```

构建与验证：

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri:build
```

`npm run build` 只验证前端。2026-09-28 的历史发布验收中，`tauri:build` 已在本机生成 MSI / NSIS 包；NSIS 当前用户安装、安装后界面/协议和卸载已通过；MSI 管理员静默安装/协议/卸载也已在 CI 通过；交互安装向导、快捷方式、MSI GUI 和原生文件选择已由维护者本机人工确认，见[发布记录](docs/performance-and-release.md)。Rust 依赖已缓存时可追加 `--offline`。默认 Rust 测试会跳过依赖私有文件或进程环境的集成测试，详见验证台账。

### Windows 安装包

在仓库根目录执行 `npm run tauri:build`，生成 MSI 与 NSIS 安装包，分别位于 `src-tauri/target/release/bundle/msi/` 和 `src-tauri/target/release/bundle/nsis/`。只生成 EXE 安装包可用 `npm run tauri:build -- --bundles nsis`。

`npm run tauri:build -- --no-bundle` 只生成 Release 程序，不生成安装包；`npm run build` 只构建前端。新构建产物不能直接沿用历史包的安装验收结论，见[发布门槛](docs/release-readiness.md)。

## 当前使用流程

1. 点击 **新建优化项目**，选择 Unity 工程目录；已有项目从首页最近列表或 **打开优化项目** 进入。
2. **导入录制 A**：可用 Unity 6000.3.23f1 / 6000.3.9f1 `.data` 或 [Editor dump JSON](docs/unity-scripts/ExtractProfilerDump.README.md)。支持范围、估算及缺失指标详见[布局验证](docs/unity6-layout-research.md)。
3. 选择已安装并登录的分析 AI，点击 **一键诊断并定位**。两阶段分别开启会话；Editor 不可用时继续离线定位并说明缺口。
4. 定位完成后独立选择修改 AI，点击 **开始优化**。无需逐项授权代码路径；每次使用新会话，修改与新增均记录备份。
5. 查看结果，自行复测玩法并录制 B。**导入复测录制 B → 对比 A 与 B → 接受或回退 → 开始下一轮**。
6. 每轮报告自动保存。打开项目直接继续当前步骤；“历史轮次”可查看各版报告、逐文件修改差异并导出，“高级选项与详细指标”可看 CPU/GC、渲染、帧调用树和 Flow。

完整[非程序员操作说明](docs/optimization-loop.md)包含停止、重试、存档路径、录制丢失和回退冲突处理。

当前没有统一录制文件大小上限；实际内存取决于输入结构。已移除未落实的“最大 500MB”提示，测量范围及已知内存峰值见[性能基线](docs/performance-and-release.md)。

## 项目约定 JSON 示例

以下是解析器 V1 结构的最小示例；数值为演示数据，不是性能基准：

```json
{
  "meta": { "unityVersion": "6000.3.23f1", "platform": "Windows" },
  "frames": [{
    "index": 0,
    "durationMs": 16.0,
    "cpuMs": 16.0,
    "gcAllocBytes": 128,
    "drawCalls": 100,
    "setPassCalls": 20,
    "mainThreadSamples": [{
      "name": "PlayerLoop", "totalMs": 16.0, "callCount": 1, "maxMs": 16.0
    }]
  }]
}
```

Editor 脚本产生的 `frames[].threads[].samples[]` dump 由独立分支直接支持，无需转换为 V1。V1 的 cpuMs 缺失时主线程指标不可用，durationMs 只作为帧时间。ExtractProfilerDump 已通过本机 Editor 编译与重新导出；ProfilerJsonExporter 示例与全新 batch 启动仍待单独验收。

## 项目信息

架构与部分解析思路参考 [librashuai/UnityPerfAgent](https://github.com/librashuai/UnityPerfAgent)。本项目采用 [MIT License](LICENSE)，Copyright (c) 2026 amuroray742-png。

[data 原始证据、Self Time 与帧对比](docs/data-evidence.md)：CPU/GC 页展开帧证据，查询 Counter/metadata 或比较完整调用路径。

### 历史与实时工作

“当前工作／历史轮次”提供各轮报告、报告版本、修改记录和逐文件差异。导入显示校验、解析、汇总、保存阶段；有准确字节总量时显示百分比，否则显示活动条。AI 面板展示真实公开输出和工具操作摘要，不展示内部思考。详细操作见[简单使用说明](docs/optimization-loop.md)。
