import { renderInlineMarkdown } from '../lib/format.ts';

interface DiagnosisStreamProps {
  text: string;
  isStreaming: boolean;
  emptyHint?: string;
}

export function DiagnosisStream({ text, isStreaming, emptyHint }: DiagnosisStreamProps) {
  if (!text && !isStreaming) {
    return (
      <div className="diagnosis-stream">
        <div className="diagnosis-empty">
          {emptyHint ?? '点击"开始 AI 诊断"让 Agent 分析性能瓶颈'}
        </div>
      </div>
    );
  }

  const html = renderInlineMarkdown(text);

  return (
    <div className="diagnosis-stream">
      <div
        className="diagnosis-content"
        // Markdown 已在 format.ts 中 escape + 简单 inline 渲染
        dangerouslySetInnerHTML={{ __html: html }}
      />
      {isStreaming && <span className="cursor-blink" aria-label="typing" />}
    </div>
  );
}