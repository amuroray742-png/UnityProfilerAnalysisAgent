import { render, screen, cleanup, fireEvent, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { FrameExplorer } from './FrameExplorer';
import { getCpuHierarchy, getFrameDetails } from '../lib/tauri';
import type { HierarchyPage, Quality } from '../types';
vi.mock('../lib/tauri', () => ({ getCpuHierarchy: vi.fn(), getFrameDetails: vi.fn() }));
const quality: Quality = { status: 'available', source: 'fixture', reasons: [], validFrames: 2, totalFrames: 2 };
const frames = [{ frameIndex: 10, ms: 1, frameTimeMs: 2 }, { frameIndex: 12, ms: 2, frameTimeMs: null }];
const page = (index: number): HierarchyPage => ({
  info: { frameIndex: index, rawFrameId: null, rawDuplicateId: null, startNs: null, source: 'fixture', cpuMs: 1, frameTimeMs: 2, gcAllocBytes: 0, warnings: [] },
  thread: { threadIndex: 17, threadId: '18446744073709551615', name: 'Main Thread', group: null, sampleCount: 3 },
  samples: [{ sampleIndex: 0, parentIndex: null, depth: 0, markerId: 1, name: `Frame ${index}`, categoryIndex: null, totalMs: 1, startMs: 0, rawStartNs: null, rawDurationNs: null, childrenCount: 0, metadataCount: 1, gcAllocBytes: 0 }],
  nextStart: 2, maxDepth: 8, depthTruncated: true,
});
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(getCpuHierarchy).mockImplementation(async (_id, index) => page(index));
  vi.mocked(getFrameDetails).mockImplementation(async (_id, index) => ({ info: page(index).info, threads: [page(index).thread], threadCount: 1, nextStart: null }));
});
afterEach(cleanup);
it('lists Worker only in frame 10 and queries its thread index', async () => {
  vi.mocked(getFrameDetails).mockImplementation(async (_id, index) => ({
    info: page(index).info,
    threads: index === 10
      ? [{ ...page(index).thread, threadIndex: 0 }, { ...page(index).thread, threadIndex: 1, name: 'Worker', sampleCount: 2 }]
      : [{ ...page(index).thread, threadIndex: 0 }],
    threadCount: index === 10 ? 2 : 1, nextStart: null,
  }));
  render(<FrameExplorer fileId="a" frames={frames} quality={quality} />);
  await screen.findByRole('option', { name: 'Worker #1' });
  fireEvent.change(screen.getByLabelText('线程'), { target: { value: '1' } });
  await waitFor(() => expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 10, 1, 0, 200, 8));
  fireEvent.change(screen.getByLabelText('帧'), { target: { value: '12' } });
  await screen.findByText('Frame 12');
  expect(screen.queryByRole('option', { name: 'Worker #1' })).not.toBeInTheDocument();
  expect(screen.getByLabelText('线程')).toHaveValue('');
  fireEvent.change(screen.getByLabelText('帧'), { target: { value: '10' } });
  await screen.findByRole('option', { name: 'Worker #1' });
});
it('queries original sparse frame IDs, page offsets and depth; preserves byte zero', async () => {
  render(<FrameExplorer fileId="a" frames={frames} quality={quality} />);
  await screen.findByText('Frame 10');
  expect(screen.getByText('0 B')).toBeInTheDocument();
  expect(screen.getByText(/18446744073709551615/)).toBeInTheDocument();
  expect(screen.getByText(/存在超出当前深度/)).toBeInTheDocument();
  fireEvent.click(screen.getByText('下一页样本'));
  await waitFor(() => expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 10, null, 2, 200, 8));
  fireEvent.change(screen.getByLabelText('帧'), { target: { value: '12' } });
  await screen.findByText('Frame 12');
  expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 12, null, 0, 200, 8);
  fireEvent.change(screen.getByLabelText('深度'), { target: { value: '64' } });
  await waitFor(() => expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 12, null, 0, 200, 64));
});
it('ignores an old frame response and clears old rows on unavailable input', async () => {
  let resolveOld!: (value: HierarchyPage) => void;
  vi.mocked(getCpuHierarchy).mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }));
  render(<FrameExplorer fileId="a" frames={frames} quality={quality} />);
  fireEvent.change(screen.getByLabelText('帧'), { target: { value: '12' } });
  await screen.findByText('Frame 12');
  resolveOld(page(10));
  await waitFor(() => expect(screen.queryByText('Frame 10')).not.toBeInTheDocument());
  vi.mocked(getCpuHierarchy).mockRejectedValueOnce('该输入没有可用的原始调用树');
  fireEvent.change(screen.getByLabelText('深度'), { target: { value: '3' } });
  expect(await screen.findByRole('alert')).toHaveTextContent('没有可用');
  expect(screen.queryByText('Frame 12')).not.toBeInTheDocument();
});

it('ranks CPU independently of recorded frame time, excludes missing values and keeps zero', async () => {
  render(<FrameExplorer fileId="a" quality={{ ...quality, status: 'partial', validFrames: 3, totalFrames: 4 }} frames={[
    { frameIndex: 10, ms: 1, frameTimeMs: 90 },
    { frameIndex: 12, ms: 20, frameTimeMs: null },
    { frameIndex: 30, ms: null, frameTimeMs: 100 },
    { frameIndex: 44, ms: 0, frameTimeMs: 0 },
  ]} />);
  await screen.findByText('Frame 10');
  const rows = within(screen.getByRole('table', { name: '慢帧列表' })).getAllByRole('row').slice(1);
  expect(rows.map(row => within(row).getAllByRole('cell')[0].textContent)).toEqual(['12', '10', '44']);
  expect(within(rows[0]).getAllByRole('cell')[2]).toHaveTextContent('—');
  expect(within(rows[2]).getAllByRole('cell')[1]).toHaveTextContent('0.0000');
  expect(screen.getByText(/可查看 3\/4 帧/)).toBeInTheDocument();
  expect(screen.getByText(/缺失帧不按零值处理/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('线程'), { target: { value: '17' } });
  fireEvent.click(await screen.findByText('下一页样本'));
  await waitFor(() => expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 10, 17, 2, 200, 8));
  fireEvent.click(screen.getByRole('button', { name: '查看帧 12 调用树' }));
  await screen.findByText('Frame 12');
  expect(screen.getByLabelText('帧')).toHaveValue('12');
  expect(screen.getByLabelText('线程')).toHaveValue('');
  expect(getCpuHierarchy).toHaveBeenLastCalledWith('a', 12, null, 0, 200, 8);
});
it('limits ranking to 20 rows and explicitly labels estimated CPU', async () => {
  render(<FrameExplorer fileId="a" quality={{ ...quality, status: 'estimated' }} frames={Array.from({ length: 25 }, (_, i) => ({ frameIndex: i * 2, ms: i, frameTimeMs: null }))} />);
  await screen.findByText('Frame 0');
  expect(screen.getByText(/主线程时间为估算值/)).toBeInTheDocument();
  expect(within(screen.getByRole('table', { name: '慢帧列表' })).getAllByRole('row')).toHaveLength(21);
  expect(screen.getByRole('button', { name: '查看帧 48 调用树' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '查看帧 0 调用树' })).not.toBeInTheDocument();
  expect(screen.getByLabelText('帧').querySelectorAll('option')).toHaveLength(25);
});
it('does not invent slow frames when CPU is unavailable', async () => {
  render(<FrameExplorer fileId="a" quality={{ ...quality, status: 'unavailable', validFrames: 0 }} frames={[{ frameIndex: 10, ms: null, frameTimeMs: 99 }]} />);
  await screen.findByText('Frame 10');
  expect(screen.getByText('没有可用于慢帧排序的主线程时间。')).toBeInTheDocument();
  expect(screen.queryByRole('table', { name: '慢帧列表' })).not.toBeInTheDocument();
});
