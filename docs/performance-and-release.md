# Windows 性能与发布验收

更新：2026-09-28。本页记录 P2 的测量方法和当前证据，整体完成判断仍以[状态台账](project-status.md)为准。

## 性能基线

使用 release 优化的[测量入口](../src-tauri/examples/profile_benchmark.rs)，调用生产 `parse_file → extractor → 单帧查询`，每个进程连续导入同一文件三次。每次释放 profile、snapshot、查询结果，并断言查询源没有剩余强引用，再等待 500 毫秒。测量不包含 Tauri WebView、AppState 缓存或 Agent，不代表整个桌面应用的内存预算。

Windows [采样脚本](../tools/measure-profile.ps1)每 50 毫秒读取进程内存。工作集峰值来自操作系统的 PeakWorkingSet64；私有内存峰值取采样最大值，可能遗漏短暂尖峰。记录 CPU、物理内存、OS、输入与二进制 SHA-256、每次解析/提取/查询时间和采样序列。JSON 报告保存在忽略目录 `.cache/benchmarks/`，私有输入与完整报告不提交仓库。

环境：Intel Core i7-13700K，24 个逻辑处理器，约 63.7 GiB 可见物理内存；Windows NT 10.0.19044.0。下表为每个版本各三次重复结果范围，未与构建任务并发执行，未清理系统文件缓存。

| 输入 | 规模 | 解析 / 次 | 聚合 / 次 | 200 节点查询 / 次 | 峰值工作集 | 采样私有峰值 | 最后释放后工作集 |
|---|---|---|---|---|---|---|---|
| dump A，合并前 | 146,127,279 字节，64 帧 | 373–394 ms | 14.5–14.7 ms | 1.77–1.83 ms | 184.83 MiB | 185.55 MiB | 8.70 MiB |
| dump A，合并后 | 同上 | 400–414 ms | 4.52–4.71 ms | 1.92–1.95 ms | 183.96 MiB | 187.34 MiB | 9.34 MiB |
| data B，合并前 | 1,162,295,388 字节，2,000 帧 | 4,557–4,892 ms | 1,766–2,059 ms | 6.51–7.27 ms | 2,233.89 MiB | 2,463.48 MiB | 23.84 MiB |
| data B，合并后 | 同上 | 4,776–4,968 ms | 203–207 ms | 5.20–5.62 ms | 215.89 MiB | 211.00 MiB | 12.31 MiB |

公开 fixture 的两次导入冒烟测试也通过，输入 3,397 字节，进程峰值工作集约 7.87 MiB。这些测量证明指定输入可以完成且释放查询源；不证明任意大文件的内存有界，也不证明不存在其他路径的缓存泄漏。

### 帧摘要合并（2026-09-28）

定位到 data B 合并前保留 12,908,115 条 CPU 摘要和 6,440,274 条 GC 站点，仅名称文本长度合计 788,733,719 字节。marker 查询检查点只有两份共享表、6,648 个条目，名称文本 283,056 字节，并非主要来源。诊断计数来自生产解析结果；字符串长度不包含容量和分配器开销，不是精确堆占用。

[帧摘要合并](../src-tauri/src/parser/compact.rs)在每帧内按 marker 名归并 CPU，按线程和归因名称归并 GC，保留调用次数、总量和最大值。合并后 data B 为 1,736,763 条 CPU 摘要、57,814 条 GC 站点、66,724,984 字节名称文本；完整原始树、线程和逐样本 metadata 仍通过 FrameStore 查询，不删减帧或 top N 截断。

同一输入的峰值工作集下降约 90.3%，三次导入进程总耗时由 24.15 秒降为 16.97 秒（含释放及三次 500 ms 等待）。解析阶段新增合并工作，未表现出明显加速；主要收益是保留量、提取和释放开销。dump 仍保留完整类型化输入，峰值几乎不变，不能据此声称所有格式都大幅降低内存。优化前后报告分别保存为 `dump-a.json` / `data-b.json` 与 `dump-a-compacted.json` / `data-b-compacted.json`。

完整 Rust 回归 81 项通过；新增公开 fixture 验证合并前后完整快照相同、嵌套原始样本保持、线程不混并和溢出报错。两份私有录制的四组对照重新通过：137 帧、1,306,026 个原始样本，逐 marker 的 inclusive 总量/次数/最大值及逐 GC 站点字节/次数/最大值均核对。GC 与次数精确比较；汇总 CPU 使用 `1e-6 × max(1, 期望值)` 的浮点容差，原始逐样本对照保持。debug 验收耗时分别为 A 首段 29.34 秒、A 高峰 32.59 秒、A 末帧 21.15 秒、B 首段 82.92 秒，其中 B 与 release 编译部分重叠，不用于性能比较。

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --release --locked --example profile_benchmark
./tools/measure-profile.ps1 -InputPath '<录制或 dump 路径>' -Repeats 3 -OutputPath './.cache/benchmarks/result.json'
```

采样脚本要求 PowerShell 7；构建依赖已经缓存时可追加 `--offline`。文件不存在、空输入、生产解析/查询错误或查询源未释放都会失败，不计为成功测量。

## 构建与 CI

首次 `npm run tauri:build` 完成 release 编译，但 WiX 打包因配置仅列出 PNG 而找不到 ICO。已将仓库现有 `icons/icon.ico` 加入 bundle 配置；修正后 `npm run tauri:build` 成功生成两种未签名安装包。

[Windows 工作流](../.github/workflows/windows.yml)使用 Windows runner、Node 24 和 stable Rust，执行 npm ci、前端测试/构建、完整 Rust 测试、公开研究脚本测试、安装包构建与公开 fixture 性能冒烟。只上传生成的未签名安装包，不执行真实模型或读取私有录制。本地 YAML 解析通过；尚未在远端 CI 运行，不能标记为新环境构建成功。

### 本地构建结果

| 产物 | 字节 | SHA-256 |
|---|---:|---|
| `Unity Profiler Analysis Agent_0.1.0_x64_en-US.msi` | 2,613,248 | `2BE3F84B3EDA87152A736F6D625E8DE6060D9F3DE5AB6AA137607DD9FD1344C1` |
| `Unity Profiler Analysis Agent_0.1.0_x64-setup.exe` | 1,789,107 | `4F9CE0F4B3982D33AE55AA7467AF6E70F0CFB01941D482DACD83D75EB5B67670` |

本表为 2026-09-28 帧摘要合并、分页遮挡、MCP 查询警告与指标语义修正后重新运行 `npm run tauri:build` 的产物，位于 `src-tauri/target/release/bundle/`，不提交仓库。体积只描述本次安装包，不包含目标机器运行环境安装成本，也不作为单文件或跨平台体积承诺。

以 `UPAA_TEST_APP_EXE` 指向生成的 release EXE，执行 `mcp_wire` 和 `acp_stdio_roundtrip`：8 项进程测试通过、2 项真实模型测试按默认忽略。该次测试由 debug 测试宿主启动 **release 桥接子进程**，验证其无控制台模式下的协议、查询、退出和隔离；不是完整 release GUI/后端生命周期验收。CI 已纳入同一检查。

```powershell
$env:UPAA_TEST_APP_EXE = (Resolve-Path 'src-tauri/target/release/unity-profiler-analysis-agent.exe').Path
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test mcp_wire --test acp_stdio_roundtrip
```

### NSIS 安装与卸载验证（2026-09-28）

[安装回归脚本](../tools/test-windows-install.ps1)拒绝覆盖已有安装，使用当前用户模式静默安装到仓库 `.cache/installer-smoke/app`，关闭快捷方式创建。验证登记路径后，以安装目录中的 EXE 执行 8 项真实桥接进程测试，再调用该安装生成的卸载器。复验通过，安装程序 SHA-256 与上表 NSIS 一致，安装后的 EXE 为 `478571A4558BBDB3FC54A8B97EA9F92BA2EA8229F49C57661A145377D9435FB1`。日志与结果保存在 `.cache/installer-smoke/`，CI 已纳入此脚本，远端尚未执行。

卸载删除程序目录和当前用户的 Uninstall 登记；默认保留 `HKCU\Software\ray\Unity Profiler Analysis Agent` 中的安装路径偏好。首次严格残留检查因此失败，检查生成的 NSIS 脚本后确认它只在选择删除应用数据时清除此键。回归现明确记录保留项，仅在该键测试前不存在、值恰好等于本次目录且没有其他值/子键时，由测试清理它。未改变产品的数据保留策略，也不把测试清理算作卸载器行为。

```powershell
./tools/test-windows-install.ps1
```

NSIS 当前用户静默模式验收不覆盖安装向导、快捷方式或 MSI；桌面业务交互的独立复验见下节。生成的 MSI 为 perMachine，需要独立的具备管理员权限的环境；当前进程没有管理员权限，尚未安装 MSI。

### release WebView 与 IPC（2026-09-28）

[桌面冒烟脚本](../tools/desktop-smoke.py)通过 Tauri 官方 WebDriver 路径启动实际 release EXE，检查初始页面，再从真实 WebView 调用 upload → analyze → cpu_hierarchy → release_file。公开 fixture 的 2 个导出帧 / 20 个声明帧、CPU 12 ms、GC 32 B、缺失渲染 null、嵌套分配 20 B / 4 B，以及释放后不可查询均通过。启动截图已检查；结束后没有遗留应用或驱动进程。随后使用安装回归的 `-DesktopSmoke` 选项，在 NSIS 安装目录中的 EXE 上重复此检查并正常卸载，也通过。使用 tauri-driver 2.1.0、Edge WebDriver / WebView2 153.0.4234.48，驱动、浏览器数据目录与报告仅保存在忽略的 `.cache/`。

后续增加 `--ui`，在真实 release WebView 中点击导入、页面、帧/线程、深度及分页控件。仅在测试进程中替代原生文件选择返回值，其他 Tauri 请求和事件仍走真实后端，不修改产品入口。正常、零 GC、部分 GC、损坏结构后的恢复、211 节点分页均通过。发现 sticky 状态栏遮挡分页按钮，修复为正常文档流后重新构建并通过实际点击回归。NSIS 安装目录中的同版程序也通过 `-DesktopUi` 复验。

`python tools/desktop-smoke.py --ui --real-agent` 的 10 组检查全部通过：真实 Claude Code 首次诊断完成并调用 MCP；第二次在 MCP 活动期间取消，恢复就绪且无迟到正文；第三次再次完成诊断。此项使用公开 fixture，未发送私有录制，不证明模型结论总是正确。独立 release EXE SHA-256 为 `51BC551F592095508C73E440000DCF7231628107D41F838B5405CEB1D2FA6CE9`；NSIS 会修改 bundle 类型，安装后 EXE 哈希单独记录。报告与截图在 `.cache/desktop-ui/`，本次真实 Agent 报告备份为 `real-agent-result.json`。

原生 Windows 控制工具可以读取窗口，但截图和坐标输入失败，故原生文件选择框本身仍未验证。该限制不影响已完成的结果页、查询控件和诊断按钮验收。
```powershell
cargo install tauri-driver --version 2.1.0 --locked --root .cache/webdriver
# 从 Microsoft 官方下载与本机 WebView2 对应的 Edge Driver，解压到 .cache/webdriver/edge
python tools/desktop-smoke.py
# 准备公开输入，检查真实界面及 Agent
python tools/prepare-manual-fixtures.py
python tools/desktop-smoke.py --ui --real-agent
# 检查安装后的界面（不调用真实模型），再卸载
./tools/test-windows-install.ps1 -DesktopUi
```

可通过 `--application`、`--driver`、`--native-driver` 指定其他绝对路径。原生文件选择、MSI 管理员安装、远端 CI 和签名/发布责任尚未验收，不因此标记 MVP 完成。方法参考 [Tauri 手动 WebDriver 配置](https://v2.tauri.app/develop/tests/webdriver/manual-setup/)和 [Microsoft Edge Driver](https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/)。

后续发布门槛：


- MSI / NSIS 生成及哈希记录已完成；后续源代码变更需重新构建。
- 验证 release 可执行文件的 MCP stdio 模式及诊断进程清理。
- NSIS 静默安装/卸载与残留已记录；MSI、安装向导、快捷方式和原生文件选择框仍需验收。
- 明确签名、应用标识、许可证文件及发布责任；未签名本地构建不等于公开发布。
- 完整应用峰值内存、并发查询和缓存释放预算；更多版本与平台分别验收。

参考：[Tauri Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)、[GitHub checkout](https://github.com/actions/checkout)、[setup-node](https://github.com/actions/setup-node)、[upload-artifact](https://github.com/actions/upload-artifact)。

### 查询警告修正后的 release 复验（2026-09-28）

本段是查询警告修正时的历史验证；上方安装包表与 NSIS 安装后哈希以末节最新构建为准。`npm run tauri:build` 通过，release 编译约 1 分 50 秒。`python tools/desktop-smoke.py --ui --real-agent` 的 10 组检查及 `./tools/test-windows-install.ps1 -DesktopUi` 均通过，后者包含安装后 8 项协议回归、界面与卸载。原生文件选择仍只替代返回值，未扩大原生窗口覆盖范围。

本次独立 EXE SHA-256 为 `52F8BA7A1D3F1A785A5B6319E0F6F698E6B047A82E86B980BCFD3ECF154081CB`；真实 Agent 报告与两次完整正文保存在 `.cache/desktop-ui/query-warning-real-agent-result.json`，此前报告仍作为历史证据保留。两次回答均明确包含嵌套 4 B、主线程 24 B、Worker 8 B 与帧 32 B 的核对。

人工阅读正文仍发现解释错误：首次回答称约 10 ms CPU 未解释，缺乏正确的直接子样本依据；另一次将 p50=32 与帧值 [32,0] 称为口径差异，但当前统计实现是排序后取 `round((n-1)*q)` 索引，两个值的 p50 正是 32。以上记录为诊断内容质量未通过的证据，不能用协议成功或两次 GC 正确代替全面准确性验收。

### 指标语义补充后的复验（2026-09-28）

MCP 摘要增加 metricSemantics，明确有效帧统计、分位数取索引算法、inclusive CPU 和未提供 self 时间的限制，详见[集成说明](acp-mcp-integration.md#统计解释约束2026-09-28)。统计数值和 Tauri 快照不变。

完整 Rust 81 项通过（11 项环境测试默认忽略），MSI/NSIS 重新构建成功，release 界面/真实 Agent 的 10 组检查通过，安装后界面与 8 项协议回归及卸载通过。独立 EXE SHA-256 为 `E3C4B6538FA47E456F73B852B3E5D0801386834F496A0D8422FB8BB5A3D9EED4`；上方产物表为本次构建，报告与两次正文在 `.cache/desktop-ui/metric-semantics-real-agent-result.json`。

逐项复核两次正文：均正确保留 2/20 帧范围、CPU 12 ms、帧 GC 32/0 B、主线程 20+4=24 B 加 Worker 8 B；不再将 p50=32 视为冲突，也未推断未知 self/剩余 CPU。渲染缺失未作确定性判断。本次限定样本内容检查通过；只有同一合成输入的两次回答，不证明更大数据、更多指标、其他模型或未来调用总是准确。此前错误正文保留为历史证据。

### 实际桌面进程树的 IPC 内存基线（2026-09-28）

[桌面测量模块](../tools/desktop_memory.py)通过 `desktop-smoke.py --measure-input` 启动现有 release 程序，在真实 WebView 内调用 upload/analyze，保留快照直到首末帧的两项并发有界查询完成，然后 release_file 并断言后续查询被拒绝。每个输入在同一进程重复三次，每次释放后观察 2 秒，不强制 JavaScript GC。未调用 Agent，私有输入与采样报告只在忽略目录 `.cache/desktop-memory/`。

psutil 5.9.0 按应用 EXE 精确定位由 driver 启动的唯一进程，采集该进程及当时所有后代（本次包括 WebView2 和 conhost）。每轮采样后等待 50 ms，进程枚举本身还有开销，因此实际采样间隔不严格等于 50 ms。驱动与 Python 测量进程排除；各进程工作集相加可能重复计算共享页，采样可能漏掉短暂峰值。哈希计算及启动前的公开 IPC 冒烟会预热环境，未清理 OS 文件缓存。硬件与本页前述环境一致，使用“指标语义补充”节同一个 release EXE。

| 输入 | 帧数 / 输入字节 | 导入分析含 IPC | 两项并发查询合计 | 进程树采样峰值工作集 / 私有内存 | 第三次释放后工作集 / 私有内存 |
|---|---|---|---|---|---|
| 公开 fixture | 2 / 3,397 | 1.6–2.9 ms | 0.3–0.7 ms | 461.14 / 285.58 MiB | 461.14 / 285.58 MiB |
| data B | 2,000 / 1,162,295,388 | 5,135–5,269 ms | 8.0–8.9 ms | 667.13 / 500.46 MiB | 472.51 / 304.81 MiB |
| dump A | 64 / 146,127,279 | 393–409 ms | 4.3–4.7 ms | 638.39 / 477.37 MiB | 464.64 / 297.95 MiB |

data B 查询帧 0/1999，dump A 查询帧 0/63，各返回 200 个样本，limit=200、maxDepth=8；不是完整树遍历。data B 导入前基线约 453.02 / 286.80 MiB，三次释放后私有内存分别为 299.55、303.45、304.81 MiB，仍略有增长。dump A 对应基线 453.38 / 287.05 MiB，释放后私有内存 292.08、298.38、297.95 MiB。不能据此声称已经证明无泄漏或长期稳定上限。

报告为 `public-result.json`、`data-b-result.json`、`dump-a-result.json`，完整进程采样分别为同名前缀的 `*-samples.json`。公开样本很快，首轮导入阶段可能没有采样点，未将其内存峰值当作精确测量。

```powershell
# 需要 psutil；本机验证版本为 5.9.0，以及前述两个 WebDriver
python tools/desktop-smoke.py --measure-input '<本地录制或 dump 绝对路径>'
```

该模式禁止与 `--ui` 或 `--real-agent` 联用。它测量实际桌面进程与真实 IPC，但结果快照由测试脚本持有，**不包含 React 结果页渲染、UI 状态保留或 Agent 进程**，不能作为完整交互工作负载的最终预算。应用源代码未变，安装包不需重建；下一步仍需覆盖结果页保留、长时间重复导入及更高查询并发后再设内存预算。

### 结果页保留与重置的测量方法（2026-09-28）

在同一测量入口追加 `--measure-rendered`，使用真实 WebDriver 点击文件区、概览、CPU、末帧、GC 与重置；只替代原生文件选择框返回值，不替代 upload/analyze、查询或 React 状态。每轮在概览、调用树和 GC 页各保留 2 秒，重置后观察 2 秒，不强制 GC。`--measure-repeats` 可指定 1–30 轮，默认 3；与 `--real-agent` 互斥，不发送输入给模型。

```powershell
python tools/desktop-smoke.py --measure-input '<本地录制路径>' --measure-rendered --measure-repeats 10
```

本模式导入时长包括自动化点击、轮询等待、渲染及控件操作，不能与独立解析耗时直接比较。查询时长同样包含切页与 WebDriver 往返。线程、采样方法及工作集合计的局限沿用上节；重置回初始 UI 证明交互状态恢复，不单凭内存回落推断所有资源均已释放。

#### 结果页测量结果

同一 release EXE（指标语义补充版）、硬件与 WebView 环境下，公开 fixture 三轮、data B 十轮、dump A 三轮全部完成页面操作和重置。每轮 CPU 页选择首帧后切末帧，data B 为 0/1999、dump A 为 0/63，末帧返回 200 行。保留概览、调用树和 GC 页各 2 秒；本轮没有调用 Agent，截图和数据仅保存在 `.cache/`。

| 输入 / 轮数 | 导入至概览就绪（含自动化） | CPU 页及末帧查询（含自动化） | 采样峰值工作集 / 私有内存 | 最后一轮重置后工作集 / 私有内存 |
|---|---|---|---|---|
| 公开 fixture / 3 | 94–156 ms | 140–219 ms | 502.79 / 327.25 MiB | 502.58 / 326.95 MiB |
| data B / 10 | 5,297–5,562 ms | 109–234 ms | 837.80 / 647.59 MiB | 643.26 / 455.78 MiB |
| dump A / 3 | 547–672 ms | 109–235 ms | 702.20 / 541.75 MiB | 551.38 / 388.55 MiB |

data B 每轮重置后的私有内存依次为 372.68、422.71、396.71、416.48、400.52、410.88、424.47、431.37、446.81、455.78 MiB。虽然存在回落，后半段仍持续增长，不能把本次操作通过计为长期稳定性通过。从原始进程采样看，主要增长来自一个 WebView2 子进程：首轮重置后 98.93 MiB、第十轮 170.59 MiB；Rust 主进程对应 43.96→52.72 MiB。仅凭进程名未确认该 WebView2 子进程角色，也未区分浏览器缓存、延迟 GC 与应用引用泄漏；下一步需要堆/DOM 与自然空闲回收证据后再决定修复。

报告和逐进程采样分别保存在 `rendered-public-result.json` / `rendered-public-samples.json`、`rendered-data-b-result.json` / `rendered-data-b-samples.json`、`rendered-dump-a-result.json` / `rendered-dump-a-samples.json`。此前 IPC 模式报告保留独立文件，不能混用其峰值。`--measure-rendered` 覆盖结果页状态与渲染，仍不覆盖真实原生选择框、Agent、任意并发和长时间运行。

### 堆与 DOM 回收诊断方法（2026-09-28）

在 rendered 测量命令上追加 `--measure-heap`，通过当前 WebView 的 DevTools 通道读取 [Runtime.getHeapUsage](https://chromedevtools.github.io/devtools-protocol/tot/Runtime/#method-getHeapUsage) 与 [Memory.getDOMCounters](https://chromedevtools.github.io/devtools-protocol/tot/Memory/#method-getDOMCounters)，另记录文档中 attachedElements。JavaScript heap 是对应 isolate 的统计，不等于整个 WebView2 私有内存；DOM counters 也不等同于当前文档内可见元素数量。

每轮重置后只读取计数，十轮结束后自然空闲 30 秒再读取；最后单独使用 [HeapProfiler.collectGarbage](https://chromedevtools.github.io/devtools-protocol/tot/HeapProfiler/#method-collectGarbage)，等待 2 秒取诊断对照。前面的自然负载未强制回收，最终 GC 阶段单独标记，不能作为用户正常运行的验收结果。该模式显式失败时不回退为缺失计数或伪造成功。

```powershell
python tools/desktop-smoke.py --measure-input '<本地录制路径>' --measure-rendered --measure-repeats 10 --measure-heap
```

#### 十轮堆/DOM 观察

data B 十轮 rendered 负载再次完成。每轮重置后 attachedElements 恒为 33，但 DevTools DOM nodes 从首轮 22,097 增至第十轮 170,853，JavaScript used heap 从 12.64 MiB 增至 45.18 MiB。自然空闲 30 秒后，DOM nodes 为 165,759、JS used heap 为 40.64 MiB；单独诊断 GC 后 JS used heap 降至 10.24 MiB，但 DOM nodes 仍为 165,759。

这排除了“所有增长只需一次 JavaScript GC 就会消失”的解释，但尚不能区分应用引用、原生控件行为与 WebDriver 元素句柄保留；不能直接将其计为应用泄漏。报告为 `.cache/desktop-memory/heap-data-b-result.json` 与 `heap-data-b-samples.json`。公开样本的一轮接口验证为 `heap-public-*`。

新增 `--measure-dom-control` 对照参数（要求 `--measure-heap`）：使用页面内 DOM click/change 触发同一 React 导入、查询和重置流程，不获取 WebDriver 元素句柄。此模式仅用于定位测量干扰，不能替代真实用户控件可点击性验收；其报告显式包含 domEventControl=true。

#### DOM 事件对照结果

data B 三轮 `--measure-dom-control` 通过。重置后 DOM nodes 依次为 22,094、21,469、17,007；自然空闲 30 秒后降至 89（文档附着元素仍为 33），JS used heap 约 12.49 MiB；诊断 GC 后 DOM 仍为 89，JS used heap 约 2.45 MiB。与真实 WebDriver 元素点击模式前三轮 DOM 22,097→33,596→50,163 的增长模式明显不同。

该结果说明控件自动化路径会影响 DOM 保留，不能把早先 WebDriver 私有内存增长直接认定为业务泄漏。对照同时改变了元素句柄使用和控件事件触发方式，**尚未孤立证明具体是 WebDriver 句柄缓存还是原生控件路径**；三轮也不能外推长期无泄漏。应用源码未因该测量被盲目修改。记录为 `.cache/desktop-memory/heap-dom-control-result.json` / `heap-dom-control-samples.json`。

后续内存验收需区分实际控件体验测试与资源保留对照，扩大相同轮数对照并定位引用来源，再设定产品预算。已有真实 WebDriver 的分页/线程切换操作验收仍有效；资源稳定性结论继续保留边界。

### WebDriver 元素句柄对照（2026-09-28）

在 DOM 事件对照上增加 `--measure-handle-control`，只在同一事件触发前额外执行 WebDriver 元素查找并立即丢弃 Python 中的返回值，不调用 WebDriver click。输入、三轮数、页面事件、自然空闲与诊断 GC 均保持一致。

只增加查找后，自然空闲及诊断 GC 后 DOM nodes 均为 49,791；此前不查找为 89。诊断 GC 后 JS used heap 为 4.93 MiB。该对照支持“WebDriver 元素注册/句柄使用使这些节点在测试会话中被保留”，而非必须由原生 click 或 React 业务流程导致；没有修改或清理浏览器内部缓存来掩盖现象。报告为 `heap-handle-control-result.json` / `heap-handle-control-samples.json`。

真实控件可点击性继续由 WebDriver 检查；资源回收测量使用无元素句柄的 DOM 事件对照，并明确其不覆盖原生点击路径。不能将旧 WebDriver 会话的 DOM 线性增长直接作为产品泄漏或正常用户内存预算依据。

#### 同轮数确认：十轮无句柄对照

data B 十轮 DOM 事件对照完成相同的结果页保留和重置。重置后 DOM nodes 在约 18,000–22,300 之间波动，没有句柄模式中的逐轮累积；自然空闲 30 秒后降至 89，监听器为 140，文档附着元素仍为 33。自然空闲后 JS used heap 约 32.06 MiB，诊断 GC 后约 2.68 MiB（基线约 2.07 MiB），DOM 仍为 89。与三轮无句柄及三轮增加句柄的实验一起，支持早先 DOM 累积是该 WebDriver 元素注册路径带来的测量干扰。

本次进程树采样峰值工作集/私有内存为 761.93/587.14 MiB；第十轮重置后的私有内存约 397.16 MiB，自然空闲后 381.32 MiB、诊断 GC 后 351.77 MiB，启动基线 287.22 MiB。进程内存仍未完全回到基线，不将诊断 GC 当成正常释放策略，也不将一次十轮运行视为长期无泄漏或严格容量保证。

报告及采样为 `.cache/desktop-memory/heap-dom-control-10-result.json` / `heap-dom-control-10-samples.json`。应用源代码和安装包未改；本次交付是可复现的测量对照与归因证据。实际原生点击、Agent 进程占用、长期运行及更多录制仍各有独立验证边界。

### 远端 CI 与 MSI 后续验收

`05c19dc` 的[Windows PR 工作流](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/actions/runs/36397645450)全部成功，并生成 `windows-unsigned-validation`（4,117,992 字节，包含未签名 MSI/NSIS）。这项记录更新了上文历史“远端尚未执行”的状态；新 runner 上已完成 NSIS 静默安装、安装后协议及卸载。

新增[MSI 回归脚本](../tools/test-windows-msi.ps1)，通过 Windows Installer 数据库只读取得产品标识，检查无已有相关产品/目录/登记后，在已具备管理员权限的环境中执行 per-machine 安装、确认 HKLM 安装路径、对安装后的 EXE 运行 8 项协议回归，再卸载并验证目录及登记清除。不覆盖版本升级、交互向导或 GUI。命令依据 [Microsoft msiexec 文档](https://learn.microsoft.com/windows-server/administration/windows-commands/msiexec)。

```powershell
./tools/test-windows-msi.ps1 -InspectOnly # 无需管理员，仅核对包信息
./tools/test-windows-msi.ps1              # 必须已是管理员；不会请求 UAC
```

本机 InspectOnly 通过；后续 `3cd15e8` 的远端安装步骤也已通过，详见下节。失败不会被当作跳过。安装/卸载日志与结果在 `.cache/msi-smoke/`，CI 的 `windows-installation-evidence` 同时保存 NSIS 和 MSI 日志，失败时也尝试上传。

### MSI 与 NSIS 远端安装证据（2026-09-28）

`3cd15e8` 的 [push 工作流](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/actions/runs/36400361838) 和 [PR 工作流](https://github.com/amuroray742-png/UnityProfilerAnalysisAgent/actions/runs/36400633656) 均成功。下载 push 运行的 `windows-installation-evidence` 后，核对结果如下：

| 安装方式 | 安装 / 协议 / 卸载 | 残留边界 |
|---|---|---|
| MSI per-machine | 三项均为 true；8 项协议测试通过，2 项真实 Agent 测试忽略 | 安装目录和产品登记已清除，vendorRegistryRetained=false |
| NSIS current-user | 三项均为 true；8 项协议测试通过 | 保留的安装路径偏好由脚本清理；CI GUI 字段为 null |

MSI 安装包 SHA-256 为 `07D4F650ECD0D1EFAA32B5E6354CC33D1234CDEB0FE0C986D17ECF0C38422B56`，安装后 EXE 为 `CC9F1846422CB6E8DA6EE8363657919934A8B5B51CEA4866726F1180D8AB3D6F`。ProductCode 为 `{FED84D99-2DFB-4D1E-AC1E-D5EF489CF2D8}`。MSI 的 install.log、uninstall.log、protocol.log 与 result.json 已核对；本地证据下载目录为 `.cache/msi-ci-evidence/`，不提交日志原件。

NSIS 安装包 SHA-256 为 `DA8C6F1B71CA73C2518A7290C1A4FEF9B1D038EA69681768B7BF341763355751`。此 CI 不证明 GUI 或版本升级；基线 CI NSIS 的本机安装后界面复验与本次静默安装分别记录。Artifact 按工作流保留 7 天，哈希及运行链接在此长期记录，不将临时 artifact 链接当成正式分发渠道。
