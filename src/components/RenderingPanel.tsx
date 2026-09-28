import type { RenderingMetrics } from '../types';
import { MetricCard } from './MetricCard';
import { HotspotTable } from './HotspotTable';

export function RenderingPanel({ metrics }: { metrics: RenderingMetrics }) {
  return <>
    <div className="metrics-grid">
      {([
        ['Draw Call', metrics.drawCalls], ['SetPass', metrics.setPassCalls],
        ['Batches', metrics.batches], ['Triangles', metrics.triangles], ['Vertices', metrics.vertices],
      ] as const).map(([label, stats]) => <MetricCard key={label} label={`${label} p95`}
        value={stats.p95} quality={stats.quality} unit="count" detail={`p50 ${stats.p50 ?? '—'} · max ${stats.max ?? '—'}`} />)}
      <MetricCard label="SRP Batcher 节省" value={metrics.batchesSavedBySrpBatcher}
        unit="count" detail="输入未提供独立观测值" />
    </div>
    <p>计数按有效帧统计，缺失帧不按零值处理。渲染 CPU marker 包含子样本和等待，不能相加为总耗时，也不代表 GPU 时间；仅凭 Draw Call 数量不能判定 GPU 瓶颈。</p>
    <HotspotTable title="渲染 CPU marker 热点" quality={metrics.eventQuality}
      hotspots={metrics.topRenderEvents} valueColumn="ms" />
  </>;
}
