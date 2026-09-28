import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { RenderingPanel } from './RenderingPanel';
import type { FrameTimeStats } from '../types';
afterEach(cleanup);
const stats = (value: number | null, partial = false): FrameTimeStats => ({
  p50: value, p95: value, p99: value, max: value,
  quality: { status: value === null ? 'unavailable' : partial ? 'partial' : 'available',
    source: 'public-fixture', reasons: value === null ? ['缺少计数'] : [], validFrames: value === null ? 0 : partial ? 1 : 2, totalFrames: 2 },
});
it('shows rendering counts, real zero, unavailable metrics and partial coverage without GPU claims', () => {
  const { container } = render(<RenderingPanel metrics={{ drawCalls: stats(0), setPassCalls: stats(null),
    batches: stats(12, true), triangles: stats(123), vertices: stats(456), batchesSavedBySrpBatcher: null,
    eventQuality: stats(0).quality, topRenderEvents: [] }} />);
  const card = (label: string) => screen.getByText(label).closest('.metric-card')!;
  expect(card('Draw Call p95').querySelector('.metric-value')).toHaveTextContent('0');
  expect(card('SetPass p95').querySelector('.metric-value')).toHaveTextContent('—');
  expect(card('SetPass p95')).toHaveTextContent('缺少计数');
  expect(card('Batches p95')).toHaveTextContent('部分可用 · 有效帧 1/2');
  expect(card('Triangles p95').querySelector('.metric-value')).toHaveTextContent('123');
  expect(card('Vertices p95').querySelector('.metric-value')).toHaveTextContent('456');
  expect(container.querySelector('.danger')).toBeNull();
  expect(screen.getByText(/不代表 GPU 时间/)).toBeInTheDocument();
});
