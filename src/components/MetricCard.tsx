import { formatBytes, formatCount, formatMs } from '../lib/format';
import type { Quality } from '../types';
export type Severity = 'normal' | 'warning' | 'danger';
interface Props {
  label: string; value: number | null; unit: 'ms' | 'bytes' | 'count';
  detail?: string; quality?: Quality; thresholds?: { warning: number; danger: number };
}
export function QualityNote({ quality }: { quality?: Quality }) {
  if (!quality) return null;
  const labels = { available: '可用', partial: '部分可用', unavailable: '不可用', estimated: '估算', unverified: '版本待验证' };
  return <div className="metric-detail" title={quality.reasons.join('；')}>
    {labels[quality.status]} · 有效帧 {quality.validFrames}/{quality.totalFrames} · {quality.source}
    {quality.reasons.length > 0 && <div>{quality.reasons.join('；')}</div>}
  </div>;
}
export function MetricCard({ label, value, unit, detail, quality, thresholds }: Props) {
  const usable = value !== null && Number.isFinite(value) && quality?.status !== 'unavailable';
  const classify = usable && (!quality || quality.status === 'available') && thresholds;
  const severity: Severity = classify && value! >= classify.danger ? 'danger'
    : classify && value! >= classify.warning ? 'warning' : 'normal';
  const text = !usable ? '—' : unit === 'bytes' ? formatBytes(value!)
    : unit === 'ms' ? formatMs(value!) : formatCount(value!);
  return <div className={`metric-card ${severity}`}>
    <div className="metric-label">{label}</div><div className="metric-value">{text}</div>
    {detail && <div className="metric-detail">{detail}</div>}
    <QualityNote quality={quality} />
  </div>;
}
