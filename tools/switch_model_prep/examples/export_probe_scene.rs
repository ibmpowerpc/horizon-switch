//! Export a small **untextured, static diagnostic scene** of real baked
//! Minecraft block models into the Switch terrain probe's `CTMESH01` mesh file.
//!
//! This is a depth/vertex/camera hardware probe fixture, not a renderer and not
//! a parity claim. It selects nine example block states **by block name and
//! properties** from the pinned state facts, looks their rows up in the verified
//! selection plan, keeps **all** matching parts, and explicitly takes the first
//! weighted model option of each part (no RNG draw, no vanilla random parity).
//! Those requests are deduplicated and baked once with the shared
//! [`craftrustnx_model_prep::bake_requested`] API, so every vertex comes from an
//! upstream `ModelQuad`. There is no invented cube and no solid fallback: a
//! sample whose state has no baked quads fails the whole export.
//!
//! Colors are flat opaque diagnostic colors per sample, dimmed per face
//! direction. They are **not** textures, tint, biome color, block light, sky
//! light, transparency or fluid rendering. Positions are upstream quad
//! positions translated by a 3x3 grid (spacing 2.0, centered on the origin);
//! each quad is split into triangles `[0,1,2,0,2,3]`.
//!
//! # Binary format (exactly what `terrain_app.h` decodes)
//!
//! ```text
//! offset 0   ASCII "CTMESH01"            (8 bytes)
//! offset 8   LE u32 protocol 777
//! offset 12  LE u32 format 1
//! offset 16  LE u32 vertex count (multiple of 3, <= TP_MAX_VERTICES)
//! offset 20  LE u32 reserved 0
//! offset 24  six LE f32: min XYZ, max XYZ over all vertices
//! offset 48  count * 16-byte vertices: 3 LE f32 position + LE u32 RGBA
//! ```
//!
//! `RGBA` is the memory byte order R,G,B,A (low byte R). Total length is
//! exactly `48 + 16 * count`. The mesh file and its private JSON report are
//! published atomically (sibling temp file + hard link) and never replace an
//! existing file.
//!
//! # Usage
//!
//! ```sh
//! cargo run --offline --locked --jobs 1 \
//!   --manifest-path tools/switch_model_prep/Cargo.toml \
//!   --target-dir tools/switch_model_prep/build/host-target \
//!   --example export_probe_scene -- [--facts FILE] [--selection FILE] \
//!   [--assets DIR] [--output FILE]
//! ```
//!
//! All paths are optional; the defaults are the ignored `.local/` inputs and
//! `.local/switch-models/26.3/terrain-probe-mesh.bin`. A single positional
//! argument is accepted as the output path. The report is written beside the
//! mesh with a `.json` extension. Nothing reads game jars, starts a JVM, uses
//! the network or a device, or runs git; only bounded regular local files are
//! read.

use craftrustnx_model_prep::{bake_requested, BakeRequest};
use horizon_assets::bake::ModelQuad;
use horizon_assets::model::Direction;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const PIN: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
const MINECRAFT: &str = "26.3";
const PROTOCOL: u32 = 777;
const MESH_MAGIC: &[u8; 8] = b"CTMESH01";
const MESH_FORMAT: u32 = 1;
const HEADER_BYTES: usize = 48;
const VERTEX_BYTES: usize = 16;
/// `TP_MAX_VERTICES` from `platform/switch/terrain-probe/include/terrain_gpu.h`.
const TP_MAX_VERTICES: usize = 262_140;
/// Static diagnostic safety bound on every emitted coordinate.
const COORD_LIMIT: f32 = 1024.0;
const GRID_SPACING: f32 = 2.0;
const GRID_SIDE: usize = 3;

const DEFAULT_FACTS: &str = ".local/switch-state-facts/26.3/state_facts.json";
const DEFAULT_SELECTION: &str = ".local/switch-models/26.3/model-selection.json";
const DEFAULT_ASSETS: &str = ".local/switch-assets/26.3";
const DEFAULT_OUTPUT: &str = ".local/switch-models/26.3/terrain-probe-mesh.bin";

const MAX_FACTS: usize = 12 * 1024 * 1024;
const MAX_PLAN: usize = 12 * 1024 * 1024;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_ASSET_TOTAL: usize = 8 * 1024 * 1024;
const MAX_STATES: usize = 65_536;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fail(message: impl Into<String>) -> Box<dyn std::error::Error> {
    message.into().into()
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

/// One requested diagnostic sample. `properties` is the complete state tuple:
/// missing or extra properties are not accepted as a match.
struct SampleSpec {
    block: &'static str,
    properties: &'static [(&'static str, &'static str)],
    /// Flat opaque diagnostic color, not a texture or tint.
    color: [u8; 3],
}

const SAMPLES: [SampleSpec; 9] = [
    SampleSpec {
        block: "minecraft:stone",
        properties: &[],
        color: [0xb0, 0xb0, 0xb0],
    },
    SampleSpec {
        block: "minecraft:oak_stairs",
        properties: &[
            ("facing", "north"),
            ("half", "bottom"),
            ("shape", "straight"),
            ("waterlogged", "false"),
        ],
        color: [0xb8, 0x8a, 0x4e],
    },
    SampleSpec {
        block: "minecraft:cobblestone_wall",
        properties: &[
            ("up", "true"),
            ("north", "low"),
            ("east", "none"),
            ("south", "none"),
            ("west", "none"),
            ("waterlogged", "false"),
        ],
        color: [0x80, 0x80, 0x80],
    },
    SampleSpec {
        block: "minecraft:oak_fence",
        properties: &[
            ("north", "true"),
            ("east", "true"),
            ("south", "false"),
            ("west", "false"),
            ("waterlogged", "false"),
        ],
        color: [0xb0, 0x84, 0x50],
    },
    SampleSpec {
        block: "minecraft:oak_slab",
        properties: &[("type", "bottom"), ("waterlogged", "false")],
        color: [0xc0, 0x98, 0x60],
    },
    SampleSpec {
        block: "minecraft:grass_block",
        properties: &[("snowy", "false")],
        color: [0x5f, 0xa0, 0x4a],
    },
    SampleSpec {
        block: "minecraft:dandelion",
        properties: &[],
        color: [0xe8, 0xd0, 0x30],
    },
    SampleSpec {
        block: "minecraft:glass",
        properties: &[],
        color: [0x7f, 0xc8, 0xe0],
    },
    SampleSpec {
        block: "minecraft:torch",
        properties: &[],
        color: [0xff, 0xb0, 0x40],
    },
];

/// Diagnostic per-face intensity, in `Direction` declaration order
/// (down, up, north, south, west, east). Untextured shading only.
const FACE_SHADE: [u8; 6] = [130, 255, 205, 185, 165, 230];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vertex {
    position: [f32; 3],
    rgba: u32,
}

fn shade(base: [u8; 3], facing: Direction) -> u32 {
    let factor = u32::from(FACE_SHADE[facing as usize]);
    let channel = |value: u8| u32::from(value) * factor / 255;
    channel(base[0]) | (channel(base[1]) << 8) | (channel(base[2]) << 16) | (255 << 24)
}

/// Grid cell of one sample: 3x3, spacing 2.0, centered on the origin on the
/// X/Z plane.
fn grid_offset(index: usize) -> Result<[f32; 3]> {
    if index >= SAMPLES.len() {
        return Err(fail("sample index outside the 3x3 grid"));
    }
    let column = (index % GRID_SIDE) as f32;
    let row = (index / GRID_SIDE) as f32;
    Ok([
        (column - 1.0) * GRID_SPACING,
        0.0,
        (row - 1.0) * GRID_SPACING,
    ])
}

/// Triangulate one upstream quad: `[0,1,2, 0,2,3]`.
const TRIANGLES: [usize; 6] = [0, 1, 2, 0, 2, 3];

/// Append every quad's triangles at the sample's grid translation. A sample
/// with no quads is an error, never replaced by a synthesized cube.
fn append_geometry(
    vertices: &mut Vec<Vertex>,
    quads: &[&ModelQuad],
    translation: [f32; 3],
    color: [u8; 3],
    label: &str,
) -> Result<usize> {
    if quads.is_empty() {
        return Err(fail(format!(
            "sample `{label}` baked no geometry; refusing an empty sample"
        )));
    }
    for quad in quads {
        let rgba = shade(color, quad.facing);
        for corner in TRIANGLES {
            let source = quad.positions[corner];
            vertices.push(Vertex {
                position: [
                    source[0] + translation[0],
                    source[1] + translation[1],
                    source[2] + translation[2],
                ],
                rgba,
            });
        }
    }
    Ok(quads.len())
}

/// Encode the `CTMESH01` mesh. The header is exactly 48 bytes and every
/// vertex is 16 bytes; see the module documentation.
fn encode_mesh(vertices: &[Vertex]) -> Result<Vec<u8>> {
    if vertices.is_empty() {
        return Err(fail("mesh has no vertices"));
    }
    if !vertices.len().is_multiple_of(3) {
        return Err(fail("mesh vertex count is not a triangle list"));
    }
    if vertices.len() > TP_MAX_VERTICES {
        return Err(fail(format!(
            "{} vertices exceed TP_MAX_VERTICES {TP_MAX_VERTICES}",
            vertices.len()
        )));
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in vertices {
        for (axis, value) in vertex.position.iter().enumerate() {
            if !value.is_finite() || value.abs() > COORD_LIMIT {
                return Err(fail("vertex coordinate is non-finite or out of bounds"));
            }
            min[axis] = min[axis].min(*value);
            max[axis] = max[axis].max(*value);
        }
    }
    let count = u32::try_from(vertices.len()).map_err(|_| fail("vertex count overflow"))?;
    let mut bytes = Vec::with_capacity(HEADER_BYTES + VERTEX_BYTES * vertices.len());
    bytes.extend_from_slice(MESH_MAGIC);
    bytes.extend_from_slice(&PROTOCOL.to_le_bytes());
    bytes.extend_from_slice(&MESH_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for value in min.iter().chain(max.iter()) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for vertex in vertices {
        for value in &vertex.position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&vertex.rgba.to_le_bytes());
    }
    debug_assert_eq!(bytes.len(), HEADER_BYTES + VERTEX_BYTES * vertices.len());
    Ok(bytes)
}

/// Publish `bytes` at `path` without ever replacing an existing file: a unique
/// sibling temp file is written and hard-linked into place, then removed.
fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(fail(format!(
            "output `{}` already exists; refusing to replace it",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(".terrain-probe-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Atomic no-replace publication.
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    result
}

fn canonical_model(model: &str) -> String {
    if model.contains(':') {
        model.to_owned()
    } else {
        format!("minecraft:{model}")
    }
}

fn signature(request: &BakeRequest) -> (String, u16, u16, bool) {
    (
        canonical_model(&request.model),
        request.x,
        request.y,
        request.uvlock,
    )
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
            || manifest["minecraft"] != MINECRAFT
            || manifest["protocol"] != PROTOCOL
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

fn properties_of(state: &Value) -> Result<BTreeMap<String, String>> {
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
    Ok(properties)
}

/// Find the single state whose block id and complete property tuple equal the
/// requested sample. Zero or several matches fail; nothing is substituted.
fn fact_state_id(facts: &Value, spec: &SampleSpec) -> Result<u64> {
    let states = facts["states"]
        .as_array()
        .ok_or_else(|| fail("facts are missing states"))?;
    let expected: BTreeMap<String, String> = spec
        .properties
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let mut matches = Vec::new();
    for state in states {
        if state["block"].as_str() == Some(spec.block)
            && properties_of(state).ok().as_ref() == Some(&expected)
        {
            matches.push(
                state["id"]
                    .as_u64()
                    .ok_or_else(|| fail("state is missing an id"))?,
            );
        }
    }
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(fail(format!(
            "no unique state for block `{}` with properties {:?}",
            spec.block, expected
        ))),
        _ => Err(fail(format!(
            "block `{}` with properties {:?} matched {} states",
            spec.block,
            expected,
            matches.len()
        ))),
    }
}

struct SampleChoice {
    requests: Vec<BakeRequest>,
    parts: usize,
    options_total: usize,
}

/// Every matching part of one plan row, each with its first weighted option
/// chosen explicitly for this static diagnostic. No weighted draw is made.
fn sample_choices(models: &[Value], row: &Value, label: &str) -> Result<SampleChoice> {
    let parts = row["parts"]
        .as_array()
        .ok_or_else(|| fail(format!("sample `{label}` is missing parts")))?;
    if parts.is_empty() {
        return Err(fail(format!(
            "sample `{label}` selected no parts; refusing empty state geometry"
        )));
    }
    let mut requests = Vec::new();
    let mut options_total = 0usize;
    for part in parts {
        let options = part["options"]
            .as_array()
            .ok_or_else(|| fail(format!("sample `{label}` has a part without options")))?;
        if options.is_empty() {
            return Err(fail(format!(
                "sample `{label}` has a part without a model option"
            )));
        }
        options_total += options.len();
        let option = &options[0];
        if option["weight"].as_u64().unwrap_or(0) == 0 {
            return Err(fail(format!("sample `{label}` has an invalid weight")));
        }
        let index = usize::try_from(
            option["model"]
                .as_u64()
                .ok_or_else(|| fail("invalid model index"))?,
        )
        .map_err(|_| fail("model index out of range"))?;
        let model = models
            .get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| fail(format!("sample `{label}` model index outside the table")))?;
        let x = u16::try_from(option["x"].as_u64().unwrap_or(u64::MAX))
            .map_err(|_| fail("invalid x rotation"))?;
        let y = u16::try_from(option["y"].as_u64().unwrap_or(u64::MAX))
            .map_err(|_| fail("invalid y rotation"))?;
        let uvlock = option["uvlock"]
            .as_bool()
            .ok_or_else(|| fail("invalid uvlock"))?;
        requests.push(BakeRequest {
            model: canonical_model(model),
            x,
            y,
            uvlock,
        });
    }
    Ok(SampleChoice {
        requests,
        parts: parts.len(),
        options_total,
    })
}

fn report_path(output: &Path) -> Result<PathBuf> {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("output path has no file name"))?;
    if name.is_empty() {
        return Err(fail("output path has no file name"));
    }
    let report = name
        .strip_suffix(".bin")
        .map(|stem| format!("{stem}.json"))
        .unwrap_or_else(|| format!("{name}.json"));
    Ok(output.with_file_name(report))
}

fn parse_args() -> Result<BTreeMap<String, PathBuf>> {
    let mut paths = BTreeMap::new();
    let mut args = std::env::args_os().skip(1);
    let mut positional = None;
    while let Some(arg) = args.next() {
        let name = arg
            .to_str()
            .ok_or_else(|| fail("argument is not valid UTF-8"))?;
        if name.starts_with("--") {
            let value = args
                .next()
                .ok_or_else(|| fail(format!("missing value for `{name}`")))?;
            if !["--facts", "--selection", "--assets", "--output"].contains(&name)
                || paths
                    .insert(name.to_owned(), PathBuf::from(value))
                    .is_some()
            {
                return Err(fail(format!("unknown or duplicate argument `{name}`")));
            }
        } else if positional.replace(PathBuf::from(arg)).is_some() {
            return Err(fail("at most one positional output path is accepted"));
        }
    }
    if let Some(output) = positional {
        if paths.insert("--output".to_owned(), output).is_some() {
            return Err(fail(
                "output given both as a positional argument and --output",
            ));
        }
    }
    let defaults = [
        ("--facts", DEFAULT_FACTS),
        ("--selection", DEFAULT_SELECTION),
        ("--assets", DEFAULT_ASSETS),
        ("--output", DEFAULT_OUTPUT),
    ];
    for (name, value) in defaults {
        paths
            .entry(name.to_owned())
            .or_insert_with(|| PathBuf::from(value));
    }
    Ok(paths)
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let output = args["--output"].clone();
    let report = report_path(&output)?;

    // Refuse both outputs before doing any work: neither file is replaced.
    for path in [&output, &report] {
        if fs::symlink_metadata(path).is_ok() {
            return Err(fail(format!(
                "output `{}` already exists; refusing to replace it",
                path.display()
            )));
        }
    }

    let assets = Assets::load(&args["--assets"])?;
    let plan_bytes = read_bounded(&args["--selection"], MAX_PLAN)?;
    let plan: Value = serde_json::from_slice(&plan_bytes)?;
    if plan["schema"] != 1
        || plan["minecraft"] != MINECRAFT
        || plan["protocol"] != PROTOCOL
        || plan["source_sha1"] != PIN
    {
        return Err(fail("selection plan pin mismatch"));
    }
    if plan["asset_manifest_sha256"].as_str() != Some(assets.manifest_hash.as_str()) {
        return Err(fail(
            "selection plan does not belong to this asset manifest",
        ));
    }

    let facts_bytes = read_bounded(&args["--facts"], MAX_FACTS)?;
    let facts: Value = serde_json::from_slice(&facts_bytes)?;
    let meta = &facts["metadata"];
    if meta["schema"] != 2
        || meta["version"] != MINECRAFT
        || meta["protocol"] != PROTOCOL
        || meta["jar_sha1"] != PIN
    {
        return Err(fail("state-facts pin mismatch"));
    }
    let facts_hash = hash(&facts_bytes);
    if plan["facts_sha256"].as_str() != Some(facts_hash.as_str()) {
        return Err(fail("selection plan does not belong to these state facts"));
    }
    let facts_states = facts["states"]
        .as_array()
        .ok_or_else(|| fail("facts are missing states"))?;
    if facts_states.is_empty()
        || facts_states.len() > MAX_STATES
        || meta["state_count"].as_u64() != Some(facts_states.len() as u64)
    {
        return Err(fail("state count mismatch/limit"));
    }
    for (index, state) in facts_states.iter().enumerate() {
        if state["id"].as_u64() != Some(index as u64) {
            return Err(fail("facts have noncontiguous state IDs"));
        }
        properties_of(state)?;
    }

    let models = plan["models"]
        .as_array()
        .ok_or_else(|| fail("selection plan is missing models"))?
        .clone();
    let rows: BTreeMap<u64, &Value> = plan["states"]
        .as_array()
        .ok_or_else(|| fail("selection plan is missing states"))?
        .iter()
        .map(|row| {
            row["id"]
                .as_u64()
                .map(|id| (id, row))
                .ok_or_else(|| fail("plan row is missing an id"))
        })
        .collect::<Result<_>>()?;
    if rows.len() != facts_states.len() || rows.keys().copied().ne(0..facts_states.len() as u64) {
        return Err(fail("selection plan rows do not match the facts state ids"));
    }

    let mut choices = Vec::with_capacity(SAMPLES.len());
    for spec in &SAMPLES {
        let id = fact_state_id(&facts, spec)?;
        let row = rows
            .get(&id)
            .ok_or_else(|| fail(format!("plan is missing state id {id}")))?;
        choices.push((id, sample_choices(&models, row, spec.block)?));
    }

    let mut unique: BTreeSet<BakeRequest> = BTreeSet::new();
    for (_, choice) in &choices {
        for request in &choice.requests {
            unique.insert(request.clone());
        }
    }
    let requests: Vec<BakeRequest> = unique.into_iter().collect();
    let baked = bake_requested(&assets.files, &requests).map_err(|error| {
        fail(format!(
            "bake refused for the diagnostic scene: {}",
            error.message()
        ))
    })?;
    if baked.len() != requests.len() {
        return Err(fail("baker returned a different number of models"));
    }
    let mut lookup = BTreeMap::new();
    for (index, model) in baked.iter().enumerate() {
        if lookup.insert(signature(&model.request), index).is_some() {
            return Err(fail("baker returned duplicate model signatures"));
        }
    }

    let mut vertices = Vec::new();
    let mut sample_report = Vec::with_capacity(SAMPLES.len());
    let mut quad_total = 0usize;
    for (index, (spec, (id, choice))) in SAMPLES.iter().zip(&choices).enumerate() {
        let translation = grid_offset(index)?;
        let mut quads = Vec::new();
        for request in &choice.requests {
            let model = baked
                .get(*lookup.get(&signature(request)).ok_or_else(|| {
                    fail(format!("sample `{}` has an unbaked request", spec.block))
                })?)
                .ok_or_else(|| fail("baked model index out of range"))?;
            quads.extend(model.quads.iter());
        }
        let count = append_geometry(&mut vertices, &quads, translation, spec.color, spec.block)?;
        quad_total += count;
        sample_report.push(json!({
            "block": spec.block,
            "state_id": id,
            "properties": spec.properties.iter().map(|(key, value)| {
                ((*key).to_owned(), (*value).to_owned())
            }).collect::<BTreeMap<String, String>>(),
            "parts": choice.parts,
            "options": choice.options_total,
            "chosen_options": choice.requests.len(),
            "quads": count,
            "vertices": count * TRIANGLES.len(),
            "signatures": choice.requests.iter().map(|request| json!({
                "model": signature(request).0,
                "x": request.x,
                "y": request.y,
                "uvlock": request.uvlock,
            })).collect::<Vec<_>>(),
        }));
    }
    let mesh = encode_mesh(&vertices)?;
    let mesh_hash = hash(&mesh);
    let min = vertices.iter().fold([f32::INFINITY; 3], |acc, vertex| {
        [0, 1, 2].map(|axis| acc[axis].min(vertex.position[axis]))
    });
    let max = vertices.iter().fold([f32::NEG_INFINITY; 3], |acc, vertex| {
        [0, 1, 2].map(|axis| acc[axis].max(vertex.position[axis]))
    });

    let report_value = json!({
        "schema": 1,
        "purpose": "Untextured static diagnostic scene for the Switch terrain probe's depth/vertex/camera path. Not a rendering, texture, lighting, transparency or gameplay parity result.",
        "minecraft": MINECRAFT,
        "protocol": PROTOCOL,
        "source_sha1": PIN,
        "facts_sha256": facts_hash,
        "plan_sha256": hash(&plan_bytes),
        "asset_manifest_sha256": assets.manifest_hash,
        "verified_asset_files": assets.files.len(),
        "verified_asset_bytes": assets.total_bytes,
        "selection": "States matched by block name and complete property tuple; all parts kept; first weighted option chosen explicitly for this diagnostic, no RNG draw and no parity claim.",
        "color_policy": "Flat opaque per-sample colors dimmed per face direction; no textures, tint, biome, block/sky light, transparency or fluids.",
        "grid": {"side": GRID_SIDE, "spacing": GRID_SPACING, "origin": [0.0, 0.0, 0.0]},
        "samples": sample_report,
        "counts": {
            "samples": SAMPLES.len(),
            "unique_requests": requests.len(),
            "baked_models": baked.len(),
            "quads": quad_total,
            "triangles": vertices.len() / 3,
            "vertices": vertices.len(),
        },
        "mesh": {
            "path": output.display().to_string(),
            "bytes": mesh.len(),
            "sha256": mesh_hash,
            "format": MESH_FORMAT,
            "vertex_count": vertices.len(),
            "bounds": {"min": min, "max": max},
        },
    });
    let report_bytes = serde_json::to_vec_pretty(&report_value)?;

    // Mesh first, then its report; if the report cannot be published, remove
    // the mesh this run created so no half-exported pair is left behind.
    publish(&output, &mesh)?;
    if let Err(error) = publish(&report, &report_bytes) {
        let _ = fs::remove_file(&output);
        return Err(error);
    }

    println!(
        "{}",
        json!({
            "ok": true,
            "mesh_bytes": mesh.len(),
            "vertex_count": vertices.len(),
            "triangles": vertices.len() / 3,
            "quads": quad_total,
            "samples": SAMPLES.len(),
            "unique_requests": requests.len(),
            "sha256": mesh_hash,
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Probe scene export refused: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static UNIQUE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let unique = UNIQUE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "probe-scene-test-{}-{stamp}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn read_f32(bytes: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn quad(positions: [[f32; 3]; 4], facing: Direction) -> ModelQuad {
        ModelQuad {
            positions,
            uv: [[0.0, 0.0]; 4],
            facing,
            cull: None,
            material: horizon_assets::model::Material {
                sprite: "synthetic".to_owned(),
                force_translucent: false,
            },
            tint_index: -1,
            shade_direction_override: None,
            light_emission: 0,
            light: [0, 0],
            color: None,
            ambient_occlusion: None,
            render_type: None,
            shade: true,
        }
    }

    #[test]
    fn mesh_layout_is_exact_and_triangulates_each_quad() {
        let quad = quad(
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            Direction::Up,
        );
        let mut vertices = Vec::new();
        let quads = [&quad];
        assert_eq!(
            append_geometry(
                &mut vertices,
                &quads,
                [2.0, 0.0, -2.0],
                [255, 0, 0],
                "synthetic"
            )
            .unwrap(),
            1
        );
        let mesh = encode_mesh(&vertices).unwrap();

        assert_eq!(mesh.len(), 48 + 16 * 6);
        assert_eq!(&mesh[0..8], b"CTMESH01");
        assert_eq!(read_u32(&mesh, 8), 777);
        assert_eq!(read_u32(&mesh, 12), 1);
        assert_eq!(read_u32(&mesh, 16), 6);
        assert_eq!(read_u32(&mesh, 20), 0);
        // Independent expected bounds: quad at x=2..3, y=0..1, z=-2.
        assert_eq!(
            [
                read_f32(&mesh, 24),
                read_f32(&mesh, 28),
                read_f32(&mesh, 32),
                read_f32(&mesh, 36),
                read_f32(&mesh, 40),
                read_f32(&mesh, 44),
            ],
            [2.0, 0.0, -2.0, 3.0, 1.0, -2.0]
        );
        // Triangle list is [0,1,2, 0,2,3]; every vertex is 16 bytes.
        let expected = [
            [2.0, 0.0, -2.0],
            [3.0, 0.0, -2.0],
            [3.0, 1.0, -2.0],
            [2.0, 0.0, -2.0],
            [3.0, 1.0, -2.0],
            [2.0, 1.0, -2.0],
        ];
        for (index, expected_position) in expected.iter().enumerate() {
            let base = 48 + 16 * index;
            assert_eq!(
                [
                    read_f32(&mesh, base),
                    read_f32(&mesh, base + 4),
                    read_f32(&mesh, base + 8),
                ],
                *expected_position
            );
            // Up face keeps full red and sets opaque alpha; low byte is R.
            assert_eq!(read_u32(&mesh, base + 12), 0xff00_00ff);
        }
        assert_eq!(vertices.len() % 3, 0);
    }

    #[test]
    fn encode_refuses_empty_ragged_nonfinite_and_oversized_meshes() {
        assert!(encode_mesh(&[]).is_err());
        let vertex = Vertex {
            position: [0.0, 0.0, 0.0],
            rgba: 0,
        };
        assert!(encode_mesh(&[vertex; 4]).is_err());
        let mut nonfinite = [vertex; 3];
        nonfinite[1].position[1] = f32::NAN;
        assert!(encode_mesh(&nonfinite).is_err());
        let mut too_far = [vertex; 3];
        too_far[2].position[0] = 2048.0;
        assert!(encode_mesh(&too_far).is_err());
        let oversized = vec![vertex; TP_MAX_VERTICES + 3];
        assert!(encode_mesh(&oversized).is_err());
    }

    #[test]
    fn an_empty_sample_is_refused_instead_of_a_solid_fallback() {
        let mut vertices = Vec::new();
        assert!(append_geometry(&mut vertices, &[], [0.0; 3], [1, 2, 3], "synthetic").is_err());
        assert!(vertices.is_empty());
    }

    #[test]
    fn state_selection_requires_one_exact_match() {
        let facts = json!({"states": [
            {"id": 0, "block": "minecraft:stone", "properties": {}},
            {"id": 1, "block": "minecraft:stone", "properties": {}},
            {"id": 2, "block": "minecraft:glass", "properties": {}},
        ]});
        assert!(fact_state_id(&facts, &SAMPLES[0]).is_err());
        assert_eq!(fact_state_id(&facts, &SAMPLES[7]).unwrap(), 2);
        let facts = json!({"states": [
            {"id": 0, "block": "minecraft:stone", "properties": {}},
            {"id": 1, "block": "minecraft:stone", "properties": {"snowy": "false"}},
        ]});
        assert_eq!(fact_state_id(&facts, &SAMPLES[0]).unwrap(), 0);
    }

    #[test]
    fn empty_parts_and_missing_model_options_are_refused() {
        let models = json!(["minecraft:block/synthetic"]);
        let models = models.as_array().unwrap();
        let empty_parts = json!({"id": 0, "parts": []});
        assert!(sample_choices(models, &empty_parts, "synthetic").is_err());
        let no_options = json!({"id": 0, "parts": [{"weighted": true, "options": []}]});
        assert!(sample_choices(models, &no_options, "synthetic").is_err());
        let missing_model = json!({"id": 0, "parts": [{"weighted": false,
            "options": [{"model": 3, "x": 0, "y": 0, "uvlock": false, "weight": 1}]}]});
        assert!(sample_choices(models, &missing_model, "synthetic").is_err());
        let first_option = json!({"id": 0, "parts": [{"weighted": true,
            "options": [
                {"model": 0, "x": 0, "y": 270, "uvlock": false, "weight": 2},
                {"model": 0, "x": 0, "y": 90, "uvlock": false, "weight": 1}]}]});
        let choice = sample_choices(models, &first_option, "synthetic").unwrap();
        assert_eq!(choice.requests.len(), 1);
        assert_eq!(choice.requests[0].y, 270);
        assert_eq!(choice.options_total, 2);
    }

    #[test]
    fn publication_never_replaces_an_existing_file() {
        let temp = Temp::new();
        let output = temp.0.join("scene.bin");
        publish(&output, b"first").unwrap();
        assert!(publish(&output, b"second").is_err());
        assert_eq!(fs::read(&output).unwrap(), b"first");
        // Only the output remains: no sibling temp is left behind.
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }

    #[test]
    fn asset_verification_refuses_corruption() {
        let temp = Temp::new();
        let path = "assets/minecraft/models/block/synthetic.json";
        let file = temp.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"{}").unwrap();
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
}
