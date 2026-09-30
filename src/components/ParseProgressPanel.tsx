import type { ParseProgress } from '../types/optimization';
const stages:Record<string,string>={verify:'校验文件',parse:'解析录制',aggregate:'汇总指标',save:'保存结果'};
export function ParseProgressPanel({value}:{value:ParseProgress|null|undefined}){
 if(!value)return null;
 if(value.status==='completed')return <p className="parse-complete" role="status">✓ {value.stage==='save'?'导入完成，记录已保存。':'录制已就绪。'}</p>;
 if(value.status==='cancelled')return <p role="status">导入已取消，原有录制保持不变。</p>;
 const percent=value.total!=null&&value.total>0&&value.done!=null?Math.min(100,Math.floor(value.done/value.total*100)):null;
 return <section className="workflow-card parse-progress" aria-label="录制解析进度"><h3>{value.status==='completed'?(value.stage==='save'?'导入完成':'录制已就绪'):value.status==='failed'?`${stages[value.stage]??value.stage}失败`:stages[value.stage]??value.stage}</h3>
 <ol className="workflow-steps">{Object.entries(stages).map(([key,name])=><li key={key} className={key===value.stage?'active':''}>{name}</li>)}</ol>
 {value.status==='running'&&<><progress aria-label={stages[value.stage]??value.stage} {...(percent==null?{}:{value:percent,max:100})}/><p>{percent==null?'正在处理，当前阶段无法准确计算比例。':`${percent}%${value.stage==='save'?'':` · ${((value.done??0)/1048576).toFixed(1)} / ${((value.total??0)/1048576).toFixed(1)} MiB`}`}</p></>}
 {value.reason&&<p role="alert">{value.reason}</p>}</section>;
}
