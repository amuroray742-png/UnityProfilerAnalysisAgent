import {render,screen,fireEvent,waitFor,cleanup,within} from '@testing-library/react';
import {beforeEach,afterEach,it,expect,vi} from 'vitest';
import {invoke} from '@tauri-apps/api/core';
import {open,save} from '@tauri-apps/plugin-dialog';
import {ProjectWorkflow,stepOf,readyReport} from './ProjectWorkflow';
import type {Project,Round} from '../types/optimization';
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
vi.mock('./CaptureInspector',()=>({CaptureInspector:({captureId}:{captureId:string})=><div>指标 {captureId}</div>}));
afterEach(cleanup);beforeEach(()=>vi.clearAllMocks());
const agents=[{id:'codex',label:'Codex',available:true},{id:'claude-code',label:'Claude',available:true}];
const base=():Project=>({id:'p',name:'公开工程',root:'C:/Public',directory:'C:/Records',rootAvailable:true,busy:false,budgets:{},captures:[],rounds:[]});
const round=():Round=>({id:'r',baseline:'a',candidate:null,reports:[],runs:[],tasks:[],tests:[],comparison:null,correctness:'pending',decision:'pending',taskVersion:1,taskVerifications:{},performanceStatus:'待录制'});
function mock(p:Project|null){vi.mocked(invoke).mockImplementation(async(name,args)=>{if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;if(a?.op==='recent')return [];if(a?.op==='activity')return {available:false,rows:[],nextCursor:0,hasMore:false};return p;});}
it('starts at the project home and creates a project before importing a recording',async()=>{
 mock(null);render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('新建优化项目'));
 expect(screen.queryByText('导入录制 A')).not.toBeInTheDocument();
 fireEvent.change(screen.getByLabelText('Unity 工程目录'),{target:{value:'C:/Public'}});fireEvent.change(screen.getByLabelText('项目名称'),{target:{value:'公开工程'}});fireEvent.click(screen.getByText('创建并进入'));
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'create',root:'C:/Public',name:'公开工程',directory:null}}));
});
it('one analysis button sends both agents and only persistent round identity',async()=>{
 const p=base();p.rounds=[round()];mock(p);render(<ProjectWorkflow/>);
 const b=await screen.findByText('一键诊断并定位');await waitFor(()=>expect(b).not.toBeDisabled());fireEvent.click(b);
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'analyze',roundId:'r',analysisAgent:'codex',localizationAgent:'codex',restart:false}}));
 expect(screen.queryByText('选择源码目录并定位')).not.toBeInTheDocument();
});
it('opening a saved localization never starts AI automatically and allows independent modification AI',async()=>{
 const p=base();const r=round();r.reports=[{reportId:'pr',stage:'project',status:'completed',agentId:'codex'}];p.rounds=[r];mock(p);render(<ProjectWorkflow/>);
 const button=await screen.findByText('开始优化');await waitFor(()=>expect(button).not.toBeDisabled());
 expect(vi.mocked(invoke).mock.calls.some(c=>JSON.stringify(c).includes('startAutomatic'))).toBe(false);
 fireEvent.change(screen.getByLabelText('修改 AI'),{target:{value:'claude-code'}});fireEvent.click(button);
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('optimization_command',{action:{op:'startAutomatic',roundId:'r',agentId:'claude-code',requirements:''}}));
});
it('retains incomplete performance reports and resumes the pipeline without resetting history',async()=>{
 const p=base();const r=round();r.reports=[{reportId:'p1',stage:'performance',status:'completed',agentId:'codex'},{reportId:'p2',stage:'project',status:'failed',agentId:'codex',parentReportId:'p1'}];p.rounds=[r];mock(p);render(<ProjectWorkflow/>);
 expect(await screen.findByText('继续诊断定位')).toBeInTheDocument();expect(screen.queryByText('开始优化')).not.toBeInTheDocument();expect(stepOf(r)).toBe(1);
});
it('a restarted incomplete diagnosis cannot use an obsolete completed localization',()=>{
 const r=round();r.reports=[{reportId:'p1',stage:'performance',status:'completed',agentId:'codex'},{reportId:'p2',stage:'project',status:'completed',agentId:'codex',parentReportId:'p1'},{reportId:'p3',stage:'performance',status:'failed',agentId:'codex'}];expect(readyReport(r)).toBe(false);expect(stepOf(r)).toBe(1);
});
it('saved round export uses its identity and cancelling the save dialog invokes no write',async()=>{
 const p=base();p.rounds=[round()];mock(p);vi.mocked(save).mockResolvedValue(null);render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('历史轮次'));fireEvent.click(await screen.findByText('导出本轮 HTML'));await waitFor(()=>expect(save).toHaveBeenCalled());expect(vi.mocked(invoke).mock.calls.some(c=>JSON.stringify(c).includes('"op":"export"'))).toBe(false);
});
it('imports B into the current round and displays next round only after a decision',async()=>{
 const p=base();const r=round();r.runs=[{id:'run',agentId:'codex',sessionId:'new',status:'modified',reason:null,createdAt:'',checks:[],changes:[{path:'Assets/Work.cs',state:'applied',beforeHash:'a',afterHash:'b'}]}];p.rounds=[r];mock(p);vi.mocked(open).mockResolvedValue('B.json');render(<ProjectWorkflow/>);
 fireEvent.click(await screen.findByText('导入复测录制 B'));await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'bind',path:'B.json',role:'b',operationId:expect.any(String)}}));expect(screen.queryByText('开始下一轮')).not.toBeInTheDocument();
});
it('can stop while optimization startup IPC is still pending',async()=>{
 const p=base();const r=round();r.reports=[{reportId:'pr',stage:'project',status:'completed',agentId:'codex'}];p.rounds=[r];
 vi.mocked(invoke).mockImplementation(async(name,args)=>{if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;if(a?.op==='recent')return [];if(a?.op==='activity')return {available:false,rows:[],nextCursor:0,hasMore:false};if(a?.op==='startAutomatic')return new Promise(()=>{});return p;});
 render(<ProjectWorkflow/>);const start=await screen.findByText('开始优化');await waitFor(()=>expect(start).not.toBeDisabled());fireEvent.click(start);
 fireEvent.click(await screen.findByText('停止当前任务'));await waitFor(()=>expect(invoke).toHaveBeenCalledWith('optimization_command',{action:{op:'cancel'}}));
});
it('does not tell users to record a Marker that was rolled back',async()=>{
 const p=base();const r=round();r.decision='rolled_back';r.runs=[{id:'run',agentId:'codex',sessionId:'new',status:'rolled_back',reason:null,createdAt:'',checks:[],changes:[]}];r.tasks=[{id:'t',kind:'marker',title:'补采样',evidence:'公开样例',files:{},instructions:'补采样',acceptance:'重录',constraints:'',selected:true}];p.rounds=[r];mock(p);render(<ProjectWorkflow/>);
 expect(await screen.findByText(/代码已回退。以下是回退前/)).toBeInTheDocument();expect(screen.queryByText(/包含新增采样点，需要重新录制/)).not.toBeInTheDocument();expect(screen.getByText('开始下一轮')).toBeInTheDocument();
});

function withBaseline(){
 const p=base();p.captures=['a','b'].map(id=>({id,path:`${id}.json`,snapshot:{fileName:`${id}.json`,frameCount:id==='a'?2:21,unityVersion:'6000.3'},conditions:{device:'',platform:'',scenario:'',operation:'',build:'',quality:'',resolution:'',profiling:'',codeVersion:''}}));p.rounds=[round()];return p;
}
function nextProject(){
 const p=withBaseline();p.rounds[0].candidate='b';p.rounds[0].decision='accepted';p.rounds.push({...round(),id:'next',baseline:'b'});return p;
}
function diagnosedProject(status='interrupted'){
 const p=nextProject(),r=p.rounds[1];
 r.reports=[{reportId:'old-performance',stage:'performance',status:'completed',agentId:'claude-code'},{reportId:'old-project',stage:'project',status,agentId:'codex',parentReportId:'old-performance'}];
 r.workflow={stage:'project',status,reason:'旧诊断原因',analysisAgent:'claude-code',localizationAgent:'codex'};
 return p;
}
it.each(['completed','failed','cancelled','interrupted'])('offers A replacement after %s diagnosis without starting AI',async status=>{
 mock(diagnosedProject(status));render(<ProjectWorkflow/>);
 const replace=await screen.findByText('重新导入 A');expect(replace).not.toBeDisabled();
 expect(screen.getByLabelText('本轮 A 录制')).toHaveTextContent('b.json');
 expect(screen.getByText('重新导入会放弃本轮已有诊断与定位结果，轮次编号不变，需要重新诊断。')).toBeVisible();
 if(status==='completed')expect(screen.getByText('开始优化')).toBeVisible();else expect(screen.getByText('继续诊断定位')).toBeVisible();
 expect(vi.mocked(invoke).mock.calls.some(c=>['analyze','startAutomatic'].includes((c[1] as {action?:{op:string}})?.action?.op??''))).toBe(false);
});
it.each(['completed','interrupted'])('abandons %s diagnosis in the same round and requires fresh manual diagnosis',async status=>{
 let p=diagnosedProject(status);const prior=structuredClone(p.rounds[0]);vi.mocked(open).mockResolvedValue('fresh.json');
 vi.mocked(invoke).mockImplementation(async(name,args)=>{
  if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;
  if(a?.op==='recent')return [];if(a?.op==='activity')return {available:false,rows:[],nextCursor:0,hasMore:false};
  if(a?.op==='bind'){
   p=structuredClone(p);p.captures.push({...p.captures[0],id:'fresh',path:'fresh.json',snapshot:{fileName:'fresh.json',frameCount:7,unityVersion:'6000.3'}});
   p.rounds[1]={...round(),id:'next',baseline:'fresh',taskVersion:2,workflow:{stage:'',status:'',reason:null,analysisAgent:'claude-code',localizationAgent:'codex'}};
  }
  return p;
 });
 render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('查看本轮 A 指标'));expect(await screen.findByText('指标 b')).toBeInTheDocument();
 fireEvent.change(screen.getByLabelText('A 帧区间'),{target:{value:'10-20'}});fireEvent.change(screen.getByLabelText('设备'),{target:{value:'旧设备'}});fireEvent.click(screen.getByLabelText('已核对两份录制的复现条件'));
 fireEvent.click(screen.getByText('重新导入 A'));
 const diagnose=await screen.findByText('一键诊断并定位');await waitFor(()=>expect(diagnose).not.toBeDisabled());
 expect(screen.getByText('公开工程 · 第 2 轮')).toBeVisible();expect(screen.getByLabelText('本轮 A 录制')).toHaveTextContent('fresh.json');
 expect(screen.queryByText('开始优化')).not.toBeInTheDocument();expect(screen.queryByText('旧诊断原因')).not.toBeInTheDocument();expect(screen.queryByLabelText('工作阶段')).not.toBeInTheDocument();
 expect(screen.queryByText('指标 b')).not.toBeInTheDocument();expect(screen.getByLabelText('A 帧区间')).toHaveValue('');expect(screen.getByLabelText('设备')).toHaveValue('');expect(screen.getByLabelText('已核对两份录制的复现条件')).not.toBeChecked();
 expect(p.rounds).toHaveLength(2);expect(p.rounds[0]).toEqual(prior);
 expect(vi.mocked(invoke).mock.calls.some(c=>['analyze','startAutomatic'].includes((c[1] as {action?:{op:string}})?.action?.op??''))).toBe(false);
 expect(screen.getByLabelText('分析 AI')).toHaveValue('claude-code');expect(screen.getByLabelText('定位 AI')).toHaveValue('codex');
 fireEvent.click(diagnose);await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'analyze',roundId:'next',analysisAgent:'claude-code',localizationAgent:'codex',restart:false}}));
});
it('preserves an existing diagnosis when A file selection is cancelled',async()=>{
 const p=diagnosedProject('completed'),before=structuredClone(p);mock(p);vi.mocked(open).mockResolvedValue(null);render(<ProjectWorkflow/>);
 fireEvent.click(await screen.findByText('重新导入 A'));await waitFor(()=>expect(open).toHaveBeenCalled());await waitFor(()=>expect(screen.getByText('重新导入 A')).not.toBeDisabled());
 expect(screen.getByText('开始优化')).toBeVisible();expect(screen.getByText('旧诊断原因')).toBeVisible();expect(p).toEqual(before);
 expect(vi.mocked(invoke).mock.calls.some(c=>(c[1] as {action?:{op:string}})?.action?.op==='bind')).toBe(false);
});
it('starts the next round with B and offers replacement beside diagnosis without starting AI',async()=>{
 const p=withBaseline();p.rounds[0].candidate='b';p.rounds[0].decision='accepted';const next=nextProject();next.parseProgress={projectId:'p',roundId:'r',operationId:'old-import',stage:'save',status:'completed',done:1,total:1};
 vi.mocked(invoke).mockImplementation(async(name,args)=>{if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;if(a?.op==='recent')return [];if(a?.op==='next')return next;return p;});
 render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('开始下一轮'));
 expect(await screen.findByText('沿用上一轮 B')).toBeVisible();expect(within(screen.getByLabelText('本轮 A 录制')).getByText(/b.json · 21 帧/)).toBeVisible();expect(screen.getByText('重新导入 A')).toBeVisible();expect(screen.queryByText('替换本轮 A')).not.toBeInTheDocument();
 expect(screen.queryByText(/导入完成，记录已保存/)).not.toBeInTheDocument();
 expect(vi.mocked(invoke).mock.calls.some(c=>['analyze','startAutomatic'].includes((c[1] as {action:{op:string}})?.action?.op))).toBe(false);
 const analyze=screen.getByText('一键诊断并定位');await waitFor(()=>expect(analyze).not.toBeDisabled());fireEvent.click(analyze);
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'analyze',roundId:'next',analysisAgent:'codex',localizationAgent:'codex',restart:false}}));
});
it('replaces only the current A, resets its view and options, and diagnoses the same new round',async()=>{
 let p=nextProject();const prior=structuredClone(p.rounds[0]);vi.mocked(open).mockResolvedValue('fresh.json');
 vi.mocked(invoke).mockImplementation(async(name,args)=>{if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;if(a?.op==='recent')return [];if(a?.op==='bind'){p=structuredClone(p);p.captures.push({...p.captures[0],id:'fresh',path:'fresh.json',snapshot:{fileName:'fresh.json',frameCount:7,unityVersion:'6000.3'}});p.rounds[1].baseline='fresh';}return p;});
 render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('查看本轮 A 指标'));expect(await screen.findByText('指标 b')).toBeInTheDocument();
 fireEvent.change(screen.getByLabelText('A 帧区间'),{target:{value:'10-20'}});fireEvent.change(screen.getByLabelText('设备'),{target:{value:'旧设备'}});fireEvent.click(screen.getByLabelText('已核对两份录制的复现条件'));
 fireEvent.click(screen.getByText('重新导入 A'));
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'bind',path:'fresh.json',role:'a',operationId:expect.any(String)}}));
 expect(await within(screen.getByLabelText('本轮 A 录制')).findByText(/fresh.json · 7 帧/)).toBeVisible();expect(screen.queryByText('沿用上一轮 B')).not.toBeInTheDocument();expect(screen.queryByText('指标 b')).not.toBeInTheDocument();expect(screen.getByLabelText('A 帧区间')).toHaveValue('');expect(screen.getByLabelText('设备')).toHaveValue('');expect(screen.getByLabelText('已核对两份录制的复现条件')).not.toBeChecked();expect(p.rounds[0]).toEqual(prior);
 expect(vi.mocked(invoke).mock.calls.some(c=>(c[1] as {action:{op:string}})?.action?.op==='analyze')).toBe(false);
 fireEvent.click(screen.getByText('一键诊断并定位'));await waitFor(()=>expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'analyze',roundId:'next',analysisAgent:'codex',localizationAgent:'codex',restart:false}}));
});
it('offers replacement in round one and preserves A when the file dialog is cancelled',async()=>{
 mock(withBaseline());vi.mocked(open).mockResolvedValue(null);render(<ProjectWorkflow/>);const replace=await screen.findByText('重新导入 A');fireEvent.click(replace);await waitFor(()=>expect(open).toHaveBeenCalled());await waitFor(()=>expect(replace).not.toBeDisabled());expect(screen.getByLabelText('本轮 A 录制')).toHaveTextContent('a.json');expect(vi.mocked(invoke).mock.calls.some(c=>(c[1] as {action:{op:string}})?.action?.op==='bind')).toBe(false);
});
it.each(['解析失败','导入已取消','保存失败'])('preserves the inherited A and allows retry after %s',async error=>{
 const p=diagnosedProject(),before=structuredClone(p);vi.mocked(open).mockResolvedValue('fresh.json');vi.mocked(invoke).mockImplementation(async(name,args)=>{if(name==='list_agents')return agents;const a=(args as {action?:{op:string}})?.action;if(a?.op==='recent')return [];if(a?.op==='activity')return {available:false,rows:[],nextCursor:0,hasMore:false};if(a?.op==='bind')throw error;return p;});render(<ProjectWorkflow/>);fireEvent.click(await screen.findByText('重新导入 A'));expect(await screen.findByText(error)).toBeInTheDocument();expect(screen.getByLabelText('本轮 A 录制')).toHaveTextContent('b.json');expect(screen.getByText('沿用上一轮 B')).toBeVisible();expect(screen.getByText('重新导入 A')).not.toBeDisabled();expect(screen.getByText('继续诊断定位')).not.toBeDisabled();expect(screen.getByText('旧诊断原因')).toBeVisible();expect(p).toEqual(before);
});
it('blocks duplicate imports and diagnosis while choosing and importing a new A',async()=>{
 let select!:(p:string)=>void,finish!:(p:Project)=>void;const p=nextProject();mock(p);vi.mocked(open).mockImplementation(()=>new Promise(resolve=>{select=resolve;}));
 render(<ProjectWorkflow/>);const replace=await screen.findByText('重新导入 A');fireEvent.click(replace);expect(replace).toBeDisabled();expect(screen.getByText('一键诊断并定位')).toBeDisabled();
 vi.mocked(invoke).mockImplementation(async(name,args)=>{if((args as {action:{op:string}})?.action?.op==='bind')return new Promise(resolve=>{finish=resolve;});return name==='list_agents'?agents:p;});select('fresh.json');await waitFor(()=>expect(screen.getByText('取消导入')).toBeInTheDocument());expect(replace).toBeDisabled();expect(screen.getByText('一键诊断并定位')).toBeDisabled();finish(p);await waitFor(()=>expect(replace).not.toBeDisabled());expect(open).toHaveBeenCalledTimes(1);
});
it.each(['reports','workflow','failed','cancelled','interrupted','accepted','rolled_back','running'] as const)('does not allow replacement after %s',async state=>{
 const p=withBaseline(),r=p.rounds[0];if(state==='reports')r.reports=[{stage:'performance',agentId:'codex',status:'running'}];else if(state==='workflow')r.workflow={stage:'performance',status:'running',reason:null,analysisAgent:'codex',localizationAgent:'codex'};else if(['failed','cancelled','interrupted'].includes(state))r.runs=[{id:'run',agentId:'codex',sessionId:'s',status:state,reason:null,createdAt:'',checks:[],changes:[]}];else if(state==='running')p.busy=true;else r.decision=state;
 mock(p);render(<ProjectWorkflow/>);await screen.findByText('公开工程 · 第 1 轮');expect(screen.queryByText('重新导入 A')).not.toBeInTheDocument();expect(screen.queryByText('替换本轮 A')).not.toBeInTheDocument();
});
it('shows rolled-back inheritance and restores a saved replacement without starting AI',async()=>{
 const p=nextProject();p.rounds[0].decision='rolled_back';p.rounds[1].baseline='a';mock(p);const view=render(<ProjectWorkflow/>);expect(await screen.findByText('沿用上一轮 A（上一轮已回退）')).toBeVisible();view.unmount();
 p.captures.push({...p.captures[0],id:'fresh',path:'fresh.json',snapshot:{fileName:'fresh.json',frameCount:7,unityVersion:'6000.3'}});p.rounds[1].baseline='fresh';mock(p);render(<ProjectWorkflow/>);expect(await within(await screen.findByLabelText('本轮 A 录制')).findByText(/fresh.json · 7 帧/)).toBeVisible();expect(screen.queryByText('沿用上一轮 A（上一轮已回退）')).not.toBeInTheDocument();expect(vi.mocked(invoke).mock.calls.some(c=>(c[1] as {action:{op:string}})?.action?.op==='analyze')).toBe(false);
});
