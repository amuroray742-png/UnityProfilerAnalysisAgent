import { useEffect, useRef, useState } from 'react';
import { getFlowEvents } from '../lib/tauri';
import type { FlowPage } from '../types';

export function FlowEvents({ fileId, frame, frames }: { fileId: string; frame: number; frames: { frameIndex: number }[] }) {
  const [end, setEnd] = useState(frame);
  const [id, setId] = useState('');
  const [page, setPage] = useState<FlowPage | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  async function load(start = 0) {
    const token = ++generation.current;
    setError('');
    const value = id.trim() === '' ? null : Number(id);
    if (value !== null && (!/^\d+$/.test(id.trim()) || !Number.isInteger(value) || value < 0 || value > 4294967295)) {
      setError('Flow ID 必须是 0–4294967295 的整数'); return;
    }
    setBusy(true);
    try {
      const result = await getFlowEvents(fileId, frame, end, value, start);
      if (token === generation.current) setPage(result);
    } catch (e) { if (token === generation.current) setError(String(e)); }
    finally { if (token === generation.current) setBusy(false); }
  }
  return <details><summary>Flow 跨线程事件</summary>
    <p>从当前帧 {frame} 开始查询，最多连续 8 帧；按同一 Flow ID 查关联样本。事件不直接证明等待耗时或关键路径。</p>
    <label>结束帧 <select aria-label="Flow 结束帧" disabled={busy} value={end} onChange={e => { setEnd(Number(e.target.value)); setPage(null); }}>
      {frames.filter(f => f.frameIndex >= frame && f.frameIndex < frame + 8).map(f => <option key={f.frameIndex} value={f.frameIndex}>{f.frameIndex}</option>)}
    </select></label>{' '}
    <label>Flow ID <input aria-label="Flow ID" disabled={busy} value={id} placeholder="全部" onChange={e => { setId(e.target.value); setPage(null); }} /></label>{' '}
    <button disabled={busy} onClick={() => load()}>读取 Flow</button>
    {busy && <p role="status">正在读取 Flow…</p>}{error && <p role="alert">{error}</p>}
    {page && <>
      <p>{page.available ? `窗口内 ${page.total} 个事件 · Begin ${page.beginCount} / End ${page.endCount} · 未知类型 ${page.unknownTypes}` : '不可用：该输入未提供 Flow 数据。'}</p>
      <p>{page.scope}</p>
      <div style={{ overflowX: 'auto' }}><table className="hotspot-table" aria-label="Flow 事件"><thead><tr><th>帧 / 线程 / 样本</th><th>Flow ID</th><th>类型</th><th>所属 Marker</th></tr></thead>
        <tbody>{page.rows.map(r => <tr key={`${r.frameIndex}:${r.threadIndex}:${r.eventIndex}`}>
          <td>{r.frameIndex} / {r.thread} #{r.threadIndex} / {r.sampleIndex < 0 ? '未绑定样本' : r.sampleIndex}</td>
          <td><button disabled={busy} onClick={() => { setId(String(r.flowId)); setPage(null); }}>{r.flowId}</button></td>
          <td>{r.kind} ({r.eventType})</td><td style={{ overflowWrap: 'anywhere' }}>{r.marker ?? '—'}</td>
        </tr>)}</tbody></table></div>
      <button disabled={busy} onClick={() => load()}>Flow 首页</button>{' '}
      <button disabled={busy || page.nextStart === null} onClick={() => load(page.nextStart!)}>下一页 Flow</button>
    </>}
  </details>;
}
