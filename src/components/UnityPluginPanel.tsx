import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { UnityPluginStatus } from '../types/optimization';
export function UnityPluginPanel({projectId,busy,onBusy}:{projectId:string;busy:boolean;onBusy:(b:boolean)=>void}) {
 const [status,setStatus]=useState<UnityPluginStatus|null>(null),[error,setError]=useState(''),[working,setWorking]=useState(false);
 const generation=useRef(0);
 async function query(install=false){const g=++generation.current;setWorking(true);setError('');onBusy(true);
  try{const s=await invoke<UnityPluginStatus>('workflow_command',{action:{op:install?'pluginInstall':'pluginStatus',projectId}});if(g===generation.current)setStatus(s);}
  catch(e){if(g===generation.current)setError(String(e));}
  finally{if(g===generation.current){setWorking(false);onBusy(false);}}
 }
 useEffect(()=>{setStatus(null);setError('');return()=>{generation.current++;};},[projectId]);
 return <section className="workflow-card"><h3>Unity 插件</h3>
 <p>安装到工程后可随工程一起提交，其他人不需要相同的本地目录。不安装也可以继续离线分析。</p>
 {status?<><p>{status.state==='installed'?'插件文件已安装':status.state==='missing'?'插件未安装':status.state==='migrate'?'需要迁移本地插件':status.state==='interrupted'?'安装未完成':'插件存在冲突'} · {status.version}</p><p>{status.reason}</p>
 <p>{status.editor?.status==='ready'?'Editor 已连接':'Editor 尚未连接或不可用'}{status.editor?.reason?`：${status.editor.reason}`:''}</p>
 {status.lockWarning&&<p role="alert">{status.lockWarning}</p>}
 {status.state==='installed'&&<p>请打开 Unity 等待依赖解析和编译，将 Packages/com.upaa.inspector、变更后的 manifest.json 及 Unity 更新的 packages-lock.json 一起提交。文件已安装不代表编译通过。</p>}
 {['missing','migrate','interrupted'].includes(status.state)&&<><p>请先关闭目标工程的 Unity Editor。此操作增加插件文件；迁移时只移除本插件的外部路径引用，不更改其他依赖。不会自动升级插件或安装 Unity CLI。</p><button disabled={busy||working} onClick={()=>query(true)}>{status.state==='migrate'?'迁移到工程内':status.state==='interrupted'?'继续安装':'安装到工程'}</button></>}
 {status.record&&<details><summary>安装记录</summary><p>{status.record.status} · {status.record.reason??'记录已保存'}</p></details>}
 </>:<p>点击检查，查看当前工程的插件文件和 Editor 连接状态。</p>}
 <button disabled={busy||working} onClick={()=>query()}>{working?'正在处理…':'重新检查'}</button>{error&&<p role="alert">{error}</p>}
 </section>;
}
