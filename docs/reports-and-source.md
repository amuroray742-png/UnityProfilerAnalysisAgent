# 诊断报告导出与 C# 源码定位

> 桌面主流程已升级为 [Unity 工程联合定位](project-diagnosis.md)。本文保留 C# 旧接口与此前验收记录，`source_*` 权限仍仅限 C#。

## 使用方式

1. 导入录制，选择可用 Agent，运行首轮性能诊断。
2. 有正文后即可在“报告”区域选择性能诊断、源码定位或合并报告，再选择 Markdown / HTML 并点击“导出报告”。系统保存窗口取消不报错；保存失败可重试。未完成、取消、失败和超限报告会保留状态及原因。
3. **首轮完整完成后**，在“选择源码目录并定位”区域选择目录或粘贴本地目录。当前选择的 Agent 会接收按需读取的代码片段，可先在页面上方切换 Agent。应选择与录制版本一致的源码。
4. 点击“开始源码定位”。原性能报告保留，另行显示源码报告。失败或取消不影响首轮，可以换目录重试；新定位启动后替换上次定位。上方“取消”也可停止目录准备。

源码定位只给出建议，不修改源码、不生成可应用补丁，不提供持续对话或报告历史。报告仅存于本次应用内存；重置、切换录制、退出会清除。主动导出的文件继续保留。

## 导出内容与限制

导出来自后端保存的完整正文，不依赖前端事件日志或显示缓冲。单次报告正文最多 **2 MiB UTF-8**；超限停止该次诊断，保留已收到的前缀并标记不完整，不能用它启动源码定位。

导出包含录制名、Unity 版本、分析帧数、CPU / GC / 渲染计数覆盖率、Agent、生成时间、状态、正文与证据边界。不会默认打包录制、完整源码或通信日志。两份合并报告必须属于同一录制并有父子关联。

Markdown 保留正文格式。界面与 HTML 导出共用 Rust Markdown 渲染器，支持标题、列表、表格和代码块；原始 HTML 按文字转义，图片不加载，危险链接失效。HTML 自带样式和打印样式，可离线用浏览器阅读并打印为 PDF；应用不直接导出 PDF。

## 源码访问边界

- 只索引所选目录下的 `.cs`，允许本地 `Packages`。不遍历符号链接和 Windows junction。
- 递归排除 `.git`、`Library`、`Temp`、`Obj`、`Logs`、`Build`、`Builds`、`bin`、`obj`、`node_modules`，大小写不敏感。
- 最多扫描 50,000 个 C# 文件，超限需缩小目录；单文件上限 2 MiB。支持 UTF-8（含 BOM）、带 BOM 的 UTF-16 LE/BE。无法访问、编码错误或过大的文件跳过，并说明覆盖缺口。
- 文件列表 / 字面量搜索每页最多 100 项；读取每次最多 400 行，响应上限 64 KiB。`nextStart`、`lineTruncated` 表达分页和截断，部分结果不能视为完整搜索。
- 路径须为索引内相对路径。每次读取重新检查目录范围及链接，在 Windows 上再核对打开的文件句柄路径。SHA-256 与索引时不同则拒绝读取，提示重新分析，避免引用旧行号。
- 扫描、读取在阻塞任务执行并检查取消标记。每次准备生成独立范围 ID，绑定录制；会话结束后失效，重试重新准备。

Agent 仍在临时目录运行。普通性能会话只有性能查询工具；源码会话增加 `source_files`、`source_search`、`source_read`，保留原性能工具并带入完整首轮报告。ACP 终端和写入请求不会因源码分析而获准，也没有新增网络工具。这是应用协议和工具层的访问约束，不是对任意第三方 Agent 进程的操作系统沙箱承诺。

## 报告应如何解释

源码报告按热点引用性能帧 / 线程 / marker、实际读取的 C# 相对路径、行号、代码片段和哈希；给出修改建议、适用条件、代价及复测方式。**名称相同只是候选，不能证明因果**。引擎内部、找不到源码、同名方法、源码版本未知或只有上层调用时，应明确证据不足，不猜测行号、GC 原因或收益。

AI 输出仍需要维护者核对，工具返回真实代码不等于所有建议正确。公开验收中特别核对了数组容量与测得 GC 字节数不完全一致，报告没有将二者强行等同。

## 开发与复现

新增 Tauri 命令：`list_reports`、`render_report_markdown`、`export_reports`、`prepare_source`、`cancel_source_preparation`、`diagnose_source`。旧 `diagnose` / `cancel_diagnose` 及性能查询接口不变。启动源码诊断只传首轮报告 ID 与范围 ID，完整上下文由后端取得。

公开 C# 样例位于 `src-tauri/tests/fixtures/source-project`；包含分配数组的 `AllocationWork.Update` 和另一个同名 `OtherWork.Update`。真实 Agent 集成测试使用公开 `isolated-peak.json`，在测试内调整 marker 名称，保留一个无法定位的 `Unmapped.Native`；没有发送私有项目。

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline
npm run tauri:build -- --no-bundle
$env:UPAA_TEST_APP_EXE = (Resolve-Path src-tauri/target/release/unity-profiler-analysis-agent.exe).Path
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test acp_stdio_roundtrip --test mcp_wire --test reports_source
$env:UPAA_REAL_AGENT = 'codex-acp'
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test reports_source real_agent_locates_public_csharp_with_evidence -- --ignored --nocapture
python tools/desktop-smoke.py --report-source --agent-id codex
python tools/report-html-smoke.py
```

真实 Agent 集成测试和桌面源码流程会调用 Agent，使用公开样例，并要求本机已安装且已认证。未设置环境变量时显式运行真实 Agent 测试会失败，不计为通过。桌面脚本通过真实 release WebView 和后端执行，只替代系统目录 / 保存对话框的返回值，原生对话框的交互外观仍需人工核对。最后的 HTML 检查使用 headless Edge 阅读并打印已导出的合并报告，不调用 Agent。

验证结果集中记录在[项目状态](project-status.md)，不将单次回答外推为所有项目的定位准确性。
