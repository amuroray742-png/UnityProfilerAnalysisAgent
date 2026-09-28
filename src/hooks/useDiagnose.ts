// Hook：诊断流程管理。上传 / 解析 / 诊断 / 取消 / 事件订阅

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  uploadProfiler,
  analyzeProfiler,
  diagnose,
  cancelDiagnose,
  listAgents,
  onDiagnoseEvent,
  releaseProfiler,
} from '../lib/tauri.ts';
import type {
  AgentPreset,
  DiagnoseEvent,
  MetricsSnapshot,
  UploadResult,
} from '../types/index.ts';

export type Phase = 'idle' | 'uploading' | 'analyzing' | 'ready' | 'diagnosing' | 'done' | 'error';

export interface DiagnoseState {
  phase: Phase;
  upload: UploadResult | null;
  snapshot: MetricsSnapshot | null;
  agents: AgentPreset[];
  selectedAgent: string | null;
  sessionId: string | null;
  streamedText: string;
  events: DiagnoseEvent[];
  errorMessage: string | null;
}

export function useDiagnose() {
  const [state, setState] = useState<DiagnoseState>({
    phase: 'idle',
    upload: null,
    snapshot: null,
    agents: [],
    selectedAgent: null,
    sessionId: null,
    streamedText: '',
    events: [],
    errorMessage: null,
  });

  const sessionRef = useRef<string | null>(null);
  const pendingSession = useRef<{ fileId: string; events: DiagnoseEvent[] } | null>(null);
  const inputEpoch = useRef(0);
  const uploadRef = useRef<string | null>(null);
  const releaseInput = useCallback(() => {
    inputEpoch.current++;
    if (sessionRef.current) void cancelDiagnose(sessionRef.current).catch(console.error);
    sessionRef.current = null;
    pendingSession.current = null;
    if (uploadRef.current) void releaseProfiler(uploadRef.current).catch(console.error);
    uploadRef.current = null;
  }, []);
  useEffect(() => releaseInput, [releaseInput]);

  // 启动时加载 Agent 列表
  useEffect(() => {
    listAgents()
      .then((agents) => {
        const firstAvailable = agents.find((a) => a.available);
        setState((s) => ({
          ...s,
          agents,
          // 仅在有可用 agent 时默认选中；否则保持 null，让前端禁用按钮
          selectedAgent: firstAvailable?.id ?? null,
        }));
      })
      .catch((err) => {
        console.error('Failed to list agents:', err);
      });
  }, []);

  const applyEvent = useCallback((event: DiagnoseEvent) => {
    if (event.sessionId !== sessionRef.current || event.fileId !== uploadRef.current) return;
    setState(s => {
      if (s.phase !== 'diagnosing') return s;
      const events = [...s.events, event].slice(-500);
      switch (event.kind) {
        case 'chunk': return { ...s, events, streamedText: (s.streamedText + event.text).slice(-2 * 1024 * 1024) };
        case 'finished': return { ...s, events, phase: 'done' };
        case 'cancelled': return { ...s, events, phase: 'ready', sessionId: null };
        case 'error': return { ...s, events, phase: 'error', errorMessage: event.message };
        default: return { ...s, events };
      }
    });
  }, []);
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    onDiagnoseEvent(event => {
      if (!active) return;
      const pending = pendingSession.current;
      if (pending && pending.fileId === event.fileId) {
        pending.events.push(event);
        if (pending.events.length > 500) pending.events.shift();
      } else applyEvent(event);
    }).then(u => { if (active) unlisten = u; else u(); });
    return () => { active = false; unlisten?.(); };
  }, [applyEvent]);

  // 上传并解析
  const handleFile = useCallback(async (filePath: string) => {
    releaseInput();
    const epoch = inputEpoch.current;
    setState((s) => ({ ...s, phase: 'uploading', upload: null, snapshot: null, errorMessage: null }));
    try {
      const upload = await uploadProfiler(filePath);
      if (epoch !== inputEpoch.current) { await releaseProfiler(upload.fileId); return; }
      uploadRef.current = upload.fileId;
      setState((s) => ({ ...s, phase: 'analyzing', upload }));

      const snapshot = await analyzeProfiler(upload.fileId);
      if (epoch !== inputEpoch.current) return;
      setState((s) => ({ ...s, phase: 'ready', snapshot }));
    } catch (err) {
      if (epoch !== inputEpoch.current) return;
      const message = err instanceof Error ? err.message : String(err);
      setState((s) => ({ ...s, phase: 'error', errorMessage: message }));
    }
  }, [releaseInput]);

  // 选择 Agent（带 available 校验，不可用 agent 不让进 selectedAgent）
  const safeSelectAgent = useCallback((agentId: string) => {
    setState((s) => {
      const agent = s.agents.find((a) => a.id === agentId);
      return { ...s, selectedAgent: agent?.available ? agentId : null };
    });
  }, []);

  // 启动诊断
  const startDiagnose = useCallback(async () => {
    const { upload, snapshot, selectedAgent } = state;
    if (!upload || !snapshot || !selectedAgent) return;

    setState((s) => ({
      ...s,
      phase: 'diagnosing',
      streamedText: '',
      events: [],
      errorMessage: null,
    }));

    const epoch = inputEpoch.current;
    const pending = { fileId: upload.fileId, events: [] as DiagnoseEvent[] };
    if (pendingSession.current) return;
    pendingSession.current = pending;
    sessionRef.current = null;
    try {
      const { sessionId } = await diagnose(upload.fileId, selectedAgent);
      if (epoch !== inputEpoch.current || pendingSession.current !== pending) {
        await cancelDiagnose(sessionId); return;
      }
      sessionRef.current = sessionId;
      pendingSession.current = null;
      setState(s => ({ ...s, sessionId }));
      pending.events.forEach(applyEvent);
    } catch (err) {
      if (epoch !== inputEpoch.current || pendingSession.current !== pending) return;
      pendingSession.current = null;
      const message = err instanceof Error ? err.message : String(err);
      setState(s => ({ ...s, phase: 'error', errorMessage: message }));
    }
  }, [state.upload, state.snapshot, state.selectedAgent, applyEvent]);

  const cancel = useCallback(async () => {
    const sessionId = sessionRef.current;
    pendingSession.current = null;
    try {
      if (sessionId) await cancelDiagnose(sessionId);
      if (sessionRef.current === sessionId) {
        sessionRef.current = null;
        setState(s => ({ ...s, phase: s.snapshot ? 'ready' : 'idle', sessionId: null }));
      }
    } catch (err) {
      setState(s => ({ ...s, errorMessage: String(err) }));
    }
  }, []);

  // 重置
  const reset = useCallback(() => {
    releaseInput();
    sessionRef.current = null;
    setState((s) => ({
      ...s,
      phase: 'idle',
      upload: null,
      snapshot: null,
      sessionId: null,
      streamedText: '',
      events: [],
      errorMessage: null,
    }));
  }, [releaseInput]);

  return { state, handleFile, selectAgent: safeSelectAgent, startDiagnose, cancel, reset };
}
