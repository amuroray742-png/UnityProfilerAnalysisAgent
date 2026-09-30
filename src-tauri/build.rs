use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn collect(root: &Path, dir: &Path, rows: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        assert!(
            !fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "linked plugin resource"
        );
        if path.is_dir() {
            collect(root, &path, rows);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let bytes = fs::read(&path).unwrap();
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
                .join("plugin")
                .join(&name);
            fs::create_dir_all(output.parent().unwrap()).unwrap();
            fs::write(&output, bytes).unwrap();
            rows.push(format!(
                "({name:?}, include_bytes!({path:?}), {hash:?})",
                path = output.to_str().unwrap()
            ));
        }
    }
}
fn main() {
    let root = Path::new("../unity/Packages/com.upaa.inspector");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut rows = vec![];
    collect(root, root, &mut rows);
    assert!(
        rows.iter().any(|r| r.starts_with("(\"package.json\"")),
        "plugin package missing"
    );
    fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("plugin_bundle.rs"),
        format!("&[{}]", rows.join(",\n")),
    )
    .unwrap();
    tauri_build::build();
}
