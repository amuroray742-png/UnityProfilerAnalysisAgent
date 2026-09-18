// Tauri IPC 桥：包装 invoke + listen，提供类型安全的前后端调用

import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type {
  UploadResult,
  MetricsSnapshot,
  AgentPreset,
  DiagnoseEvent,
} from '../types/index.ts';

/**
 * 上传 Profiler 文件。
 * 注意 Tauri v2 中文件读取走 dialog/fs 插件，传 path 给后端。
 */
export async function uploadProfiler(filePath: string): Promise<UploadResult> {
  return invoke<UploadResult>('upload', { filePath });
}

/**
 * 触发解析与指标提取。
 */
export async function analyzeProfiler(fileId: string): Promise<MetricsSnapshot> {
  return invoke<MetricsSnapshot>('analyze', { fileId });
}

/**
 * 启动 AI 诊断。返回一次性 Promise resolve 时表示"已启动开始流式输出"。
 * 流式内容通过 listen('diagnose-event', ...) 接收。
 */
export async function diagnose(
  fileId: string,
  agentId: string
): Promise<{ sessionId: string }> {
  return invoke<{ sessionId: string }>('diagnose', { fileId, agentId });
}

/**
 * 取消正在进行的 AI 诊断。
 */
export async function cancelDiagnose(sessionId: string): Promise<void> {
  return invoke<void>('cancel_diagnose', { sessionId });
}

/**
 * 列出可用 Agent 预设。
 */
export async function listAgents(): Promise<AgentPreset[]> {
  return invoke<AgentPreset[]>('list_agents');
}

/**
 * 监听 AI 诊断事件流。返回取消监听的函数。
 */
export async function onDiagnoseEvent(
  handler: (event: DiagnoseEvent) => void
): Promise<UnlistenFn> {
  return listen<DiagnoseEvent>('diagnose-event', (e) => handler(e.payload));
}

/**
 * 监听解析警告。
 */
export async function onParseWarning(handler: (warning: string) => void): Promise<UnlistenFn> {
  return listen<string>('parse-warning', (e) => handler(e.payload));
}