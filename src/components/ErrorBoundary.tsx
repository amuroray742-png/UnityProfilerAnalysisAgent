import { Component, type ErrorInfo, type ReactNode } from 'react';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

/**
 * 捕获渲染期错误，避免整个 React 树黑屏。
 * 错误信息展示给用户 + 给出重置按钮。
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null, errorInfo: null };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error('[ErrorBoundary]', error, errorInfo);
    this.setState({ errorInfo });
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null, errorInfo: null });
  };

  handleReload = () => {
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      return (
        <div style={{ padding: 24, fontFamily: 'system-ui', background: '#0e1117', color: '#c9d1d9', height: '100vh', overflow: 'auto' }}>
          <h1 style={{ color: '#f85149', marginBottom: 12 }}>⚠️ 界面渲染出错</h1>
          <p style={{ marginBottom: 12, color: '#8b949e' }}>
            React 组件抛出了未捕获的错误。请按下方按钮重置或重载窗口。
          </p>
          <pre
            style={{
              background: '#161b22',
              border: '1px solid #30363d',
              borderRadius: 6,
              padding: 12,
              fontSize: 12,
              overflow: 'auto',
              marginBottom: 12,
            }}
          >
            {this.state.error?.message}
            {'\n\n'}
            {this.state.error?.stack}
          </pre>
          <div style={{ display: 'flex', gap: 8 }}>
            <button onClick={this.handleReset} style={btnStyle}>
              重置界面状态
            </button>
            <button onClick={this.handleReload} style={{ ...btnStyle, background: '#58a6ff', color: '#0e1117' }}>
              重新加载窗口
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

const btnStyle: React.CSSProperties = {
  padding: '8px 16px',
  border: '1px solid #30363d',
  borderRadius: 6,
  background: '#161b22',
  color: '#c9d1d9',
  fontSize: 13,
  cursor: 'pointer',
};