import { useEffect, useState } from 'react';
import { renderReportMarkdown } from '../lib/tauri';
interface DiagnosisStreamProps { text: string; isStreaming: boolean; emptyHint?: string }
export function DiagnosisStream({ text, isStreaming, emptyHint }: DiagnosisStreamProps) {
 const [html, setHtml] = useState('');
 useEffect(() => {
  let active = true; setHtml('');
  const timer = setTimeout(() => { if (text) void renderReportMarkdown(text).then(value => { if (active) setHtml(value); }).catch(() => {}); }, isStreaming ? 150 : 0);
  return () => { active = false; clearTimeout(timer); };
 }, [text, isStreaming]);
 if (!text && !isStreaming) return <div className="diagnosis-stream"><div className="diagnosis-empty">{emptyHint ?? '点击“开始 AI 诊断”让 Agent 分析性能瓶颈'}</div></div>;
 return <div className="diagnosis-stream">
  {html ? <div className="diagnosis-content" dangerouslySetInnerHTML={{ __html: html }} /> : <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{text}</pre>}
  {isStreaming && <span className="cursor-blink" aria-label="typing" />}
 </div>;
}
