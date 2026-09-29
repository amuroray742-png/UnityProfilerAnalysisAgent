import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { FrameEvidence } from './FrameEvidence';
import { compareFrames, getFrameEvidence } from '../lib/tauri';
import type { EvidencePage } from '../types';
vi.mock('../lib/tauri', () => ({ compareFrames: vi.fn(), getFrameEvidence: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
const evidence: EvidencePage = { frameIndex: 10, source: 'data', total: 2, nextStart: 1, scope: '分页', rows: [{
 threadIndex: 0, threadId: '1', thread: 'Main Thread', sampleIndex: 3, markerId: 8, marker: 'Memory', isCounter: true, metadataCount: 2, metadataTruncated: false,
 metadata: [{ fieldIndex: 0, definition: null, payloadType: 5, byteLength: 8, value: '18446744073709551615', unit: 'bytes', status: 'available', reason: null, rawHex: 'ffffffffffffffff' },
 { fieldIndex: 1, definition: null, payloadType: 88, byteLength: 80, value: null, unit: null, status: 'unavailable', reason: '未知类型', rawHex: '01' }],
}] };
it('keeps wide integers and unavailable distinct, follows pagination and compares explicit frames', async () => {
 vi.mocked(getFrameEvidence).mockResolvedValue(evidence);
 vi.mocked(compareFrames).mockResolvedValue({ frameIndex: 10, baselineFrameIndex: 12, thread: { threadIndex: 0, threadId: '1', name: 'Main Thread', group: null, sampleCount: 1 }, rows: [], total: 0, nextStart: null, interpretation: '对照不证明正常' });
 render(<FrameEvidence fileId="capture" frame={10} thread={null} frames={[{ frameIndex: 10 }, { frameIndex: 12 }]} />);
 fireEvent.click(screen.getByText('帧证据与调用路径对比'));
 fireEvent.click(screen.getByText('读取帧证据'));
 await screen.findByText(/18446744073709551615/);
 expect(screen.getAllByText(/未知类型/).length).toBeGreaterThan(0);
 fireEvent.click(screen.getByText('下一页证据'));
 await waitFor(() => expect(getFrameEvidence).toHaveBeenLastCalledWith('capture', 10, 1, true));
 await waitFor(() => expect(screen.getByText('比较调用路径')).not.toBeDisabled());
 fireEvent.click(screen.getByText('比较调用路径'));
 await screen.findByText(/对照不证明正常/);
 expect(compareFrames).toHaveBeenCalledWith('capture', 10, 12, null, 0);
});
it('unmounted capture ignores delayed evidence and errors remain retryable', async () => {
 let resolve!: (value: EvidencePage) => void;
 vi.mocked(getFrameEvidence).mockImplementationOnce(() => new Promise(r => { resolve = r; }));
 const app=render(<FrameEvidence key="a" fileId="a" frame={10} thread={null} frames={[{ frameIndex: 10 }]} />);
 fireEvent.click(screen.getByText('帧证据与调用路径对比'));fireEvent.click(screen.getByText('读取帧证据'));
 app.rerender(<FrameEvidence key="b" fileId="b" frame={10} thread={null} frames={[{ frameIndex: 10 }]} />);
 resolve(evidence);
 expect(screen.queryByText(/18446744073709551615/)).not.toBeInTheDocument();
 vi.mocked(getFrameEvidence).mockRejectedValueOnce(new Error('文件已变化'));
 fireEvent.click(screen.getByText('帧证据与调用路径对比'));fireEvent.click(screen.getByText('读取帧证据'));
 await screen.findByRole('alert');expect(screen.getByText('读取帧证据')).not.toBeDisabled();
});
