import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import type { Project, Round, Run } from '../types/optimization';
import { DiagnosisStream } from './DiagnosisStream';
import { WorkActivity } from './WorkActivity';
const names:Record<string,string>={pending:'待处理',accepted:'已接受',rolled_back:'已回退',completed:'完成',running:'进行中',failed:'失败',cancelled:'已停止',interrupted:'已中断',incomplete:'不完整',modified:'已修改',partial:'部分完成',investigated:'调查完成，尚未修改',passed:'通过',unavailable:'未完成',applied:'已应用',prepared:'待确认',conflict:'冲突',problem:'存在问题'};
const label=(s:string)=>names[s]??s;
const time=(s?:string)=>s&&Number.isFinite(Date.parse(s))?new Date(s).toLocaleString('zh-CN',{hour12:false}):'尚无时间记录';
type Tab='overview'|'performance'|'project'|'changes'|'checks';
type Page={text:string;nextStart:number|null;total?:number};
export function HistoryPanel({project}:{project:Project}){
 const [selected,setSelected]=useState(project.rounds.at(-1)?.id??''),[tab,setTab]=useState<Tab>('overview'),[attempt,setAttempt]=useState(''),[runId,setRunId]=useState(''),[change,setChange]=useState<number|null>(null),[page,setPage]=useState<Page|null>(null),[start,setStart]=useState(0),[error,setError]=useState(''),[loading,setLoading]=useState(false),[activity,setActivity]=useState(false);
 const epoch=useRef(0);const round=project.rounds.find(r=>r.id===selected)??project.rounds.at(-1);
 const reports=round?.reports.filter(p=>p.stage===tab)??[];
 const report=reports.find(p=>p.reportId===attempt)??reports.at(-1);
 const run=round?.runs.find(r=>r.id===runId)??round?.runs.at(-1);
 useEffect(()=>{setAttempt('');setRunId('');setChange(null);setActivity(false);},[selected,tab]);
 useEffect(()=>{if(!attempt&&report?.reportId)setAttempt(report.reportId);},[attempt,report?.reportId]);
 useEffect(()=>{if(!runId&&run?.id)setRunId(run.id);},[runId,run?.id]);
 const scope=`${project.id}:${round?.id}:${tab}:${report?.reportId}:${run?.id}:${change}`;
 async function read(offset=0){if(!round)return;const token=++epoch.current;setLoading(true);setError('');setPage(null);try{
  let action:Record<string,unknown>|null=null;
  if((tab==='performance'||tab==='project')&&report?.reportId)action={op:'report',roundId:round.id,reportId:report.reportId,start:offset};
  if(tab==='changes'&&run)action={op:change==null?'runText':'change',roundId:round.id,runId:run.id,...(change==null?{}:{index:change}),start:offset};
  if(action){const p=await invoke<Page>('workflow_command',{action});if(token===epoch.current){setPage(p);setStart(offset);}}
 }catch(e){if(token===epoch.current)setError(String(e));}finally{if(token===epoch.current)setLoading(false);}}
 useEffect(()=>{void read();return()=>{++epoch.current;};},[scope]);
 async function exportRound(format:string){if(!round)return;const rid=round.id;try{const path=await save({defaultPath:`${project.name}-第${project.rounds.indexOf(round)+1}轮.${format==='html'?'html':'md'}`});if(path)await invoke('workflow_command',{action:{op:'export',roundId:rid,path,format}});}catch(e){setError(String(e));}}
 return <section className="workflow-card history-panel"><h2>历史轮次</h2><p>只读查看每轮报告、修改和结论，不影响当前工作。</p>
 <div className="history-layout"><nav className="round-list" aria-label="选择历史轮次">{project.rounds.map((r,i)=><button key={r.id} aria-pressed={r.id===round?.id} onClick={()=>setSelected(r.id)}><strong>第 {i+1} 轮 · {label(r.decision)}</strong><span>{time(r.reports[0]?.createdAt??r.runs[0]?.createdAt)}</span><span>{r.runs.length?'修改：':'诊断定位：'}{label(r.runs.at(-1)?.status??r.workflow?.status??'pending')} · {new Set(r.runs.flatMap(s=>s.changes).filter(c=>!c.path.endsWith('.meta')&&c.kind!=='directory').map(c=>c.path)).size} 个代码文件</span></button>)}</nav>
 <div className="round-content">{!round?<p>导入 A 后会建立第一轮记录。</p>:<><h3>第 {project.rounds.indexOf(round)+1} 轮 · {label(round.decision)}</h3>
 <nav className="history-tabs" aria-label="轮次记录分类">{([['overview','本轮概览'],['performance','性能诊断'],['project','工程定位'],['changes','代码修改'],['checks','检查与对比']] as [Tab,string][]).map(([key,name])=><button key={key} aria-pressed={tab===key} onClick={()=>setTab(key)}>{name}</button>)}</nav>
 {error&&<div role="alert">{error}<button onClick={()=>read(start)}>重试读取记录</button></div>}
 {tab==='overview'&&<><h4>发现了什么</h4>{round.tasks.length?<ul>{round.tasks.map(t=><li key={t.id}><strong>{t.title}</strong><p>{t.evidence}</p></li>)}</ul>:<p>{round.reports.length?'分析结论保存在诊断和定位报告中，点击上方分类查看全文。':'尚未生成诊断报告。'}</p>}<h4>改了什么</h4>{round.runs.length?round.runs.map(r=><RunSummary key={r.id} run={r}/>):<p>尚未修改代码。</p>}<h4>检查结果</h4><p>{round.runs.at(-1)?.checks.length?label(round.runs.at(-1)!.checks.at(-1)!.status):'尚未编译验证'}；{round.tests.length?'已配置测试':'未配置测试，仅检查编译'}。</p><h4>最终决定</h4><p>{label(round.decision)} · 玩法：{label(round.correctness)} · 性能：{round.performanceStatus}</p>{round.workflow?.reason&&<p>{round.workflow.reason}</p>}</>}
 {(tab==='performance'||tab==='project')&&<>{reports.length?<><label>报告版本<select value={report?.reportId??''} onChange={e=>{setAttempt(e.target.value);setActivity(false);}}>{reports.map((p,i)=><option key={p.reportId} value={p.reportId}>第 {p.attempt??i+1} 次 · {p.agentId} · {label(p.status??'')} · {time(p.createdAt)}</option>)}</select></label><p>{report?.reason}</p><button onClick={()=>setActivity(v=>!v)}>查看此会话工作过程</button></>:<p>本轮尚无{tab==='project'?'工程定位':'性能诊断'}报告。</p>}</>}
 {tab==='changes'&&<>{round.runs.length?<><label>修改运行<select value={run?.id??''} onChange={e=>{setRunId(e.target.value);setChange(null);setActivity(false);}}>{round.runs.map((r,i)=><option key={r.id} value={r.id}>第 {i+1} 次 · {r.agentId} · {label(r.status)} · {time(r.createdAt)}</option>)}</select></label><RunSummary run={run!}/><button onClick={()=>setChange(null)}>优化说明</button><button onClick={()=>setActivity(v=>!v)}>查看此会话工作过程</button><h4>修改文件（包括附属文件）</h4>{run?.changes.map((c,i)=><button className="change-file" key={i} aria-pressed={change===i} onClick={()=>setChange(i)}>{c.path} · {c.kind==='create'?'新增':c.kind==='directory'?'新增目录':'修改'} · {label(c.state)}</button>)}{change!=null&&<p>实际存档差异：{run?.changes[change]?.path}；回退不会删除本次变更记录。</p>}</>:<p>本轮尚无代码修改。</p>}</>}
 {loading&&<p role="status">正在读取记录…</p>}{page&&<>{change!=null&&tab==='changes'?<pre className="file-diff">{page.text||'此记录没有文本差异。'}</pre>:<DiagnosisStream text={page.text} isStreaming={false} emptyHint="此会话尚无公开正文。"/>}<p>{page.nextStart!=null?'当前为部分内容，请继续查看下一页。':'已到最后一页。'}</p>{start>0&&<button onClick={()=>read(Math.max(0,start-12000))}>上一页</button>}{page.nextStart!=null&&<button onClick={()=>read(page.nextStart!)}>下一页</button>}</>}
 {activity&&((tab==='changes'&&run)||(report?.reportId))&&<WorkActivity key={`${tab}:${tab==='changes'?run!.id:report!.reportId}`} projectId={project.id} roundId={round.id} runId={tab==='changes'?run!.id:report!.reportId!} agent={tab==='changes'?run!.agentId:report!.agentId} stage={tab==='changes'?'代码优化':tab==='project'?'工程定位':'性能诊断'} status={tab==='changes'?run!.status:report!.status??''} startedAt={tab==='changes'?run!.createdAt:report!.createdAt}/>}
 {tab==='checks'&&<Checks round={round}/>}<div className="workflow-actions"><button onClick={()=>exportRound('markdown')}>导出本轮 Markdown</button><button onClick={()=>exportRound('html')}>导出本轮 HTML</button></div>
 </>}</div></div><p>存档位置：{project.directory}</p></section>;
}
function RunSummary({run}:{run:Run}){return <div><p>{run.agentId} · {label(run.status)} · {time(run.createdAt)}</p><p>{run.reason}</p><p>{run.changes.length?`保存了 ${run.changes.length} 条文件／目录变更。`:'没有已记录的文件修改。'}</p>{run.status==='rolled_back'&&<p>本次修改已回退，以下报告与差异仅作历史记录。</p>}</div>;}
function Checks({round}:{round:Round}){return <><h4>检查结果</h4>{round.runs.map(r=><div key={r.id}><p>{r.agentId} · {time(r.createdAt)}</p>{r.checks.length?r.checks.map((c,i)=><p key={i}>第 {i+1} 次：{label(c.status)} {c.reason}</p>):<p>尚未编译验证。</p>}</div>)}<p>{round.tests.length?`选定测试：${round.tests.join('、')}`:'未配置测试，仅检查编译。'}</p><h4>对比与结论</h4><p>玩法：{label(round.correctness)} · 决定：{label(round.decision)}</p>{round.comparison?<><p>{round.performanceStatus} · {round.comparison.comparability}</p><p>未知条件：{round.comparison.unknown.join('、')||'无'}；不同条件：{round.comparison.mismatch.join('、')||'无'}</p><table><thead><tr><th>指标 P95</th><th>A</th><th>B</th><th>B − A</th></tr></thead><tbody>{round.comparison.metrics.map(m=><tr key={m.metric}><td>{m.metric}</td><td>{m.a.p95??'—'}（{m.a.validFrames}/{m.a.totalFrames}）</td><td>{m.b.p95??'—'}（{m.b.validFrames}/{m.b.totalFrames}）</td><td>{m.delta.p95.absolute??'—'}</td></tr>)}</tbody></table></>:<p>尚无 A/B 对比，不能判断性能改善。</p>}</>;}
