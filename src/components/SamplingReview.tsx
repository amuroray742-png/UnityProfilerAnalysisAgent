import { useState } from 'react';
import type { Round } from '../types/optimization';
import { CaptureInspector } from './CaptureInspector';

export function SamplingReview({round,busy,onVerify}:{round:Round;busy:boolean;onVerify:(taskId:string,status:string)=>unknown}) {
 const [inspect,setInspect]=useState(false);
 const tasks=round.tasks.filter(t=>t.selected&&t.kind==='marker');
 if(!tasks.length||!round.runs.length||round.decision==='rolled_back')return null;
 const finished=round.decision==='accepted';
 return <section className="workflow-card" aria-label="补采样与重录核对">
  <h3>补采样后，要验证什么</h3>
  <p>按相同设备、场景、操作和录制设置重录 B。先确认采样出现，再看它能否回答问题；补点和编译通过都不代表性能改善。</p>
  {tasks.map(task=><article key={task.id}>
   <h4>{task.title}</h4><p>补点方案：{task.instructions}</p><p>验收要求：{task.acceptance}</p>
   <p>固定采样名称：{task.markerNames?.join('、')||'旧记录未提供，请按方案人工核对'}</p>
   {round.comparison?.sampling?.rows.filter(row=>row.taskId===task.id).map((row,i)=><div key={i}>
    <p>{row.name??'人工核对'}：{row.status==='observed'?'B 中已观察到':row.status==='notObserved'?'B 所选范围未观察到':row.status==='unavailable'?'无法读取 B 采样证据':'需人工检查'}{row.reason?` · ${row.reason}`:''}</p>
    {row.matches.map((match,j)=><p key={j}>{match.roleAndPath[1]} / {match.roleAndPath[2].join(' → ')}：出现于 {match.observation.observedFrames}/{match.observation.totalFrames} 帧，每帧平均 {match.observation.callsPerFrame.toFixed(2)} 次、inclusive {match.observation.inclusiveMsPerFrame.toFixed(3)} ms。</p>)}
    {row.truncated&&<p>共 {row.totalPaths} 条路径，此处仅显示 {row.matches.length} 条，请查看 B 调用树。</p>}
   </div>)}
   <label>新采样是否回答调查问题：{task.title}<select disabled={busy||finished||!round.candidate} value={round.taskVerifications[task.id]??'pending'} onChange={e=>onVerify(task.id,e.target.value)}>
    <option value="pending">待确认</option><option value="passed">已核对通过</option><option value="problem">仍有问题</option>
   </select></label>
  </article>)}
  {round.comparison?.sampling&&<p>{round.comparison.sampling.scope}</p>}
  {!round.candidate?<p>请先导入重新录制的 B，再核对新采样。</p>:<><button disabled={busy} onClick={()=>setInspect(v=>!v)}>{inspect?'收起 B 采样证据':'查看 B 调用树，核对新采样'}</button>{inspect&&<CaptureInspector key={round.candidate} captureId={round.candidate}/>}</>}
  <p>未出现时检查是否执行了目标操作、录制范围和采样设置。出现但仍无法解释时，将核对结果记为“仍有问题”，在下一轮继续调查；不要把新 Marker 在 A 中缺失当成零耗时。</p>
 </section>;
}
