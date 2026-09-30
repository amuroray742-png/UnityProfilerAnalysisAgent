import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { MemoryMetrics, MemoryObservation } from '../types';
type Row={frameIndex:number;observation:MemoryObservation|null};
type Page={rows:Row[];nextStart:number|null;total:number};
export function MemoryPanel({metrics,fileId,onFrame}:{metrics?:MemoryMetrics;fileId:string;onFrame:(index:number)=>void}){
 const names=Object.keys(metrics?.counters??{});
 const [name,setName]=useState(names[0]??''),[start,setStart]=useState(0),[remote,setRemote]=useState<Page|null>(null),[error,setError]=useState('');
 useEffect(()=>{let live=true;setRemote(null);setError('');if(fileId&&name)void invoke<Page>('memory_series',{fileId,name,start,limit:50}).then(p=>{if(live)setRemote(p);}).catch(e=>{if(live)setError(String(e));});return()=>{live=false;};},[fileId,name,start]);
 if(!metrics||!names.length)return <p>该存档没有内存指标，请重新导入录制。</p>;
 const summary=metrics.counters[name];
 const page=remote??{rows:metrics.frames.slice(start,start+50).map(f=>({frameIndex:f.frameIndex,observation:f.counters[name]??null})),total:metrics.frames.length,nextStart:start+50<metrics.frames.length?start+50:null};
 const values=page.rows.map(r=>r.observation?.value==null?null:BigInt(r.observation.value));
 const max=values.reduce<bigint>((a,v)=>v!==null&&v>a?v:a,0n);
 const xy=(i:number,v:bigint)=>`${20+i*560/Math.max(1,values.length-1)},${150-(max?Number(v*13000n/max)/100:0)}`;
 const segments:string[]=[];let line:string[]=[];
 values.forEach((v,i)=>{if(v===null){if(line.length)segments.push(line.join(' '));line=[];}else line.push(xy(i,v));});if(line.length)segments.push(line.join(' '));
 return <section aria-label="内存指标"><h3>内存 Counter</h3><p>内存指标待 Editor 对照。单位为字节；首末变化不证明泄漏，缺失点不连线。</p>
 <label>指标 <select value={name} onChange={e=>{setName(e.target.value);setStart(0);}}>{names.map(n=><option key={n}>{n}</option>)}</select></label>
 <p>峰值 {summary?.peak??'—'} B · 首个有效值 {summary?.first??'—'} B → 末个有效值 {summary?.last??'—'} B · 变化 {summary?.delta??'—'} B · 覆盖 {summary?.validFrames??0}/{summary?.totalFrames??0} 帧</p>
 {summary?.peakFrame!=null&&<button onClick={()=>onFrame(summary.peakFrame!)}>查看峰值帧 {summary.peakFrame} 的 CPU / GC</button>}
 <svg viewBox="0 0 600 175" role="img" aria-label="当前分页内存曲线" style={{width:'100%',maxWidth:800}}><text x="20" y="12" fill="currentColor">本页最大值 {max.toString()} B</text><line x1="20" y1="150" x2="580" y2="150" stroke="currentColor"/>{segments.map((s,i)=><polyline key={i} points={s} fill="none" stroke="#61b6ff" strokeWidth="2"/>)}{values.map((v,i)=>v===null?null:<circle key={i} cx={Number(xy(i,v).split(',')[0])} cy={Number(xy(i,v).split(',')[1])} r="3" fill="#61b6ff"><title>帧 {page.rows[i].frameIndex}：{v.toString()} B</title></circle>)}</svg>
 <table className="hotspot-table"><thead><tr><th>帧</th><th>bytes</th><th>证据 / 原因</th><th>调用树</th></tr></thead><tbody>{page.rows.map(r=><tr key={r.frameIndex}><td>{r.frameIndex}</td><td>{r.observation?.value??'—'}</td><td>{r.observation?.reason??`${r.observation?.sourceCount??0} 个观测`}</td><td><button onClick={()=>onFrame(r.frameIndex)}>查看 CPU / GC</button></td></tr>)}</tbody></table>
 <button disabled={start===0} onClick={()=>setStart(Math.max(0,start-50))}>上一页内存</button><button disabled={page.nextStart===null} onClick={()=>setStart(page.nextStart!)}>下一页内存</button>{error&&<p role="alert">{error}</p>}</section>;
}
