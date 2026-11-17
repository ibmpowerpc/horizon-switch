//! Host-only model selection. No terrain is baked or rendered by this tool.
use craftrustnx_model_prep::selector::select;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const PIN: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
const MAX_INPUT: usize = 12 * 1024 * 1024;
const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_OPTIONS: usize = 262_144;
const MAX_PARTS: usize = 131_072;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fail(message: &str) -> Box<dyn std::error::Error> {
    message.to_owned().into()
}

fn read_bounded(path: &Path, cap: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(fail("input must be a regular file, not a symlink"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(fail("input exceeds byte limit"));
    }
    Ok(bytes)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn resource_path(id: &str, kind: &str) -> Result<String> {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    if namespace.is_empty()
        || path.is_empty()
        || id.len() > 256
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_./-".contains(&c))
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(fail("invalid resource identifier"));
    }
    Ok(format!("assets/{namespace}/{kind}/{path}.json"))
}

struct Assets {
    files: BTreeMap<String, Vec<u8>>,
    manifest_hash: String,
}

impl Assets {
    fn load(root: &Path) -> Result<Self> {
        let raw = read_bounded(&root.join("manifest.json"), 8 * 1024 * 1024)?;
        let manifest: Value = serde_json::from_slice(&raw)?;
        if manifest["schema"] != 1
            || manifest["minecraft"] != "26.3"
            || manifest["protocol"] != 777
            || manifest["source_sha1"] != PIN
        {
            return Err(fail("asset manifest pin mismatch"));
        }
        let entries = manifest["files"]
            .as_array()
            .ok_or_else(|| fail("missing asset entries"))?;
        if entries.len() > 12_000 || manifest["count"].as_u64() != Some(entries.len() as u64) {
            return Err(fail("asset count mismatch/limit"));
        }
        let mut files = BTreeMap::new();
        let mut folded = BTreeSet::new();
        let mut total = 0usize;
        for entry in entries {
            let path = entry["path"]
                .as_str()
                .ok_or_else(|| fail("invalid manifest path"))?;
            if !path.starts_with("assets/")
                || path.contains('\\')
                || path
                    .split('/')
                    .any(|c| c.is_empty() || c == "." || c == "..")
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                || !folded.insert(path.to_lowercase())
            {
                return Err(fail("unsafe/duplicate manifest path"));
            }
            let mut file = root.to_path_buf();
            for component in Path::new(path).components() {
                file.push(component.as_os_str());
                if fs::symlink_metadata(&file)?.file_type().is_symlink() {
                    return Err(fail("asset path contains symlink"));
                }
            }
            let bytes = read_bounded(&file, MAX_FILE)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| fail("asset length overflow"))?;
            if total + raw.len() > 8 * 1024 * 1024
                || entry["size"].as_u64() != Some(bytes.len() as u64)
                || entry["sha256"].as_str() != Some(hash(&bytes).as_str())
            {
                return Err(fail("asset hash/size mismatch or total limit"));
            }
            files.insert(path.to_owned(), bytes);
        }
        if manifest["total_bytes"].as_u64() != Some(total as u64) {
            return Err(fail("asset total_bytes mismatch"));
        }
        Ok(Self {
            files,
            manifest_hash: hash(&raw),
        })
    }

    fn json(&self, path: &str) -> Result<Value> {
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| fail(&format!("unlisted/missing asset: {path}")))?;
        let value: Value = serde_json::from_slice(bytes)?;
        if !value.is_object() {
            return Err(fail("asset JSON must be an object"));
        }
        Ok(value)
    }
}

fn prepare(facts_path: &Path, assets: &Assets) -> Result<Value> {
    let bytes = read_bounded(facts_path, MAX_INPUT)?;
    let facts: Value = serde_json::from_slice(&bytes)?;
    let meta = &facts["metadata"];
    if meta["schema"] != 2
        || meta["version"] != "26.3"
        || meta["protocol"] != 777
        || meta["jar_sha1"] != PIN
    {
        return Err(fail("state-facts pin mismatch"));
    }
    let states = facts["states"]
        .as_array()
        .ok_or_else(|| fail("missing states"))?;
    if states.is_empty()
        || states.len() > 65_536
        || meta["state_count"].as_u64() != Some(states.len() as u64)
    {
        return Err(fail("state count mismatch/limit"));
    }
    let mut definitions = BTreeMap::new();
    let mut models = Vec::new();
    let mut model_ids = BTreeMap::new();
    let mut rows = Vec::new();
    let mut tuples = BTreeSet::new();
    let (mut model_states, mut empty_states, mut parts_total, mut options_total) =
        (0, 0, 0usize, 0usize);
    for (id, state) in states.iter().enumerate() {
        if state["id"].as_u64() != Some(id as u64) {
            return Err(fail("noncontiguous state IDs"));
        }
        let block = state["block"]
            .as_str()
            .ok_or_else(|| fail("missing block identifier"))?;
        let path = resource_path(block, "blockstates")?;
        let mut properties = BTreeMap::new();
        for (key, value) in state["properties"]
            .as_object()
            .ok_or_else(|| fail("missing property map"))?
        {
            let value = value
                .as_str()
                .ok_or_else(|| fail("property values must be strings"))?;
            if key.is_empty() || key.len() > 64 || value.is_empty() || value.len() > 64 {
                return Err(fail("property name/value limit"));
            }
            properties.insert(key.clone(), value.to_owned());
        }
        if !tuples.insert((block.to_owned(), properties.clone())) {
            return Err(fail("duplicate block/property tuple"));
        }
        let mut parts_out = Vec::new();
        match state["render_shape"].as_str() {
            Some("MODEL") => {
                model_states += 1;
                if !definitions.contains_key(&path) {
                    definitions.insert(path.clone(), assets.json(&path)?);
                }
                let parts = select(&definitions[&path], &properties)
                    .map_err(|e| fail(&format!("state {id}: {e}")))?;
                if parts.is_empty() {
                    empty_states += 1;
                }
                parts_total = parts_total
                    .checked_add(parts.len())
                    .ok_or_else(|| fail("part overflow"))?;
                if parts_total > MAX_PARTS {
                    return Err(fail("aggregate part limit"));
                }
                for part in parts {
                    options_total = options_total
                        .checked_add(part.options.len())
                        .ok_or_else(|| fail("option overflow"))?;
                    if options_total > MAX_OPTIONS {
                        return Err(fail("aggregate option limit"));
                    }
                    let mut options = Vec::new();
                    for option in part.options {
                        let model_id = if let Some(&index) = model_ids.get(&option.model) {
                            index
                        } else {
                            let model_path = resource_path(&option.model, "models")?;
                            assets.json(&model_path)?;
                            let index = models.len();
                            models.push(option.model.clone());
                            model_ids.insert(option.model.clone(), index);
                            index
                        };
                        options.push(json!({"model":model_id,"x":option.x,"y":option.y,"uvlock":option.uvlock,"weight":option.weight}));
                    }
                    parts_out.push(json!({"weighted":part.weighted,"options":options}));
                }
            }
            Some("INVISIBLE" | "ENTITYBLOCK_ANIMATED") => {}
            _ => return Err(fail("invalid render shape")),
        }
        rows.push(json!({"id":id,"parts":parts_out}));
    }
    Ok(json!({
        "schema":1,"minecraft":"26.3","protocol":777,"source_sha1":PIN,
        "facts_sha256":hash(&bytes),"asset_manifest_sha256":assets.manifest_hash,
        "scope":"Model selection only. No RNG choice, inheritance/texture baking, geometry, lighting or rendering.",
        "counts":{"states":states.len(),"model_states":model_states,"empty_model_selections":empty_states,
                  "parts":parts_total,"options":options_total,"unique_models":models.len(),"verified_asset_files":assets.files.len()},
        "models":models,"states":rows
    }))
}

fn publish(output: &Path, value: &Value) -> Result<usize> {
    if output.try_exists()? || fs::symlink_metadata(output).is_ok() {
        return Err(fail("output exists; refusing overwrite"));
    }
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_INPUT {
        return Err(fail("output exceeds 12 MiB"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(
        ".model-selection-{}-{stamp}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        // Atomic no-replace publication. Never rename over an existing file.
        fs::hard_link(&temp, output)?;
        Ok(())
    })();
    drop(file);
    fs::remove_file(&temp)?;
    result?;
    Ok(bytes.len())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 6 {
        return Err(fail(
            "usage: craftrustnx-model-prep --facts FILE --assets DIR --output FILE",
        ));
    }
    let mut paths = BTreeMap::new();
    let (pairs, _) = args.as_chunks::<2>();
    for pair in pairs {
        let name = pair[0].to_str().ok_or_else(|| fail("invalid argument"))?;
        if !["--facts", "--assets", "--output"].contains(&name)
            || paths.insert(name, PathBuf::from(&pair[1])).is_some()
        {
            return Err(fail("unknown or duplicate argument"));
        }
    }
    let assets = Assets::load(&paths["--assets"])?;
    let plan = prepare(&paths["--facts"], &assets)?;
    let bytes = publish(&paths["--output"], &plan)?;
    println!(
        "{}",
        json!({"counts":plan["counts"],"output_bytes":bytes,"sha256":hash(&read_bounded(&paths["--output"],MAX_INPUT)?)})
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Model preparation refused: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir()
                .join(format!("model-prep-test-{}-{stamp}", std::process::id()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn manifest_validation_detects_corruption_before_preparation() {
        let temp = Temp::new();
        let path = "assets/minecraft/models/block/synthetic.json";
        let file = temp.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"{}").unwrap();
        // Independent known SHA256 of the two-byte JSON object.
        let manifest = json!({"schema":1,"minecraft":"26.3","protocol":777,"source_sha1":PIN,
            "count":1,"total_bytes":2,"files":[{"path":path,"size":2,
            "sha256":"44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"}]});
        fs::write(
            temp.0.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(Assets::load(&temp.0).unwrap().files.len(), 1);
        fs::write(&file, b"[]").unwrap();
        assert!(Assets::load(&temp.0).is_err());
    }

    #[test]
    fn plan_preserves_parts_weights_and_empty_model_selections() {
        let temp = Temp::new();
        let facts_path = temp.0.join("facts.json");
        let facts = json!({"metadata":{"schema":2,"version":"26.3","protocol":777,"jar_sha1":PIN,"state_count":2},
            "states":[{"id":0,"block":"minecraft:synthetic","properties":{"arm":"none"},"render_shape":"MODEL"},
                      {"id":1,"block":"minecraft:synthetic","properties":{"arm":"side"},"render_shape":"MODEL"}]});
        fs::write(&facts_path, serde_json::to_vec(&facts).unwrap()).unwrap();
        let definition=br#"{"multipart":[{"when":{"arm":"side|up"},"apply":[{"model":"block/a","weight":3},{"model":"block/b","weight":1,"y":90}]}]}"#;
        let assets = Assets {
            files: BTreeMap::from([
                (
                    "assets/minecraft/blockstates/synthetic.json".to_owned(),
                    definition.to_vec(),
                ),
                (
                    "assets/minecraft/models/block/a.json".to_owned(),
                    b"{}".to_vec(),
                ),
                (
                    "assets/minecraft/models/block/b.json".to_owned(),
                    b"{}".to_vec(),
                ),
            ]),
            manifest_hash: "synthetic".to_owned(),
        };
        let plan = prepare(&facts_path, &assets).unwrap();
        assert_eq!(plan["states"][0]["parts"], json!([]));
        assert_eq!(
            plan["states"][1]["parts"],
            json!([{"weighted":true,"options":[
            {"model":0,"weight":3,"x":0,"y":0,"uvlock":false},
            {"model":1,"weight":1,"x":0,"y":90,"uvlock":false}]}])
        );
        assert_eq!(
            plan["models"],
            json!(["minecraft:block/a", "minecraft:block/b"])
        );
        assert_eq!(plan["counts"]["empty_model_selections"], 1);
    }

    #[test]
    fn publication_never_replaces_an_existing_output() {
        let temp = Temp::new();
        let output = temp.0.join("plan.json");
        publish(&output, &json!({"first":true})).unwrap();
        let original = fs::read(&output).unwrap();
        assert!(publish(&output, &json!({"second":true})).is_err());
        assert_eq!(fs::read(&output).unwrap(), original);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }
}
