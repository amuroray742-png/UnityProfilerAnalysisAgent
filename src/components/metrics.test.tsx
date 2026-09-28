import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { MetricCard } from './MetricCard';
import { HotspotTable } from './HotspotTable';
import type { Quality } from '../types';
afterEach(cleanup);
const quality=(status:Quality['status']):Quality=>({status,source:'fixture',reasons:status==='unavailable'?['缺少数据']:[],validFrames:status==='unavailable'?0:1,totalFrames:2});
describe('quality and units',()=>{
  it('distinguishes zero bytes from missing data',()=>{
    const {rerender}=render(<MetricCard label="GC" value={0} unit="bytes" quality={quality('available')}/>);
    expect(screen.getByText('0 B')).toBeInTheDocument();
    rerender(<MetricCard label="GC" value={null} unit="bytes" quality={quality('unavailable')}/>);
    expect(screen.getByText('—')).toBeInTheDocument();
    expect(screen.getByText('缺少数据')).toBeInTheDocument();
  });
  it('shows byte scale once and applies thresholds in bytes',()=>{
    const {container}=render(<MetricCard label="GC" value={8*1024*1024} unit="bytes" thresholds={{warning:4*1024*1024,danger:16*1024*1024}} quality={quality('available')}/>);
    expect(screen.getByText('8.0 MB')).toBeInTheDocument();
    expect(container.firstChild).toHaveClass('warning');
  });
  it('partial and unavailable values do not receive normal threshold diagnosis',()=>{
    const {container,rerender}=render(<MetricCard label="CPU" value={100} unit="ms" thresholds={{warning:10,danger:20}} quality={quality('partial')}/>);
    expect(screen.getByText(/有效帧 1\/2/)).toBeInTheDocument();
    expect(container.firstChild).not.toHaveClass('danger');
    rerender(<MetricCard label="CPU" value={100} unit="ms" thresholds={{warning:10,danger:20}} quality={quality('unavailable')}/>);
    expect(screen.getByText('—')).toBeInTheDocument();
    expect(container.firstChild).not.toHaveClass('danger');
  });
  it('allocation table uses byte columns and thread attribution',()=>{
    render(<HotspotTable title="GC sites" valueColumn="bytes" hotspots={[{name:'Update',thread:'Main Thread #0',totalBytes:2048,avgBytes:1024,maxBytes:1024,callCount:2}]}/>);
    expect(screen.getByText('总字节')).toBeInTheDocument();
    expect(screen.getByText('2.0 KB')).toBeInTheDocument();
    expect(screen.getByText('Main Thread #0 / Update')).toBeInTheDocument();
    expect(screen.queryByText('总耗时')).not.toBeInTheDocument();
  });
});
