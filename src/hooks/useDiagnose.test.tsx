import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useDiagnose } from './useDiagnose';
import { analyzeProfiler, releaseProfiler, uploadProfiler, diagnose, cancelDiagnose, listAgents, onDiagnoseEvent } from '../lib/tauri';
import type { MetricsSnapshot, UploadResult, DiagnoseEvent } from '../types';
vi.mock('../lib/tauri', () => ({
  uploadProfiler: vi.fn(), analyzeProfiler: vi.fn(), releaseProfiler: vi.fn(),
  diagnose: vi.fn(), cancelDiagnose: vi.fn(), listAgents: vi.fn(async () => []),
  onDiagnoseEvent: vi.fn(async () => () => {}),
}));
beforeEach(() => {
  vi.mocked(cancelDiagnose).mockResolvedValue(undefined);
  vi.mocked(releaseProfiler).mockResolvedValue(undefined);
  vi.mocked(uploadProfiler).mockResolvedValue({ fileId: 'a', filename: 'a', sizeBytes: 0, extension: 'json' });
});

async function readyHook() {
  let emit!: (event: DiagnoseEvent) => void;
  vi.mocked(onDiagnoseEvent).mockImplementation(async handler => { emit = handler; return () => {}; });
  vi.mocked(listAgents).mockResolvedValue([{id:'agent',label:'test',command:'test',args:[],available:true}]);
  vi.mocked(analyzeProfiler).mockResolvedValue({} as MetricsSnapshot);
  const hook=renderHook(useDiagnose);
  await act(async()=>{await hook.result.current.handleFile('a');});
  await waitFor(()=>expect(hook.result.current.state.selectedAgent).toBe('agent'));
  return {...hook,emit:(event: DiagnoseEvent)=>act(()=>emit(event))};
}
it('buffers early session events and never changes an error terminal to success',async()=>{
  const hook=await readyHook();
  let resolve!: (value:{sessionId:string})=>void;
  vi.mocked(diagnose).mockImplementationOnce(()=>new Promise(r=>{resolve=r;}));
  let pending!:Promise<void>;
  act(()=>{pending=hook.result.current.startDiagnose();});
  hook.emit({kind:'chunk',text:'wrong',fileId:'a',sessionId:'old'});
  hook.emit({kind:'chunk',text:'correct',fileId:'a',sessionId:'new'});
  await act(async()=>{resolve({sessionId:'new'});await pending;});
  expect(hook.result.current.state.streamedText).toBe('correct');
  hook.emit({kind:'log',message:'stderr',fileId:'a',sessionId:'new'});
  expect(hook.result.current.state.phase).toBe('diagnosing');
  hook.emit({kind:'error',message:'failed',fileId:'a',sessionId:'new'});
  hook.emit({kind:'finished',totalChunks:1,stopReason:'end_turn',fileId:'a',sessionId:'new'});
  expect(hook.result.current.state.phase).toBe('error');
  expect(hook.result.current.state.errorMessage).toBe('failed');
});
it('cancels by session ID and rejects old events after retry',async()=>{
  const hook=await readyHook();
  vi.mocked(diagnose).mockResolvedValueOnce({sessionId:'one'}).mockResolvedValueOnce({sessionId:'two'});
  await act(async()=>{await hook.result.current.startDiagnose();});
  await act(async()=>{await hook.result.current.cancel();});
  expect(cancelDiagnose).toHaveBeenCalledWith('one');
  await act(async()=>{await hook.result.current.startDiagnose();});
  hook.emit({kind:'chunk',text:'stale',fileId:'a',sessionId:'one'});
  hook.emit({kind:'chunk',text:'current',fileId:'a',sessionId:'two'});
  expect(hook.result.current.state.streamedText).toBe('current');
  act(()=>hook.result.current.reset());
  expect(cancelDiagnose).toHaveBeenCalledWith('two');
  hook.emit({kind:'error',message:'late',fileId:'a',sessionId:'two'});
  expect(hook.result.current.state.phase).toBe('idle');
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });
it('reset releases a pending analysis and ignores its late result', async () => {
  let resolve!: (s: MetricsSnapshot) => void;
  vi.mocked(analyzeProfiler).mockImplementation(() => new Promise(r => { resolve = r; }));
  const { result } = renderHook(useDiagnose);
  let pending!: Promise<void>;
  act(() => { pending = result.current.handleFile('a'); });
  await waitFor(() => expect(result.current.state.phase).toBe('analyzing'));
  act(() => result.current.reset());
  expect(releaseProfiler).toHaveBeenCalledWith('a');
  await act(async () => { resolve({} as MetricsSnapshot); await pending; });
  expect(result.current.state.phase).toBe('idle');
  expect(result.current.state.snapshot).toBeNull();
});
it('a late upload is released without starting analysis', async () => {
  let resolve!: (s: UploadResult) => void;
  vi.mocked(uploadProfiler).mockImplementationOnce(() => new Promise(r => { resolve = r; }));
  const { result } = renderHook(useDiagnose);
  let pending!: Promise<void>;
  act(() => { pending = result.current.handleFile('a'); });
  act(() => result.current.reset());
  await act(async () => { resolve({ fileId: 'late', filename: 'a', sizeBytes: 0, extension: 'json' }); await pending; });
  expect(releaseProfiler).toHaveBeenCalledWith('late');
  expect(analyzeProfiler).not.toHaveBeenCalled();
  expect(result.current.state.upload).toBeNull();
});
