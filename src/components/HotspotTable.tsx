import type { Hotspot, AllocHotspot, Quality } from '../types';
import { formatBytes, formatCount, formatMs } from '../lib/format';
import { QualityNote } from './MetricCard';
type Props = { title: string; topN?: number; quality?: Quality } & (
  { valueColumn: 'ms'; hotspots: Hotspot[] } | { valueColumn: 'bytes'; hotspots: AllocHotspot[] }
);
export function HotspotTable(props: Props) {
  const {title,topN=10,quality}=props;
  const rows=props.valueColumn==='bytes'
    ? props.hotspots.map(h=>({name:`${h.thread} / ${h.name}`,total:h.totalBytes,avg:h.avgBytes,max:h.maxBytes,calls:h.callCount}))
    : props.hotspots.map(h=>({name:h.name,total:h.totalMs,avg:h.avgMs,max:h.maxMs,calls:h.callCount}));
  rows.sort((a,b)=>b.total-a.total);
  const fmt=props.valueColumn==='bytes'?formatBytes:formatMs;
  return <div className="section"><div className="section-title">{title}</div>
    <QualityNote quality={quality} />
    {props.valueColumn==='ms' && <p>Inclusive 耗时包含子样本，父子耗时不可相加。</p>}
    {rows.length===0 ? <p>{quality?.status==='unavailable'?'不可用':'无样本'}</p> :
    <table className="hotspot-table"><thead><tr><th>名称</th><th>{props.valueColumn==='bytes'?'总字节':'总耗时'}</th><th>调用次数</th><th>平均</th><th>峰值</th></tr></thead>
    <tbody>{rows.slice(0,topN).map((r,i)=><tr key={i}><td>{r.name}</td><td>{fmt(r.total)}</td><td>{formatCount(r.calls)}</td><td>{fmt(r.avg)}</td><td>{fmt(r.max)}</td></tr>)}</tbody></table>}
  </div>;
}
