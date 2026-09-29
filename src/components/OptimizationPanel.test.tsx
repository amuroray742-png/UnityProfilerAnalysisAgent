import { render,screen,fireEvent,waitFor,cleanup } from '@testing-library/react';
import { beforeEach,afterEach,it,expect,vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { OptimizationPanel,modificationAvailability,emptyConditions } from './OptimizationPanel';
import type { DiagnoseState } from '../hooks/useDiagnose';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
afterEach(cleanup);beforeEach(()=>vi.clearAllMocks());
const state:DiagnoseState={phase:'done',upload:null,snapshot:null,agents:[{id:'codex',label:'Codex',command:'codex-acp',args:[],available:true},{id:'claude-code',label:'Claude',command:'claude-code-acp',args:[],available:true},{id:'gemini',label:'Gemini',command:'gemini',args:[],available:true}],selectedAgent:'claude-code',sessionId:'diagnostic',streamedText:'',events:[],errorMessage:null,reports:[],activeStage:'project',sourceInfo:null};
function project(){return {id:'p',name:'Public',root:'C:/Public',directory:'C:/Records',busy:false,budgets:{},captures:[{id:'a',path:'a.json',conditions:emptyConditions(),snapshot:{fileName:'a.json',frameCount:2,unityVersion:'6000.3'}}],rounds:[{id:'r',baseline:'a',candidate:null,tasks:[{id:'t',title:'减少分配',kind:'optimize',evidence:'帧 10 已读代码',files:{'Assets/Work.cs':'hash'},instructions:'缓存固定数据',constraints:'保持玩法',acceptance:'重录',selected:true}],tests:[],runs:[],comparison:null,correctness:'pending',decision:'pending',reports:[{stage:'project',agentId:'codex'}]}]};}
it('selects modification Agent independently and starts by round ID, never an old session ID',async()=>{
 const p=project();vi.mocked(invoke).mockResolvedValue(p);render(<OptimizationPanel state={state}/>);fireEvent.click(screen.getByText(/打开优化项目/));
 const select=await screen.findByLabelText(/修改 Agent（/);await waitFor(()=>expect(select).toHaveValue('codex'));
 fireEvent.change(select,{target:{value:'claude-code'}});fireEvent.click(screen.getByText('确认本轮范围，开始修改（新会话）'));
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('optimization_command',{action:{op:'start',roundId:'r',agentId:'claude-code'}}));
 expect(state.selectedAgent).toBe('claude-code');expect(p.rounds[0].reports[0].agentId).toBe('codex');
 expect(JSON.stringify(vi.mocked(invoke).mock.calls)).not.toContain('diagnostic');
});
it('does not replace unavailable Agents and treats cancelled export as a normal no-op',async()=>{
 expect(modificationAvailability({...state.agents[0],available:false})).toContain('未检测');expect(modificationAvailability(state.agents[2])).toContain('尚未验证');
 vi.mocked(invoke).mockResolvedValue(project());vi.mocked(save).mockResolvedValue(null);render(<OptimizationPanel state={state}/>);fireEvent.click(screen.getByText(/打开优化项目/));await screen.findByText('导出 Markdown');fireEvent.click(screen.getByText('导出 Markdown'));await waitFor(()=>expect(save).toHaveBeenCalled());expect(vi.mocked(invoke).mock.calls.filter(c=>JSON.stringify(c).includes('"op":"export"'))).toHaveLength(0);
});
it('renders truthful zero, missing percentages, partial coverage and unavailable targets',async()=>{
 const p=project();Object.assign(p.rounds[0],{comparison:{baseline:'a',candidate:'b',comparability:'有限可比',unknown:['device'],mismatch:[],scope:'不证明因果',hotspots:{rows:[],total:0,nextStart:null},metrics:[{metric:'GC bytes',a:{p95:0,validFrames:2,totalFrames:2},b:{p95:0,validFrames:1,totalFrames:2},delta:{p95:{absolute:0,percent:null}},verdict:'不可判定'}]}});
 vi.mocked(invoke).mockResolvedValue(p);render(<OptimizationPanel state={state}/>);fireEvent.click(screen.getByText(/打开优化项目/));expect(await screen.findByText('0 · 1/2')).toBeInTheDocument();expect(screen.getByText('不可判定')).toBeInTheDocument();expect(screen.getByText(/0 \/ —/)).toBeInTheDocument();
});

it('rechecks cancelled edits without starting an Agent or reusing a session',async()=>{
 const p=project();Object.assign(p.rounds[0],{runs:[{id:'run',agentId:'codex',sessionId:'old-session',status:'partial',reason:'已取消',changes:[{path:'Assets/Work.cs',state:'applied'}],checks:[]}]});
 vi.mocked(invoke).mockResolvedValue(p);render(<OptimizationPanel state={state}/>);fireEvent.click(screen.getByText(/打开优化项目/));fireEvent.click(await screen.findByText(/重新执行 Unity 检查/));
 await waitFor(()=>expect(invoke).toHaveBeenCalledWith('optimization_command',{action:{op:'check',roundId:'r'}}));
 expect(vi.mocked(invoke).mock.calls.some(c=>JSON.stringify(c).includes('"op":"start"'))).toBe(false);
});
