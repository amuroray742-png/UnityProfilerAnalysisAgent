import {render,screen,fireEvent,waitFor,cleanup,act} from '@testing-library/react';
import {afterEach,beforeEach,it,expect,vi} from 'vitest';
import {invoke} from '@tauri-apps/api/core';
import {HistoryPanel} from './HistoryPanel';
import {ParseProgressPanel} from './ParseProgressPanel';
import {WorkActivity,activityBlocks} from './WorkActivity';
import type {Project,Round,ActivityRow} from '../types/optimization';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock('@tauri-apps/plugin-dialog',()=>({save:vi.fn(async()=>null)}));
afterEach(cleanup);beforeEach(()=>vi.clearAllMocks());
const round=(id:string):Round=>({id,baseline:'a',candidate:null,reports:[{reportId:id+'p',agentId:'codex',stage:'performance',status:'completed',createdAt:'2026-09-30'}],runs:[],tasks:[],tests:[],comparison:null,correctness:'pending',decision:'accepted',taskVersion:1,taskVerifications:{},performanceStatus:'待录制'});
const project=():Project=>({id:'p',name:'公开',root:'missing',directory:'records',busy:false,budgets:{},captures:[],rounds:[round('r1'),round('r2')]});
it('shows every historical round and rejects late report pages from a previous selection',async()=>{
 let resolve!:(v:unknown)=>void;vi.mocked(invoke).mockImplementation(async(name,args)=>{const a=(args as any)?.action;if(name==='render_report_markdown')return '';if(a?.reportId==='r2p')return new Promise(r=>{resolve=r;});return {text:'第一轮完整报告',nextStart:null};});
 render(<HistoryPanel project={project()}/>);fireEvent.click(screen.getByText('性能诊断'));await waitFor(()=>expect(resolve).toBeDefined());fireEvent.click(screen.getByRole('button',{name:/第 1 轮/}));expect(await screen.findByText('第一轮完整报告')).toBeInTheDocument();await act(async()=>resolve({text:'迟到第二轮',nextStart:null}));expect(screen.queryByText('迟到第二轮')).not.toBeInTheDocument();
});
it('selects latest report attempt, keeps incomplete versions and reads later pages',async()=>{
 const p=project();p.rounds[1].reports.push({reportId:'new',agentId:'claude-code',stage:'performance',status:'failed',reason:'已中断'});
 vi.mocked(invoke).mockImplementation(async(name,args)=>name==='render_report_markdown'?'':{text:(args as any).action.start?'后续正文':'第一页',nextStart:(args as any).action.start?null:12000});render(<HistoryPanel project={p}/>);fireEvent.click(screen.getByText('性能诊断'));expect(await screen.findByText('第一页')).toBeInTheDocument();expect(screen.getByLabelText('报告版本')).toHaveValue('new');fireEvent.click(screen.getByText('下一页'));expect(await screen.findByText('后续正文')).toBeInTheDocument();expect(screen.getByText('已中断')).toBeInTheDocument();
});
it('reads individual stored file diffs even after rollback and with missing source project',async()=>{
 const p=project();p.rootAvailable=false;p.rounds[1].runs=[{id:'run',agentId:'codex',sessionId:'s',status:'rolled_back',createdAt:'',reason:null,checks:[],changes:[{path:'Assets/中文.cs',state:'rolled_back',kind:'create',beforeHash:'a',afterHash:'b'}]}];
 vi.mocked(invoke).mockImplementation(async(name,args)=>name==='render_report_markdown'?'':{text:(args as any).action.op==='change'?'+ public class Helper {}':'曾新增辅助代码',nextStart:null});render(<HistoryPanel project={p}/>);fireEvent.click(screen.getByText('代码修改'));fireEvent.click(screen.getByRole('button',{name:/Assets\/中文.cs/}));expect(await screen.findByText('+ public class Helper {}')).toBeInTheDocument();expect(invoke).toHaveBeenCalledWith('workflow_command',{action:{op:'change',roundId:'r2',runId:'run',index:0,start:0}});expect(screen.getByText(/本次修改已回退/)).toBeInTheDocument();
});
it('uses determinate bytes only when known and does not mark parsed input as saved',()=>{
 const v={projectId:'p',roundId:'r',operationId:'o',stage:'parse',done:40,total:100,status:'running'};const {rerender}=render(<ParseProgressPanel value={v}/>);expect(screen.getByRole('progressbar')).toHaveAttribute('value','40');rerender(<ParseProgressPanel value={{...v,done:null,total:null}}/>);expect(screen.getByRole('progressbar')).not.toHaveAttribute('value');expect(screen.queryByText('导入完成')).not.toBeInTheDocument();rerender(<ParseProgressPanel value={{...v,stage:'save',status:'failed',reason:'磁盘已满'}}/>);expect(screen.getByRole('alert')).toHaveTextContent('磁盘已满');
});
function row(sequence:number,event:ActivityRow['event']):ActivityRow{return {projectId:'p',roundId:'r',runId:'s',sequence,nextCursor:sequence+1,time:'2026-09-30T00:00:00Z',event};}
it('coalesces public chunks and updates tools by call identity instead of tool name',()=>{
 const blocks=activityBlocks([row(0,{kind:'chunk',text:'中文'}),row(1,{kind:'chunk',text:'说明'}),row(2,{kind:'tool',callId:'one',tool:'project_read',status:'running'}),row(3,{kind:'tool',callId:'two',tool:'project_read',status:'running'}),row(4,{kind:'tool',callId:'one',tool:'project_read',status:'failed'})]);expect(blocks).toHaveLength(3);expect(blocks[0].text).toBe('中文说明');expect(blocks[1].row?.event.status).toBe('failed');expect(blocks[2].row?.event.status).toBe('running');
});
it('loads archived activity with explicit compatibility message and exposes no fake thought',async()=>{
 vi.mocked(invoke).mockResolvedValue({available:false,rows:[],nextCursor:0,hasMore:false});render(<WorkActivity projectId="p" roundId="r" runId="s" agent="Codex" stage="性能诊断" status="completed"/>);expect(await screen.findByText(/此版本未保存工作过程/)).toBeInTheDocument();
});
it('streams only matching identities and supports following after the user scrolls up',async()=>{
 vi.mocked(invoke).mockImplementation(async(name)=>name==='render_report_markdown'?'':{available:true,rows:[row(0,{kind:'chunk',text:'真实公开说明'}),{...row(1,{kind:'chunk',text:'其他项目'}),projectId:'other'}],nextCursor:2,hasMore:false});render(<WorkActivity projectId="p" roundId="r" runId="s" agent="Codex" stage="性能诊断" status="running"/>);expect(await screen.findByText('真实公开说明')).toBeInTheDocument();expect(screen.queryByText('其他项目')).not.toBeInTheDocument();const box=document.querySelector('.activity-scroll')!;Object.defineProperty(box,'scrollHeight',{value:1000});Object.defineProperty(box,'clientHeight',{value:200});fireEvent.scroll(box,{target:{scrollTop:0}});fireEvent.click(screen.getByText('回到最新'));await waitFor(()=>expect(box.scrollTop).toBe(1000));
});
