# Unity Profiler Analysis Agent

面向 Unity Profiler 离线录制的本地桌面分析原型，使用 Tauri 2、React / TypeScript 和 Rust。首要目标是 **Windows 上的 Unity 6000.3 可信分析**，通过 ACP Agent 与 MCP 数据查询辅助诊断。

当前 CPU/GC 功能链路已完成本轮限定范围的本地验收：Editor dump 与 Unity 6000.3.23f1 `.data` 导入、指标聚合、慢帧/高分配帧定位、原始线程调用树，以及 Windows Claude Code ACP 的 MCP 查询、流式诊断、取消和重新诊断。两份录制的 137 个参考帧通过样本、CPU/GC 和站点归因对照，包含加载高峰和末帧；该证据不覆盖所有录制或 Unity 版本。缺失指标显示“—”，有效零值仍显示为零。AI 正文是辅助解释，须与原始样本核对。

维护者确认本轮先完成 CPU/GC 与诊断；渲染、版本升级、签名及长期性能预算后续推进，不作为本轮功能验收阻塞。原生文件选择与 EXE/MSI 安装、启动、快捷方式、卸载已由维护者人工确认通过。完整可信分析 MVP 与正式发布仍按[状态台账](docs/project-status.md)分别验收。

## 文档导航

- [项目状态、验证证据与路线图](docs/project-status.md)：完成判断的唯一详细台账。
- [架构与数据契约](docs/architecture.md)：现有数据流、目标数据流及实现边界。
- [原始帧与调用树查询](docs/frame-queries.md)：线程选择、分页、数据来源和临时存储生命周期。
- [Windows 性能与发布验收](docs/performance-and-release.md)：release 测量方法、基线与安装验收边界。
- [Windows 桌面验收](docs/manual-acceptance.md)：已验证的结果页/诊断流程与剩余原生窗口、安装检查。
- [本地交付与发布门槛](docs/release-readiness.md)：本轮交付范围、检查结果及需要维护者决定的事项。
- [ACP / MCP 集成状态](docs/acp-mcp-integration.md)：协议缺口和后续验收条件。
- [Unity 导出与对照操作](docs/unity-scripts/ExtractProfilerDump.README.md)：研究用 dump 与应用 JSON 的区别。

## 当前能力

状态定义及完整证据见[项目状态](docs/project-status.md#状态口径)。下表为台账的入口摘要，不代表所有版本和输入均已验证。

| 能力 | 状态 | 边界 |
|---|---|---|
| 文件选择、解析进度、指标和诊断输出界面 | 已验证 | 限定 Windows release 主流程回归通过；自动化替代文件选择返回值，原生选择与取消另由维护者人工确认 |
| Editor dump JSON 导入 | 已验证 | 64 帧参考 dump 与合成样本；CPU/GC 范围与有效帧数明确 |
| 项目约定 JSON 解析与指标聚合 | 部分实现 | 有合成输入单元测试，不是任意 Unity JSON 通用导入器 |
| Unity 2022.3 `.data` | 部分实现 | 有采样树与 GC metadata 解码；Draw Call / SetPass 标为不可用 |
| Unity 6000.3.23f1 `.data` | 部分实现 | 两份录制的指定范围通过 CPU/GC 对照；帧时间来自下一帧起点，末帧不可用；渲染计数与更广版本支持待完成 |
| `.pd3u` / `.raw` | 占位 | 文件头识别及帧数估算，不具备实质性能分析能力 |
| ACP / MCP | 部分实现 | Windows Claude Code ACP 0.16.2 的 MCP 查询、流式诊断和取消通过；release 完成/取消/重新诊断已验证；其他 Agent 与广泛诊断准确性待验收 |
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

`npm run build` 只验证前端；`tauri:build` 已在本机生成 MSI / NSIS 包；NSIS 当前用户安装、安装后界面/协议和卸载已通过；MSI 管理员静默安装/协议/卸载也已在 CI 通过；交互安装向导、快捷方式、MSI GUI 和原生文件选择已由维护者本机人工确认，见[发布记录](docs/performance-and-release.md)。Rust 依赖已缓存时可追加 `--offline`。默认 Rust 测试会跳过依赖私有文件或进程环境的集成测试，详见验证台账。

## 当前使用流程

1. 启动桌面应用，点击选择本地 Profiler 文件。
2. 应用登记原文件路径，读取并解析，再展示指标与解析警告；不会复制文件到上传目录。
3. 可选择 [ExtractProfilerDump.cs](docs/unity-scripts/ExtractProfilerDump.README.md) 导出的 `.dump.json`，或 Unity 6000.3.23f1 `.data`，查看 CPU/GC 指标及有效帧覆盖。`.data` 末帧的录制帧时间和全部渲染计数不可用；其他 Unity 6 版本仅提供帧头 CPU 估算。支持范围见[布局验证记录](docs/unity6-layout-research.md)。
4. CPU / GC 页分别从慢帧或高分配帧列表定位原始帧，选择线程和深度，分页查看真实样本及 GC 字节；完整帧下拉仍保留。重置会释放当前快照和查询源。
5. 选择已配置登录的 ACP Agent，点击“开始 AI 诊断”。当前验证了 Claude Code ACP 0.16.2；Agent 通过 MCP 查询本次录制。可点击取消，认证或协议失败会明确显示原因。

当前没有统一文件大小上限；实际内存取决于输入结构。已移除未落实的“最大 500MB”提示，测量范围及已知内存峰值见[性能基线](docs/performance-and-release.md)。

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
