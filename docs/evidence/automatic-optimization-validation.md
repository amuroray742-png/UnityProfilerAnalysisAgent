# 一键代码优化验证（2026-09-29）

范围：Windows、Unity 6000.3.23f1、公开隔离工程。没有用私有业务工程执行修改验收。原有用户桌面窗口保留运行。

## 自动回归

- 完整 Rust：139 项通过，22 项环境/真实 Agent 测试默认忽略；忽略不算通过。
- 前端：39 项通过；覆盖无候选任务时一键启动、独立选择修改 AI、缺失/零值和导出取消等。
- 覆盖继续调查后读取新文件再编辑、新代码/meta/目录恢复、外部变更与引用冲突、版本 1 备份迁移、元数据规范化、普通诊断无写入权限。
- 最终生产构建通过；最终 EXE 协议回归 42 项通过、9 项默认忽略；Python 工具回归 8 项通过。
- 新增取消工程扫描与关闭只读会话不影响后续检查的回归，以及恢复 meta 后尚未落盘的中断回退回归。

## 真实 Agent 与 Unity

显式运行 `UPAA_PUBLIC_UNITY_PROJECT=<带 .upaa-public-fixture 的公开工程> cargo test --manifest-path src-tauri/Cargo.toml --test optimization real_automatic -- --ignored --nocapture --test-threads=1`。

两个 Agent 均实际读取代码、修改 AutomaticHotspots.cs、新增 AutomaticBufferCache.cs、完成 Unity 编译检查，并由后端恢复原字节、移除新代码及 meta。耗时合计约 200.42 秒。

| Agent | 新 ACP 会话 | 编译 | 精确回退 |
|---|---|---|---|
| Claude Code | 05872511-1813-44d1-80a2-6cead8134914 | 通过 | 通过 |
| Codex | 01a0ed58-7b99-7aa3-9c22-1487f7821637 | 通过 | 通过 |

证据在忽略目录 `.cache/automatic-agents.log`、`.cache/automatic-claude-code.json`、`.cache/automatic-codex.json` 及 `.cache/automatic-records-*`。检查插件先刷新 AssetDatabase，避免回退后编译缓存继续引用已删除辅助代码。没有选 EditMode 测试，本次是编译检查，不算玩法或行为测试通过。

公开调查样例还包含批量实例化／加载业务入口、调用下游 getter 的分配、可补 Marker 的业务边界和无法定位的 Unmapped.Native，见 `src-tauri/tests/fixtures/unity-project/INVESTIGATION.md`。这些为调查素材，不是所有热点都能自动定位的证明。

## 边界

性能输入为公开合成数据，未重录真实游戏 B，不证明性能收益。Marker 的实际采样有效性仍需重录。Shader/HLSL/Compute 的全部平台／变体仍未全面验证。回退依赖检查是有覆盖上限的保守文本/GUID/类型名检查，不是完整 C# 编译器依赖分析；无法确认或覆盖不完整时停止删除。仅自动接纳生成 meta 的格式及已知空默认字段变化，未知导入设置变化保留为冲突。

## 正式桌面与导出验收

通过真实界面完成：公开合成录制导入 → Claude 首轮诊断 → 工程定位 → 直接点击开始优化 → 自动建立保存目录及本轮记录 → 修改已有 AllocationWork.cs 并新增 DesktopBufferCache.cs → Unity 编译通过 → 点击 Markdown / HTML 导出 → 点击撤销并核对原字节、新文件及 meta 移除 → 打开保存记录。

没有勾选任务、确认文件列表或选择保存目录。系统打开/保存对话框的返回值由测试替代，不计为原生对话框交互验收。端到端证据：`.cache/desktop-optimization/result.json`，该次 EXE 为 `5ab18f2c23a969df278e9fc73c5de734311cbfbe7c608cb5a7de5f58b62103f8`。

后续补充历史轮次选择及恢复边界后，再以最终 EXE 打开同一保存记录，验证默认折叠详情、分页与实际导出按钮。最终 EXE SHA-256：`f63fa4106b536239c1203fa63e5c12fbc948fb50fbfc1bd5de70ba2652a82cb6`。证据：`.cache/desktop-optimization-saved/result.json`。HTML 通过浏览器安全节点、无横向溢出和打印验证；未将模拟 B 或代码编译通过当作性能收益。
