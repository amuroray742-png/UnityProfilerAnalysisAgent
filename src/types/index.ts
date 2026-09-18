// 与 Rust 端共享的 DTO 类型。
// 这些类型在 Rust 端通过 serde 派生，前端 TypeScript 通过 invoke<'upload', Args, Result> 使用。
// 如果未来用 ts-rs 自动生成，可以替换为 `import type { ... } from '../bindings';`

export interface UploadResult {
  fileId: string;
  filename: string;
  sizeBytes: number;
  extension: string;
}

export interface Hotspot {
  name: string;
  totalMs: number;
  callCount: number;
  avgMs: number;
  maxMs: number;
}

export interface FrameTimeStats {
  p50: number;
  p95: number;
  p99: number;
  max: number;
}

export interface CpuMetrics {
  mainThreadMs: FrameTimeStats;
  topHotspots: Hotspot[];
  frameTimeline: Array<{ frameIndex: number; ms: number }>;
}

export interface GcMetrics {
  totalAllocBytes: number;
  allocPerFrameBytes: FrameTimeStats;
  genCollections: { gen0: number; gen1: number; gen2: number };
  topAllocSites: Hotspot[];
}

export interface RenderingMetrics {
  drawCalls: FrameTimeStats;
  setPassCalls: FrameTimeStats;
  batchesSavedBySrpBatcher: number;
  topRenderEvents: Hotspot[];
}

export interface MetricsSnapshot {
  meta: {
    fileName: string;
    durationMs: number;
    frameCount: number;
    platform: string | null;
    unityVersion: string | null;
  };
  cpu: CpuMetrics;
  gc: GcMetrics;
  rendering: RenderingMetrics;
  warnings: string[];
}

export interface AgentPreset {
  id: string;
  label: string;
  command: string;
  args: string[];
  available: boolean;
}

export type DiagnoseEvent =
  | { kind: 'started'; agentId: string }
  | { kind: 'chunk'; text: string }
  | { kind: 'mcp-call'; tool: string; args: unknown }
  | { kind: 'mcp-result'; tool: string; result: unknown }
  | { kind: 'finished'; totalChunks: number }
  | { kind: 'error'; message: string };

export interface ErrorPayload {
  code: string;
  message: string;
  hint?: string;
}