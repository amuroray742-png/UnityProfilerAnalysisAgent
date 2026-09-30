# 项目入口工作流验证（2026-09-30）

范围：Windows、Unity 6000.3.23f1、仓库公开样例与隔离工程。没有对私有工程执行 AI 修改。

## 自动回归与最终构建

- 完整 Rust：144 项通过，22 项环境／真实 Agent 测试默认忽略。忽略不算通过。
- 前端：48 项通过；Python 工具：8 项通过；生产前端和 Release EXE 构建通过。
- 最终 EXE 协议与优化回归：47 项通过，9 项默认忽略，覆盖 `mcp_wire`、`acp_stdio_roundtrip`、`reports_source`、`optimization`。
- 新增项目首页、一键诊断参数、独立修改 AI、重开不启动 AI、旧定位不可覆盖新失败诊断、保存取消、导入 B、启动准备阶段停止、回退后不误报新增 Marker 的前端回归。
- 后端新增分对象存档、正文日志恢复及重复防护、工程缺失仍可读、对象指纹损坏、只读磁盘写入失败、旧版完整备份迁移和下一轮基线选择测试；原有文件／meta 回退与协议测试继续通过。

最终 EXE SHA-256：`3f2f7f059e134d117556e0dbd96e65d89b35344baaff754ae83e5a9c9cbe74eb`。

本机证据在忽略目录：`.cache/workflow-release-rust.log`、`.cache/workflow-protocol-final.log`、`.cache/workflow-frontend-final.log`、`.cache/workflow-python.log`、`.cache/workflow-release-final.log`。

## 真实桌面两轮流程

公开素材：`src-tauri/tests/fixtures/isolated-peak.json` 与 `src-tauri/tests/fixtures/unity-project`。隔离工程位于 `.cache/unity-project-public`，必须带 `.upaa-public-fixture`；原生文件／保存对话框返回值由测试替代，不计为系统对话框交互验收。

1. 从首页新建项目，选择公开 Unity 工程，导入 A，关闭并重新打开存档。
2. Claude 一键完成性能诊断和工程定位；重开后启动新修改会话。实际修改 `AutomaticHotspots.cs`，新增 `WorkflowBufferCache.cs` 和后端管理的 meta，Unity 编译通过。
3. 修改后及导入 B 后分别重开；完成 A/B、Markdown／HTML 导出并接受修改。下一轮自动以 B 为 A。
4. Codex 完成第二轮诊断和定位，再以独立修改会话在已有业务方法添加静态 `Public.AutomaticHotspots.Update` Marker；编译通过，再次导入 B、对比及导出。
5. 回退第二轮后，再通过后端回退第一轮以清理公开样例；核对原代码字节完全恢复，辅助代码和 meta 均移除。存档保留两轮完整报告与回退记录。

六个实际 ACP 会话互不相同：

| 阶段 | Claude 第一轮 | Codex 第二轮 |
|---|---|---|
| 诊断 | `45d0dbc9-f921-4c52-a832-e5111c0bad16` | `01a0f07d-d0d0-7511-a7ad-e5ebb5185277` |
| 定位 | `4c35782b-e4fa-45ac-8ef3-7678c5c7f435` | `01a0f07f-454f-7151-a108-96f2bd601e9e` |
| 修改 | `f0724af0-469b-4125-959e-0099d493f0c9` | `01a0f082-0176-74f0-9153-fd3a0123f66b` |

完整真实流程使用 EXE `b4ba44369ae63f1e35884613dc5e86be566df7d4d0734d1fa456a7894cc75b91`，证据 `.cache/desktop-workflow/result.json`。首次测试在后台完成与按钮启用之间点击过早，修正测试等待条件后从已存档的第一轮定位继续；没有重复伪造报告。

之后补充启动取消、路径身份保护、回退提示等，并使用上述最终 EXE 在另一个应用进程打开同一存档，验证两轮恢复及实际导出按钮。证据 `.cache/desktop-workflow-saved/result.json`。最终 HTML 在无界面 Edge 中阅读／打印通过：2 张表、9 个代码块、可执行／远程资源节点 0、无横向溢出；PDF 610,773 字节。

## 复现

公开工程按既有[公开优化验收](automatic-optimization-validation.md)准备，使用匹配 Unity 和检查插件，保持 Editor 就绪；不要把测试入口指向私有工程。

```powershell
python -X utf8 tools/desktop-smoke.py --project-workflow
python -X utf8 tools/desktop-smoke.py --workflow-saved <公开项目存档目录>
```

测试保存位置从 `result.json` 的 `records` 读取。`--workflow-saved` 只重开及导出，不调用 AI。旧优化桌面参数兼容为新项目流程入口。

## 未外推的范围

- A/B 是合成回归输入，没有真实游戏重录或性能收益证明；纯 Marker 的有效性仍需实际采样验证。
- 本次真实流程没有选 EditMode 测试，编译通过不等于玩法正确。
- 日志中断恢复通过模拟未检查点化日志的自动测试验证；桌面流程验证正常关闭／重开，不声称进行了全部断电／强杀场景。
- 保留已有代码修改边界，未扩大至场景、资源、设置或依赖修改；不把 Agent 工具权限描述为操作系统沙箱。
- 本机生成 EXE，未重新交互验收 MSI／NSIS 安装器；远程 CI 结果单独查看。
