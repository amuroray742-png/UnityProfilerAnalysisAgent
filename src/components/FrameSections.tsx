import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
type Page={rows:{name:string;threadIndex:number|null;offset:number;byteLength:number;count:number|null;rawHex:string;rawTruncated:boolean}[];total:number;nextStart:number|null;scope:string};
export function FrameSections({fileId,frame}:{fileId:string;frame:number}){
 const [page,setPage]=useState<Page|null>(null),[start,setStart]=useState<number|null>(null),[error,setError]=useState('');
 useEffect(()=>{let live=true;if(start!==null){setPage(null);setError('');void invoke<Page>('frame_sections',{fileId,frameIndex:frame,start,limit:20}).then(p=>{if(live)setPage(p);}).catch(e=>{if(live)setError(String(e));});}return()=>{live=false;};},[fileId,frame,start]);
 return <details><summary>未解释的二进制区段</summary><button onClick={()=>setStart(0)}>读取区段</button>{page&&<><p>{page.scope} 共 {page.total} 段。</p>{page.rows.map((r,i)=><p key={i} style={{overflowWrap:'anywhere'}}>{r.name} · 线程 {r.threadIndex??'帧级'} · 偏移 {r.offset} · {r.byteLength} 字节 · 数量 {r.count??'未知'}<br/><code>{r.rawHex}{r.rawTruncated?'…（截断）':''}</code></p>)}<button disabled={page.nextStart===null} onClick={()=>setStart(page.nextStart)}>下一页区段</button></>}{error&&<p role="alert">{error}</p>}</details>;
}
