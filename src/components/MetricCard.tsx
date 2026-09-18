import { formatCount, formatMs } from '../lib/format.ts';

export type Severity = 'normal' | 'warning' | 'danger';

interface MetricCardProps {
  label: string;
  value: number;
  unit: 'ms' | 'bytes' | 'count';
  detail?: string;
  /** p95 阈值（ms），超过 → warning；超过 → danger */
  thresholds?: { warning: number; danger: number };
}

function classify(value: number, thresholds?: MetricCardProps['thresholds']): Severity {
  if (!thresholds) return 'normal';
  if (value >= thresholds.danger) return 'danger';
  if (value >= thresholds.warning) return 'warning';
  return 'normal';
}

function formatValue(value: number, unit: MetricCardProps['unit']): string {
  if (unit === 'ms') return formatMs(value).replace(/[^\d.]/g, '');
  if (unit === 'bytes') return formatBytesShort(value);
  return formatCount(value);
}

function formatBytesShort(bytes: number): string {
  if (bytes < 1024) return `${bytes}`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)}`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)}`;
}

function unitLabel(unit: MetricCardProps['unit']): string {
  if (unit === 'ms') return 'ms';
  if (unit === 'bytes') return 'MB';
  return '';
}

export function MetricCard({ label, value, unit, detail, thresholds }: MetricCardProps) {
  const severity = classify(value, thresholds);
  return (
    <div className={`metric-card ${severity}`}>
      <div className="metric-label">{label}</div>
      <div className="metric-value">
        {formatValue(value, unit)}
        <span className="metric-unit">{unitLabel(unit)}</span>
      </div>
      {detail && <div className="metric-detail">{detail}</div>}
    </div>
  );
}