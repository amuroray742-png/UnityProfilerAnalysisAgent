import { useState } from 'react';
import { UploadDropzone } from './components/UploadDropzone.tsx';
import { MetricCard } from './components/MetricCard.tsx';
import { HotspotTable } from './components/HotspotTable.tsx';
import { DiagnosisStream } from './components/DiagnosisStream.tsx';
import { AgentLogDrawer } from './components/AgentLogDrawer.tsx';
import { ParseProgressBar } from './components/ParseProgressBar.tsx';
import { useDiagnose } from './hooks/useDiagnose.ts';

type Tab = 'overview' | 'cpu' | 'gc' | 'rendering' | 'log';

export default function App() {
  const { state, handleFile, selectAgent, startDiagnose, cancel, reset } = useDiagnose();
  const [tab, setTab] = useState<Tab>('overview');

  const isBusy = state.phase === 'uploading' || state.phase === 'analyzing';
  const isDiagnosing = state.phase === 'diagnosing';
  const showResults = state.snapshot != null;

  // 是否有可用 Agent（任意一个 available=true）
  const hasAvailableAgent = state.agents.some((a) => a.available);
  // 当前选中的 Agent 是否可用（null 或 unavailable 都视为不可用）
  const selectedAgentAvailable = state.selectedAgent != null
    && state.agents.find((a) => a.id === state.selectedAgent)?.available === true;

  const statusLabel = (() => {
    switch (state.phase) {
      case 'idle':
        return '就绪';
      case 'uploading':
        return '上传中...';
      case 'analyzing':
        return '解析中...';
      case 'ready':
        return '已就绪，等待 AI 诊断';
      case 'diagnosing':
        return 'AI 诊断中...';
      case 'done':
        return '诊断完成';
      case 'error':
        return '出错';
    }
  })();

  const statusClass = (() => {
    if (state.phase === 'error') return 'error';
    if (isBusy || isDiagnosing) return 'busy';
    if (state.phase === 'ready' || state.phase === 'done') return 'ready';
    return '';
  })();

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1>Unity Profiler Analysis Agent</h1>
          <div className="subtitle">
            自动分析 Unity Profiler 数据，通过 ACP 接入 Claude Code / Gemini CLI
          </div>
        </div>
        <div className="agent-selector">
          {state.agents.length > 0 && (
            <select
              value={state.selectedAgent ?? ''}
              onChange={(e) => selectAgent(e.target.value)}
              disabled={isDiagnosing || state.agents.every((a) => !a.available)}
            >
              {state.agents.map((agent) => (
                <option key={agent.id} value={agent.id} disabled={!agent.available}>
                  {agent.label} {agent.available ? '' : '(未安装)'}
                </option>
              ))}
            </select>
          )}
          {showResults && !isDiagnosing && (
            <button
              className="btn btn-primary"
              onClick={startDiagnose}
              disabled={!selectedAgentAvailable}
              title={selectedAgentAvailable ? '' : '请先在 Agent 下拉里选择一个已安装的 Agent'}
            >
              开始 AI 诊断
            </button>
          )}
          {showResults && !isDiagnosing && !hasAvailableAgent && (
            <span className="hint-text" style={{ color: '#d97706', fontSize: 13 }}>
              未检测到可用 Agent（Claude Code / Gemini CLI / Codex CLI），请先安装
            </span>
          )}
          {isDiagnosing && (
            <button className="btn" onClick={cancel}>
              取消
            </button>
          )}
          {showResults && !isBusy && !isDiagnosing && (
            <button className="btn" onClick={reset}>
              重置
            </button>
          )}
        </div>
      </header>

      {state.errorMessage && <div className="error-banner">{state.errorMessage}</div>}
      {state.snapshot?.warnings && state.snapshot.warnings.length > 0 && (
        <div className="warning-banner">
          ⚠️ 解析警告：
          <ul style={{ marginTop: 4, paddingLeft: 20 }}>
            {state.snapshot.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 解析进度条：仅在 analyzing 阶段显示 */}
      {state.phase === 'analyzing' && state.upload && (
        <ParseProgressBar fileId={state.upload.fileId} />
      )}

      {!showResults && (
        <UploadDropzone onFileSelected={handleFile} disabled={isBusy} />
      )}

      {showResults && state.snapshot && (
        <>
          <div className="tabs">
            {(['overview', 'cpu', 'gc', 'rendering', 'log'] as Tab[]).map((t) => (
              <button
                key={t}
                className={`tab ${tab === t ? 'active' : ''}`}
                onClick={() => setTab(t)}
              >
                {t === 'overview'
                  ? '概览'
                  : t === 'cpu'
                  ? 'CPU'
                  : t === 'gc'
                  ? 'GC'
                  : t === 'rendering'
                  ? '渲染'
                  : 'Agent 日志'}
              </button>
            ))}
          </div>

          {tab === 'overview' && (
            <>
              <div className="metrics-grid">
                <MetricCard
                  label="主线程 p95"
                  value={state.snapshot.cpu.mainThreadMs.p95}
                  unit="ms"
                  thresholds={{ warning: 16.67, danger: 33.33 }}
                  detail={`p50 ${state.snapshot.cpu.mainThreadMs.p50.toFixed(2)} ms · max ${state.snapshot.cpu.mainThreadMs.max.toFixed(2)} ms`}
                />
                <MetricCard
                  label="GC 分配（每帧 p95）"
                  value={state.snapshot.gc.allocPerFrameBytes.p95 / (1024 * 1024)}
                  unit="bytes"
                  thresholds={{ warning: 4, danger: 16 }}
                  detail={`总分配 ${(state.snapshot.gc.totalAllocBytes / 1024 / 1024).toFixed(1)} MB`}
                />
                <MetricCard
                  label="Draw Call p95"
                  value={state.snapshot.rendering.drawCalls.p95}
                  unit="count"
                  thresholds={{ warning: 1500, danger: 3000 }}
                  detail={`SetPass p95 ${state.snapshot.rendering.setPassCalls.p95}`}
                />
                <MetricCard
                  label="帧数 / 时长"
                  value={state.snapshot.meta.frameCount}
                  unit="count"
                  detail={`${(state.snapshot.meta.durationMs / 1000).toFixed(1)} s · ${state.snapshot.meta.unityVersion ?? 'Unity 版本未知'}`}
                />
              </div>

              <DiagnosisStream
                text={state.streamedText}
                isStreaming={isDiagnosing}
                emptyHint={
                  state.phase === 'ready'
                    ? '选择 Agent 后点击"开始 AI 诊断"'
                    : '点击"开始 AI 诊断"让 Agent 分析性能瓶颈'
                }
              />
            </>
          )}

          {tab === 'cpu' && (
            <>
              <div className="metrics-grid">
                <MetricCard
                  label="主线程 p50"
                  value={state.snapshot.cpu.mainThreadMs.p50}
                  unit="ms"
                />
                <MetricCard
                  label="主线程 p99"
                  value={state.snapshot.cpu.mainThreadMs.p99}
                  unit="ms"
                  thresholds={{ warning: 16.67, danger: 33.33 }}
                />
                <MetricCard
                  label="主线程 max"
                  value={state.snapshot.cpu.mainThreadMs.max}
                  unit="ms"
                  thresholds={{ warning: 33.33, danger: 50 }}
                />
                <MetricCard
                  label="帧时间样本"
                  value={state.snapshot.cpu.frameTimeline.length}
                  unit="count"
                />
              </div>
              <HotspotTable
                title="主线程 Top 10 热点"
                hotspots={state.snapshot.cpu.topHotspots}
                valueColumn="ms"
              />
            </>
          )}

          {tab === 'gc' && (
            <>
              <div className="metrics-grid">
                <MetricCard
                  label="总 GC 分配"
                  value={state.snapshot.gc.totalAllocBytes / (1024 * 1024)}
                  unit="bytes"
                />
                <MetricCard
                  label="每帧分配 p95"
                  value={state.snapshot.gc.allocPerFrameBytes.p95 / (1024 * 1024)}
                  unit="bytes"
                  thresholds={{ warning: 4, danger: 16 }}
                />
                <MetricCard
                  label="Gen0 回收"
                  value={state.snapshot.gc.genCollections.gen0}
                  unit="count"
                />
                <MetricCard
                  label="Gen2 回收"
                  value={state.snapshot.gc.genCollections.gen2}
                  unit="count"
                  thresholds={{ warning: 5, danger: 20 }}
                />
              </div>
              <HotspotTable
                title="GC 分配 Top 10 热点"
                hotspots={state.snapshot.gc.topAllocSites}
                valueColumn="ms"
              />
            </>
          )}

          {tab === 'rendering' && (
            <>
              <div className="metrics-grid">
                <MetricCard
                  label="Draw Call p95"
                  value={state.snapshot.rendering.drawCalls.p95}
                  unit="count"
                />
                <MetricCard
                  label="SetPass p95"
                  value={state.snapshot.rendering.setPassCalls.p95}
                  unit="count"
                />
                <MetricCard
                  label="SRP Batcher 节省"
                  value={state.snapshot.rendering.batchesSavedBySrpBatcher}
                  unit="count"
                />
                <MetricCard
                  label="渲染事件数"
                  value={state.snapshot.rendering.topRenderEvents.length}
                  unit="count"
                />
              </div>
              <HotspotTable
                title="渲染事件 Top 10"
                hotspots={state.snapshot.rendering.topRenderEvents}
                valueColumn="ms"
              />
            </>
          )}

          {tab === 'log' && <AgentLogDrawer events={state.events} />}
        </>
      )}

      <div className="status-bar">
        <div>
          <span className={`status-dot ${statusClass}`}></span>
          {statusLabel}
        </div>
        <div>
          {state.snapshot && (
            <>
              {state.snapshot.meta.fileName} · {state.snapshot.meta.platform ?? 'Unknown Platform'}
            </>
          )}
        </div>
      </div>
    </div>
  );
}