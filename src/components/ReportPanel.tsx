import { useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { exportReports } from '../lib/tauri';
import type { DiagnoseState } from '../hooks/useDiagnose';
import { DiagnosisStream } from './DiagnosisStream';
const statusLabel = { running: '生成中', completed: '已完成', cancelled: '已取消（不完整）', failed: '失败（不完整）', incomplete: '不完整' };
export function ReportPanel({ state, startSource, startProject }: { state: DiagnoseState; startSource: (root: string) => Promise<void>; startProject?: (root: string) => Promise<void> }) {
 const [root, setRoot] = useState(''); const [selection, setSelection] = useState('performance');
 const [format, setFormat] = useState<'markdown' | 'html'>('markdown'); const [message, setMessage] = useState(''); const [saving, setSaving] = useState(false);
 const performance = state.reports.find(r => r.stage === 'performance'); const source = state.reports.find(r => r.stage === 'project' || r.stage === 'source');
 const running = state.phase === 'diagnosing'; const busy = running || state.phase === 'preparing';
 const live = (stage: string) => state.activeStage === stage && state.streamedText ? state.sessionId : null;
 const perfId = live('performance') || (performance?.text ? performance.reportId : null);
 const sourceId = live('project') || live('source') || (source?.text ? source.reportId : null);
 const options = [{ value: 'performance', label: '性能诊断', ids: perfId ? [perfId] : [] }, { value: 'source', label: startProject ? '工程定位' : '源码定位', ids: sourceId ? [sourceId] : [] }, { value: 'combined', label: '合并报告', ids: perfId && sourceId ? [perfId, sourceId] : [] }].filter(o => o.ids.length);
 const choice = options.find(o => o.value === selection) ?? options[0];
 async function pickRoot() { try { const path = await open({ directory: true, multiple: false, title: startProject ? '选择 Unity 工程根目录' : '选择 C# 源码目录' }); if (typeof path === 'string') setRoot(path); } catch (e) { setMessage(String(e)); } }
 async function exportCurrent(requested?: string) {
  const selected = requested ? options.find(o => o.value === requested) : choice;
  if (!selected || !state.upload) return; setSaving(true); setMessage('');
  try {
   const extension = format === 'html' ? 'html' : 'md';
   const name = state.upload.filename.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_');
   const path = await save({ defaultPath: `${name}-${selected.value}.${extension}`, filters: [{ name: format === 'html' ? 'HTML' : 'Markdown', extensions: [extension] }] });
   if (!path) return;
   await exportReports(state.upload.fileId, selected.ids, format, path); setMessage('报告已导出');
  } catch (e) { setMessage(`导出失败：${String(e)}`); } finally { setSaving(false); }
 }
 return <section aria-label="诊断报告">
  {options.length > 0 && <div className="report-actions">
   <label>报告 <select aria-label="导出报告范围" value={choice?.value} onChange={e => setSelection(e.target.value)}>{options.map(o => <option value={o.value} key={o.value}>{o.label}</option>)}</select></label>
   <label>格式 <select aria-label="导出格式" value={format} onChange={e => setFormat(e.target.value as typeof format)}><option value="markdown">Markdown</option><option value="html">HTML</option></select></label>
   <button className="btn" onClick={() => void exportCurrent()} disabled={saving}>导出报告</button>
   {running && <span>当前报告尚未完成，导出会注明状态。</span>}
  </div>}
  {message && <p role="status">{message}</p>}
  <h3>性能诊断报告</h3>
  {performance && <p>{performance.agentId} · {statusLabel[performance.status]} {performance.incompleteReason}</p>}
  <DiagnosisStream text={state.activeStage === 'performance' ? state.streamedText || performance?.text || '' : performance?.text || ''} isStreaming={running && state.activeStage === 'performance'} />
  {performance?.status === 'completed' && <div className="source-controls">
   <h3>{startProject ? '选择 Unity 工程并联合定位' : '选择源码目录并定位'}</h3>
   <p>{startProject ? '只读分析代码与关联资源，按需从同一工程的 Editor 采集信息。代码片段和资源字段会交给当前选择的 AI Agent；请确认工程与录制版本对应。Editor 需安装 UPAA 只读采集插件，无法连接时将明确标记离线分析。' : '只读分析 C#，不修改源码。读取到的代码片段会交给当前选择的 AI Agent；请确认目录与录制版本对应。'}</p>
   {startProject && <details><summary>首次连接 Editor：安装采集插件</summary><p>安装 Unity CLI，并在目标工程的 Package Manager 中选择“Install package from disk”，选取本仓库 unity/Packages/com.upaa.inspector/package.json。等待编译完成后重新开始定位。应用不会自动修改 Packages 或升级 Unity。</p><p>离线扫描 Assets、ProjectSettings 与工程内嵌 Packages；外部包仅通过 Editor 提供资源摘要。</p></details>}
   <label>{startProject ? 'Unity 工程目录' : '源码目录'} <input aria-label={startProject ? 'Unity 工程目录' : '源码目录'} value={root} onChange={e => setRoot(e.target.value)} disabled={busy} placeholder="选择或粘贴项目／脚本目录" /></label>
   <button className="btn" onClick={pickRoot} disabled={busy}>选择目录</button>
   <button className="btn btn-primary" onClick={() => { setMessage(''); void (startProject ?? startSource)(root); }} disabled={busy || !root.trim() || !state.selectedAgent}>{startProject ? '开始工程联合定位' : '开始源码定位'}</button>
   {state.phase === 'preparing' && <p role="status">正在扫描目录并检查 Editor，可使用上方“取消”停止。</p>}
   {state.sourceInfo && <p>{state.sourceInfo.root} · {state.sourceInfo.fileCount} 个 C# 文件{state.sourceInfo.warnings.map((w, i) => <span className="source-warning" key={i}>{w}</span>)}</p>}
  </div>}
  {state.projectInfo && <div className="source-controls"><p>{state.projectInfo.root} · Unity {state.projectInfo.unityVersion} · {state.projectInfo.fileCount} 个文件</p><p>准备时 Editor：{state.projectInfo.editor.status === 'ready' ? `已连接 · ${state.projectInfo.editor.targetPlatform}` : `不可用，仅离线定位：${state.projectInfo.editor.reason}`}</p>{state.projectInfo.warnings.map((w,i) => <p className="source-warning" key={i}>{w}</p>)}</div>}
  {(source || state.activeStage === 'source' || state.activeStage === 'project') && <>
   <h3>{source?.stage === 'project' || state.activeStage === 'project' ? 'Unity 工程性能定位报告' : '源码定位报告'}</h3>
   {source && <p>{source.agentId} · {statusLabel[source.status]} {source.incompleteReason}</p>}
   <DiagnosisStream text={state.activeStage !== 'performance' ? state.streamedText || source?.text || '' : source?.text || ''} isStreaming={running && state.activeStage !== 'performance'} />
   {source?.projectContext != null && <details><summary>实际工程与 Editor 采集范围</summary><pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{JSON.stringify(source.projectContext, null, 2)}</pre></details>}
   <div className="report-actions">
    <label>定位报告格式 <select aria-label="定位报告格式" value={format} onChange={e => setFormat(e.target.value as typeof format)}><option value="markdown">Markdown</option><option value="html">HTML</option></select></label>
    <button className="btn" disabled={saving || !sourceId} onClick={() => void exportCurrent('source')}>导出定位报告</button>
    <button className="btn" disabled={saving || !sourceId || !perfId} onClick={() => void exportCurrent('combined')}>合并导出</button>
   </div>
  </>}
 </section>;
}
