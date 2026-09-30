import { UnityPluginPanel } from './UnityPluginPanel';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { AgentPreset } from '../types';
import type { Project, Round, Statistic, ParseProgress } from '../types/optimization';
import { DiagnosisStream } from './DiagnosisStream';
import { modificationAvailability } from './OptimizationPanel';
import './ProjectWorkflow.css';
import { listen } from '@tauri-apps/api/event';
import { HistoryPanel } from './HistoryPanel';
import { WorkActivity } from './WorkActivity';
import { ParseProgressPanel } from './ParseProgressPanel';
import { CaptureInspector } from './CaptureInspector';

type Recent = {id:string;name:string;root:string;directory:string;updatedAt:string};
const labels:Record<string,string>={running:'正在进行',completed:'完成',failed:'失败',cancelled:'已停止',interrupted:'已中断',incomplete:'不完整',modified:'已修改',partial:'部分完成',rolled_back:'已回退',investigated:'调查完成，尚未修改',passed:'通过',unavailable:'未完成',pending:'待确认',accepted:'已接受',problem:'存在问题'};
const label=(s:string)=>labels[s]??s;
export function readyReport(r?:Round):boolean {if(!r)return false;const a=r.reports.filter(p=>p.stage==='performance').at(-1);return r.reports.some(p=>p.stage==='project'&&p.status==='completed'&&(!a||(a.status==='completed'&&p.parentReportId===a.reportId)));}
const metricNames:Record<string,string>={cpuMs:'主线程耗时（ms）',frameMs:'录制帧耗时（ms）',gcBytes:'每帧 GC 分配（B）','Draw Calls Count':'绘制调用次数','SetPass Calls Count':'渲染状态切换次数','Batches Count':'渲染批次','Triangles Count':'三角形数量','Vertices Count':'顶点数量'};
const conditionNames:Record<string,string>={device:'设备',platform:'平台',scenario:'场景',operation:'操作',build:'构建',quality:'画质',resolution:'分辨率',profiling:'录制设置',codeVersion:'代码版本'};
export function stepOf(round?:Round):number {
 if(!round)return 0;
 if(round.decision==='accepted'||round.decision==='rolled_back'||round.comparison)return 4;
 if(round.runs.some(r=>r.changes.some(c=>['applied','prepared','conflict'].includes(c.state))))return 3;
 if(readyReport(round))return 2;
 return 1;
}
export function ProjectWorkflow(){
 const [project,setProject]=useState<Project|null>(null),[recents,setRecents]=useState<Recent[]>([]),[agents,setAgents]=useState<AgentPreset[]>([]);
 const [operation,setOperation]=useState(''),[loading,setLoading]=useState(false),[error,setError]=useState(''),[creating,setCreating]=useState(false),[root,setRoot]=useState(''),[name,setName]=useState(''),[directory,setDirectory]=useState('');
 const [analysis,setAnalysis]=useState(''),[localization,setLocalization]=useState(''),[modifier,setModifier]=useState(''),[requirements,setRequirements]=useState('');
 const [statistic,setStatistic]=useState<Statistic>('p95');
 const [view,setView]=useState<'current'|'history'>('current'),[parseProgress,setParseProgress]=useState<ParseProgress|null>(null);
 const parseOperation=useRef<string|null>(null),projectRef=useRef<Project|null>(null);projectRef.current=project;
 const [aRange,setARange]=useState(''),[bRange,setBRange]=useState(''),[confirmed,setConfirmed]=useState(false),[inspect,setInspect]=useState(false);
 const [tests,setTests]=useState(''),[conditions,setConditions]=useState<Record<string,Record<string,string>>>({}),[budgets,setBudgets]=useState<Record<string,number>>({});
 const epoch=useRef(0),inFlight=useRef(false);
 const current=project?.rounds.at(-1),round=current;
 const [pluginBusy,setPluginBusy]=useState(false);
 const busy=loading||pluginBusy||!!project?.busy, step=stepOf(current),run=round?.runs.at(-1);
 const finished=!!current&&['accepted','rolled_back'].includes(current.decision);
 const complete=readyReport(current);
 const available=(id:string)=>!!agents.find(a=>a.id===id&&a.available);
 async function refreshRecent(){setRecents(await invoke<Recent[]>('workflow_command',{action:{op:'recent'}}));}
 useEffect(()=>{let live=true;Promise.all([invoke<Project|null>('optimization_command',{action:{op:'get'}}),invoke<AgentPreset[]>('list_agents'),invoke<Recent[]>('workflow_command',{action:{op:'recent'}})]).then(([p,a,r])=>{if(live){setProject(p);setAgents(a);setRecents(r);setAnalysis(a.find(a=>a.available)?.id??'');}}).catch(e=>{if(live)setError(String(e));});return()=>{live=false;};},[]);
 useEffect(()=>{if(!project)return;let live=true;const timer=setInterval(()=>{const generation=epoch.current;invoke<Project>('optimization_command',{action:{op:'get'}}).then(p=>{if(live&&generation===epoch.current)setProject(p);}).catch(e=>{if(live)setError(String(e));});},1200);return()=>{live=false;clearInterval(timer);};},[project?.id]);
 useEffect(()=>{setInspect(false);setARange('');setBRange('');setConfirmed(false);setConditions({});setBudgets(project?.budgets??{});setTests(current?.tests.join('\n')??'');setModifier(current?.runs.at(-1)?.agentId??'');setRequirements(current?.runs.at(-1)?.requirements??'');},[project?.id,current?.id]);
 useEffect(()=>{if(!current)return;const a=current.workflow?.analysisAgent;if(a)setAnalysis(a);setLocalization(current.workflow?.localizationAgent&&current.workflow.localizationAgent!==a?current.workflow.localizationAgent:'');},[current?.id]);
 useEffect(()=>{const a=current?.reports.filter(p=>p.stage==='project'&&p.status==='completed').at(-1)?.agentId;if(a)setModifier(previous=>previous||a);},[current?.reports.length,current?.id]);
 useEffect(()=>{let alive=true;const pending=listen<ParseProgress>('workflow-progress',e=>{const p=e.payload,active=projectRef.current;if(alive&&active&&p.projectId===active.id&&(p.roundId===''||p.roundId===active.rounds.at(-1)?.id)&&(!parseOperation.current||p.operationId===parseOperation.current))setParseProgress(p);});return()=>{alive=false;void pending.then(f=>f()).catch(()=>{});};},[]);
 useEffect(()=>{if(project?.parseProgress){const p=project.parseProgress;if(!parseOperation.current||parseOperation.current===p.operationId)setParseProgress(p);}},[project?.parseProgress]);
 useEffect(()=>{parseOperation.current=null;setParseProgress(null);setView('current');},[project?.id]);
 async function call(command:string,action:Record<string,unknown>){if(inFlight.current)return;if(action.op==='startAutomatic'){parseOperation.current=null;setParseProgress(null);}inFlight.current=true;setOperation(String(action.op));setLoading(true);setError('');++epoch.current;try{const p=await invoke<Project|null>(command,{action});setProject(p);return p;}catch(e){setError(String(e));return undefined;}finally{++epoch.current;inFlight.current=false;setLoading(false);setOperation('');}}
 async function stop(){try{await invoke('optimization_command',{action:{op:'cancel'}});}catch(e){setError(String(e));}}
 const flow=(action:Record<string,unknown>)=>call('workflow_command',action);
 const opt=(action:Record<string,unknown>)=>call('optimization_command',action);
 async function chooseRoot(){try{const path=await open({directory:true,title:'选择 Unity 工程根目录'});if(typeof path==='string'){setRoot(path);setName(path.replace(/[\\/]+$/,'').split(/[\\/]/).at(-1)??'优化项目');}}catch(e){setError(String(e));}}
 async function openProject(path?:string){try{const directory=path??await open({directory:true,title:'打开优化项目存档目录'});if(typeof directory==='string'){await flow({op:'open',directory});}}catch(e){setError(String(e));}}
 async function importCapture(role:'a'|'b'){try{const path=await open({multiple:false,title:role==='a'?'选择优化前录制 A':'选择重新录制的 B',filters:[{name:'Profiler',extensions:['data','json','raw','pd3u']}]});if(typeof path==='string'){const operationId=crypto.randomUUID();parseOperation.current=operationId;setParseProgress({projectId:project!.id,roundId:current?.id??'',operationId,stage:'verify',status:'running',done:null,total:null});await flow({op:'bind',path,role,operationId});}}catch(e){setError(String(e));}}
 async function analyze(restart=false){parseOperation.current=null;setParseProgress(null);if(current)await flow({op:'analyze',roundId:current.id,analysisAgent:analysis,localizationAgent:localization||analysis,restart});}
 async function compare(){if(!current)return;try{const parse=(s:string)=>{if(!s.trim())return null;const n=s.split('-').map(Number);if(n.length!==2||n.some(n=>!Number.isSafeInteger(n)||n<0)||n[0]>n[1])throw Error('帧区间应为起始帧-结束帧');return n;};const rangeA=parse(aRange),rangeB=parse(bRange);if(await saveOptions())await opt({op:'compare',roundId:current.id,candidateId:current.candidate,rangeA,rangeB,confirmed});}catch(e){setError(String(e));}}
 async function saveOptions(){if(!current)return;return opt({op:'update',roundId:current.id,tasks:current.tasks,budgets,conditions:Object.fromEntries((project?.captures??[]).map(c=>[c.id,{...c.conditions,...conditions[c.id]}])),tests:tests.split('\n').map(t=>t.trim()).filter(Boolean)});}
 async function goHome(){if(await opt({op:'close'})!==undefined){setView('current');setParseProgress(null);await refreshRecent();}}
 const agentSelect=(title:string,value:string,set:(s:string)=>void,modify=false)=><label>{title}<select aria-label={title} value={value} disabled={busy} onChange={e=>set(e.target.value)}><option value="">{title==='定位 AI'?'沿用分析 AI':'请选择 AI'}</option>{agents.map(a=><option key={a.id} value={a.id} disabled={modify?!!modificationAvailability(a):!a.available}>{a.label}{(modify?modificationAvailability(a):!a.available?'不可用':'')?`（${modify?modificationAvailability(a):'不可用'}）`:''}</option>)}</select></label>;
 return <main className="project-workflow">
 <header><h1>Unity 性能优化</h1><p>分析卡顿 → 优化代码 → 重录验证</p></header>
 {loading&&parseProgress?.status==='running'&&<button onClick={stop}>取消导入</button>}
 {error&&<div role="alert" className="error-banner">{error}</div>}
 {!project?<section className="workflow-card"><h2>优化项目</h2><p>先选择你的 Unity 工程。每一轮报告和修改记录都会自动保存在本机。</p><div className="workflow-actions"><button disabled={busy} onClick={()=>setCreating(true)}>新建优化项目</button><button disabled={busy} onClick={()=>openProject()}>打开优化项目</button></div>
 {creating&&<form onSubmit={async e=>{e.preventDefault();const p=await flow({op:'create',root,name:name.trim()||'优化项目',directory:directory||null});if(p)setCreating(false);}}><label>Unity 工程目录<input value={root} onChange={e=>setRoot(e.target.value)} placeholder="包含 Assets、Packages、ProjectSettings 的目录"/></label><button type="button" onClick={chooseRoot}>选择工程目录</button><label>项目名称<input value={name} onChange={e=>setName(e.target.value)}/></label><details><summary>高级设置：存档位置</summary><p>默认保存在应用数据目录，不能放在 Unity 工程内。</p><input aria-label="存档位置" value={directory} onChange={e=>setDirectory(e.target.value)}/><button type="button" onClick={async()=>{const d=await open({directory:true,title:'选择工程之外的空存档目录'});if(typeof d==='string')setDirectory(d);}}>选择存档位置</button></details><button className="primary" disabled={busy||!root.trim()} type="submit">创建并进入</button></form>}
 <h3>最近项目</h3>{recents.length?recents.map(r=><button className="recent-project" key={r.id} disabled={busy} onClick={()=>openProject(r.directory)}><strong>{r.name}</strong><span>{r.root}</span></button>):<p>还没有优化项目。</p>}</section>:<>
 <section className="workflow-card project-summary"><div className="workflow-heading"><div><h2>{project.name} · 第 {project.rounds.length||1} 轮</h2><p>{project.root}</p><p role="status">{project.saveError?`存档失败：${project.saveError}`:loading?'正在处理并保存…':'记录已自动保存到本机'}</p></div><button disabled={busy} onClick={goHome}>返回项目首页</button></div></section>
 <nav className="project-tabs" aria-label="项目页面"><button aria-pressed={view==='current'} onClick={()=>setView('current')}>当前工作</button><button aria-pressed={view==='history'} onClick={()=>setView('history')}>历史轮次</button></nav>
 {view==='history'&&<HistoryPanel key={project.id} project={project}/>}
 <div hidden={view!=='current'}>
 <UnityPluginPanel key={project.id} projectId={project.id} busy={loading||!!project.busy} onBusy={setPluginBusy}/>
 <section className="workflow-card">
 {project.rootAvailable===false&&<div role="alert">Unity 工程不可用，目前只能查看存档；恢复原目录后再继续。</div>}
 <ol className="workflow-steps">{['导入录制','诊断定位','优化代码','重录对比','本轮结论'].map((s,i)=><li key={s} aria-current={i===step?'step':undefined} className={i===step?'active':''}>{i+1}. {s}</li>)}</ol>
 {current?.workflow?.reason&&<p className="workflow-note">{current?.workflow?.reason}</p>}
 {project.busy||operation==='startAutomatic'?<div className="workflow-running"><h3>{pluginBusy?'正在检查或安装 Unity 插件…':current?.workflow?.status==='running'?(current?.workflow?.stage==='project'?'正在定位工程中的问题…':'正在诊断性能热点…'):'AI 正在优化和检查代码…'}</h3><p>记录持续保存。停止后可重新打开项目继续。</p>{!pluginBusy&&!current?.reports.some(p=>p.status==='running')&&!current?.runs.some(r=>r.status==='running')&&<button onClick={stop}>停止当前任务</button>}</div>:<div className="workflow-primary">
 {step===0&&<><h3>第一步：导入优化前的录制</h3><p>在 Unity Profiler 中录制要优化的场景，保存后在这里导入。</p><button className="primary" disabled={busy} onClick={()=>importCapture('a')}>导入录制 A</button></>}
 {step===1&&<><h3>找出性能热点及工程中的相关代码</h3>{agentSelect('分析 AI',analysis,setAnalysis)}<details><summary>高级设置：单独选择定位 AI</summary>{agentSelect('定位 AI',localization,setLocalization)}</details><button className="primary" disabled={busy||!available(analysis)||!available(localization||analysis)||project.rootAvailable===false} onClick={()=>analyze()}>{current?.reports.length?'继续诊断定位':'一键诊断并定位'}</button><p>自动完成性能诊断和工程定位；此步骤不会修改代码。</p></>}
 {step===2&&<><h3>{current?.runs.length?'上次记录已保存，可以继续优化':'定位完成，准备优化代码'}</h3>{agentSelect('修改 AI',modifier,setModifier,true)}<label>补充要求（选填）<textarea value={requirements} onChange={e=>setRequirements(e.target.value)} placeholder="例如：保持玩法不变，优先减少卡顿"/></label><button className="primary" disabled={busy||!available(modifier)||!!modificationAvailability(agents.find(a=>a.id===modifier)??{id:'',command:'',label:'',args:[],available:false})||project.rootAvailable===false} onClick={()=>opt({op:'startAutomatic',roundId:current!.id,agentId:modifier,requirements})}>开始优化</button><p>点击后 AI 会在新会话中调查并修改代码，修改前自动保存可回退记录。</p></>}
 {step===3&&<><h3>接下来：按相同场景和操作重新录制</h3><p>代码检查通过不代表性能一定改善，请在 Unity 中重录 B。</p><button className="primary" disabled={busy} onClick={()=>importCapture('b')}>{current?.candidate?'重新选择录制 B':'导入复测录制 B'}</button>{current?.candidate&&<button className="primary" disabled={busy} onClick={compare}>对比 A 与 B</button>}</>}
 {step===4&&<><h3>{finished?'本轮已结束':'查看结果，决定是否保留修改'}</h3>{finished?<><p>{current?.decision==='accepted'?'已接受修改，下一轮默认使用 B 作为基线。':'已回退修改，下一轮默认沿用原 A。'}</p><button className="primary" disabled={busy} onClick={()=>flow({op:'next'})}>开始下一轮</button></>:<><label>玩法是否正常<select value={current?.correctness} disabled={busy} onChange={e=>opt({op:'decide',roundId:current!.id,decision:'pending',correctness:e.target.value})}><option value="pending">还没确认</option><option value="passed">正常</option><option value="problem">有问题</option></select></label><button className="primary" disabled={busy} onClick={()=>opt({op:'decide',roundId:current!.id,decision:'accepted',correctness:current!.correctness})}>接受本轮修改</button></>}</>}
 </div>}
 {current?.runs.length&&!finished?<button disabled={busy||project.rootAvailable===false} onClick={()=>opt({op:'rollback',roundId:current.id})}>回退本轮修改</button>:null}
 </section>
 <ParseProgressPanel value={parseProgress}/>
 {current&&<CurrentActivity project={project} round={current} onStop={stop}/>}
 {current?.runs.length?<section className="workflow-card"><h3>本轮优化结果</h3>{current.decision==='rolled_back'&&<p>代码已回退。以下是回退前的修改与检查记录，不代表当前仍保留这些修改。</p>}<p>改了什么：涉及 {new Set(current.runs.flatMap(r=>r.changes).filter(c=>!c.path.endsWith('.meta')&&c.kind!=='directory').map(c=>c.path)).size} 个代码文件。</p><ul>{current.tasks.filter(t=>t.kind!=='investigate').slice(0,3).map(t=><li key={t.id}>{t.title}</li>)}</ul><p>检查是否通过：{current.runs.at(-1)?.checks.length?label(current.runs.at(-1)!.checks.at(-1)!.status):'尚未编译验证'}{current.tests.length?'':'（未配置测试，仅检查编译）'}</p><p>{current.runs.at(-1)?.reason}</p><details><summary>查看 AI 的优化说明</summary><DiagnosisStream text={current.runs.at(-1)?.text??''} isStreaming={!!project.busy}/>{current.runs.at(-1)?.textPartial&&<p>此处为预览，完整报告可查看记录或导出。</p>}</details>{current.decision!=='rolled_back'&&current.tasks.some(t=>t.kind==='marker')&&<p>包含新增采样点，需要重新录制；补点不代表性能已经改善。</p>}</section>:null}
 {current?.comparison&&<section className="workflow-card"><h3>A / B 对比</h3><p>性能结论：{current.performanceStatus}</p><p>{current.comparison.comparability} · 条件未知：{current.comparison.unknown.map(s=>conditionNames[s]??s).join('、')||'无'} · 条件不同：{current.comparison.mismatch.map(s=>conditionNames[s]??s).join('、')||'无'}</p><details><summary>查看其他分布统计</summary><label>统计口径<select value={statistic} onChange={e=>setStatistic(e.target.value as Statistic)}>{(['mean','p50','p95','p99','max'] as Statistic[]).map(s=><option key={s} value={s}>{s.toUpperCase()}</option>)}</select></label></details><table><thead><tr><th>指标 {statistic.toUpperCase()}</th><th>A（有效帧）</th><th>B（有效帧）</th><th>变化 B − A</th></tr></thead><tbody>{current.comparison.metrics.map(m=><tr key={m.metric}><td>{metricNames[m.metric]??m.metric}</td><td>{m.a[statistic]??'—'}（{m.a.validFrames}/{m.a.totalFrames}）</td><td>{m.b[statistic]??'—'}（{m.b.validFrames}/{m.b.totalFrames}）</td><td>{m.delta[statistic].absolute??'—'} / {m.delta[statistic].percent==null?'—':`${m.delta[statistic].percent.toFixed(1)}%`}</td></tr>)}</tbody></table><p>单次差异不能证明修改因果；请同时核对玩法与复现条件。</p></section>}
 {current&&<section className="workflow-card"><details><summary>高级选项与详细指标</summary><button onClick={()=>setInspect(v=>!v)}>查看本轮 A 指标</button>{inspect&&<CaptureInspector key={round!.baseline} captureId={round!.baseline}/>}
 {current.reports.length===0&&<button disabled={busy} onClick={()=>importCapture('a')}>替换本轮 A</button>}
 {!current.runs.length&&<button disabled={busy||!available(analysis)||!available(localization||analysis)||project.rootAvailable===false} onClick={()=>analyze(true)}>重新诊断定位（保留旧报告）</button>}
 {current.runs.length&&!finished&&complete?<><h4>继续优化／重试（新会话）</h4>{agentSelect('重试修改 AI',modifier,setModifier,true)}<textarea aria-label="重试补充要求" value={requirements} onChange={e=>setRequirements(e.target.value)}/><button disabled={busy||!available(modifier)||project.rootAvailable===false} onClick={()=>opt({op:'startAutomatic',roundId:current.id,agentId:modifier,requirements})}>新会话继续优化</button></>:null}
 {current.tasks.filter(t=>t.selected).map(t=><label key={t.id}>{t.kind==='marker'?'新采样是否回答调查问题':'任务复验'}：{t.title}<select disabled={busy||finished} value={current.taskVerifications[t.id]??'pending'} onChange={e=>opt({op:'verifyTask',roundId:current.id,taskId:t.id,status:e.target.value})}><option value="pending">待确认</option><option value="passed">已核对通过</option><option value="problem">仍有问题</option></select></label>)}
 <label>EditMode 测试名（选填，一行一个）<textarea value={tests} disabled={busy||finished} onChange={e=>setTests(e.target.value)}/></label>
 <button disabled={busy||finished} onClick={saveOptions}>保存检查与对比选项</button>{run&&<button disabled={busy||finished||run.checks.length>=3||project.rootAvailable===false} onClick={async()=>{if(await saveOptions())await opt({op:'check',roundId:current.id});}}>重新编译检查（最多 3 次）</button>}
 <h4>对比范围（留空为整段）</h4><label>A 帧区间<input value={aRange} onChange={e=>setARange(e.target.value)}/></label><label>B 帧区间<input value={bRange} onChange={e=>setBRange(e.target.value)}/></label>
 {project.captures.filter(c=>c.id===current.baseline||c.id===current.candidate).map(c=><details key={c.id}><summary>{c.id===current.baseline?'A':'B'}：{c.snapshot.fileName} · 复现条件</summary>{Object.entries({device:'设备',platform:'平台',scenario:'场景',operation:'操作',build:'构建',quality:'画质',resolution:'分辨率',profiling:'录制设置',codeVersion:'代码版本'}).map(([k,n])=><label key={k}>{n}<input value={conditions[c.id]?.[k]??(c.conditions as unknown as Record<string,string>)[k]} disabled={busy||finished} onChange={e=>{setConditions(v=>({...v,[c.id]:{...v[c.id],[k]:e.target.value}}));setConfirmed(false);}}/></label>)}<button disabled={busy} onClick={async()=>{const path=await open({multiple:false,title:'重新定位同一份录制文件'});if(typeof path==='string')await flow({op:'relocate',captureId:c.id,path});}}>重新定位录制文件</button></details>)}
 {Object.entries(metricNames).map(([k,n])=><label key={k}>{n} P95 预算<input type="number" min="0" value={budgets[k]??''} onChange={e=>setBudgets(v=>{const next={...v};if(e.target.value==='')delete next[k];else next[k]=Number(e.target.value);return next;})}/></label>)}
 <label><input type="checkbox" checked={confirmed} onChange={e=>setConfirmed(e.target.checked)}/>已核对两份录制的复现条件</label>{current.candidate&&!finished&&<button disabled={busy} onClick={compare}>按以上设置重新对比</button>}
 </details></section>}
 </div></>}
 </main>;
}

function CurrentActivity({project,round,onStop}:{project:Project;round:Round;onStop:()=>void}){
 const sessions=[...round.reports.filter(p=>p.reportId).map(p=>({id:p.reportId!,agent:p.agentId,stage:p.stage==='project'?'工程定位':'性能诊断',status:p.status??'',time:p.createdAt})),...round.runs.map(r=>({id:r.id,agent:r.agentId,stage:'代码优化',status:r.status,time:r.createdAt}))];
 const [selected,setSelected]=useState('');const latest=sessions.at(-1)?.id;
 useEffect(()=>{setSelected(latest??'');},[latest]);
 const active=sessions.find(s=>s.id===selected)??sessions.at(-1);
 if(!active)return null;
 return <><label>工作阶段<select aria-label="工作阶段" value={active.id} onChange={e=>setSelected(e.target.value)}>{sessions.map(s=><option key={s.id} value={s.id}>{s.stage} · {s.agent} · {label(s.status)}</option>)}</select></label><WorkActivity projectId={project.id} roundId={round.id} runId={active.id} stage={active.stage} agent={active.agent} status={active.status} startedAt={active.time} onStop={onStop}/></>;
}
