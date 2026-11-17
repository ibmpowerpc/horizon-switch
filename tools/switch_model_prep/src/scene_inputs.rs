//! Shared pinned input validation for the host-side scene and atlas tools.
use crate::BakeRequest;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const PIN: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
const CAP: usize = 8 * 1024 * 1024;
pub fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
pub fn read(path: &Path, cap: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err("regular input file required".into());
    }
    let mut b = Vec::new();
    fs::File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut b)?;
    if b.len() > cap {
        return Err("input byte limit".into());
    }
    Ok(b)
}
pub fn pinned(j: &Value) -> bool {
    j["schema"] == 1 && j["minecraft"] == "26.3" && j["protocol"] == 777 && j["source_sha1"] == PIN
}
pub fn load_manifest(manifest_path: &Path) -> Result<(BTreeMap<String, Vec<u8>>, String)> {
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err("asset root must be a regular directory".into());
    }
    let raw = read(manifest_path, CAP)?;
    let j: Value = serde_json::from_slice(&raw)?;
    if !pinned(&j) {
        return Err("asset pin mismatch".into());
    }
    let entries = j["files"].as_array().ok_or("missing manifest entries")?;
    if entries.len() > 12000 || j["count"].as_u64() != Some(entries.len() as u64) {
        return Err("asset count mismatch".into());
    }
    let mut files = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut total = 0;
    for e in entries {
        let p = e["path"].as_str().ok_or("bad asset path")?;
        if !p.starts_with("assets/")
            || p.contains('\\')
            || p.contains(':')
            || p.split('/').any(|s| s.is_empty() || s == "." || s == "..")
            || !folded.insert(p.to_lowercase())
        {
            return Err("unsafe or duplicate path".into());
        }
        let mut full = root.to_path_buf();
        for s in p.split('/') {
            full.push(s);
            if fs::symlink_metadata(&full)?.file_type().is_symlink() {
                return Err("symlink asset path".into());
            }
        }
        let b = read(&full, 2 * 1024 * 1024)?;
        total += b.len();
        if total + raw.len() > CAP
            || e["size"].as_u64() != Some(b.len() as u64)
            || e["sha256"].as_str() != Some(hash(&b).as_str())
        {
            return Err("asset size/hash/total mismatch".into());
        }
        files.insert(p.to_owned(), b);
    }
    if j["total_bytes"].as_u64() != Some(total as u64) {
        return Err("manifest total mismatch".into());
    }
    Ok((files, hash(&raw)))
}
pub fn request(j: &Value) -> Result<BakeRequest> {
    Ok(BakeRequest {
        model: j["model"].as_str().ok_or("missing model")?.to_owned(),
        x: u16::try_from(j["x"].as_u64().ok_or("missing x")?)?,
        y: u16::try_from(j["y"].as_u64().ok_or("missing y")?)?,
        uvlock: j["uvlock"].as_bool().ok_or("missing uvlock")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn manifest_array_checks_hashes_paths_and_total_before_baking() {
        let root = Scratch(std::env::temp_dir().join(format!(
            "switch-atlas-input-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )));
        fs::create_dir_all(root.0.join("assets/test/models")).unwrap();
        fs::write(root.0.join("assets/test/models/a.json"), b"abc").unwrap();
        let manifest = json!({"schema":1,"minecraft":"26.3","protocol":777,"source_sha1":PIN,
            "count":1,"total_bytes":3,"files":[{"path":"assets/test/models/a.json","size":3,
                "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]});
        let path = root.0.join("manifest.json");
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let (files, _) = load_manifest(&path).unwrap();
        assert_eq!(files["assets/test/models/a.json"], b"abc");
        for (field, value) in [
            ("size", json!(2)),
            ("sha256", json!("bad")),
            ("path", json!("assets/../secret")),
        ] {
            let mut bad = manifest.clone();
            bad["files"][0][field] = value;
            fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
            assert!(load_manifest(&path).is_err());
        }
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(root.0.join("assets/test/models/a.json"), b"abd").unwrap();
        assert!(load_manifest(&path).is_err());
    }
}
