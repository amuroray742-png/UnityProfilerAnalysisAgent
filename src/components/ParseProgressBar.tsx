import { useEffect, useState } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ParseProgress {
  fileId: string;
  doneBytes: number;
  totalBytes: number;
  currentFrame: number;
}

interface ParseProgressBarProps {
  /** 当前活跃的 fileId；切换后只订阅该 fileId 的进度 */
  fileId: string | null;
}

/**
 * 订阅 backend `parse-progress` Tauri event，渲染解析进度条。
 *
 * - 进度按 (doneBytes / totalBytes) 百分比显示
 * - 大文件（GB 级）进度事件每帧触发一次，进度条平滑推进
 * - 完成后 backend 会再发一次 100% 事件
 */
export function ParseProgressBar({ fileId }: ParseProgressBarProps) {
  const [progress, setProgress] = useState<ParseProgress | null>(null);

  useEffect(() => {
    if (!fileId) {
      setProgress(null);
      return;
    }
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    (async () => {
      const handler = (event: { payload: ParseProgress }) => {
        if (event.payload.fileId === fileId) {
          setProgress(event.payload);
        }
      };
      const u = await listen<ParseProgress>('parse-progress', handler);
      if (cancelled) {
        u();
        return;
      }
      unlisten = u;
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [fileId]);

  if (!progress || progress.totalBytes === 0) {
    return null;
  }

  const pct = Math.min(100, Math.round((progress.doneBytes / progress.totalBytes) * 100));
  const doneMb = (progress.doneBytes / (1024 * 1024)).toFixed(1);
  const totalMb = (progress.totalBytes / (1024 * 1024)).toFixed(1);

  return (
    <div className="parse-progress">
      <div className="parse-progress-header">
        <span className="parse-progress-label">解析中...</span>
        <span className="parse-progress-percent">{pct}%</span>
      </div>
      <div className="parse-progress-track">
        <div
          className="parse-progress-fill"
          style={{ width: `${pct}%` }}
        />
      </div>
      <div className="parse-progress-detail">
        {doneMb} MB / {totalMb} MB
        {progress.currentFrame > 0 && ` · frame ${progress.currentFrame}`}
      </div>
    </div>
  );
}