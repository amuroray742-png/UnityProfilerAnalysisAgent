import { useEffect, useState } from 'react';
import type { Project } from '../types/optimization';

export function ScenePanel({ project, pending, onAction }: {
  project: Project; pending: boolean;
  onAction: (action: Record<string, unknown>, sceneId?: string) => Promise<unknown>;
}) {
  const scenes = project.scenes ?? [];
  const scene = scenes.find(s => s.id === project.activeSceneId);
  const [editing, setEditing] = useState<'new' | 'edit' | null>(null);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const unresolved = scenes.some(s => s.unresolved);
  const latestLabel = (sid: string) => {
    const r = project.rounds.filter(r => r.sceneId === sid).at(-1);
    if (!r) return '等待导入';
    const status = r.decision !== 'pending' ? r.decision : r.runs.at(-1)?.status || r.workflow?.status || 'pending';
    return ({accepted:'已接受',rolled_back:'已回退',running:'正在进行',completed:'诊断完成',modified:'待复测',partial:'部分完成',failed:'失败',cancelled:'已停止',interrupted:'已中断',investigated:'调查完成',pending:'待处理'} as Record<string,string>)[status] ?? '待处理';
  };
  useEffect(() => { setEditing(null); }, [project.id, project.activeSceneId]);
  return <section className="workflow-card" aria-label="优化场景">
    <div className="workflow-heading"><h2>优化场景</h2><button disabled={pending || project.busy || unresolved} onClick={() => {
      setEditing('new'); setName(''); setDescription('');
    }}>新建场景</button></div>
    <p>按测试情境组织工作，例如“主城跑图”或“百人战斗”。所有场景共用同一份工程代码。</p>
    <nav className="scene-list" aria-label="选择优化场景">{scenes.map(s => <button key={s.id} disabled={pending} aria-pressed={s.id === scene?.id} onClick={() => onAction({ op: 'sceneSelect' }, s.id)}>
      <strong>{s.name}</strong><span>{s.completed ? '已完成' : '进行中'} · {s.roundCount} 轮</span>
      {s.latestStatus && <span>最近一轮：{latestLabel(s.id)}</span>}
      {s.needsRecording && <span>需要重新录制</span>}{s.unresolved && <span>有待处理修改</span>}
    </button>)}</nav>
    {!scenes.length && <p>先新建并命名一个场景，再导入第一轮录制。</p>}
    {scene && <><h3>{scene.name}</h3><p>{scene.description || '尚未填写复现说明。'}</p>
      <div className="workflow-actions"><button disabled={pending} onClick={() => { setEditing('edit'); setName(scene.name); setDescription(scene.description); }}>编辑场景</button>
        <button disabled={pending || project.busy || unresolved} onClick={() => onAction({ op: 'sceneComplete', completed: !scene.completed }, scene.id)}>{scene.completed ? '重新开启场景' : '标记场景已完成'}</button></div>
      {scene.completed && <p>“已完成”是人工管理标记，不代表性能达标。继续工作前请重新开启。</p>}
      {scene.needsRecording && <p role="status">工程代码已由其他场景修改，或旧录制的代码状态未知。历史结论保留，继续优化前必须重新录制并导入 A。</p>}
      {scene.blockedBy && <p role="alert">其他场景还有待处理修改。<button disabled={pending} onClick={() => onAction({ op: 'sceneSelect' }, scene.blockedBy!)}>返回待处理场景</button></p>}
    </>}
    {editing && <form onSubmit={async e => {
      e.preventDefault();
      const result = await onAction({ op: editing === 'new' ? 'sceneCreate' : 'sceneEdit', name, description }, editing === 'edit' ? scene?.id : undefined);
      if (result) setEditing(null);
    }}>
      <label>场景名称<input required maxLength={80} value={name} onChange={e => setName(e.target.value)} /></label>
      <label>复现说明<textarea maxLength={4000} value={description} onChange={e => setDescription(e.target.value)} placeholder="记录初始状态、操作步骤和录制范围" /></label>
      <button disabled={pending || !name.trim()} type="submit">保存场景</button><button type="button" onClick={() => setEditing(null)}>取消编辑</button>
    </form>}
  </section>;
}
