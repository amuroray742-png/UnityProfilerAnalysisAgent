// Public protocol fixture: performs actual MCP calls through the shipped bridge.
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const mode = process.argv[2] || 'success';
const sessionId = 'fixture-session';
let config, promptId;
const permissions = new Map();
const send = m => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...m }) + '\n');
const result = (id, result) => send({ id, result });
const chunk = (text, session = sessionId) => send({ method: 'session/update', params: { sessionId: session, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text } } } });
async function query() {
  const env = { ...process.env, ...Object.fromEntries(config.env.map(e => [e.name, e.value])) };
  const bridge = spawn(config.command, config.args, { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  let id = 0;
  const pending = new Map();
  readline.createInterface({ input: bridge.stdout }).on('line', line => {
    const m = JSON.parse(line); const entry = pending.get(m.id);
    if (entry) { pending.delete(m.id); m.error ? entry.reject(new Error(JSON.stringify(m.error))) : entry.resolve(m.result); }
  });
  bridge.on('exit', () => { for (const p of pending.values()) p.reject(new Error('bridge exited')); });
  const notify = (method, params) => bridge.stdin.write(JSON.stringify({ jsonrpc: '2.0', method, params }) + '\n');
  const request = (method, params) => new Promise((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    bridge.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  await request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'acp-fixture', version: '1' } });
  notify('notifications/initialized', {});
  const list = await request('tools/list', {});
  if (list.tools.length !== 7) throw new Error('tools missing');
  const data = await request('tools/call', { name: 'performance_cpu_hierarchy', arguments: { frame_index: 10, start: 2, limit: 2, max_depth: 64 } });
  if (data.structuredContent.samples[0].gcAllocBytes !== 20) throw new Error('wrong data');
  chunk('帧 10 的嵌套分配为 20 B 与 4 B。');
  bridge.stdin.end();
}
readline.createInterface({ input: process.stdin }).on('line', async line => {
  const m = JSON.parse(line);
  if (!m.method && permissions.has(m.id)) { permissions.get(m.id)(m.result); permissions.delete(m.id); return; }
  if (m.method === 'initialize') {
    if (mode === 'hang-initialize') return;
    result(m.id, { protocolVersion: mode === 'version' ? 2 : 1, agentCapabilities: {} });
  } else if (m.method === 'session/new') {
    if (m.params.mcpServers.length !== 1) throw new Error('MCP configuration missing');
    config = m.params.mcpServers[0]; result(m.id, { sessionId });
  } else if (m.method === 'session/prompt') {
    promptId = m.id;
    process.stderr.write('ordinary diagnostic log\n');
    chunk('STALE', 'other-session');
    if (mode === 'error') { send({ id: m.id, error: { code: -32000, message: 'fixture failure' } }); return; }
    if (mode === 'malformed') { process.stdout.write('not JSON\n'); return; }
    if (mode === 'cancel' || mode === 'uncooperative') {
      const descendant = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { windowsHide: true, stdio: 'ignore' });
      process.stderr.write(`DESCENDANT_PID=${descendant.pid}\n`); chunk('waiting'); return;
    }
    try {
      const permission = (id, title) => new Promise(resolve => {
        permissions.set(id, resolve);
        send({ id, method: 'session/request_permission', params: { sessionId, toolCall: { toolCallId: id, title }, options: [{ kind: 'allow_once', optionId: 'allow' }] } });
      });
      const allowed = await permission('p1', 'mcp__unity-profiler__performance_cpu_hierarchy');
      if (allowed.outcome.optionId !== 'allow') throw new Error('read-only MCP permission denied');
      const denied = await permission('p2', 'Terminal');
      if (denied.outcome.outcome !== 'cancelled') throw new Error('unexpected terminal permission');
      await query(); result(m.id, { stopReason: mode === 'limit' ? 'max_tokens' : 'end_turn' });
    }
    catch (error) { send({ id: m.id, error: { code: -32000, message: error.message } }); }
  } else if (m.method === 'session/cancel' && mode !== 'uncooperative') {
    result(promptId, { stopReason: 'cancelled' });
  }
});
