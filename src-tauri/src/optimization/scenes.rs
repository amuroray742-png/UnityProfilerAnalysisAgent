//! Scene identity is independent of display names and shared source-code state.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub id: String,
    pub name: String,
    pub description: String,
    pub completed: bool,
    pub budgets: BTreeMap<String, f64>,
    pub last_code_revision: u64,
}

pub fn current<'a>(d: &'a Project, scene: &str) -> Option<&'a Round> {
    d.rounds.iter().rev().find(|r| r.scene_id == scene)
}

pub fn unresolved(r: &Round) -> bool {
    r.runs.iter().any(|run| {
        run.changes.iter().any(|c| {
            matches!(c.state.as_str(), "prepared" | "conflict")
                || (c.state == "applied" && r.decision != "accepted")
        })
    })
}

pub fn blocker(d: &Project, scene: &str) -> Option<String> {
    d.rounds
        .iter()
        .find(|r| r.scene_id != scene && unresolved(r))
        .map(|r| r.scene_id.clone())
}

pub fn access(d: &Project, scene: &str) -> Result<(), String> {
    let s = d
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .ok_or("请先创建并选择场景")?;
    if s.completed {
        return Err("场景已完成，请先重新开启".into());
    }
    if let Some(id) = blocker(d, scene) {
        let name = &d
            .scenes
            .iter()
            .find(|s| s.id == id)
            .ok_or("场景索引缺失")?
            .name;
        return Err(format!("请先接受或回退场景「{name}」的修改"));
    }
    Ok(())
}

pub fn stale(d: &Project, c: &Capture) -> bool {
    c.code_revision.is_none_or(|revision| {
        d.scenes
            .iter()
            .any(|s| s.id != c.scene_id && s.last_code_revision > revision)
    })
}

pub fn round_access(d: &Project, rid: &str, fresh: bool) -> Result<(), String> {
    let r = d.rounds.iter().find(|r| r.id == rid).ok_or("轮次不存在")?;
    access(d, &r.scene_id)?;
    if current(d, &r.scene_id).is_none_or(|last| last.id != rid) || r.decision != "pending" {
        return Err("历史轮次只读，请继续当前轮次".into());
    }
    if fresh {
        let c = d
            .captures
            .iter()
            .find(|c| c.id == r.baseline)
            .ok_or("基线录制不存在，请导入 A")?;
        if c.scene_id != r.scene_id || stale(d, c) {
            return Err("代码状态已变化或未知，请重新录制并导入 A，再诊断定位".into());
        }
    }
    Ok(())
}

// Reserve a revision durably before writing. Interrupted/failed writes are
// conservatively treated as possible changes, never as evidence of freshness.
pub fn touch(d: &mut Project, scene: &str) -> Result<(), String> {
    let s = d
        .scenes
        .iter_mut()
        .find(|s| s.id == scene)
        .ok_or("修改场景不存在")?;
    d.code_revision = d.code_revision.checked_add(1).ok_or("工程修订号溢出")?;
    s.last_code_revision = d.code_revision;
    Ok(())
}
pub fn touch_run(d: &mut Project, run: &str) -> Result<(), String> {
    let scene = d
        .rounds
        .iter()
        .find(|r| r.runs.iter().any(|s| s.id == run))
        .ok_or("运行不存在")?
        .scene_id
        .clone();
    touch(d, &scene)
}

pub fn create(d: &mut Project, name: String, description: String) -> Result<String, String> {
    fields(d, None, &name, &description)?;
    if d.rounds.iter().any(unresolved) {
        return Err("请先接受或回退当前修改".into());
    }
    let sid = id();
    d.scenes.push(Scene {
        id: sid.clone(),
        name: name.trim().into(),
        description,
        completed: false,
        budgets: BTreeMap::new(),
        last_code_revision: 0,
    });
    d.active_scene_id = sid.clone();
    Ok(sid)
}
pub fn fields(d: &Project, sid: Option<&str>, name: &str, description: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 80
        || name.chars().any(char::is_control)
        || description.chars().count() > 4000
    {
        return Err("场景名称需为 1..80 个字符，复现说明最多 4000 个字符".into());
    }
    if d.scenes
        .iter()
        .any(|s| Some(s.id.as_str()) != sid && s.name == name)
    {
        return Err("同一项目内场景名称不能重复".into());
    }
    Ok(())
}
pub fn edit(d: &mut Project, sid: &str, name: String, description: String) -> Result<(), String> {
    fields(d, Some(sid), &name, &description)?;
    let s = d
        .scenes
        .iter_mut()
        .find(|s| s.id == sid)
        .ok_or("场景不存在")?;
    s.name = name.trim().into();
    s.description = description;
    Ok(())
}
pub fn select(d: &mut Project, sid: &str) -> Result<(), String> {
    if !d.scenes.iter().any(|s| s.id == sid) {
        return Err("场景不存在".into());
    }
    d.active_scene_id = sid.into();
    Ok(())
}
pub fn complete(d: &mut Project, sid: &str, completed: bool) -> Result<(), String> {
    if d.rounds.iter().any(unresolved) {
        return Err("请先接受或回退修改".into());
    }
    d.scenes
        .iter_mut()
        .find(|s| s.id == sid)
        .ok_or("场景不存在")?
        .completed = completed;
    Ok(())
}
pub fn migrate(d: &mut Project) {
    d.code_revision = 0;
    d.run_sequence = 0;
    let sid = id();
    d.scenes = vec![Scene {
        id: sid.clone(),
        name: "默认场景".into(),
        description: String::new(),
        completed: false,
        budgets: d.budgets.clone(),
        last_code_revision: 0,
    }];
    d.active_scene_id = sid.clone();
    for c in &mut d.captures {
        c.scene_id = sid.clone();
        c.code_revision = None;
    }
    for r in &mut d.rounds {
        r.scene_id = sid.clone();
        for run in &mut r.runs {
            d.run_sequence += 1;
            run.sequence = d.run_sequence;
        }
    }
    d.version = 4;
}
pub fn validate(d: &Project) -> Result<(), String> {
    let ids: std::collections::HashSet<_> = d.scenes.iter().map(|s| &s.id).collect();
    if ids.len() != d.scenes.len()
        || (!d.active_scene_id.is_empty() && !ids.contains(&d.active_scene_id))
    {
        return Err("场景索引无效".into());
    }
    for c in &d.captures {
        if !ids.contains(&c.scene_id) {
            return Err("录制场景不存在".into());
        }
    }
    for r in &d.rounds {
        if !ids.contains(&r.scene_id) {
            return Err("轮次场景不存在".into());
        }
        for cid in std::iter::once(&r.baseline).chain(r.candidate.iter()) {
            if d.captures
                .iter()
                .any(|c| &c.id == cid && c.scene_id != r.scene_id)
            {
                return Err("录制不属于当前场景".into());
            }
        }
    }
    Ok(())
}
