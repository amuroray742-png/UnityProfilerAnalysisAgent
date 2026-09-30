import { useEffect, useRef, useState } from 'react';
import { compareFrames, getFrameEvidence } from '../lib/tauri';
import type { ComparePage, EvidencePage } from '../types';
import { FrameSections } from './FrameSections';
import { FlowEvents } from './FlowEvents';

export function FrameEvidence({ fileId, frame, thread, frames }: {
  fileId: string; frame: number; thread: number | null; frames: { frameIndex: number }[];
}) {
  const [baseline, setBaseline] = useState(frames.find(f => f.frameIndex !== frame)?.frameIndex ?? frame);
  const [evidence, setEvidence] = useState<EvidencePage | null>(null);
  const [comparison, setComparison] = useState<ComparePage | null>(null);
  const [counters, setCounters] = useState(true);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  async function load(kind: 'evidence' | 'compare', start = 0) {
    const id = ++generation.current;
    setBusy(true); setError('');
    try {
      if (kind === 'evidence') {
        const page = await getFrameEvidence(fileId, frame, start, counters);
        if (id === generation.current) setEvidence(page);
      } else {
        const page = await compareFrames(fileId, frame, baseline, thread, start);
        if (id === generation.current) setComparison(page);
      }
    } catch (e) { if (id === generation.current) setError(String(e)); }
    finally { if (id === generation.current) setBusy(false); }
  }
  return <details><summary>帧证据与调用路径对比</summary>
    <FrameSections key={`sections:${fileId}:${frame}`} fileId={fileId} frame={frame} />
    <FlowEvents key={`${fileId}:${frame}`} fileId={fileId} frame={frame} frames={frames} />
    <p>按需读取当前帧的全部已导出线程。整数以十进制字符串保留精度；未知类型只显示原始字节。对象 ID 不能直接对应当前 Editor 对象。</p>
    <label><input type="checkbox" checked={counters} disabled={busy} onChange={e => { setCounters(e.target.checked); setEvidence(null); }} />仅 Counter</label>{' '}
    <button disabled={busy} onClick={() => load('evidence')}>读取帧证据</button>
    {evidence && <>
      <p>证据总计 {evidence.total} 条，本页 {evidence.rows.length} 条。{evidence.scope}</p>
      <div style={{ overflowX: 'auto' }}><table className="hotspot-table" aria-label="帧证据"><thead><tr><th>线程 / 样本</th><th>Marker</th><th>字段 / 值 / 单位</th></tr></thead>
        <tbody>{evidence.rows.map(r => <tr key={`${r.threadIndex}:${r.sampleIndex}`}><td>{r.thread} #{r.threadIndex} / {r.sampleIndex}</td><td>{r.marker}</td><td style={{ overflowWrap: 'anywhere' }}>
          {r.metadata.map(m => <div key={m.fieldIndex}>{m.definition?.name || `字段 ${m.fieldIndex}`}{m.definition?.nameTruncated ? '…（字段名截断）' : ''}：{m.value ?? '—'} {m.unit ?? '单位未知'}
            {m.reason && <span> · {m.reason}</span>}
            <details><summary>原始证据</summary>类型 {m.payloadType} · {m.byteLength} 字节 · hex {m.rawHex}{m.byteLength > 64 ? '…（仅前 64 字节）' : ''}</details>
          </div>)}
          {r.metadataTruncated && <p>{r.metadataReason ?? '字段未完整读取（每样本最多 16 项、每帧最多 100000 项）；不能当作完整 metadata。'}</p>}
          {r.metadataCount === 0 && <p>未记录值，不代表零。</p>}
        </td></tr>)}</tbody></table></div>
      <button disabled={busy} onClick={() => load('evidence')}>证据首页</button>{' '}
      <button disabled={busy || evidence.nextStart === null} onClick={() => load('evidence', evidence.nextStart!)}>下一页证据</button>
    </>}
    <p><label>对照帧 <select aria-label="对照帧" disabled={busy} value={baseline} onChange={e => { setBaseline(Number(e.target.value)); setComparison(null); }}>
      {frames.filter(f => f.frameIndex !== frame).map(f => <option key={f.frameIndex} value={f.frameIndex}>{f.frameIndex}</option>)}
    </select></label>{' '}<button disabled={busy || baseline === frame} onClick={() => load('compare')}>比较调用路径</button></p>
    {comparison && <>
      <p>帧 {comparison.frameIndex} 对比 {comparison.baselineFrameIndex} · {comparison.thread.name} · {comparison.total} 条路径。{comparison.interpretation}</p>
      <div style={{ overflowX: 'auto' }}><table className="hotspot-table" aria-label="调用路径对比"><thead><tr><th>完整路径</th><th>调用次数（对照 → 当前）</th><th>Inclusive Δ ms</th><th>Self Δ ms</th><th>直接 GC Δ B</th></tr></thead>
        <tbody>{comparison.rows.map((r,i) => <tr key={i}><td style={{ overflowWrap: 'anywhere' }}>{r.path.join(' → ')}</td><td>{r.baseline.calls} → {r.current.calls}</td><td>{r.inclusiveDeltaMs.toFixed(4)}</td><td>{r.selfDeltaMs === null ? '—' : r.selfDeltaMs.toFixed(4)}</td><td>{r.gcDeltaBytes ?? '—'}</td></tr>)}</tbody></table></div>
      <button disabled={busy} onClick={() => load('compare')}>对比首页</button>{' '}
      <button disabled={busy || comparison.nextStart === null} onClick={() => load('compare', comparison.nextStart!)}>下一页路径</button>
    </>}
    {busy && <p role="status">正在读取帧证据…</p>}
    {error && <p role="alert">{error}</p>}
  </details>;
}
