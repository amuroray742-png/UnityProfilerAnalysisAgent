import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { MetricsSnapshot } from '../types';
import { MetricCard } from './MetricCard';
import { HotspotTable } from './HotspotTable';
import { FrameExplorer } from './FrameExplorer';
import { RenderingPanel } from './RenderingPanel';

export function CaptureInspector({captureId}:{captureId:string}){
 const [snapshot,setSnapshot]=useState<MetricsSnapshot|null>(null),[fileId,setFileId]=useState(''),[error,setError]=useState(''),[tab,setTab]=useState('cpu'),[loading,setLoading]=useState(false);
 const mounted=useRef(true);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 useEffect(()=>{let live=true;invoke<MetricsSnapshot>('workflow_command',{action:{op:'snapshot',captureId}}).then(s=>{if(live)setSnapshot(s);}).catch(e=>{if(live)setError(String(e));});return()=>{live=false;};},[captureId]);
 useEffect(()=>()=>{if(fileId)void invoke('release_file',{fileId});},[fileId]);
 async function details(){setLoading(true);try{const r=await invoke<{fileId:string;snapshot:MetricsSnapshot}>('workflow_command',{action:{op:'inspect',captureId}});if(!mounted.current){await invoke('release_file',{fileId:r.fileId});return;}setFileId(r.fileId);setSnapshot(r.snapshot);}catch(e){if(mounted.current)setError(String(e));}finally{if(mounted.current)setLoading(false);}}
 if(!snapshot)return <p>{error||'读取存档指标…'}</p>;
 return <section><p>{snapshot.meta.fileName} · {snapshot.meta.frameCount} 帧 · {snapshot.meta.unityVersion}</p>{error&&<p role="alert">{error}</p>}<p>{snapshot.warnings?.join('；')}</p><div className="workflow-actions">{['cpu','gc','render'].map(t=><button key={t} onClick={()=>setTab(t)}>{t==='render'?'渲染':t.toUpperCase()}</button>)}</div>
 {tab==='render'?<RenderingPanel metrics={snapshot.rendering}/>:<><MetricCard label={tab==='gc'?'每帧 GC P95':'主线程 CPU P95'} value={tab==='gc'?snapshot.gc.allocPerFrameBytes.p95:snapshot.cpu.mainThreadMs.p95} quality={tab==='gc'?snapshot.gc.allocPerFrameBytes.quality:snapshot.cpu.mainThreadMs.quality} unit={tab==='gc'?'bytes':'ms'}/>{tab==='cpu'&&<HotspotTable title="主线程热点" hotspots={snapshot.cpu.topHotspots} quality={snapshot.cpu.hotspotQuality} valueColumn="ms"/>}{tab==='gc'&&<HotspotTable title="GC 分配热点" hotspots={snapshot.gc.topAllocSites} quality={snapshot.gc.siteQuality} valueColumn="bytes"/>}{fileId?<FrameExplorer fileId={fileId} frames={snapshot.cpu.frameTimeline} quality={tab==='gc'?snapshot.gc.allocPerFrameBytes.quality:snapshot.cpu.mainThreadMs.quality} mode={tab==='gc'?'gc':'cpu'}/>:<button disabled={loading} onClick={details}>{loading?'正在读取原录制…':'读取原录制，查看帧调用树与 Flow'}</button>}</>}
 </section>;
}
