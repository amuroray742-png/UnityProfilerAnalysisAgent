import { useEffect, useRef, useState } from 'react';
import { renderReportMarkdown } from '../lib/tauri';
interface DiagnosisStreamProps { text: string; isStreaming: boolean; emptyHint?: string }
export function DiagnosisStream({ text, isStreaming, emptyHint }: DiagnosisStreamProps) {
 const [rendered, setRendered] = useState({source:'',html:''});
 const latest=useRef(text);latest.current=text;
 useEffect(()=>{
  let active=true,busy=false,last='';
  const render=async()=>{const source=latest.current;if(busy||!source||source===last)return;busy=true;try{const html=await renderReportMarkdown(source);if(active){last=source;setRendered({source,html});}}catch{ /* Plain text stays readable if rendering fails. */ }finally{busy=false;}};
  void render();const timer=setInterval(()=>{void render();},150);
  return()=>{active=false;clearInterval(timer);};
 },[]);
 if (!text && !isStreaming) return <div className="diagnosis-stream"><div className="diagnosis-empty">{emptyHint ?? '点击“开始 AI 诊断”让 Agent 分析性能瓶颈'}</div></div>;
 const compatible=!!rendered.html&&text.startsWith(rendered.source);
 return <div className="diagnosis-stream">
  {compatible ? <><div className="diagnosis-content" dangerouslySetInnerHTML={{ __html: rendered.html }} />{text.length>rendered.source.length&&<pre className="stream-tail">{text.slice(rendered.source.length)}</pre>}</> : <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{text}</pre>}
  {isStreaming && <span className="cursor-blink" aria-label="typing" />}
 </div>;
}
