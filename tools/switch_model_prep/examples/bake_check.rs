//! Host-only check that every unique block-model request in a verified
//! selection plan bakes with the shared Horizon resolver.
//!
//! It reads the local plan and the pinned asset manifest, verifies every listed
//! asset's size and SHA256, deduplicates all `model`/`x`/`y`/`uvlock` options
//! (weighted alternatives included) and calls
//! [`craftrustnx_model_prep::bake_requested`]. Only aggregate counts reach
//! stdout: no model names, facts or quads are generated or printed, and no file
//! is written. A refused batch prints the model-specific error and exits
//! non-zero instead of hiding the defect.
//!
//! Usage, from the repository root:
//!
//! ```sh
//! cargo run --offline --locked --jobs 1 \
//!   --manifest-path tools/switch_model_prep/Cargo.toml \
//!   --target-dir tools/switch_model_prep/build/host-target --example bake_check
//! ```
//!
//! Optional arguments: the selection plan and the asset root, in that order.

use craftrustnx_model_prep::{bake_requested, BakeRequest};
use horizon_assets::bake::RenderType;
use horizon_assets::model::Direction;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const PIN: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
const DEFAULT_SELECTION: &str = ".local/switch-models/26.3/model-selection.json";
const DEFAULT_ASSETS: &str = ".local/switch-assets/26.3";
const MAX_SELECTION: usize = 12 * 1024 * 1024;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_ASSET_TOTAL: usize = 8 * 1024 * 1024;

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

/// The manifest-verified asset bytes, keyed by relative path.
struct Assets {
    files: BTreeMap<String, Vec<u8>>,
    manifest_hash: String,
    total_bytes: usize,
}

impl Assets {
    fn load(root: &Path) -> Result<Self> {
        let raw = read_bounded(&root.join("manifest.json"), MAX_MANIFEST)?;
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
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || Path::new(path)
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
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
            if total + raw.len() > MAX_ASSET_TOTAL
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
            total_bytes: total,
        })
    }
}

/// Every option of every state, deduplicated by `model`/`x`/`y`/`uvlock`.
fn collect_requests(plan: &Value) -> Result<(Vec<BakeRequest>, usize, usize)> {
    let models = plan["models"]
        .as_array()
        .ok_or_else(|| fail("missing models"))?;
    let states = plan["states"]
        .as_array()
        .ok_or_else(|| fail("missing states"))?;
    let mut requests = BTreeSet::new();
    let mut options = 0usize;
    for state in states {
        for part in state["parts"]
            .as_array()
            .ok_or_else(|| fail("missing parts"))?
        {
            if !part["weighted"].is_boolean() {
                return Err(fail("missing part weighted flag"));
            }
            for option in part["options"]
                .as_array()
                .ok_or_else(|| fail("missing options"))?
            {
                options += 1;
                let index = usize::try_from(
                    option["model"]
                        .as_u64()
                        .ok_or_else(|| fail("invalid model index"))?,
                )
                .map_err(|_| fail("model index out of range"))?;
                let model = models
                    .get(index)
                    .and_then(Value::as_str)
                    .ok_or_else(|| fail("model index outside the model table"))?;
                let x = u16::try_from(
                    option["x"]
                        .as_u64()
                        .ok_or_else(|| fail("invalid rotation"))?,
                )
                .map_err(|_| fail("rotation out of range"))?;
                let y = u16::try_from(
                    option["y"]
                        .as_u64()
                        .ok_or_else(|| fail("invalid rotation"))?,
                )
                .map_err(|_| fail("rotation out of range"))?;
                let uvlock = option["uvlock"]
                    .as_bool()
                    .ok_or_else(|| fail("invalid uvlock"))?;
                requests.insert(BakeRequest {
                    model: model.to_owned(),
                    x,
                    y,
                    uvlock,
                });
            }
        }
    }
    Ok((requests.into_iter().collect(), options, states.len()))
}

fn render_name(render_type: Option<RenderType>) -> &'static str {
    match render_type {
        None => "default",
        Some(RenderType::Unsupported) => "unsupported",
        Some(RenderType::Solid) => "solid",
        Some(RenderType::Cutout) => "cutout",
        Some(RenderType::CutoutMipped) => "cutout_mipped",
        Some(RenderType::Translucent) => "translucent",
        Some(RenderType::Tripwire) => "tripwire",
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let selection = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SELECTION));
    let assets_root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSETS));
    if args.next().is_some() {
        return Err(fail(
            "usage: bake_check [model-selection.json] [asset-root]",
        ));
    }
    let assets = Assets::load(&assets_root)?;
    let plan_bytes = read_bounded(&selection, MAX_SELECTION)?;
    let plan: Value = serde_json::from_slice(&plan_bytes)?;
    if plan["schema"] != 1
        || plan["minecraft"] != "26.3"
        || plan["protocol"] != 777
        || plan["source_sha1"] != PIN
    {
        return Err(fail("selection plan pin mismatch"));
    }
    if plan["asset_manifest_sha256"].as_str() != Some(assets.manifest_hash.as_str()) {
        return Err(fail(
            "selection plan does not belong to this asset manifest",
        ));
    }
    let (requests, options, states) = collect_requests(&plan)?;
    let summary = json!({
        "verified_asset_files": assets.files.len(),
        "verified_asset_bytes": assets.total_bytes,
        "states": states,
        "options": options,
        "unique_requests": requests.len(),
    });
    let baked = match bake_requested(&assets.files, &requests) {
        Ok(baked) => baked,
        Err(error) => {
            println!(
                "{}",
                json!({"ok": false, "error": error.to_string(), "bake": summary})
            );
            return Err(format!("bake refused: {error}").into());
        }
    };
    if baked.len() != requests.len() {
        return Err(fail("baker returned a different number of models"));
    }
    let mut quad_total = 0usize;
    let mut empty = 0usize;
    let mut tinted = 0usize;
    let mut translucent = 0usize;
    let mut culled = 0usize;
    let mut shaded = 0usize;
    let mut facings = [0usize; 6];
    let mut materials = BTreeSet::new();
    let mut render_types: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (model, request) in baked.iter().zip(&requests) {
        if &model.request != request {
            return Err(fail("baker returned a model for a different request"));
        }
        if model.quads.is_empty() {
            empty += 1;
        }
        for quad in &model.quads {
            quad_total += 1;
            if quad.tint_index >= 0 {
                tinted += 1;
            }
            if quad.material.force_translucent {
                translucent += 1;
            }
            if quad.cull.is_some() {
                culled += 1;
            }
            if quad.shade {
                shaded += 1;
            }
            facings[quad.facing as usize] += 1;
            materials.insert(quad.material.sprite.as_str());
            *render_types
                .entry(render_name(quad.render_type))
                .or_insert(0) += 1;
        }
    }
    println!(
        "{}",
        json!({
            "ok": true,
            "bake": summary,
            "baked_models": baked.len(),
            "empty_models": empty,
            "quads": quad_total,
            "distinct_materials": materials.len(),
            "tinted_quads": tinted,
            "translucent_quads": translucent,
            "culled_quads": culled,
            "shaded_quads": shaded,
            "facings": {
                "down": facings[Direction::Down as usize],
                "up": facings[Direction::Up as usize],
                "north": facings[Direction::North as usize],
                "south": facings[Direction::South as usize],
                "west": facings[Direction::West as usize],
                "east": facings[Direction::East as usize],
            },
            "render_types": render_types,
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Bake check refused: {error}");
        std::process::exit(1);
    }
}
