# ACP / MCP 集成指南

本应用同时是 **ACP Client**（启动 AI Agent）和 **MCP Server**（暴露 Profiler 数据工具）。
本指南说明如何把这两种能力集成到编辑器（Zed / JetBrains / Cursor 等）。

## 在 Zed 里使用

`~/.config/zed/settings.json`：

```json
{
  "agent_servers": {
    "UnityProfilerAnalysis": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "PATH_TO_THIS_REPO/src-tauri/Cargo.toml", "--bin", "unity-profiler-mcp"],
      "env": {}
    }
  }
}
```

> 注意：上面的 `unity-profiler-mcp` 二进制需要在 `Cargo.toml` 里额外声明 `[[bin]]`。
> 临时方案：通过 `cargo run --bin unity-profiler-mcp` 直接启动 MCP server。

## 在 JetBrains 里使用

JetBrains 2024.3+ 支持 ACP。在 Settings → Tools → Agent Client Protocol 中添加 Agent：

- Command: `cargo`
- Args: `["run", "--manifest-path", "..."]`

## 在 Cursor / VSCode 里使用

通过 [`use-acp`](https://www.npmjs.com/package/use-acp) React Hooks 接入，或在 settings.json 中配置 MCP：

```json
{
  "mcpServers": {
    "unity-profiler": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "..."]
    }
  }
}
```

## MCP 工具集

应用内置的 MCP Server 暴露 5 个工具：

| 工具 | 用途 | 参数 |
|---|---|---|
| `performance_session_summary` | 会话摘要 | 无 |
| `performance_frames` | 帧范围查询 | `start: int, limit: int (≤500)` |
| `performance_frame` | 单帧详情 | `frame_index: int` |
| `performance_cpu_hierarchy` | CPU 调用层级 | `frame_index: int, max_depth: int` |
| `performance_analysis` | 综合分析 | `focus: 'cpu' \| 'gc' \| 'rendering' \| 'all'` |

## 推荐 ACP 兼容 Agent

| Agent | 安装 | 启动命令 |
|---|---|---|
| Claude Code ACP | `npm i -g @anthropic-ai/claude-code` | `claude-code-acp` |
| Gemini CLI | `npm i -g @google/gemini-cli` | `gemini --experimental-acp` |
| Codex CLI | `npm i -g @openai/codex` | `codex-acp` |

## 故障排查

### Agent 未被检测到

确认命令在 PATH 中：

```bash
which claude-code-acp  # 或 claude-code-acp.cmd on Windows
```

应用启动时会通过 `which` 检测，如果检测失败会显示"(未安装)"。

### Agent 启动但 MCP 工具未出现

检查 Agent 是否支持 `session/new.mcpServers` 注入（MCP 2025-11-25 spec）。

### 流式输出卡住

查看菜单 → Tools → Toggle DevTools，看 Console 是否有 Tauri IPC 错误。