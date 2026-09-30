import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ActivityPage, ActivityRow } from '../types/optimization';
import { DiagnosisStream } from './DiagnosisStream';
const toolNames:Record<string,string>={optimization_read:'读取代码',optimization_replace:'修改代码',optimization_create:'新增代码',optimization_task:'整理优化任务',optimization_check:'检查编译',optimization_context:'查看本轮依据',project_asset:'查看资源关联',project_references:'查找资源引用',project_read:'读取工程文件',project_search:'搜索工程',project_files:'查看工程文件',project_summary:'查看工程概况'};
export function toolLabel(name:string){return toolNames[name]??(name.startsWith('source_')?'查询源码':'查询性能证据');}
export function activityBlocks(rows:ActivityRow[]){
 const blocks:{id:string;text?:string;row?:ActivityRow}[]=[];const tools=new Map<string,number>();
 for(const row of rows){const e=row.event;
  if(e.kind==='chunk'){const last=blocks.at(-1);if(last?.text!=null)last.text+=e.text??'';else blocks.push({id:String(row.sequence),text:e.text??''});}
  else if(e.kind==='tool'&&e.callId){const existing=tools.get(e.callId);if(existing!=null)blocks[existing]={id:e.callId,row};else{tools.set(e.callId,blocks.length);blocks.push({id:e.callId,row});}}
  else blocks.push({id:String(row.sequence),row});
 }
 return blocks;
}
export function WorkActivity({projectId,roundId,runId,agent,stage,status,startedAt,onStop}:{projectId:string;roundId:string;runId:string;agent:string;stage:string;status:string;startedAt?:string;onStop?:()=>void}){
 const [rows,setRows]=useState<ActivityRow[]>([]),[available,setAvailable]=useState(true),[error,setError]=useState(''),[more,setMore]=useState(false),[limited,setLimited]=useState(false),[follow,setFollow]=useState(true),[tick,setTick]=useState(Date.now());
 const cursor=useRef(0),box=useRef<HTMLDivElement>(null),loadRef=useRef<()=>void>(()=>{});
 const running=status==='running';const liveStatus=useRef(running);liveStatus.current=running;
 useEffect(()=>{
  let live=true,busy=false,followUpdates=liveStatus.current;cursor.current=0;setRows([]);setAvailable(true);setError('');setFollow(true);setMore(false);setLimited(false);
  async function load(){if(!live||busy)return;busy=true;try{const p=await invoke<ActivityPage>('workflow_command',{action:{op:'activity',roundId,runId,cursor:cursor.current}});if(!live)return;
   const valid=p.rows.filter(r=>r.projectId===projectId&&r.roundId===roundId&&r.runId===runId&&r.sequence>=cursor.current);
   setRows(old=>{const ids=new Set(old.map(r=>r.sequence));return [...old,...valid.filter(r=>!ids.has(r.sequence))];});cursor.current=p.nextCursor;setAvailable(p.available);setMore(p.hasMore);setLimited(!!p.limited);setError('');followUpdates=liveStatus.current||(followUpdates&&p.hasMore);
  }catch(e){if(live)setError(String(e));}finally{busy=false;}}
  loadRef.current=()=>{void load();};void load();
  const timer=setInterval(()=>{if(live&&followUpdates){setTick(Date.now());void load();}},650);
  const unlisten=listen<ActivityRow>('workflow-activity',e=>{if(e.payload.projectId===projectId&&e.payload.roundId===roundId&&e.payload.runId===runId)void load();}).catch(()=>()=>{});
  return()=>{live=false;if(timer)clearInterval(timer);void unlisten.then(f=>f());};
 },[projectId,roundId,runId]);
 useEffect(()=>{if(follow&&box.current)box.current.scrollTop=box.current.scrollHeight;},[rows,follow]);
 const seconds=running&&startedAt?Math.max(0,Math.floor((tick-Date.parse(startedAt))/1000)):null;
 return <section className="workflow-card work-activity" aria-label="AI 实时工作"><div className="workflow-heading"><h3>{stage} · {agent}</h3><span>{running?'正在工作':status==='completed'||status==='modified'?'已完成':status==='rolled_back'?'已回退':({failed:'失败',cancelled:'已停止',interrupted:'已中断',partial:'部分完成',investigated:'调查完成，尚未修改',incomplete:'不完整'} as Record<string,string>)[status]??status} {seconds!=null&&Number.isFinite(seconds)?`· ${Math.floor(seconds/60)}分${seconds%60}秒`:''}</span>{running&&onStop&&<button onClick={onStop}>停止当前任务</button>}</div>
 <p>公开工作说明与操作摘要。每个阶段使用独立会话。</p>{error&&<div role="alert">{error}<button onClick={()=>loadRef.current()}>重试读取工作记录</button></div>}
 <div className="activity-scroll" ref={box} onScroll={()=>{const e=box.current;if(e)setFollow(e.scrollHeight-e.scrollTop-e.clientHeight<48);}}>
 {!rows.length&&<p>{running?'等待 AI 输出…':available?'此会话尚无活动记录。':'此版本未保存工作过程；报告与修改仍可在历史中查看。'}</p>}
 {activityBlocks(rows).map(b=>b.text!=null?<DiagnosisStream key={b.id} text={b.text} isStreaming={false}/>:<ActivityItem key={b.id} row={b.row!} running={running}/>)}
 {running&&<span className="cursor-blink" aria-label="AI 正在工作">▍</span>}
 </div>
 {!follow&&<button onClick={()=>setFollow(true)}>回到最新</button>}{more&&<button onClick={()=>loadRef.current()}>加载后续工作记录</button>}{limited&&<p>工作过程已达保存上限，后续活动未记录；请查看完整报告与修改记录。</p>}
 </section>;
}
function ActivityItem({row,running}:{row:ActivityRow;running:boolean}){const e=row.event;
 if(e.kind==='tool')return <details className="activity-tool"><summary>{toolLabel(e.tool??'')} · {e.status==='running'?(running?'进行中':'未收到结束状态'):e.status==='completed'?'完成':'失败'}</summary><p>{e.tool}</p><pre>{JSON.stringify(e.args??{},null,2)}</pre>{e.error&&<p>{e.error}</p>}</details>;
 const title=e.kind==='session'?'已建立新会话':e.kind==='started'?'AI 已启动':e.kind==='finished'?'AI 工作结束':e.kind==='cancelled'?'已停止，已保存内容保留':e.kind==='error'?'工作失败':e.kind==='limited'?'工作过程未完整记录':'';
 return <div className="activity-status">{title}{e.message&&<p>{e.message}</p>}{e.sessionId&&<details><summary>会话详情</summary>{e.sessionId}</details>}</div>;
}
