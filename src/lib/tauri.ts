// Tauri IPC 桥：包装 invoke + listen，提供类型安全的前后端调用

import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type {
  UploadResult,
  MetricsSnapshot,
  AgentPreset,
  DiagnoseEvent,
  FramePage,
  HierarchyPage,
} from '../types/index.ts';

export function getFrameDetails(fileId: string, frameIndex: number, start = 0, limit = 128): Promise<FramePage> {
  return invoke('frame_details', { fileId, frameIndex, start, limit });
}
export function getCpuHierarchy(fileId: string, frameIndex: number, threadIndex: number | null, start = 0, limit = 200, maxDepth = 8): Promise<HierarchyPage> {
  return invoke('cpu_hierarchy', { fileId, frameIndex, threadIndex, start, limit, maxDepth });
}
export function releaseProfiler(fileId: string): Promise<void> {
  return invoke('release_file', { fileId });
}

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
export const listReports = (fileId: string) => invoke<import('../types').DiagnosisReport[]>('list_reports', { fileId });
export const prepareSource = (fileId: string, root: string) => invoke<import('../types').SourceInfo>('prepare_source', { fileId, root });
export const cancelSourcePreparation = (fileId: string) => invoke<void>('cancel_source_preparation', { fileId });
export const diagnoseSource = (fileId: string, agentId: string, parentReportId: string, scopeId: string) => invoke<{ sessionId: string }>('diagnose_source', { fileId, agentId, parentReportId, scopeId });
export const exportReports = (fileId: string, reportIds: string[], format: 'markdown' | 'html', path: string) => invoke<void>('export_reports', { fileId, reportIds, format, path });
export const renderReportMarkdown = (text: string) => invoke<string>('render_report_markdown', { text });

export const prepareProject = (fileId: string, root: string) => invoke<import('../types').ProjectInfo>('prepare_project', { fileId, root });
export const projectEditorStatus = (fileId: string, scopeId: string) => invoke<import('../types').EditorStatus>('project_editor_status', { fileId, scopeId });
export const diagnoseProject = (fileId: string, agentId: string, parentReportId: string, scopeId: string) => invoke<{ sessionId: string }>('diagnose_project', { fileId, agentId, parentReportId, scopeId });
