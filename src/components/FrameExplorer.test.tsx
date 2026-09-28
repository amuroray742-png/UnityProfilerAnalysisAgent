import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { FrameExplorer } from './FrameExplorer';
import { getCpuHierarchy, getFrameDetails } from '../lib/tauri';
import type { HierarchyPage } from '../types';
vi.mock('../lib/tauri', () => ({ getCpuHierarchy: vi.fn(), getFrameDetails: vi.fn() }));
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
  render(<FrameExplorer fileId="a" frames={[{ frameIndex: 10 }, { frameIndex: 12 }]} />);
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
  render(<FrameExplorer fileId="a" frames={[{ frameIndex: 10 }, { frameIndex: 12 }]} />);
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
  render(<FrameExplorer fileId="a" frames={[{ frameIndex: 10 }, { frameIndex: 12 }]} />);
  fireEvent.change(screen.getByLabelText('帧'), { target: { value: '12' } });
  await screen.findByText('Frame 12');
  resolveOld(page(10));
  await waitFor(() => expect(screen.queryByText('Frame 10')).not.toBeInTheDocument());
  vi.mocked(getCpuHierarchy).mockRejectedValueOnce('该输入没有可用的原始调用树');
  fireEvent.change(screen.getByLabelText('深度'), { target: { value: '3' } });
  expect(await screen.findByRole('alert')).toHaveTextContent('没有可用');
  expect(screen.queryByText('Frame 12')).not.toBeInTheDocument();
});
