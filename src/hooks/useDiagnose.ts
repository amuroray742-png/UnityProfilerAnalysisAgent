// Hook：诊断流程管理。上传 / 解析 / 诊断 / 取消 / 事件订阅

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  uploadProfiler,
  analyzeProfiler,
  diagnose,
  cancelDiagnose,
  listAgents,
  onDiagnoseEvent,
  prepareProject, diagnoseProject, releaseProfiler, listReports, prepareSource, cancelSourcePreparation, diagnoseSource,
} from '../lib/tauri.ts';
import type {
  DiagnosisReport, SourceInfo, ProjectInfo,
  AgentPreset,
  DiagnoseEvent,
  MetricsSnapshot,
  UploadResult,
} from '../types/index.ts';

export type Phase = 'idle' | 'uploading' | 'analyzing' | 'ready' | 'preparing' | 'diagnosing' | 'done' | 'error';

export interface DiagnoseState {
  reports: DiagnosisReport[]; activeStage: 'performance' | 'source' | 'project'; projectInfo?: ProjectInfo | null; sourceInfo: SourceInfo | null;
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
    reports: [], activeStage: 'performance', sourceInfo: null, projectInfo: null,
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
  const prepareEpoch = useRef(0);
  const reportEpoch = useRef(0);
  const uploadRef = useRef<string | null>(null);
  const releaseInput = useCallback(() => {
    inputEpoch.current++; prepareEpoch.current++; reportEpoch.current++;
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

  const refreshReports = useCallback(async (fileId: string) => {
    const epoch = inputEpoch.current;
    const run = reportEpoch.current;
    try { const reports = await listReports(fileId);
      if (epoch === inputEpoch.current && run === reportEpoch.current && fileId === uploadRef.current) setState(s => ({ ...s, reports, streamedText: reports.find(r => r.sessionId === sessionRef.current)?.text ?? s.streamedText }));
    } catch (e) { if (epoch === inputEpoch.current && run === reportEpoch.current && fileId === uploadRef.current) setState(s => ({ ...s, errorMessage: `读取报告失败：${String(e)}` })); }
  }, []);

  const applyEvent = useCallback((event: DiagnoseEvent) => {
    if (event.sessionId !== sessionRef.current || event.fileId !== uploadRef.current) return;
    if (['finished', 'cancelled', 'error'].includes(event.kind)) void refreshReports(event.fileId);
    setState(s => {
      if (s.phase !== 'diagnosing') return s;
      const events = [...s.events, event].slice(-500);
      switch (event.kind) {
        case 'chunk': return { ...s, events, streamedText: (s.streamedText + event.text) };
        case 'finished': return { ...s, events, phase: 'done' };
        case 'cancelled': return { ...s, events, phase: 'ready', sessionId: null };
        case 'error': return { ...s, events, phase: 'error', errorMessage: event.message };
        default: return { ...s, events };
      }
    });
  }, [refreshReports]);
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
    setState((s) => ({ ...s, phase: 'uploading', upload: null, snapshot: null, errorMessage: null, reports: [], sourceInfo: null, projectInfo: null, streamedText: '', events: [], sessionId: null, activeStage: 'performance' }));
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
  const startRun = useCallback(async (source?: { parentId: string; scopeId: string; project?: boolean }) => {
    const { upload, snapshot, selectedAgent } = state;
    if (!upload || !snapshot || !selectedAgent || pendingSession.current) return;
    reportEpoch.current++;

    setState((s) => ({
      ...s,
      phase: 'diagnosing', activeStage: source ? (source.project ? 'project' : 'source') : 'performance',
      reports: source ? s.reports : [],
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
      const { sessionId } = await (source ? (source.project ? diagnoseProject : diagnoseSource)(upload.fileId, selectedAgent, source.parentId, source.scopeId) : diagnose(upload.fileId, selectedAgent));
      if (epoch !== inputEpoch.current || pendingSession.current !== pending) {
        await cancelDiagnose(sessionId);
        if (epoch === inputEpoch.current && uploadRef.current === upload.fileId) await refreshReports(upload.fileId);
        return;
      }
      sessionRef.current = sessionId;
      pendingSession.current = null;
      setState(s => ({ ...s, sessionId, reports: source ? s.reports.filter(r => r.stage === 'performance') : s.reports }));
      pending.events.forEach(applyEvent);
    } catch (err) {
      if (epoch !== inputEpoch.current || pendingSession.current !== pending) return;
      pendingSession.current = null;
      const message = err instanceof Error ? err.message : String(err);
      setState(s => ({ ...s, phase: 'error', errorMessage: message }));
    }
  }, [state.upload, state.snapshot, state.selectedAgent, applyEvent, refreshReports]);

  const startDiagnose = useCallback(() => startRun(), [startRun]);
  const startSource = useCallback(async (root: string) => {
    const parent = state.reports.find(r => r.stage === 'performance' && r.status === 'completed');
    if (!parent || !state.upload || !root.trim() || ['diagnosing', 'preparing'].includes(state.phase)) return;
    const epoch = inputEpoch.current; const preparation = ++prepareEpoch.current;
    setState(s => ({ ...s, phase: 'preparing', errorMessage: null, sourceInfo: null, projectInfo: null }));
    try {
      const info = await prepareSource(parent.fileId, root.trim());
      if (epoch !== inputEpoch.current || preparation !== prepareEpoch.current) return;
      setState(s => ({ ...s, sourceInfo: info }));
      await startRun({ parentId: parent.reportId, scopeId: info.scopeId });
    } catch (e) { if (epoch === inputEpoch.current && preparation === prepareEpoch.current) setState(s => ({ ...s, phase: 'done', errorMessage: String(e) })); }
  }, [state.reports, state.upload, state.phase, startRun]);

  const startProject = useCallback(async (root: string) => {
    const parent = state.reports.find(r => r.stage === 'performance' && r.status === 'completed');
    if (!parent || !state.upload || !root.trim() || ['diagnosing', 'preparing'].includes(state.phase)) return;
    const epoch = inputEpoch.current; const preparation = ++prepareEpoch.current;
    setState(s => ({ ...s, phase: 'preparing', errorMessage: null, sourceInfo: null, projectInfo: null }));
    try {
      const info = await prepareProject(parent.fileId, root.trim());
      if (epoch !== inputEpoch.current || preparation !== prepareEpoch.current) return;
      setState(s => ({ ...s, projectInfo: info }));
      await startRun({ parentId: parent.reportId, scopeId: info.scopeId, project: true });
    } catch (e) { if (epoch === inputEpoch.current && preparation === prepareEpoch.current) setState(s => ({ ...s, phase: 'done', errorMessage: String(e) })); }
  }, [state.reports, state.upload, state.phase, startRun]);

  const cancel = useCallback(async () => {
    const epoch = inputEpoch.current;
    const run = reportEpoch.current;
    if (state.phase === 'analyzing' || state.phase === 'uploading') {releaseInput();setState(s=>({...s,phase:'idle',upload:null,snapshot:null}));return;}
    if (state.phase === 'preparing' && state.upload) {
      const preparation = ++prepareEpoch.current;
      try { await cancelSourcePreparation(state.upload.fileId); if (epoch === inputEpoch.current && preparation === prepareEpoch.current) setState(s => ({ ...s, phase: 'done' })); }
      catch (e) { if (epoch === inputEpoch.current && preparation === prepareEpoch.current) setState(s => ({ ...s, errorMessage: String(e) })); }
      return;
    }
    const sessionId = sessionRef.current;
    pendingSession.current = null;
    try {
      if (sessionId) await cancelDiagnose(sessionId);
      if (epoch !== inputEpoch.current || run !== reportEpoch.current) return;
      if (uploadRef.current) await refreshReports(uploadRef.current);
      if (epoch === inputEpoch.current && run === reportEpoch.current && sessionRef.current === sessionId) {
        sessionRef.current = null;
        setState(s => ({ ...s, phase: s.snapshot ? 'ready' : 'idle', sessionId: null }));
      }
    } catch (err) {
      if (epoch === inputEpoch.current && run === reportEpoch.current) setState(s => ({ ...s, errorMessage: String(err) }));
    }
  }, [state.phase, state.upload, refreshReports]);

  // 重置
  const reset = useCallback(() => {
    releaseInput();
    sessionRef.current = null;
    setState((s) => ({
      ...s,
      phase: 'idle', reports: [], sourceInfo: null, projectInfo: null, activeStage: 'performance',
      upload: null,
      snapshot: null,
      sessionId: null,
      streamedText: '',
      events: [],
      errorMessage: null,
    }));
  }, [releaseInput]);

  return { state, handleFile, selectAgent: safeSelectAgent, startDiagnose, startSource, startProject, cancel, reset };
}
