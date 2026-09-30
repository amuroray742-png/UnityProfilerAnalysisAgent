import { useState } from 'react';
import type { DiagnoseEvent } from '../types/index.ts';

interface AgentLogDrawerProps {
  events: DiagnoseEvent[];
}

function roleClass(event: DiagnoseEvent): string {
  switch (event.kind) {
    case 'chunk':
    case 'log':
      return 'role-info';
    case 'tool-activity':
    case 'mcp-call':
    case 'mcp-result':
      return 'role-tool';
    case 'session-created':
    case 'started':
    case 'finished':
    case 'cancelled':
      return 'role-system';
    case 'error':
      return 'role-error';
  }
}

function eventLabel(event: DiagnoseEvent): string {
  switch (event.kind) {
    case 'tool-activity': return `[${event.tool}] ${event.status}`;
    case 'session-created': return `[ACP session] ${event.acpSessionId}`;
    case 'started':
      return `[started] ${event.agentId}`;
    case 'chunk':
      return '[chunk]';
    case 'mcp-call':
      return `[mcp → ${event.tool}]`;
    case 'mcp-result':
      return `[mcp ← ${event.tool}]`;
    case 'finished':
      return `[finished] ${event.stopReason} · total=${event.totalChunks}`;
    case 'cancelled':
      return '[cancelled]';
    case 'log':
      return '[log]';
    case 'error':
      return `[error] ${event.message}`;
  }
}

function eventPayload(event: DiagnoseEvent): string {
  switch (event.kind) {
    case 'chunk':
      return event.text.length > 200 ? event.text.slice(0, 200) + '...' : event.text;
    case 'log':
      return event.message.slice(0, 400);
    case 'mcp-call':
      return JSON.stringify(event.args, null, 2).slice(0, 400);
    case 'mcp-result':
      return JSON.stringify(event.result, null, 2).slice(0, 400);
    default:
      return '';
  }
}

export function AgentLogDrawer({ events }: AgentLogDrawerProps) {
  const [expanded, setExpanded] = useState<number | null>(null);

  if (events.length === 0) {
    return (
      <div className="agent-log-drawer">
        <div style={{ color: 'var(--text-secondary)', fontSize: 12, fontStyle: 'italic', textAlign: 'center', padding: 8 }}>
          Agent 日志会在 AI 诊断过程中显示
        </div>
      </div>
    );
  }

  return (
    <div className="agent-log-drawer">
      <div className="section-title" style={{ marginBottom: 8 }}>
        Agent Log ({events.length})
      </div>
      {events.map((event, i) => {
        const payload = eventPayload(event);
        const isExpanded = expanded === i;
        return (
          <div key={i} className="agent-log-entry">
            <div className={`role ${roleClass(event)}`}>{eventLabel(event)}</div>
            {payload && (
              <div
                className="payload"
                onClick={() => setExpanded(isExpanded ? null : i)}
                style={
                  isExpanded
                    ? { whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto' }
                    : { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }
                }
              >
                {payload}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
