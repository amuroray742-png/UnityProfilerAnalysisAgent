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

export interface Quality {
 status: 'available' | 'partial' | 'unavailable' | 'estimated'; source: string;
 reasons: string[]; validFrames: number; totalFrames: number;
}
export interface AllocHotspot {
 name: string; thread: string; totalBytes: number; avgBytes: number; maxBytes: number; callCount: number;
}
export interface FrameTimeStats {
 quality: Quality;
  p50: number | null;
  p95: number | null;
  p99: number | null;
  max: number | null;
}

export interface CpuMetrics {
  mainThreadMs: FrameTimeStats;
  hotspotQuality: Quality;
  topHotspots: Hotspot[];
  frameTimeline: Array<{ drawCalls: number | null; setPassCalls: number | null; frameIndex: number; ms: number | null; frameTimeMs: number | null; gcAllocBytes: number | null }>;
}

export interface GcMetrics {
  totalAllocBytes: number | null;
  siteQuality: Quality;
  allocPerFrameBytes: FrameTimeStats;
  genCollections: { gen0: number | null; gen1: number | null; gen2: number | null };
  topAllocSites: AllocHotspot[];
}

export interface RenderingMetrics {
  batches: FrameTimeStats;
  triangles: FrameTimeStats;
  vertices: FrameTimeStats;
  drawCalls: FrameTimeStats;
  setPassCalls: FrameTimeStats;
  batchesSavedBySrpBatcher: number | null;
  eventQuality: Quality;
  topRenderEvents: Hotspot[];
}

export interface MetricsSnapshot {
  meta: {
    fileName: string;
    declaredFrameCount: number;
    durationQuality: Quality;
    source: string;
    durationMs: number | null;
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

export type DiagnoseEvent = {sessionId: string; fileId: string} & (
  | { kind: 'session-created'; acpSessionId: string }
  | { kind: 'started'; agentId: string }
  | { kind: 'chunk'; text: string }
  | { kind: 'mcp-call'; tool: string; args: unknown }
  | { kind: 'mcp-result'; tool: string; result: unknown }
  | { kind: 'finished'; totalChunks: number; stopReason: string }
  | { kind: 'cancelled' }
  | { kind: 'log'; message: string }
  | { kind: 'error'; message: string });

export interface ErrorPayload {
  code: string;
  message: string;
  hint?: string;
}

export interface FrameInfo {
  renderCounters: Record<string, number>;
  frameIndex: number; rawFrameId: number | null; rawDuplicateId: number | null;
  startNs: string | null; source: string; cpuMs: number | null;
  frameTimeMs: number | null; gcAllocBytes: number | null; warnings: string[];
}
export interface ThreadInfo {
  threadIndex: number; threadId: string; name: string; group: string | null; sampleCount: number;
}
export interface DetailSample {
  sampleIndex: number; parentIndex: number | null; depth: number; markerId: number;
  name: string; categoryIndex: number | null; totalMs: number; startMs: number;
  rawStartNs: string | null; rawDurationNs: number | null;
  childrenCount: number; metadataCount: number; gcAllocBytes: number | null;
  selfMs?: number | null; selfReason?: string | null; isCounter?: boolean;
}
export interface FramePage {
  info: FrameInfo; threadCount: number; threads: ThreadInfo[]; nextStart: number | null;
}
export interface HierarchyPage {
  info: FrameInfo; thread: ThreadInfo; samples: DetailSample[];
  nextStart: number | null; maxDepth: number; depthTruncated: boolean;
}
export interface DiagnosisReport {
 reportId: string; fileId: string; sessionId: string; stage: 'performance' | 'source' | 'project'; parentReportId: string | null;
 text: string; createdAt: string; agentId: string; status: 'running' | 'completed' | 'cancelled' | 'failed' | 'incomplete';
 projectContext?: unknown;
 incompleteReason: string | null; fileName: string; unityVersion: string | null; frameCount: number; coverage: string;
}
export interface SourceInfo { scopeId: string; fileId: string; root: string; fileCount: number; warnings: string[] }

export interface EditorStatus { details?: Record<string, unknown> | null; status: string; reason: string | null; unityVersion: string | null; targetPlatform: string | null; sampledAt: string | null }
export interface ProjectInfo { scopeId: string; fileId: string; root: string; unityVersion: string; fileCount: number; warnings: string[]; editor: EditorStatus }

export interface MetadataValue {
 fieldIndex: number; definition: { descriptor: number; name: string; nameTruncated?: boolean } | null;
 payloadType: number; byteLength: number; value: string | null; unit: string | null;
 status: string; reason: string | null; rawHex: string; rawTruncated?: boolean;
}
export interface EvidenceRow { threadIndex: number; threadId: string; thread: string; sampleIndex: number; markerId: number; marker: string; isCounter: boolean; metadataCount: number; metadata: MetadataValue[]; metadataTruncated: boolean; metadataReason?: string | null }
export interface EvidencePage { frameIndex: number; source: string; rows: EvidenceRow[]; total: number; nextStart: number | null; scope: string }
export interface PathTotals { calls: number; inclusiveMs: number; selfMs: number | null; selfValidSamples: number; gcBytes: string | null }
export interface ComparePage { frameIndex: number; baselineFrameIndex: number; thread: ThreadInfo; rows: { path: string[]; baseline: PathTotals; current: PathTotals; inclusiveDeltaMs: number; selfDeltaMs: number | null; gcDeltaBytes: string | null }[]; total: number; nextStart: number | null; interpretation: string }
export interface FlowPage {
  available: boolean;
  frameIndex: number;
  endFrameIndex: number;
  flowId: number | null;
  total: number;
  nextStart: number | null;
  unknownTypes: number;
  beginCount: number;
  endCount: number;
  scope: string;
  rows: { frameIndex: number; threadIndex: number; threadId: string; thread: string; eventIndex: number;
    sampleIndex: number; flowId: number; eventType: number; kind: string; marker: string | null;
    sampleStartNs: string | null; sampleDurationMs: number | null }[];
}
