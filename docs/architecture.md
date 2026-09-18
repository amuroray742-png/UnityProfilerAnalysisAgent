# 架构与设计要点

## 整体流程

```
[用户拖文件] → [前端 invoke('upload')] → [Rust commands::upload]
                                                    ↓
                                          [落盘 uploads/<uuid>.<ext>]
                                                    ↓
[前端 invoke('analyze')] → [Rust commands::analyze]
                                ↓
                  [parser::parse_file (按扩展名分派)]
                                ↓
                  [extractor::extract (CPU/GC/渲染)]
                                ↓
              [MetricsSnapshot 缓存到 AppState]
                                ↓
[用户选 Agent + 点 "AI 诊断"] → [前端 invoke('diagnose')]
                                ↓
                  [acp_client::client::spawn_agent]
                                ↓
       [Agent 子进程 stdio] ⇄ [ClientSideConnection]
                                ↓
                    [run_session: initialize → newSession]
                                ↓
                  [newSession 注入 MCP Server]
                                ↓
              [Agent 通过 stdio 调 performance_* 工具]
                                ↓
                [MCP 工具从 MetricsStore 读数据]
                                ↓
            [Agent 流式输出] → [sessionUpdate] → [stream_relay]
                                ↓
                       [app.emit('diagnose-event')]
                                ↓
                      [前端 listen + 渲染]
```

## 关键设计取舍

### 1. 为什么把 metrics 缓存到内存而非 SQLite？

- MVP 阶段简单优先，in-memory HashMap 足够
- 后续如需跨重启保留，加 SQLite + FTS 索引

### 2. 为什么 MCP Server 通过 stdio 注入而非 HTTP？

- stdio 是 ACP 规范推荐的 MCP 注入方式（`mcpServers: [{ type: 'stdio', ... }]`）
- 零网络配置，127.0.0.1 安全保证自动满足
- 简化部署：不需要端口管理

### 3. 为什么用 rmcp 而不是手写 MCP 协议？

- rmcp 是官方维护的 Rust MCP SDK
- 支持 server / client / transport-io / streamable-http 全套
- 通过 `#[tool]` 宏声明工具，类型安全

### 4. ACP SDK v0.4.1 早期风险如何应对？

- 在 `acp_client/` 模块隔离所有协议相关代码
- 关键路径（initialize / newSession / prompt）都加注释，方便升级时定位
- 准备好 fork：fork 后的 `agent-client-protocol` 可作为 git 依赖加到 Cargo.toml

### 5. 解析器分层（parser → extractor → snapshot）

- **parser**：负责文件格式解析（pd3u / data / json / raw），输出 `ParsedProfile`
- **extractor**：把 `ParsedProfile` 转成 `MetricsSnapshot`，专注于指标聚合
- **snapshot**：前端 / MCP 共享的数据结构，size 受控（~20-50KB）

这样分层的好处：
- 改一个 parser 不影响 extractor
- extractor 算法升级不影响 parser
- snapshot schema 是前后端契约，可独立演进

## 关键文件位置

| 文件 | 作用 |
|---|---|
| `src-tauri/src/lib.rs` | Tauri 入口，注册 commands + plugins |
| `src-tauri/src/commands/mod.rs` | 5 个 Tauri commands |
| `src-tauri/src/parser/mod.rs` | 解析器分派 + 通用类型 |
| `src-tauri/src/parser/json.rs` | JSON 格式（主路径） |
| `src-tauri/src/parser/raw.rs` | .raw 尽力解析 |
| `src-tauri/src/extractor/cpu.rs` | CPU 指标聚合 |
| `src-tauri/src/mcp/mod.rs` | MCP Server 注册 + 信息 |
| `src-tauri/src/mcp/transport.rs` | 5 个工具实现 |
| `src-tauri/src/acp_client/mod.rs` | ACP Client 主入口 |
| `src-tauri/src/acp_client/session.rs` | initialize / newSession / prompt |
| `src/App.tsx` | 前端主组件 |
| `src/hooks/useDiagnose.ts` | 诊断流程状态管理 |

## 性能预算

- JSON 解析 50MB：< 5s
- JSON 解析 100MB：< 15s（用 tokio::task::spawn_blocking 避免阻塞）
- MCP 工具调用：< 100ms / 次
- AI 首 token（API 后端）：< 8s
- Tauri WebView 启动：< 1s

## 安全考虑

- 前端通过 CSP 限制只能访问自身资源
- Rust 进程只暴露 Tauri commands，不开外部 HTTP
- 上传文件大小限制 500MB（前端 invoke 前检查）
- MCP Server 仅 stdio，不暴露网络接口
- Agent 子进程用 `kill_on_drop(true)` 防止泄漏
- 取消机制通过 oneshot channel + child.kill() 双重保证

## 后续扩展

- GPU 帧时间
- 纹理 / Mesh 内存明细
- Addressables / Resources.Load 分析
- 跨录制自动归因（基线对比）
- Profiler 历史快照
- Unity Editor 实时连接（通过 PlayerConnection TCP）
- CI 集成（GitHub Action 出报告）