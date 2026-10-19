import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {afterEach,expect,it,vi} from 'vitest';
import {SamplingReview} from './SamplingReview';
import type {Round} from '../types/optimization';
vi.mock('./CaptureInspector',()=>({CaptureInspector:({captureId}:{captureId:string})=><p>原始录制 {captureId}</p>}));
afterEach(cleanup);
const round=()=>({id:'r',decision:'pending',candidate:'b',taskVerifications:{},runs:[{}],tasks:[{id:'t',kind:'marker',selected:true,title:'区分加载与初始化',instructions:'分别包裹两个同步区间',acceptance:'进入关卡，核对两个区间的耗时',markerNames:['Game.Load']}]} as unknown as Round);
it('shows the plan, observed paths and B without automatically passing the task',()=>{
 const r=round(),verify=vi.fn();
 r.comparison={sampling:{scope:'名称匹配不证明因果',rows:[{taskId:'t',name:'Game.Load',status:'observed',matches:[{roleAndPath:[null,'Main Thread',['Root','Game.Load']],observation:{callsPerFrame:2,inclusiveMsPerFrame:3,gcBytesPerFrame:null,observedFrames:1,totalFrames:2}}]}]}} as unknown as Round['comparison'];
 render(<SamplingReview round={r} busy={false} onVerify={verify}/>);
 expect(screen.getByText(/进入关卡，核对两个区间/)).toBeVisible();
 expect(screen.getByText(/B 中已观察到/)).toBeVisible();expect(verify).not.toHaveBeenCalled();
 expect(screen.getByRole('combobox')).toHaveValue('pending');
 fireEvent.click(screen.getByText('查看 B 调用树，核对新采样'));expect(screen.getByText('原始录制 b')).toBeVisible();
 fireEvent.change(screen.getByRole('combobox'),{target:{value:'problem'}});expect(verify).toHaveBeenCalledWith('t','problem');
});
it('keeps old records usable and disables acceptance until B is imported',()=>{
 const r=round();r.candidate=null;delete r.tasks[0].markerNames;
 render(<SamplingReview round={r} busy={false} onVerify={vi.fn()}/>);
 expect(screen.getByText(/旧记录未提供/)).toBeVisible();expect(screen.getByRole('combobox')).toBeDisabled();
 expect(screen.queryByText('查看 B 调用树，核对新采样')).not.toBeInTheDocument();
});
it('does not request re-recording after rollback',()=>{
 const r=round();r.decision='rolled_back';render(<SamplingReview round={r} busy={false} onVerify={vi.fn()}/>);
 expect(screen.queryByLabelText('补采样与重录核对')).not.toBeInTheDocument();
});
it('distinguishes an unavailable query from a name not observed and freezes accepted records',()=>{
 const r=round();r.decision='accepted';r.comparison={sampling:{scope:'仅已导出采样',rows:[{taskId:'t',name:'Game.Load',status:'unavailable',reason:'录制已变化',matches:[]},{taskId:'t',name:'Game.Init',status:'notObserved',matches:[]}]}} as unknown as Round['comparison'];
 render(<SamplingReview round={r} busy={false} onVerify={vi.fn()}/>);
 expect(screen.getByText(/无法读取 B 采样证据/)).toHaveTextContent('录制已变化');expect(screen.getByText(/B 所选范围未观察到/)).toBeVisible();expect(screen.getByRole('combobox')).toBeDisabled();
});
