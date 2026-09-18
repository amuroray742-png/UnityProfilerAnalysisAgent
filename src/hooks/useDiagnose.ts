// Hook：诊断流程管理。上传 / 解析 / 诊断 / 取消 / 事件订阅

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  uploadProfiler,
  analyzeProfiler,
  diagnose,
  cancelDiagnose,
  listAgents,
  onDiagnoseEvent,
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

  // 订阅诊断事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onDiagnoseEvent((event) => {
      setState((s) => {
        const events = [...s.events, event];
        let streamedText = s.streamedText;
        let phase = s.phase;

        switch (event.kind) {
          case 'started':
            streamedText = '';
            phase = 'diagnosing';
            break;
          case 'chunk':
            streamedText += event.text;
            break;
          case 'finished':
            phase = 'done';
            break;
          case 'error':
            phase = 'error';
            break;
        }

        return { ...s, events, streamedText, phase };
      });
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  // 上传并解析
  const handleFile = useCallback(async (filePath: string) => {
    setState((s) => ({ ...s, phase: 'uploading', errorMessage: null }));
    try {
      const upload = await uploadProfiler(filePath);
      setState((s) => ({ ...s, phase: 'analyzing', upload }));

      const snapshot = await analyzeProfiler(upload.fileId);
      setState((s) => ({ ...s, phase: 'ready', snapshot }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setState((s) => ({ ...s, phase: 'error', errorMessage: message }));
    }
  }, []);

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

    try {
      const { sessionId } = await diagnose(upload.fileId, selectedAgent);
      sessionRef.current = sessionId;
      setState((s) => ({ ...s, sessionId }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setState((s) => ({ ...s, phase: 'error', errorMessage: message }));
    }
  }, [state.upload, state.snapshot, state.selectedAgent]);

  // 取消诊断
  const cancel = useCallback(async () => {
    const { sessionId } = state;
    if (sessionId) {
      try {
        await cancelDiagnose(sessionId);
      } catch (err) {
        console.error('Cancel failed:', err);
      }
    }
    sessionRef.current = null;
    setState((s) => ({ ...s, phase: 'ready', sessionId: null }));
  }, [state.sessionId]);

  // 重置
  const reset = useCallback(() => {
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
  }, []);

  return { state, handleFile, selectAgent: safeSelectAgent, startDiagnose, cancel, reset };
}