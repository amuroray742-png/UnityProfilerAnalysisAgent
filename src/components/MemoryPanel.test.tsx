import {render,screen,fireEvent,waitFor} from '@testing-library/react';
import {describe,it,expect,vi} from 'vitest';
import {MemoryPanel} from './MemoryPanel';
import {invoke} from '@tauri-apps/api/core';
import type {MemoryMetrics} from '../types';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
const name='Total Used Memory';
const fixture:MemoryMetrics={counters:{[name]:{peak:'18446744073709551615',peakFrame:10,first:'18446744073709551615',last:'0',delta:'-18446744073709551615',validFrames:2,totalFrames:3,status:'partial',validation:'pending-editor-comparison'}},frames:['18446744073709551615',null,'0'].map((value,i)=>({frameIndex:10+i,versionVerified:false,counters:{[name]:{value,reason:value===null?'缺失':null,sources:[],sourceCount:1}}}))};
describe('memory evidence',()=>{
 it('preserves exact bytes, breaks missing segments and jumps to original frame',()=>{
  const onFrame=vi.fn();const {container}=render(<MemoryPanel metrics={fixture} fileId="" onFrame={onFrame}/>);
  expect(screen.getByText('18446744073709551615')).toBeTruthy();expect(container.querySelectorAll('polyline').length).toBe(2);
  expect(screen.getByText('缺失')).toBeTruthy();fireEvent.click(screen.getByText('查看峰值帧 10 的 CPU / GC'));expect(onFrame).toHaveBeenCalledWith(10);
 });
 it('shows older snapshots as unavailable',()=>{render(<MemoryPanel fileId="" onFrame={()=>{}}/>);expect(screen.getByText(/没有内存指标/)).toBeTruthy();});
 it('uses backend pagination and retains null values',async()=>{
  vi.mocked(invoke).mockResolvedValue({rows:[{frameIndex:55,observation:null}],total:100,nextStart:1});
  render(<MemoryPanel metrics={fixture} fileId="a" onFrame={()=>{}}/>);
  await screen.findByText('55');fireEvent.click(screen.getByText('下一页内存'));
  await waitFor(()=>expect(invoke).toHaveBeenCalledWith('memory_series',{fileId:'a',name,start:1,limit:50}));
 });
});
