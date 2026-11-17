//! Host-side baking of selected block models into quads.
//!
//! The caller supplies assets whose bytes it already verified against the
//! pinned manifest and the deduplicated `model`/`x`/`y`/`uvlock` requests it
//! selected from blockstate definitions. This module decodes only the models
//! the requests reach (their parents), then reuses `horizon-assets` to inherit
//! geometry and textures, apply the blockstate rotation and bake
//! [`ModelQuad`]s. It never chooses among weighted alternatives, samples
//! randomness, invents cubes or resolves a sprite atlas.
//!
//! Failure is transactional: any error returns no models at all, instead of a
//! partial list. Missing parents are refused rather than replaced by the
//! upstream missing-model cube, and custom model loaders are refused rather
//! than baked as if they had only the model's own elements.

use horizon_assets::{
    bake::{bake_block_model, BlockModelTransform, ModelQuad},
    model::{CuboidModel, Loader},
    resolve::{ModelLibrary, ResolvedCuboidModel, GENERATED_MODEL, MISSING_MODEL},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Maximum number of requests in one batch.
pub const MAX_REQUESTS: usize = 16_384;
/// Maximum number of distinct models a batch may decode, parents included.
pub const MAX_REACHABLE_MODELS: usize = 4_096;
/// Maximum number of parent links above a requested model.
pub const MAX_PARENT_DEPTH: usize = 64;
/// Maximum size of one model JSON file.
pub const MAX_MODEL_BYTES: usize = 2 * 1024 * 1024;
/// Maximum total size of the distinct model JSON sources a batch decodes.
pub const MAX_DECODED_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of quads emitted by a whole batch.
pub const MAX_QUADS: usize = 1_000_000;

const MAX_MODEL_ID_LEN: usize = 512;
const MAX_NAMESPACE_LEN: usize = 64;
const MAX_PATH_LEN: usize = 256;

/// One model plus its blockstate quarter turns and UV lock, as selected from a
/// variant or multipart option. Two requests with the same canonical model and
/// the same rotation are duplicates: the caller must deduplicate before baking.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BakeRequest {
    /// Canonical `namespace:path` model id after [`bake_requested`] returns it.
    pub model: String,
    /// Blockstate `x` rotation in degrees: 0, 90, 180 or 270.
    pub x: u16,
    /// Blockstate `y` rotation in degrees: 0, 90, 180 or 270.
    pub y: u16,
    /// Whether the blockstate option locks face UVs against the rotation.
    pub uvlock: bool,
}

/// One baked request, in request order. An empty `quads` list is a valid
/// selection with no geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedModel {
    pub request: BakeRequest,
    pub quads: Vec<ModelQuad>,
}

/// Why a batch was refused. The message names the model at fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeError(String);

impl BakeError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The human-readable reason, without a trailing period.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BakeError {}

/// Bake every request from already verified asset bytes.
///
/// `files` maps repository-relative asset paths (`assets/<namespace>/models/...`)
/// to their verified bytes. Only models reachable from the requests are
/// decoded; everything else in `files` is ignored.
///
/// Requests are checked before any model is read: more than
/// [`MAX_REQUESTS`], an invalid model id or rotation, or two requests with the
/// same canonical signature fail the whole batch. Returned requests carry the
/// canonical model id, so a caller may key results by [`BakeRequest`].
pub fn bake_requested(
    files: &BTreeMap<String, Vec<u8>>,
    requests: &[BakeRequest],
) -> Result<Vec<BakedModel>, BakeError> {
    if requests.len() > MAX_REQUESTS {
        return Err(BakeError::new(format!(
            "{} requests exceed the limit of {MAX_REQUESTS}",
            requests.len()
        )));
    }
    let mut checked = Vec::with_capacity(requests.len());
    let mut seen = BTreeSet::new();
    for request in requests {
        let model = canonical_model(&request.model)?;
        validate_rotation(&model, request.x, "x")?;
        validate_rotation(&model, request.y, "y")?;
        let signature = (model.clone(), request.x, request.y, request.uvlock);
        if !seen.insert(signature) {
            return Err(BakeError::new(format!(
                "duplicate request `{model}` x={} y={} uvlock={}",
                request.x, request.y, request.uvlock
            )));
        }
        checked.push((model, request.x, request.y, request.uvlock));
    }

    let mut decoder = Decoder::new(files);
    for (model, ..) in &checked {
        decoder.reachable(model, 0)?;
    }
    // A cached parent can bypass recursive loading. Still check the complete
    // depth for each request, independent of the order models were decoded.
    for (model, ..) in &checked {
        let mut name = model.as_str();
        let mut depth = 0;
        while let Some(parent) = &decoder.decoded[name].parent {
            depth += 1;
            if depth > MAX_PARENT_DEPTH {
                return Err(BakeError::new(
                    "complete cached parent chain exceeds depth limit",
                ));
            }
            name = parent;
        }
    }
    let library = ModelLibrary::new(decoder.decoded)
        .map_err(|error| BakeError::new(format!("model library: {error}")))?;

    let mut output = Vec::with_capacity(checked.len());
    let mut quad_total = 0usize;
    for (model, x, y, uvlock) in checked {
        if !library.resolves(&model) {
            return Err(BakeError::new(format!(
                "model `{model}` does not resolve to a complete, acyclic parent chain"
            )));
        }
        let resolved = library
            .resolve(&model)
            .map_err(|error| BakeError::new(format!("model `{model}`: {error}")))?;
        if resolved.loader().is_some() {
            return Err(BakeError::new(format!(
                "model `{model}` kept an unsupported custom loader after resolution"
            )));
        }
        validate_materials(&model, &resolved)?;
        let transform = BlockModelTransform::new([(x / 90) as u8, (y / 90) as u8, 0], uvlock);
        // Block models use the block-specific flat-element treatment; the
        // generic model baker would draw the faces vanilla skips.
        let quads = bake_block_model(&resolved, transform)
            .map_err(|error| BakeError::new(format!("model `{model}`: {error}")))?;
        for quad in &quads {
            validate_quad(&model, quad)?;
        }
        quad_total = quad_total
            .checked_add(quads.len())
            .ok_or_else(|| BakeError::new("quad count overflow"))?;
        if quad_total > MAX_QUADS {
            return Err(BakeError::new(format!(
                "batch emits more than {MAX_QUADS} quads"
            )));
        }
        output.push(BakedModel {
            request: BakeRequest {
                model,
                x,
                y,
                uvlock,
            },
            quads,
        });
    }
    Ok(output)
}

/// Decodes the requested models and their parents, once each.
struct Decoder<'a> {
    files: &'a BTreeMap<String, Vec<u8>>,
    decoded: BTreeMap<String, CuboidModel>,
    visiting: BTreeSet<String>,
    source_bytes: usize,
}

impl<'a> Decoder<'a> {
    fn new(files: &'a BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            files,
            decoded: BTreeMap::new(),
            visiting: BTreeSet::new(),
            source_bytes: 0,
        }
    }

    /// Load `id` and its whole parent chain. `depth` counts parent links above
    /// the requested model.
    fn reachable(&mut self, id: &str, depth: usize) -> Result<(), BakeError> {
        if self.decoded.contains_key(id) {
            return Ok(());
        }
        if !self.visiting.insert(id.to_owned()) {
            return Err(BakeError::new(format!(
                "model `{id}` takes part in a cyclic parent chain"
            )));
        }
        let result = self.load(id, depth);
        self.visiting.remove(id);
        result
    }

    fn load(&mut self, id: &str, depth: usize) -> Result<(), BakeError> {
        // `ModelLibrary` keeps its own missing/generated models under these
        // names and would replace a resource by that name without a warning.
        if id == MISSING_MODEL || id == GENERATED_MODEL {
            return Err(BakeError::new(format!(
                "model `{id}` is an upstream built-in, not a verified block model"
            )));
        }
        let path = model_path(id);
        let bytes = self.files.get(&path).ok_or_else(|| {
            BakeError::new(format!(
                "model `{id}` is not in the verified assets ({path} missing)"
            ))
        })?;
        if bytes.len() > MAX_MODEL_BYTES {
            return Err(BakeError::new(format!(
                "model `{id}` is {} bytes, over the {MAX_MODEL_BYTES}-byte limit",
                bytes.len()
            )));
        }
        self.source_bytes = self
            .source_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| BakeError::new("decoded source size overflow"))?;
        if self.source_bytes > MAX_DECODED_SOURCE_BYTES {
            return Err(BakeError::new(format!(
                "decoded model sources exceed {MAX_DECODED_SOURCE_BYTES} bytes"
            )));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|error| BakeError::new(format!("model `{id}` is not UTF-8: {error}")))?;
        let model = CuboidModel::from_json(text)
            .map_err(|error| BakeError::new(format!("model `{id}`: {error}")))?;
        if let Some(loader) = &model.loader {
            return Err(BakeError::new(format!(
                "model `{id}` uses unsupported custom loader `{}`",
                loader_name(loader)
            )));
        }
        if let Some(parent) = &model.parent {
            let parent = canonical_model(parent).map_err(|error| {
                BakeError::new(format!("model `{id}` names an invalid parent: {error}"))
            })?;
            if depth >= MAX_PARENT_DEPTH {
                return Err(BakeError::new(format!(
                    "model `{id}` reaches more than {MAX_PARENT_DEPTH} parent links"
                )));
            }
            self.reachable(&parent, depth + 1)?;
        }
        if self.decoded.len() >= MAX_REACHABLE_MODELS {
            return Err(BakeError::new(format!(
                "batch reaches more than {MAX_REACHABLE_MODELS} models"
            )));
        }
        self.decoded.insert(id.to_owned(), model);
        Ok(())
    }
}

fn loader_name(loader: &Loader) -> String {
    match loader {
        Loader::Composite { .. } => "neoforge:composite".to_owned(),
        Loader::Obj { .. } => "neoforge:obj".to_owned(),
        Loader::Unknown(name) => name.clone(),
    }
}

/// Refuse a face whose material alias does not resolve, rather than letting the
/// upstream baker substitute `minecraft:missingno`.
fn validate_materials(model: &str, resolved: &ResolvedCuboidModel) -> Result<(), BakeError> {
    for element in resolved.elements() {
        let element = element
            .as_ref()
            .ok_or_else(|| BakeError::new(format!("model `{model}` contains a null element")))?;
        for (direction, face) in &element.faces {
            let face = face.as_ref().ok_or_else(|| {
                BakeError::new(format!(
                    "model `{model}` contains a null `{}` face",
                    direction.name()
                ))
            })?;
            if face.texture.is_empty() {
                return Err(BakeError::new(format!(
                    "model `{model}` has an empty `{}` face texture",
                    direction.name()
                )));
            }
            match resolved.material(&face.texture) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(BakeError::new(format!(
                        "model `{model}` face `{}` has unresolved material `{}`",
                        direction.name(),
                        face.texture
                    )));
                }
                Err(error) => {
                    return Err(BakeError::new(format!(
                        "model `{model}` face `{}`: {error}",
                        direction.name()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Coordinates and UVs are unit-atlas values; the upstream baker writes `f32`.
/// A non-finite value would reach the vertex buffer, so it refuses the batch.
fn validate_quad(model: &str, quad: &ModelQuad) -> Result<(), BakeError> {
    let finite = quad
        .positions
        .iter()
        .all(|position| position.iter().all(|value| value.is_finite()))
        && quad
            .uv
            .iter()
            .all(|uv| uv.iter().all(|value| value.is_finite()));
    if !finite {
        return Err(BakeError::new(format!(
            "model `{model}` baked a non-finite quad"
        )));
    }
    Ok(())
}

fn validate_rotation(model: &str, degrees: u16, axis: &str) -> Result<(), BakeError> {
    if !degrees.is_multiple_of(90) || degrees >= 360 {
        return Err(BakeError::new(format!(
            "model `{model}` has {axis} rotation {degrees}, expected 0, 90, 180 or 270"
        )));
    }
    Ok(())
}

/// `namespace:path` to `assets/<namespace>/models/<path>.json`.
fn model_path(id: &str) -> String {
    let (namespace, path) = id
        .split_once(':')
        .expect("canonical model ids always contain a namespace");
    format!("assets/{namespace}/models/{path}.json")
}

/// Normalize and check a model id with the same strict subset the selector
/// emits: lowercase `[a-z0-9_.-]` namespace, `[a-z0-9/._-]` relative path with
/// no empty, `.` or `..` segment.
fn canonical_model(raw: &str) -> Result<String, BakeError> {
    if raw.is_empty() {
        return Err(BakeError::new("model id must not be empty"));
    }
    if raw.len() > MAX_MODEL_ID_LEN {
        return Err(BakeError::new(format!(
            "model id `{raw}` is longer than {MAX_MODEL_ID_LEN} bytes"
        )));
    }
    if raw.contains('\\') {
        return Err(BakeError::new(format!(
            "model id `{raw}` must not contain a backslash"
        )));
    }
    let (namespace, path) = raw.split_once(':').unwrap_or(("minecraft", raw));
    if namespace.is_empty() {
        return Err(BakeError::new(format!(
            "model id `{raw}` has an empty namespace"
        )));
    }
    if namespace.len() > MAX_NAMESPACE_LEN {
        return Err(BakeError::new(format!(
            "model id `{raw}` has a namespace longer than {MAX_NAMESPACE_LEN} bytes"
        )));
    }
    if !namespace.bytes().all(is_namespace_byte) || namespace == "." || namespace == ".." {
        return Err(BakeError::new(format!(
            "model id `{raw}` has an unsafe namespace"
        )));
    }
    if path.is_empty() {
        return Err(BakeError::new(format!(
            "model id `{raw}` has an empty path"
        )));
    }
    if path.len() > MAX_PATH_LEN {
        return Err(BakeError::new(format!(
            "model id `{raw}` has a path longer than {MAX_PATH_LEN} bytes"
        )));
    }
    if !path.bytes().all(is_path_byte)
        || path.starts_with('/')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(BakeError::new(format!(
            "model id `{raw}` is not a safe relative resource path"
        )));
    }
    Ok(format!("{namespace}:{path}"))
}

fn is_namespace_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.')
}

fn is_path_byte(byte: u8) -> bool {
    is_namespace_byte(byte) || byte == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::model::Direction;
    use serde_json::json;

    fn files(models: &[(&str, serde_json::Value)]) -> BTreeMap<String, Vec<u8>> {
        models
            .iter()
            .map(|(id, json)| {
                (
                    model_path(&canonical_model(id).unwrap()),
                    serde_json::to_vec(json).unwrap(),
                )
            })
            .collect()
    }

    fn request(model: &str, x: u16, y: u16, uvlock: bool) -> BakeRequest {
        BakeRequest {
            model: model.to_owned(),
            x,
            y,
            uvlock,
        }
    }

    #[test]
    fn cached_parent_depth_is_checked_independently_of_request_order() {
        let mut assets = BTreeMap::new();
        for i in 0..=MAX_PARENT_DEPTH {
            let model = if i == MAX_PARENT_DEPTH {
                json!({"elements":[]})
            } else {
                json!({"parent":format!("test:chain{}",i+1)})
            };
            assets.insert(
                format!("assets/test/models/chain{i}.json"),
                serde_json::to_vec(&model).unwrap(),
            );
        }
        assets.insert(
            "assets/test/models/extension.json".to_owned(),
            br#"{"parent":"test:chain0"}"#.to_vec(),
        );
        assert!(bake_requested(&assets, &[request("test:chain0", 0, 0, false)]).is_ok());
        assert!(bake_requested(
            &assets,
            &[
                request("test:chain0", 0, 0, false),
                request("test:extension", 0, 0, false)
            ]
        )
        .is_err());
    }

    fn bounds(quads: &[ModelQuad]) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for quad in quads {
            for position in quad.positions {
                for axis in 0..3 {
                    min[axis] = min[axis].min(position[axis]);
                    max[axis] = max[axis].max(position[axis]);
                }
            }
        }
        (min, max)
    }

    #[test]
    fn a_cuboid_bakes_six_unit_faces_with_expected_corners() {
        let files = files(&[(
            "minecraft:block/cube",
            json!({
                "textures": {"all": "block/synthetic"},
                "elements": [{
                    "from": [0, 0, 0], "to": [16, 16, 16],
                    "faces": {
                        "down": {"texture": "#all", "cullface": "down"},
                        "up": {"texture": "#all", "cullface": "up"},
                        "north": {"texture": "#all", "cullface": "north"},
                        "south": {"texture": "#all", "cullface": "south"},
                        "west": {"texture": "#all", "cullface": "west"},
                        "east": {"texture": "#all", "cullface": "east"}
                    }
                }]
            }),
        )]);
        let baked = bake_requested(&files, &[request("block/cube", 0, 0, false)]).unwrap();
        assert_eq!(baked.len(), 1);
        assert_eq!(baked[0].request.model, "minecraft:block/cube");
        let quads = &baked[0].quads;
        assert_eq!(quads.len(), 6);
        assert_eq!(bounds(quads), ([0.0; 3], [1.0; 3]));
        for quad in quads {
            assert_eq!(quad.material.sprite, "minecraft:block/synthetic");
            assert_eq!(quad.cull, Some(quad.facing));
            // A cube face has four unit-square corners, each on the face plane.
            let plane = match quad.facing {
                Direction::Down => (1, 0.0),
                Direction::Up => (1, 1.0),
                Direction::North => (2, 0.0),
                Direction::South => (2, 1.0),
                Direction::West => (0, 0.0),
                Direction::East => (0, 1.0),
            };
            for position in quad.positions {
                assert_eq!(position[plane.0], plane.1);
            }
            // UVs are the full unit atlas square, in some order.
            let mut uv: Vec<_> = quad
                .uv
                .iter()
                .map(|uv| (uv[0] as u8, uv[1] as u8))
                .collect();
            uv.sort();
            assert_eq!(uv, [(0, 0), (0, 1), (1, 0), (1, 1)]);
        }
    }

    #[test]
    fn a_child_overrides_the_parents_texture_and_keeps_its_geometry() {
        let files = files(&[
            (
                "minecraft:block/cube_all",
                json!({
                    "textures": {"all": "block/parent"},
                    "elements": [{
                        "from": [0, 0, 0], "to": [16, 16, 16],
                        "faces": {
                            "up": {"texture": "#all"},
                            "down": {"texture": "#all"}
                        }
                    }]
                }),
            ),
            (
                "minecraft:block/child",
                json!({"parent": "block/cube_all", "textures": {"all": "block/child"}}),
            ),
        ]);
        let baked = bake_requested(&files, &[request("block/child", 0, 0, false)]).unwrap();
        let quads = &baked[0].quads;
        assert_eq!(quads.len(), 2);
        // The child's slot wins for the geometry inherited from its parent.
        assert!(quads
            .iter()
            .all(|quad| quad.material.sprite == "minecraft:block/child"));
        assert_eq!(bounds(quads), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    }

    #[test]
    fn a_quarter_turn_rotates_an_asymmetric_element_about_the_block_centre() {
        let files = files(&[(
            "minecraft:block/half",
            json!({
                "textures": {"all": "block/synthetic"},
                "elements": [{
                    "from": [0, 0, 0], "to": [8, 4, 16],
                    "faces": {
                        "down": {"texture": "#all"},
                        "up": {"texture": "#all"},
                        "north": {"texture": "#all"},
                        "south": {"texture": "#all"},
                        "west": {"texture": "#all"},
                        "east": {"texture": "#all"}
                    }
                }]
            }),
        )]);
        // y=90 turns the half x extent onto z and the full z extent onto x,
        // keeping the vertical extent.
        let baked = bake_requested(&files, &[request("block/half", 0, 90, false)]).unwrap();
        let (min, max) = bounds(&baked[0].quads);
        let close = |value: f32, expected: f32| (value - expected).abs() < 1e-6;
        assert!(close(min[0], 0.0) && close(max[0], 1.0), "{min:?}..{max:?}");
        assert!(
            close(min[1], 0.0) && close(max[1], 0.25),
            "{min:?}..{max:?}"
        );
        assert!(close(min[2], 0.0) && close(max[2], 0.5), "{min:?}..{max:?}");
        // UV locking is a distinct transform choice: the unrotated atlas
        // coordinates must not survive it.
        let unlocked = bake_requested(&files, &[request("block/half", 0, 90, false)]).unwrap();
        let locked = bake_requested(&files, &[request("block/half", 0, 90, true)]).unwrap();
        assert_ne!(
            unlocked[0].quads[0].uv, locked[0].quads[0].uv,
            "uvlock must change the face UVs of a rotated model"
        );
    }

    #[test]
    fn a_flat_element_keeps_only_its_face_faces() {
        // Zero Y extent: the block-specific baker draws only the up/down
        // faces; the generic model baker would also draw the four sides.
        let files = files(&[(
            "minecraft:block/slab_plane",
            json!({
                "textures": {"all": "block/synthetic"},
                "elements": [{
                    "from": [0, 8, 0], "to": [16, 8, 16],
                    "faces": {
                        "down": {"texture": "#all"}, "up": {"texture": "#all"},
                        "north": {"texture": "#all"}, "south": {"texture": "#all"},
                        "west": {"texture": "#all"}, "east": {"texture": "#all"}
                    }
                }]
            }),
        )]);
        let baked = bake_requested(&files, &[request("block/slab_plane", 0, 0, false)]).unwrap();
        let facings: BTreeSet<_> = baked[0].quads.iter().map(|quad| quad.facing).collect();
        assert_eq!(
            facings,
            BTreeSet::from([Direction::Down, Direction::Up]),
            "only the faces perpendicular to the flat axis survive"
        );
        assert_eq!(bounds(&baked[0].quads).0[1], 0.5);
    }

    #[test]
    fn an_explicitly_empty_element_list_is_valid_empty_geometry() {
        let files = files(&[("minecraft:block/empty", json!({"elements": []}))]);
        let baked = bake_requested(&files, &[request("block/empty", 0, 0, false)]).unwrap();
        assert_eq!(baked.len(), 1);
        assert!(baked[0].quads.is_empty());
        assert_eq!(
            bake_requested(&files, &[]).unwrap(),
            Vec::new(),
            "an empty batch is valid"
        );
    }

    #[test]
    fn a_missing_parent_is_refused_instead_of_using_the_missing_model() {
        let files = files(&[("minecraft:block/orphan", json!({"parent": "block/absent"}))]);
        let error = bake_requested(&files, &[request("block/orphan", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("block/absent"),
            "unexpected error: {error}"
        );
        assert!(
            error.message().contains("missing"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_cyclic_parent_chain_is_refused() {
        let files = files(&[
            ("minecraft:block/a", json!({"parent": "block/b"})),
            ("minecraft:block/b", json!({"parent": "block/a"})),
        ]);
        let error = bake_requested(&files, &[request("block/a", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("cyclic"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_too_deep_parent_chain_is_refused() {
        let mut models = Vec::new();
        for depth in 0..=MAX_PARENT_DEPTH {
            let parent = format!("block/p{}", depth + 1);
            models.push((
                format!("minecraft:block/p{depth}"),
                json!({"parent": parent}),
            ));
        }
        models.push((
            format!("minecraft:block/p{}", MAX_PARENT_DEPTH + 1),
            json!({}),
        ));
        let owned: Vec<(&str, serde_json::Value)> = models
            .iter()
            .map(|(id, json)| (id.as_str(), json.clone()))
            .collect();
        let error =
            bake_requested(&files(&owned), &[request("block/p0", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("parent links"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_oversized_model_source_is_refused() {
        // A valid JSON object over the per-model byte limit.
        let padding = "a".repeat(MAX_MODEL_BYTES);
        let mut bytes = Vec::with_capacity(MAX_MODEL_BYTES + 16);
        bytes.extend_from_slice(br#"{"ignored":""#);
        bytes.extend_from_slice(padding.as_bytes());
        bytes.extend_from_slice(br#""}"#);
        let files = BTreeMap::from([(model_path("minecraft:block/huge"), bytes)]);
        let error = bake_requested(&files, &[request("block/huge", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("limit"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn duplicate_and_invalid_requests_are_refused_before_any_model_is_read() {
        let files = BTreeMap::new();
        let duplicate = [
            request("block/a", 0, 90, true),
            request("minecraft:block/a", 0, 90, true),
        ];
        assert!(bake_requested(&files, &duplicate)
            .unwrap_err()
            .message()
            .contains("duplicate"));
        for rotation in [45, 360] {
            assert!(bake_requested(&files, &[request("block/a", rotation, 0, false)]).is_err());
        }
        for model in ["", "Block/A", "block/../a", "block//a", "block/a/"] {
            assert!(
                bake_requested(&files, &[request(model, 0, 0, false)]).is_err(),
                "`{model}` must be refused"
            );
        }
        let too_many: Vec<_> = (0..=MAX_REQUESTS)
            .map(|index| request(&format!("block/m{index}"), 0, 0, false))
            .collect();
        assert!(bake_requested(&files, &too_many)
            .unwrap_err()
            .message()
            .contains("requests"));
    }

    #[test]
    fn too_many_reachable_models_are_refused() {
        let mut files = BTreeMap::new();
        let mut requests = Vec::new();
        for index in 0..=MAX_REACHABLE_MODELS {
            files.insert(
                model_path(&format!("minecraft:block/m{index}")),
                b"{}".to_vec(),
            );
            requests.push(request(&format!("block/m{index}"), 0, 0, false));
        }
        let error = bake_requested(&files, &requests).unwrap_err();
        assert!(
            error.message().contains("reaches more than"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unresolved_face_material_is_refused_not_replaced() {
        let files = files(&[(
            "minecraft:block/alias",
            json!({
                "textures": {"all": "block/synthetic"},
                "elements": [{
                    "from": [0, 0, 0], "to": [16, 16, 16],
                    "faces": {"up": {"texture": "#absent"}}
                }]
            }),
        )]);
        let error = bake_requested(&files, &[request("block/alias", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("#absent"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_custom_model_loader_is_refused_explicitly() {
        for (loader, name) in [
            ("neoforge:composite", "neoforge:composite"),
            ("neoforge:obj", "neoforge:obj"),
            ("syntheticmod:loader", "syntheticmod:loader"),
        ] {
            let mut model = json!({"loader": loader});
            if loader == "neoforge:obj" {
                model["model"] = json!("syntheticmod:models/x.obj");
            }
            let files = files(&[("minecraft:block/custom", model)]);
            let error =
                bake_requested(&files, &[request("block/custom", 0, 0, false)]).unwrap_err();
            assert!(
                error.message().contains(name),
                "unexpected error for {loader}: {error}"
            );
        }
    }

    #[test]
    fn an_upstream_builtin_model_is_refused_not_substituted() {
        // A resource with the library's missing-model name would otherwise be
        // replaced by the built-in cube without any error.
        let files = files(&[("minecraft:builtin/missing", json!({"elements": []}))]);
        let error = bake_requested(&files, &[request("builtin/missing", 0, 0, false)]).unwrap_err();
        assert!(
            error.message().contains("built-in"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_failing_request_returns_no_partial_result() {
        let files = files(&[(
            "minecraft:block/fine",
            json!({
                "textures": {"all": "block/synthetic"},
                "elements": [{
                    "from": [0, 0, 0], "to": [16, 16, 16],
                    "faces": {"up": {"texture": "#all"}}
                }]
            }),
        )]);
        let good = request("block/fine", 0, 0, false);
        assert_eq!(
            bake_requested(&files, std::slice::from_ref(&good))
                .unwrap()
                .len(),
            1
        );
        let bad = request("block/missing", 0, 0, false);
        // One bad request poisons the batch: the caller gets an error instead
        // of the already baked `block/fine` model.
        assert!(bake_requested(&files, &[good, bad]).is_err());
    }
}
