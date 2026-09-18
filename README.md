# Unity Profiler Analysis Agent

> 通过 **ACP Client + MCP Server** 接入 Claude Code / Gemini CLI 等 Agent，自动诊断 Unity 游戏性能瓶颈。
>
> 架构参考 [`librashuai/UnityPerfAgent`](https://github.com/librashuai/UnityPerfAgent)。
>
> **目标 Unity 版本：Unity 6000.3**（Unity 6 系列）；也兼容 Unity 2022 LTS 导出。

## 是什么

一个本地桌面工具（Tauri 2 + React + Rust）：

1. **拖入** Unity Profiler 文件（`.pd3u` / `.data` / `.json` / `.raw`）
2. **解析** 出 CPU / GC / 渲染核心指标
3. **AI 诊断**：App 作为 ACP Client 启动 Claude Code / Gemini CLI 等 ACP 兼容 Agent，Agent 通过内置 MCP Server 按需查询 Profiler 数据，给出诊断与可执行建议
4. 浏览器内**流式**渲染诊断文本 + Agent Log 抽屉

## 特性

- ✅ 4 种 Profiler 格式：JSON（主）、PD3U（PlayerConnection 流）、`.data`（Unity 2022+ / Unity 6）、`.raw`（尽力）
- ✅ **`.data` 真实端到端解析**（流式，GB 级不 OOM）
  - **Unity 2022.3**：完整 port `librashuai/UnityPerfAgent` 的 `internal/capture/capture.go`：block header、frame header、Stats block、AllProfilerStats、marker 表、主线程采样树、GC.Alloc metadata strict-match、34-record Memory counter signature scan + 9 个 invariant
  - **Unity 6000.3**：block iteration + frame header + memory counter 扫描 + marker 表新格式（实测验证 Unity 6000.3.23f1 文件）
  - 已知 Unity 6 限制：Stats / 主线程采样树 / GC.Alloc metadata 布局待进一步逆向
- ✅ 3 个分析维度：CPU 帧时间、GC 分配、Draw Call / SetPass
- ✅ AI 推理交给用户配置的 ACP 兼容 Agent（Claude Code / Gemini CLI / Codex CLI 等）
- ✅ 通过 MCP Server 让 Agent 按需查询数据，多轮交互
- ✅ 单文件二进制，~5–10MB，Windows / macOS / Linux

## 架构

```
React + TypeScript WebView (Tauri)
            │
            ▼
Rust Backend (Tauri)
  ├─ parser/    pd3u · data · json · raw
  ├─ extractor/ cpu · gc · rendering
  ├─ mcp/       内置 stdio MCP Server（暴露 performance_* 工具）
  └─ acp_client/ 启动 ACP Agent + 注入 MCP Server + 流式回传
            │
            ▼
   ACP 兼容 Agent 子进程
   (Claude Code ACP / Gemini CLI / Codex CLI ...)
```

## 前置条件

1. **Node.js >= 20** + npm
2. **Rust stable**（[rustup](https://rustup.rs)）
3. Windows: **WebView2**（Win11 自带，Win10 需手动装）
4. **ACP 兼容 Agent**（至少装一个）：
   - [Claude Code ACP](https://docs.anthropic.com/en/docs/claude-code) — `npm i -g @anthropic-ai/claude-code` + 启用 ACP
   - [Gemini CLI](https://github.com/google-gemini/gemini-cli) — `gemini --experimental-acp`
   - Codex CLI — `npm i -g @openai/codex`

## 开发

```bash
# 1. 装前端依赖
npm install

# 2. 开发模式（自动启动 Tauri + Vite 热重载）
npm run tauri:dev

# 3. 生产构建（产出 Windows 安装包）
npm run tauri:build
```

### 仅跑测试

```bash
# 前端
npm run test

# Rust
cd src-tauri && cargo test
```

## 文件格式支持

| 扩展名 | 来源 | 支持度 |
|---|---|---|
| `.json` | Unity Profiler Export JSON（Unity 6000.3 / 2022 LTS） | ✅ 完整，含 Unity 6 GC.Alloc metadata 字节提取 |
| `.pd3u` | PlayerConnection Data Unity（实时流） | ⚠️ MVP：仅识别 magic，详细解析待补 |
| `.data` | Unity 2022.3 Save to file | ✅ 完整 port `librashuai/UnityPerfAgent`：block 迭代、frame header、Stats、marker 表、主线程采样树、GC.Alloc metadata、Memory counter 34-record signature |
| `.data` | Unity 6000.3 Save to file | ⚠️ 部分：block 迭代 + frame header + marker 表（新格式）+ Memory counter scan。Stats block / 主线程采样树 / GC.Alloc metadata 待逆向 |
| `.raw` | 老版本 Unity Profiler 二进制 | ⚠️ 仅识别 magic + 版本估算 |

> `.data` 推荐工作流：Unity Editor → Window → Analysis → Profiler → 录制 → 三点菜单 → **Save to file** → 选本工具打开。

## Unity 6000.3 特有支持

针对 Unity 6 系列，解析器与诊断器额外支持：

- **Marker 分类**：`PlayerLoop` / `BehaviourUpdate` / `FixedBehaviourUpdate` / `GC.Alloc` / `RenderGraph.*` / `Camera.Render` / `Gfx.WaitForPresentOnGfxThread` / `Physics.Simulate` / `JobHandle.Complete` 等按类别归类（Scripting / Memory / Rendering / RenderGraph / GfxWait / Physics / Jobs / Editor / Other）
- **GC.Alloc 字节数提取**：Unity 6 在 marker metadata 中携带分配字节数（Int64），解析时自动覆盖默认 total_ms 字段值
- **ProfilerCategory 字段**：Unity 6 新增 `category` 字段（解析时保留为元数据，未透传到前端）
- **RenderGraph markers**：Unity 6 SRP 引入，识别 `RenderGraph.Compile` / `RenderGraph.Execute` / `RenderGraph.Dispatch` 等

## 使用流程

1. 启动应用：`npm run tauri:dev`
2. 选择 Profiler 文件
3. 自动解析并展示指标卡
4. 在右上角选择 ACP Agent（需要至少装一个）
5. 点击"开始 AI 诊断"
6. Agent 通过 MCP 工具按需查询数据
7. 流式渲染诊断文本 + Agent Log 抽屉可见完整 MCP 请求 / 响应

## 在编辑器里使用（高级）

我们内置的 MCP Server 也能被 Zed / JetBrains / Cursor 等 ACP 兼容编辑器直接调用。在 `~/.config/zed/settings.json` 中添加：

```json
{
  "agent_servers": {
    "UnityProfilerAnalysis": {
      "command": "npm",
      "args": ["run", "--prefix", "PATH_TO_THIS_REPO", "mcp:serve"]
    }
  }
}
```

详见 `docs/acp-mcp-integration.md`。

## 技术栈

- **桌面壳**：[Tauri v2](https://tauri.app/)
- **前端**：React 18 + TypeScript + Vite
- **后端**：Rust 1.75+
- **MCP**：[`rmcp`](https://crates.io/crates/rmcp) 0.5（官方）
- **ACP**：[`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) 0.4（官方）

## 风险与限制

- **ACP SDK v0.4.1 早期版本**：协议变更或 API 不完整，可能需要 fork 修复
- **`.pd3u` 解析 MVP 级**：实时流详细解析待补
- **Unity 6000.x `.data` 部分字段待逆向**：Stats block / 主线程采样树 / GC.Alloc metadata 布局与 Unity 2022.3 不同，需要进一步逆向 UnityCsReference Unity 6 源码
- **Agent 必须本机安装**：应用不会替你下载 Agent
- **MVP 仅 CPU / GC / 渲染**：GPU 帧时间、纹理 / Mesh 内存、Addressables 等留待后续

## License

MIT

## 致谢

- [`librashuai/UnityPerfAgent`](https://github.com/librashuai/UnityPerfAgent) — 架构灵感
- [Anthropic MCP](https://modelcontextprotocol.io/) — MCP 协议
- [Agent Client Protocol](https://agentclientprotocol.com/) — Zed / Google ACP 协议