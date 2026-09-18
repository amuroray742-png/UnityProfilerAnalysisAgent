import { useCallback, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';

interface UploadDropzoneProps {
  onFileSelected: (filePath: string) => void;
  disabled?: boolean;
}

const SUPPORTED_EXTENSIONS = ['pd3u', 'data', 'json', 'raw'] as const;

function isSupported(path: string): boolean {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  return (SUPPORTED_EXTENSIONS as readonly string[]).includes(ext);
}

/**
 * MVP 版本：只用点击打开文件对话框（最稳的入口）。
 * Tauri 原生 drag-drop 事件已注册在 listen('tauri://drag-drop', ...)
 * 但为了避免 WebView 重启问题，先回退到最稳定的点击模式。
 * 拖文件 → 也会触发 onClick → 打开对话框（用户可以选择）。
 */
export function UploadDropzone({ onFileSelected, disabled }: UploadDropzoneProps) {
  const [dragging, setDragging] = useState(false);
  const [dropError, setDropError] = useState<string | null>(null);

  const handleOpenDialog = useCallback(async () => {
    if (disabled) return;
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: '选择 Unity Profiler 文件',
        filters: [
          {
            name: 'Unity Profiler',
            extensions: [...SUPPORTED_EXTENSIONS],
          },
          {
            name: '所有文件',
            extensions: ['*'],
          },
        ],
      });
      if (typeof selected === 'string') {
        if (!isSupported(selected)) {
          const ext = selected.split('.').pop() ?? '(无后缀)';
          setDropError(`不支持的文件类型: .${ext}。仅支持 ${SUPPORTED_EXTENSIONS.join(', ')}`);
          return;
        }
        setDropError(null);
        onFileSelected(selected);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setDropError(`打开对话框失败: ${message}`);
    }
  }, [onFileSelected, disabled]);

  // HTML5 drag 视觉反馈：仅 hover 状态；不读取 file（WebView 下读不到 path）
  const onDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (!disabled) setDragging(true);
    },
    [disabled]
  );

  const onDragLeave = useCallback(() => {
    setDragging(false);
  }, []);

  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
  }, []);

  return (
    <div
      className={`dropzone ${dragging ? 'dragover' : ''}`}
      onClick={handleOpenDialog}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => e.key === 'Enter' && handleOpenDialog()}
    >
      <div className="dropzone-icon">📊</div>
      <div className="dropzone-title">
        {disabled ? '正在解析...' : '点击选择 Unity Profiler 文件'}
      </div>
      <div className="dropzone-hint">
        支持 Profiler 录制或导出的离线文件，最大 500MB
      </div>
      <div className="dropzone-formats">
        {SUPPORTED_EXTENSIONS.map((ext) => (
          <span key={ext} className="format-tag">.{ext}</span>
        ))}
      </div>
      {dropError && (
        <div className="error-banner" style={{ marginTop: 12 }}>
          {dropError}
        </div>
      )}
    </div>
  );
}