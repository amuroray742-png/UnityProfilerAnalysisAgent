use super::*;
use crate::parser::Frame;
use serde_json::{json, Value};
pub const METRICS: [&str; 8] = [
    "cpuMs",
    "frameMs",
    "gcBytes",
    "Draw Calls Count",
    "SetPass Calls Count",
    "Batches Count",
    "Triangles Count",
    "Vertices Count",
];
fn value(f: &Frame, key: &str) -> Option<f64> {
    if f.quality.estimated || f.quality.version_verified == Some(false) {
        return None;
    }
    match key {
        "cpuMs" => f.quality.cpu.then_some(f.cpu_ms),
        "frameMs" => f.quality.duration.then_some(f.duration_ms),
        "gcBytes" => f.quality.gc.then_some(f.gc_alloc_bytes as f64),
        "Draw Calls Count" => f.quality.draw.then_some(f.draw_calls as f64),
        "SetPass Calls Count" => f.quality.set_pass.then_some(f.set_pass_calls as f64),
        _ => f.render_counters.get(key).map(|v| *v as f64),
    }
    .filter(|v| v.is_finite() && *v >= 0.)
}
fn stats(frames: &[&Frame], key: &str) -> Value {
    let mut values: Vec<_> = frames.iter().filter_map(|f| value(f, key)).collect();
    values.sort_by(f64::total_cmp);
    let p = |q: f64| {
        if values.is_empty() {
            None
        } else {
            Some(values[((values.len() - 1) as f64 * q).round() as usize])
        }
    };
    json!({"validFrames":values.len(),"totalFrames":frames.len(),"p50":p(0.5),"p95":p(0.95),"p99":p(0.99),"max":values.last(),"mean":(!values.is_empty()).then(||values.iter().sum::<f64>()/values.len() as f64),"unverifiedExcluded":frames.iter().filter(|f|f.quality.version_verified==Some(false)).count(),"estimatedExcluded":frames.iter().filter(|f|f.quality.estimated).count()})
}
pub fn compare(
    a: &Capture,
    b: &Capture,
    range_a: Option<[usize; 2]>,
    range_b: Option<[usize; 2]>,
    confirmed: bool,
    budgets: &BTreeMap<String, f64>,
) -> Result<Value, String> {
    let select = |c: &Capture, r: Option<[usize; 2]>| -> Result<Vec<usize>, String> {
        if r.is_some_and(|r| r[0] > r[1]) {
            return Err("帧区间反向".into());
        }
        let indices: Vec<_> = c
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                r.map(|r| f.index >= r[0] && f.index <= r[1])
                    .unwrap_or(true)
            })
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            return Err("所选帧区间为空".into());
        }
        Ok(indices)
    };
    let ai = select(a, range_a)?;
    let bi = select(b, range_b)?;
    let af: Vec<_> = ai.iter().map(|i| &a.frames[*i]).collect();
    let bf: Vec<_> = bi.iter().map(|i| &b.frames[*i]).collect();
    let ca = serde_json::to_value(&a.conditions).unwrap();
    let cb = serde_json::to_value(&b.conditions).unwrap();
    let mut unknown = vec![];
    let mut mismatch = vec![];
    for key in [
        "device",
        "platform",
        "scenario",
        "operation",
        "build",
        "quality",
        "resolution",
        "profiling",
    ] {
        if ca[key] == "" || cb[key] == "" {
            unknown.push(key);
        } else if ca[key] != cb[key] {
            mismatch.push(key);
        }
    }
    if a.snapshot.meta.unity_version != b.snapshot.meta.unity_version {
        mismatch.push("unityVersion");
    }
    if a.snapshot.meta.unity_version.is_none() || b.snapshot.meta.unity_version.is_none() {
        unknown.push("unityVersion");
    }
    if a.snapshot.meta.source != b.snapshot.meta.source {
        mismatch.push("metricSource");
    }
    // An exported subset cannot establish a verdict for the entire recording.
    // Explicit ranges have their own coverage, rather than inheriting recording size.
    if range_a.is_none() && a.snapshot.meta.declared_frame_count > a.frames.len() {
        unknown.push("A 部分导出，未限定复验区间");
    }
    if range_b.is_none() && b.snapshot.meta.declared_frame_count > b.frames.len() {
        unknown.push("B 部分导出，未限定复验区间");
    }
    for (frames, range, label) in [(&af, range_a, "A 区间缺帧"), (&bf, range_b, "B 区间缺帧")]
    {
        if let Some([start, end]) = range {
            if end.checked_sub(start).and_then(|n| n.checked_add(1)) != Some(frames.len()) {
                unknown.push(label);
            }
        }
    }
    let comparable = mismatch.is_empty() && unknown.is_empty() && confirmed;
    let mut rows = vec![];
    for key in METRICS {
        let av = stats(&af, key);
        let bv = stats(&bf, key);
        let full = av["validFrames"] == av["totalFrames"] && bv["validFrames"] == bv["totalFrames"];
        let mut deltas = serde_json::Map::new();
        for p in ["mean", "p50", "p95", "p99", "max"] {
            let delta = av[p].as_f64().zip(bv[p].as_f64()).map(|(a, b)| b - a);
            deltas.insert(p.into(),json!({"absolute":delta,"percent":av[p].as_f64().filter(|v|*v!=0.).zip(delta).map(|(a,d)|100.*d/a)}));
        }
        let budget = budgets.get(key).copied();
        rows.push(json!({"metric":key,"a":av,"b":bv,"delta":deltas,"budgetP95":budget,
            "verdict":if !comparable||!full {"不可判定"}else if let Some(target)=budget {if bv["p95"].as_f64().is_some_and(|v|v<=target){"达到目标"}else{"未达到目标"}}else{"未设目标"}}));
    }
    Ok(
        json!({"comparedAt":chrono::Utc::now().to_rfc3339(),"conditionsA":a.conditions,"conditionsB":b.conditions,"budgets":budgets,"baseline":a.id,"candidate":b.id,"rangeA":range_a,"rangeB":range_b,"conditionsConfirmed":confirmed,"comparability":if !mismatch.is_empty(){"不宜直接比较"}else if comparable{"条件一致已确认"}else{"有限可比"},"unknown":unknown,"mismatch":mismatch,"metrics":rows,"scope":"B − A 是观测差异，不证明修改因果或统计显著性。条件和预算按本次复验冻结；预算针对 p95；新增 Marker 无 A 值，代码版本变化是预期变化。部分覆盖/估算不作完整达标结论。"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparison_zero_missing_and_conditions() {
        let data =
            bytes::Bytes::from_static(include_bytes!("../../tests/fixtures/editor-dump.json"));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let p = runtime
            .block_on(crate::parser::json::parse(
                &data,
                "test.json",
                data.len() as u64,
            ))
            .unwrap();
        let mut a = Capture {
            id: "a".into(),
            path: PathBuf::new(),
            hash: String::new(),
            conditions: Conditions::default(),
            snapshot: crate::extractor::extract(&p),
            frames: p.frames,
        };
        let mut b = a.clone();
        b.id = "b".into();
        assert_eq!(
            compare(&a, &b, None, None, true, &BTreeMap::new()).unwrap()["comparability"],
            "有限可比"
        );
        for c in [&mut a, &mut b] {
            c.conditions = Conditions {
                device: "PC".into(),
                platform: "Windows".into(),
                scenario: "test".into(),
                operation: "same".into(),
                build: "Development".into(),
                quality: "high".into(),
                resolution: "1080".into(),
                profiling: "normal".into(),
                code_version: "1".into(),
            };
        }
        let v = compare(
            &a,
            &b,
            Some([12, 12]),
            Some([12, 12]),
            true,
            &BTreeMap::from([("cpuMs".into(), 20.)]),
        )
        .unwrap();
        assert_eq!(v["metrics"][0]["verdict"], "达到目标");
        assert_eq!(v["metrics"][2]["delta"]["p95"]["absolute"], 0.);
        assert!(v["metrics"][2]["delta"]["p95"]["percent"].is_null());
        b.conditions.device = "Other".into();
        assert_eq!(
            compare(&a, &b, None, None, true, &BTreeMap::new()).unwrap()["comparability"],
            "不宜直接比较"
        );
    }
}

/// Cross-capture association uses role and complete marker names, never numeric IDs.
pub async fn paths(
    c: &Capture,
    range: Option<[usize; 2]>,
) -> Result<BTreeMap<String, Value>, String> {
    let path = c.path.clone();
    let expected = c.hash.clone();
    let p = path.clone();
    if tokio::task::spawn_blocking(move || storage::file_hash(&p, &AtomicBool::new(false)))
        .await
        .map_err(|e| e.to_string())??
        != expected
    {
        return Err("录制已变化，不能重用已保存证据".into());
    }
    let profile = crate::parser::parse_file(&path)
        .await
        .map_err(|e| e.to_string())?;
    let store = profile.details.ok_or("录制无调用树，热点关联不可用")?;
    let indices: Vec<_> = profile
        .frames
        .iter()
        .filter(|f| {
            range
                .map(|r| f.index >= r[0] && f.index <= r[1])
                .unwrap_or(true)
        })
        .map(|f| f.index)
        .collect();
    let result=tokio::task::spawn_blocking(move||->Result<BTreeMap<String,Value>,String>{
        #[derive(Default)]struct Acc{ms:f64,calls:u64,gc:u64,gc_valid:u64,frames:usize}
        let total=indices.len();let mut output:BTreeMap<String,Acc>=BTreeMap::new();
        for index in indices {
            let frame=store.load(index).map_err(|e|e.to_string())?;let mut seen=std::collections::HashSet::new();
            for thread in &frame.threads {
                let role=if thread.info.name.starts_with("Job.Worker") {"Job.Worker".to_string()}else{thread.info.name.clone()};
                let mut stack:Vec<String>=vec![];
                for sample in &thread.samples {
                    if sample.depth>64{return Err("调用树超过 64 层，停止完整路径关联".into());}
                    if sample.depth>stack.len(){return Err("样本路径结构不完整".into());}
                    stack.truncate(sample.depth);stack.push(sample.name.clone());
                    let key=serde_json::to_string(&(thread.info.group.clone(),&role,&stack)).unwrap();
                    if !output.contains_key(&key)&&output.len()>=100000{return Err("热点路径超过 100000，缩小帧范围后重试".into());}
                    let row=output.entry(key.clone()).or_default();row.ms+=sample.total_ms;row.calls+=1;
                    if let Some(b)=sample.gc_alloc_bytes {row.gc+=b;row.gc_valid+=1;}
                    if seen.insert(key){row.frames+=1;}
                }
            }
        }
        Ok(output.into_iter().map(|(k,v)|(k,json!({"inclusiveMsPerFrame":v.ms/total as f64,"callsPerFrame":v.calls as f64/total as f64,"gcBytesPerFrame":(v.gc_valid==v.calls).then_some(v.gc as f64/total as f64),"observedFrames":v.frames,"totalFrames":total}))).collect())
    }).await.map_err(|e|e.to_string())??;
    if storage::file_hash(&path, &AtomicBool::new(false))? != expected {
        return Err("查询期间录制变化".into());
    }
    Ok(result)
}
pub fn associate(a: BTreeMap<String, Value>, b: BTreeMap<String, Value>) -> Value {
    let mut rows = vec![];
    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
    for key in keys {
        let av = a.get(key);
        let bv = b.get(key);
        let delta = av
            .and_then(|v| v["inclusiveMsPerFrame"].as_f64())
            .zip(bv.and_then(|v| v["inclusiveMsPerFrame"].as_f64()))
            .map(|(a, b)| b - a);
        rows.push(json!({"roleAndPath":serde_json::from_str::<Value>(key).unwrap(),"a":av,"b":bv,"deltaInclusiveMsPerFrame":delta,"association":if av.is_none(){"仅 B 新证据"}else if bv.is_none(){"仅 A 观测，不推断耗时归零"}else{"同角色同完整路径候选，不证明因果"}}));
    }
    rows.sort_by(|a, b| {
        b["deltaInclusiveMsPerFrame"]
            .as_f64()
            .unwrap_or(0.)
            .abs()
            .total_cmp(&a["deltaInclusiveMsPerFrame"].as_f64().unwrap_or(0.).abs())
    });
    json!({"rows":rows,"scope":"inclusive 父子不可相加；Worker 同角色汇总不代表线程对应。新/消失/改名路径没有自动对应值；不以线程、Marker、Flow 或对象 ID 跨录制关联。"})
}
