# 本地交付与发布门槛

## 当前交付索引（2026-09-30）

当前功能包括项目入口与连续轮次、受限自主代码优化、A/B 对比、历史报告与差异、解析进度及公开 AI 活动。使用流程以[简单使用说明](optimization-loop.md)为准，功能完成范围以[状态台账](project-status.md)为准。

| 对象 | 已有证据 | 边界 |
|---|---|---|
| 源码回归 | 2026-09-30 历史／实时工作增量：Rust 149、前端 55、Python 8 项通过；Rust 22 项默认忽略 | 是该次代码验收记录，不是本次文档整理重新运行的结果 |
| Release EXE | 同次最终 EXE 的协议回归、历史重开、导出与真实字节进度验收；Claude／Codex 的公开样例真实会话另有各自程序哈希 | 详见[历史与实时工作证据](evidence/history-activity-validation.md)，不把不同构建混为同一产物 |
| MSI／NSIS | 历史构建、CI 静默安装／协议／卸载及维护者交互人工确认 | 上述 EXE 增量未重新验收安装包；历史结论不自动覆盖新安装包 |

### 构建与分发

仓库根目录执行 `npm ci`（首次或依赖变化时），再执行 `npm run tauri:build`。MSI 输出到 `src-tauri/target/release/bundle/msi/`，EXE 安装包输出到 `src-tauri/target/release/bundle/nsis/`；仅需后者可用 `npm run tauri:build -- --bundles nsis`。

`npm run tauri:build -- --no-bundle` 只构建 `src-tauri/target/release/unity-profiler-analysis-agent.exe`；`npm run build` 仅构建前端。关闭正在使用目标 EXE 的窗口后再构建，避免文件占用。生成成功不等于安装验收通过；分发新包时应记录版本、哈希及对应验收结果。

版本升级、签名与正式发布身份、跨平台及长期性能预算仍保留原验收边界。本次文档整理不修改版本号、不生成安装包，也不新增验证结论。

## 历史交付记录

以下内容保留当时日期、提交、数量与处理经过，不作为当前产物状态。

历史检查日期：2026-09-28。基线 `8524d32`，后续改动已推送到 `codex/trusted-unity6-analysis`，[PR #1](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/pull/1) 已合并；MSI 验收增量见[已合并 PR #2](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/pull/2)。提交 `3cd15e8` 的 push 与 PR Windows CI 均通过。本页是交付索引；详细历史与范围以[状态台账](project-status.md)为准。

## 2026-09-28 历史交付范围

- P0：Editor dump 正式输入、类型化校验、指标质量、CPU/GC 聚合；指定 Unity 6000.3.23f1 二进制结构解析与两录制对照。
- P1：原始帧/线程/调用树分页、查询源释放；ACP 会话、只读 MCP 桥、取消与错误终态，真实 Windows Claude Code 验证。
- P2：帧摘要内存优化、Windows 构建/安装回归、桌面控件验收、进程及 DOM/堆测量；识别 WebDriver 元素句柄对 DOM 保留的影响。
- 文档、人工合成 fixture、回归与测量脚本、Windows CI 配置。

## 证据与边界

| 门槛 | 2026-09-28 证据 | 未覆盖范围 |
|---|---|---|
| 可信解析 | 两份录制，137 个参考帧、1,306,026 个样本对照；完整 Rust 81 项通过，11 项环境测试默认忽略 | 其他录制/版本不能自动沿用结论 |
| 前端与桌面 | 前端 15 项；release 界面/真实 Agent 10 组检查；零值/缺失、帧线程分页、取消及重试 | 自动化替代原生文件选择；另有维护者本机人工验收通过，截图未提供程序哈希 |
| 诊断内容 | 公开 normal.json 两次回答核对嵌套分配、分位数与 inclusive 语义 | 不证明其他输入或未来模型回答总是准确 |
| 安装包 | MSI/NSIS 构建及静默安装、安装后 8 项协议回归、卸载；NSIS 安装后界面通过；EXE/MSI 交互安装、启动、快捷方式和卸载由维护者人工确认 | 版本升级；人工反馈未附包哈希，不外推其他包 |
| 内存 | 独立解析、真实 IPC、结果页十轮及堆/DOM 对照；报告明确采样/自动化干扰 | 非严格内存上限，长期运行与 Agent 负载尚无预算 |
| CI | `3cd15e8` 远端全流程通过，已核对安装日志与结果 artifact | CI 不运行私有录制、真实 Agent 或桌面 GUI |

本地安装包位于 `src-tauri/target/release/bundle/`，当次产物哈希见[性能与发布记录](performance-and-release.md#本地构建结果)。它们是未签名验证包，不是已公开发布的正式版本。

## 2026-09-28 交付检查

- 检查 Git 已跟踪和未忽略候选文件：112 个文件（含本页），未发现 `.data` / `.raw` / `.pd3u` / 堆快照或大于 2 MiB 的候选文件。
- `.cache`、`node_modules`、Rust 构建产物和私有输入目录被忽略；采样结果、截图及完整诊断正文未列入待交付文件。
- 本地 Markdown 文件目标检查未发现失效路径；不把此检查当作外部 URL 可达性保证。
- 当次 MSI/NSIS 文件哈希与当次发布记录一致；5 项非法测量参数组合均在启动应用前拒绝。
- `git diff --check` 通过。该检查及文件范围检查不等同于全面安全审计。

## 历史发布事项处理记录

1. **公开仓库推送决策**：已核实仓库 `amuroray742-png/UnityProfilerAnalysisAgent` 为公开且当前账号具有 ADMIN 权限。推送分支和创建草稿 PR 会公开本地源码、文档及人工合成样本；维护者已授权，PR #1 已合并，MSI 验收 PR #2 与原生文件选择记录 PR #3 均已合并。
2. **许可证与发布身份**：维护者已委托补充版权声明；按仓库账号填写 Copyright (c) 2026 amuroray742-png，使用包清单既定 MIT 许可证。应用标识及签名安排仍是正式发布边界。
3. **环境边界**：MSI perMachine 安装已在管理员 CI runner 验证，无需维护者重复静默安装；原生窗口控制曾因截图/坐标输入失败而无法验收文件选择框和安装向导。原生文件选择框现已由维护者人工确认通过；已有 WebDriver 流程不重复交给维护者执行，交互安装、快捷方式与 MSI GUI 现也由维护者确认通过，该环境阻塞已解除，详见[人工验收记录](manual-acceptance.md#安装包人工验收2026-09-28)。

以上参与项不把更多平台、Agent 或任意输入扩展列为本轮发布前默认完成能力。当前仍不标记可信分析 MVP 或正式发布完成。
