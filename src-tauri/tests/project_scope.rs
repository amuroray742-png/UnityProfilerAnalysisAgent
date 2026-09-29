//! Public temporary fixtures; never reads a private Unity project.
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use unity_profiler_analysis_agent_lib::project::{editor, ProjectScope};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let t = Self(std::env::temp_dir().join(format!("upaa-project-{}", uuid::Uuid::new_v4())));
        for dir in ["Assets", "Packages", "ProjectSettings"] {
            std::fs::create_dir_all(t.0.join(dir)).unwrap();
        }
        t.write(
            "ProjectSettings/ProjectVersion.txt",
            "m_EditorVersion: 6000.3.23f1\n",
        );
        t
    }
    fn write(&self, p: &str, b: impl AsRef<[u8]>) {
        let path = self.0.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b).unwrap();
    }
    fn scope(&self) -> Arc<ProjectScope> {
        Arc::new(
            ProjectScope::prepare(
                "public".into(),
                self.0.clone(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap(),
        )
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
async fn graph_tracks_guid_file_id_missing_duplicate_and_prefab_overrides() {
    let t = Temp::new();
    let guid = "1234567890abcdef1234567890abcdef";
    t.write(
        "Assets/Child.prefab",
        "%YAML 1.1\n--- !u!1 &100\nGameObject:\n  m_Name: Child\n",
    );
    t.write("Assets/Child.prefab.meta", format!("guid: {guid}\n"));
    t.write("Assets/Parent.prefab",format!("--- !u!1001 &200\nPrefabInstance:\n  m_SourcePrefab: {{fileID: 100, guid: {guid}, type: 3}}\n  m_Modification:\n    - target: {{fileID: 100, guid: {guid}, type: 3}}\n      propertyPath: m_IsActive\n      value: 0\n  missing: {{fileID: 5, guid: ffffffffffffffffffffffffffffffff, type: 3}}\n"));
    let s = t.scope();
    let refs = s
        .query(
            "project_references",
            json!({"path":"Assets/Child.prefab","direction":"incoming","limit":1}),
        )
        .await
        .unwrap();
    assert_eq!(refs["rows"][0]["objectResolved"], true);
    assert_eq!(refs["rows"][0]["reference"]["objectId"], "200");
    assert_eq!(refs["nextStart"], 1);
    assert_eq!(refs["coverage"], "partial");
    assert!(s.info.warnings.iter().any(|w| w.contains("未解析")));
    t.write("Assets/Duplicate.mat", "--- !u!21 &21\nMaterial:\n");
    t.write("Assets/Duplicate.mat.meta", format!("guid: {guid}\n"));
    t.write("Assets/Bad.asset", "--- !u!1 &1\n--- !u!1 &1\n");
    let s = t.scope();
    assert!(s.info.warnings.iter().any(|w| w.contains("重复 GUID")));
    assert!(s.info.warnings.iter().any(|w| w.contains("重复 fileID")));
    let r = s
        .query("project_references", json!({"path":"Assets/Parent.prefab"}))
        .await
        .unwrap();
    assert_eq!(r["rows"][0]["objectResolved"], false);
    assert_eq!(r["rows"][0]["targetPaths"].as_array().unwrap().len(), 2);
    t.write(
        "Assets/Child.prefab.meta",
        "guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    );
    assert!(s
        .query("project_asset", json!({"path":"Assets/Child.prefab"}))
        .await
        .unwrap_err()
        .contains("变化"));
}
#[tokio::test]
async fn text_bounds_encoding_paging_hash_changes_and_isolation() {
    let t = Temp::new();
    t.write("Assets/中文.cs", "class Work {}\n// Work\n// last\n");
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend("// 中文 Work".encode_utf16().flat_map(u16::to_le_bytes));
    t.write("Packages/Local/Utf16.cs", utf16);
    t.write("Assets/Library/excluded.cs", "secret Work");
    t.write("Assets/large.cs", vec![b'a'; 2 * 1024 * 1024 + 1]);
    t.write("Assets/binary.asset", [0, 1, 2]);
    std::fs::File::create(t.0.join("Assets/oversize.asset"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let s = t.scope();
    let r = s
        .query("project_search", json!({"query":"Work","limit":1}))
        .await
        .unwrap();
    assert_eq!(r["nextStart"], 1);
    assert_eq!(r["rows"][0]["line"], 1);
    let r = s
        .query(
            "project_read",
            json!({"path":"Assets/中文.cs","start_line":2,"limit":1}),
        )
        .await
        .unwrap();
    assert_eq!(r["nextStart"], 3);
    assert_eq!(r["rows"][0]["text"], "// Work");
    for path in [
        "../outside.cs",
        "C:/outside.cs",
        "Assets/Library/excluded.cs",
        "Assets/large.cs",
        "Assets/binary.asset",
        "Assets/oversize.asset",
    ] {
        assert!(s.query("project_read", json!({"path":path})).await.is_err());
    }
    for args in [json!({"limit":101}), json!({"limit":0}), json!({"other":1})] {
        assert!(s.query("project_files", args).await.is_err());
    }
    let other = Temp::new();
    other.write("Assets/OnlyOther.cs", "private");
    assert!(s
        .query("project_read", json!({"path":"Assets/OnlyOther.cs"}))
        .await
        .is_err());
    t.write("Assets/中文.cs", "changed");
    assert!(s
        .query("project_search", json!({"query":"Work"}))
        .await
        .unwrap_err()
        .contains("变化"));
    s.cancelled.store(true, Ordering::Relaxed);
    assert!(s.query("project_files", json!({})).await.is_err());
}
#[tokio::test]
async fn responses_are_bounded_and_page_without_losing_rows() {
    let t = Temp::new();
    t.write(
        "Assets/long.cs",
        (0..300)
            .map(|_| format!("// {}\n", "字".repeat(800)))
            .collect::<String>(),
    );
    let s = t.scope();
    let mut start = 1;
    let mut total = 0;
    loop {
        let v = s
            .query(
                "project_read",
                json!({"path":"Assets/long.cs","start_line":start,"limit":400}),
            )
            .await
            .unwrap();
        let wire = json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&v).unwrap()}],"structuredContent":v});
        assert!(serde_json::to_vec(&wire).unwrap().len() < 64 * 1024);
        let rows = wire["structuredContent"]["rows"].as_array().unwrap();
        assert!(rows.iter().all(|r| r["lineTruncated"] == true));
        total += rows.len();
        match wire["structuredContent"]["nextStart"].as_u64() {
            Some(n) => start = n,
            None => break,
        }
    }
    assert_eq!(total, 300);
}
#[test]
fn editor_identity_protocol_and_busy_are_validated() {
    let a = Temp::new();
    let b = Temp::new();
    let mut v = json!({"protocolVersion":1,"projectRoot":a.0,"status":"ready"});
    assert!(editor::validate(&v, &a.0).is_ok());
    assert!(editor::validate(&v, &b.0).is_err());
    v["protocolVersion"] = json!(2);
    assert!(editor::validate(&v, &a.0).is_err());
    v["protocolVersion"] = json!(1);
    v["status"] = json!("busy");
    assert!(editor::validate(&v, &a.0).is_err());
}
#[cfg(windows)]
#[tokio::test]
async fn junctions_are_not_followed() {
    let a = Temp::new();
    let b = Temp::new();
    b.write("secret.cs", "secret");
    let link = a.0.join("Assets").join("Link");
    let result = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&b.0)
        .output()
        .unwrap();
    assert!(result.status.success());
    let s = a.scope();
    assert!(s
        .query("project_read", json!({"path":"Assets/Link/secret.cs"}))
        .await
        .is_err());
    std::fs::remove_dir(&link).unwrap();
}
#[tokio::test]
#[ignore = "requires UPAA_PUBLIC_UNITY_PROJECT with running Unity 6000.3 and public fixture marker"]
async fn live_editor_public_assets_match_known_fixture() {
    let root = PathBuf::from(
        std::env::var_os("UPAA_PUBLIC_UNITY_PROJECT").expect("set public fixture path"),
    );
    assert!(root.join(".upaa-public-fixture").is_file());
    fn hashes(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, String> {
        use sha2::{Digest, Sha256};
        let mut result = std::collections::BTreeMap::new();
        let mut pending = vec![
            root.join("Assets"),
            root.join("Packages"),
            root.join("ProjectSettings"),
        ];
        while let Some(p) = pending.pop() {
            for entry in std::fs::read_dir(p).unwrap() {
                let e = entry.unwrap();
                if e.file_type().unwrap().is_dir() {
                    pending.push(e.path());
                } else {
                    result.insert(
                        e.path(),
                        format!("{:x}", Sha256::digest(std::fs::read(e.path()).unwrap())),
                    );
                }
            }
        }
        result
    }
    let before = hashes(&root);
    let s = Arc::new(
        ProjectScope::prepare(
            "public".into(),
            root.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let status = editor::status(&root, &s.cancelled).await;
    assert_eq!(status.status, "ready", "{status:?}");
    assert_eq!(status.unity_version.as_deref(), Some("6000.3.23f1"));
    let mut records = vec![];
    for path in [
        "Assets/PublicMesh.asset",
        "Assets/PublicTexture.png",
        "Assets/PublicMaterial.mat",
        "Assets/Public.prefab",
        "Assets/PublicVariant.prefab",
        "Assets/PublicScene.unity",
    ] {
        let v = s
            .query("project_asset", json!({"path":path,"limit":30}))
            .await
            .unwrap();
        assert_eq!(v["editorEvidence"]["status"], "ready", "{v}");
        records.push(v);
    }
    let mesh = records[0]["editorEvidence"]["rows"].as_array().unwrap();
    assert!(mesh
        .iter()
        .any(|r| r["property"] == "vertexCount" && r["value"] == 3));
    let tex = records[1]["editorEvidence"]["rows"].as_array().unwrap();
    assert!(tex.iter().any(|r| r["property"] == "Android"
        && r["value"]["overridden"] == true
        && r["value"]["maxTextureSize"] == 32));
    let mut start = 0;
    let mut renderer = false;
    let mut lod = false;
    loop {
        let page = s
            .query(
                "project_asset",
                json!({"path":"Assets/PublicVariant.prefab","start":start,"limit":20}),
            )
            .await
            .unwrap();
        let evidence = &page["editorEvidence"];
        assert_eq!(evidence["status"], "ready", "{page}");
        for row in evidence["rows"].as_array().unwrap() {
            if row["kind"] == "renderer" {
                assert_eq!(row["value"]["shadowCasting"], "Off");
                renderer = true;
            }
            if row["kind"] == "lod" {
                assert_eq!(row["value"][0]["renderers"], 1);
                lod = true;
            }
        }
        if renderer && lod {
            break;
        }
        start = evidence["nextStart"]
            .as_u64()
            .expect("variant should expose Renderer and LOD");
        assert!(start < 500, "fixture query must stay bounded");
    }
    println!(
        "PUBLIC_EDITOR_EVIDENCE {}",
        serde_json::to_string_pretty(&records).unwrap()
    );
    assert_eq!(records[5]["editorEvidence"]["sceneDirty"], false);
    assert_eq!(
        before,
        hashes(&root),
        "read commands must not modify business assets/settings/packages"
    );
    let offline = Temp::new();
    let result = editor::status(&offline.0, &AtomicBool::new(false)).await;
    assert_eq!(result.status, "unavailable");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let target = root.clone();
    let task = tokio::spawn(async move {
        editor::inspect(&target, "Assets/Public.prefab", 0, 20, &flag).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    cancel.store(true, Ordering::Relaxed);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(
        editor::status(&root, &AtomicBool::new(false)).await.status,
        "ready",
        "cancellation must not close Editor"
    );
}
