import { useEffect, useState } from 'react';
import { getFrameDetails, getCpuHierarchy } from '../lib/tauri';
import type { FramePage, HierarchyPage } from '../types';

export function FrameExplorer({ fileId, frames }: { fileId: string; frames: Array<{ frameIndex: number }> }) {
  const [frame, setFrame] = useState(frames[0]?.frameIndex ?? 0);
  const [thread, setThread] = useState<number | null>(null);
  const [depth, setDepth] = useState(8);
  const [threadStart, setThreadStart] = useState(0);
  const [offsets, setOffsets] = useState([0]);
  const [metadata, setMetadata] = useState<FramePage | null>(null);
  const [page, setPage] = useState<HierarchyPage | null>(null);
  const [error, setError] = useState('');
  const start = offsets[offsets.length - 1];
  useEffect(() => {
    let active = true;
    setMetadata(null); setError('');
    getFrameDetails(fileId, frame, threadStart).then(p => { if (active) setMetadata(p); })
      .catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [fileId, frame, threadStart]);
  useEffect(() => {
    let active = true;
    setPage(null); setError('');
    getCpuHierarchy(fileId, frame, thread, start, 200, depth).then(p => { if (active) setPage(p); })
      .catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [fileId, frame, thread, start, depth]);
  return <section className="section" aria-label="原始调用树">
    <h3>单帧调用树</h3>
    <p>耗时包含子样本；父子耗时不可相加为总 CPU。GC 列仅展示已校验的分配样本字节数。</p>
    <label>帧 <select aria-label="帧" value={frame} onChange={e => {
      setFrame(Number(e.target.value)); setThread(null); setThreadStart(0); setOffsets([0]);
    }}>{frames.map(f => <option key={f.frameIndex} value={f.frameIndex}>{f.frameIndex}</option>)}</select></label>{' '}
    <label>线程 <select aria-label="线程" value={thread ?? ''} onChange={e => {
      setThread(e.target.value === '' ? null : Number(e.target.value)); setOffsets([0]);
    }}><option value="">Main Thread（唯一匹配）</option>
      {metadata?.threads.map(t => <option key={t.threadIndex} value={t.threadIndex}>{t.name} #{t.threadIndex}</option>)}
    </select></label>{' '}
    {threadStart > 0 && <button onClick={() => { setThreadStart(0); setThread(null); setOffsets([0]); }}>首批线程</button>}
    {metadata?.nextStart != null && <button onClick={() => { setThreadStart(metadata.nextStart!); setThread(null); setOffsets([0]); }}>更多线程</button>}{' '}
    <label>深度 <select aria-label="深度" value={depth} onChange={e => { setDepth(Number(e.target.value)); setOffsets([0]); }}>
      {[3, 8, 16, 64].map(d => <option key={d}>{d}</option>)}
    </select></label>
    {error && <p role="alert">{error}</p>}
    {!page && !error && <p role="status">正在读取调用树…</p>}
    {page && <>
      <p>帧 {page.info.frameIndex} · 原始帧 ID {page.info.rawFrameId ?? '—'} · {page.info.source} · 线程 ID {page.thread.threadId} · {page.thread.sampleCount} 个样本 · 帧 GC {page.info.gcAllocBytes === null ? '—' : `${page.info.gcAllocBytes} B`}</p>
      {page.info.warnings.map((w, i) => <p key={i}>{w}</p>)}
      {page.depthTruncated && <p>存在超出当前深度的样本，请增加深度查看。</p>}
      <div style={{ overflowX: 'auto' }}><table className="hotspot-table"><thead><tr><th>样本 / 父样本</th><th>Marker</th><th>Inclusive ms</th><th>GC 字节</th></tr></thead>
        <tbody>{page.samples.map(s => <tr key={s.sampleIndex}>
          <td>{s.sampleIndex} / {s.parentIndex ?? '—'}</td>
          <td style={{ paddingLeft: Math.min(s.depth, 16) * 12 }}>{s.name} <small>#{s.markerId} · 深度 {s.depth}</small></td>
          <td>{s.totalMs.toFixed(4)}</td><td>{s.gcAllocBytes === null ? '—' : `${s.gcAllocBytes} B`}</td>
        </tr>)}</tbody></table></div>
      <button disabled={offsets.length === 1} onClick={() => setOffsets(a => a.slice(0, -1))}>上一页样本</button>{' '}
      <button disabled={page.nextStart === null} onClick={() => setOffsets(a => [...a, page.nextStart!])}>下一页样本</button>
    </>}
  </section>;
}
