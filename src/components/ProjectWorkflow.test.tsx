import {render,screen,fireEvent,waitFor,cleanup} from '@testing-library/react';
import {beforeEach,afterEach,it,expect,vi} from 'vitest';
import {invoke} from '@tauri-apps/api/core';
import {open,save} from '@tauri-apps/plugin-dialog';
import {ProjectWorkflow,stepOf,readyReport} from './ProjectWorkflow';
import type {Project,Round} from '../types/optimization';
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
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
