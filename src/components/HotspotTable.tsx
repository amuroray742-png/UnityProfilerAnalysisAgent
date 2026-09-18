import type { Hotspot } from '../types/index.ts';
import { formatCount, formatMs } from '../lib/format.ts';

interface HotspotTableProps {
  title: string;
  hotspots: Hotspot[];
  /** 按 totalMs / bytes 排序后取 top N */
  topN?: number;
  /** 列：耗时 / 字节 */
  valueColumn: 'ms' | 'bytes';
}

export function HotspotTable({ title, hotspots, topN = 10, valueColumn }: HotspotTableProps) {
  const rows = [...hotspots]
    .sort((a, b) => (valueColumn === 'ms' ? b.totalMs - a.totalMs : b.totalMs - a.totalMs))
    .slice(0, topN);

  if (rows.length === 0) {
    return (
      <div className="section">
        <div className="section-title">{title}</div>
        <div style={{ color: 'var(--text-secondary)', fontSize: 12, fontStyle: 'italic' }}>
          无数据
        </div>
      </div>
    );
  }

  return (
    <div className="section">
      <div className="section-title">{title}</div>
      <table className="hotspot-table">
        <thead>
          <tr>
            <th>名称</th>
            <th className="col-numeric">总耗时</th>
            <th className="col-numeric">调用次数</th>
            <th className="col-numeric">平均</th>
            <th className="col-numeric">峰值</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((h, i) => (
            <tr key={i}>
              <td>{h.name}</td>
              <td className="col-numeric">{formatMs(h.totalMs)}</td>
              <td className="col-numeric">{formatCount(h.callCount)}</td>
              <td className="col-numeric">{formatMs(h.avgMs)}</td>
              <td className="col-numeric">{formatMs(h.maxMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}