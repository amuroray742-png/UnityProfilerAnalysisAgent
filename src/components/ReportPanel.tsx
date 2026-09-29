import { useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { exportReports } from '../lib/tauri';
import type { DiagnoseState } from '../hooks/useDiagnose';
import { DiagnosisStream } from './DiagnosisStream';
const statusLabel = { running: '生成中', completed: '已完成', cancelled: '已取消（不完整）', failed: '失败（不完整）', incomplete: '不完整' };
export function ReportPanel({ state, startSource }: { state: DiagnoseState; startSource: (root: string) => Promise<void> }) {
 const [root, setRoot] = useState(''); const [selection, setSelection] = useState('performance');
 const [format, setFormat] = useState<'markdown' | 'html'>('markdown'); const [message, setMessage] = useState(''); const [saving, setSaving] = useState(false);
 const performance = state.reports.find(r => r.stage === 'performance'); const source = state.reports.find(r => r.stage === 'source');
 const running = state.phase === 'diagnosing'; const busy = running || state.phase === 'preparing';
 const live = (stage: string) => state.activeStage === stage && state.streamedText ? state.sessionId : null;
 const perfId = live('performance') || (performance?.text ? performance.reportId : null);
 const sourceId = live('source') || (source?.text ? source.reportId : null);
 const options = [{ value: 'performance', label: '性能诊断', ids: perfId ? [perfId] : [] }, { value: 'source', label: '源码定位', ids: sourceId ? [sourceId] : [] }, { value: 'combined', label: '合并报告', ids: perfId && sourceId ? [perfId, sourceId] : [] }].filter(o => o.ids.length);
 const choice = options.find(o => o.value === selection) ?? options[0];
 async function pickRoot() { try { const path = await open({ directory: true, multiple: false, title: '选择 C# 源码目录' }); if (typeof path === 'string') setRoot(path); } catch (e) { setMessage(String(e)); } }
 async function exportCurrent() {
  if (!choice || !state.upload) return; setSaving(true); setMessage('');
  try {
   const extension = format === 'html' ? 'html' : 'md';
   const name = state.upload.filename.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_');
   const path = await save({ defaultPath: `${name}-${choice.value}.${extension}`, filters: [{ name: format === 'html' ? 'HTML' : 'Markdown', extensions: [extension] }] });
   if (!path) return;
   await exportReports(state.upload.fileId, choice.ids, format, path); setMessage('报告已导出');
  } catch (e) { setMessage(`导出失败：${String(e)}`); } finally { setSaving(false); }
 }
 return <section aria-label="诊断报告">
  {options.length > 0 && <div className="report-actions">
   <label>报告 <select aria-label="导出报告范围" value={choice?.value} onChange={e => setSelection(e.target.value)}>{options.map(o => <option value={o.value} key={o.value}>{o.label}</option>)}</select></label>
   <label>格式 <select aria-label="导出格式" value={format} onChange={e => setFormat(e.target.value as typeof format)}><option value="markdown">Markdown</option><option value="html">HTML</option></select></label>
   <button className="btn" onClick={exportCurrent} disabled={saving}>导出报告</button>
   {running && <span>当前报告尚未完成，导出会注明状态。</span>}
  </div>}
  {message && <p role="status">{message}</p>}
  <h3>性能诊断报告</h3>
  {performance && <p>{performance.agentId} · {statusLabel[performance.status]} {performance.incompleteReason}</p>}
  <DiagnosisStream text={state.activeStage === 'performance' ? state.streamedText || performance?.text || '' : performance?.text || ''} isStreaming={running && state.activeStage === 'performance'} />
  {performance?.status === 'completed' && <div className="source-controls">
   <h3>选择源码目录并定位</h3>
   <p>只读分析 C#，不修改源码。读取到的代码片段会交给当前选择的 AI Agent；请确认目录与录制版本对应。</p>
   <label>源码目录 <input aria-label="源码目录" value={root} onChange={e => setRoot(e.target.value)} disabled={busy} placeholder="选择或粘贴项目／脚本目录" /></label>
   <button className="btn" onClick={pickRoot} disabled={busy}>选择目录</button>
   <button className="btn btn-primary" onClick={() => { setMessage(''); void startSource(root); }} disabled={busy || !root.trim() || !state.selectedAgent}>开始源码定位</button>
   {state.phase === 'preparing' && <p role="status">正在准备源码目录，可使用上方“取消”停止。</p>}
   {state.sourceInfo && <p>{state.sourceInfo.root} · {state.sourceInfo.fileCount} 个 C# 文件{state.sourceInfo.warnings.map((w, i) => <span className="source-warning" key={i}>{w}</span>)}</p>}
  </div>}
  {(source || state.activeStage === 'source') && <>
   <h3>源码定位报告</h3>
   {source && <p>{source.agentId} · {statusLabel[source.status]} {source.incompleteReason}</p>}
   <DiagnosisStream text={state.activeStage === 'source' ? state.streamedText || source?.text || '' : source?.text || ''} isStreaming={running && state.activeStage === 'source'} />
  </>}
 </section>;
}
