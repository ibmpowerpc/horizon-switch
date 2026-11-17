//! Bounded host-side decoding of verified block textures into one static RGBA
//! atlas.
//!
//! The caller keeps the bytes contract: `files` is a manifest-verified asset
//! map (`assets/<namespace>/textures/<path>.png` to its verified bytes), and
//! `sprites` are the material sprite ids the baked models actually use. This
//! module never reads a file, guesses a placeholder, samples randomness or
//! resolves an animation: a sprite whose PNG or animation metadata it cannot
//! prove static fails the whole build, instead of quietly taking frame zero.
//!
//! Decoding accepts 8-bit PNG (RGB, RGBA, grayscale, grayscale-alpha and
//! indexed, including `tRNS`) and expands everything to straight,
//! unpremultiplied RGBA8. 16-bit samples, APNG animations and animated
//! `.mcmeta` sidecars are [`AtlasError::Unsupported`]. No gamma or colour-space
//! conversion happens here: the native client's sRGB choice is a parent
//! decision, and the pixels are the source pixels.
//!
//! Packing is deterministic: sprites are ordered by padded height, padded width
//! and canonical id, placed on shelves, and the smallest power-of-two square
//! that fits is chosen. Every sprite is copied pixel for pixel and its edge
//! rows and columns are extruded into a fixed two-pixel gutter (corners
//! included); unused atlas space stays zero. Failure returns no atlas at all.

use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Maximum size of one PNG source file.
pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum width or height of one PNG source image.
pub const MAX_SOURCE_DIMENSION: u32 = 4096;
/// Extruded gutter around every sprite, in pixels.
pub const GUTTER: u32 = 2;

const MAX_MCMETA_BYTES: usize = 256 * 1024;
const MAX_SPRITE_ID_LEN: usize = 512;
const MAX_NAMESPACE_LEN: usize = 64;
const MAX_PATH_LEN: usize = 256;
const CHANNELS: usize = 4;

/// Bounds for one atlas build. They bound host memory and output shape only;
/// they say nothing about what the Switch device can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasLimits {
    /// Largest square atlas dimension, inclusive. Packing chooses the smallest
    /// fitting power of two that does not exceed this value.
    pub max_dimension: u32,
    /// Maximum number of distinct sprite ids.
    pub max_sprites: usize,
    /// Maximum total decoded RGBA bytes across all sprites.
    pub max_decoded_bytes: usize,
}

impl Default for AtlasLimits {
    fn default() -> Self {
        Self {
            max_dimension: 1024,
            max_sprites: 256,
            max_decoded_bytes: 64 * 1024 * 1024,
        }
    }
}

/// One built atlas. `rgba` is row-major straight RGBA8, `width * height * 4`
/// bytes long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Sprite id to its inner rectangle, excluding the extruded gutter.
    pub sprites: BTreeMap<String, SpriteRect>,
}

/// A sprite's inner rectangle inside the atlas. `x`/`y` point at the first
/// source pixel, not at the gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// At least one decoded pixel is not fully opaque.
    pub has_alpha: bool,
    /// At least one decoded pixel is neither fully transparent nor fully
    /// opaque. The render layer is a separate decision: alpha flags come from
    /// the pixels, so a sprite with only 0/255 alpha can still require a
    /// translucent render layer even when `has_partial_alpha` is false.
    pub has_partial_alpha: bool,
}

/// Why an atlas was refused. Every variant leaves no atlas behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtlasError {
    /// A sprite id is not a canonical `namespace:path` resource id.
    InvalidId(String),
    /// The verified asset bytes name no PNG for the sprite.
    MissingBytes(String),
    /// The sprite is not something a static proof can decode (16-bit samples,
    /// APNG, animated or unreadable animation metadata).
    Unsupported(String),
    /// The PNG bytes are not a decodable PNG.
    Decode(String),
    /// A configured or fixed bound was exceeded.
    Limit(String),
}

impl AtlasError {
    /// The human-readable reason, without a trailing period.
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidId(message)
            | Self::MissingBytes(message)
            | Self::Unsupported(message)
            | Self::Decode(message)
            | Self::Limit(message) => message,
        }
    }
}

impl fmt::Display for AtlasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for AtlasError {}

/// Decode `sprites` from already verified asset bytes and pack one static
/// atlas.
///
/// Repeats after canonicalisation (for example `stone` and `minecraft:stone`)
/// are one sprite. Every id needs its own PNG; nothing is replaced by a
/// placeholder and a failed build returns no partial atlas.
pub fn build_static_atlas(
    files: &BTreeMap<String, Vec<u8>>,
    sprites: &[String],
    limits: AtlasLimits,
) -> Result<Atlas, AtlasError> {
    validate_limits(&limits)?;

    let mut ids = BTreeSet::new();
    for raw in sprites {
        ids.insert(canonical_id(raw)?);
    }
    if ids.len() > limits.max_sprites {
        return Err(AtlasError::Limit(format!(
            "{} distinct sprites exceed the limit of {}",
            ids.len(),
            limits.max_sprites
        )));
    }

    let mut decoded = Vec::with_capacity(ids.len());
    let mut decoded_bytes = 0usize;
    for id in &ids {
        let sprite = decode_sprite(files, id, limits.max_decoded_bytes - decoded_bytes)?;
        decoded_bytes = decoded_bytes
            .checked_add(sprite.rgba.len())
            .ok_or_else(|| AtlasError::Limit("decoded atlas size overflow".to_owned()))?;
        decoded.push(sprite);
    }

    let mut order: Vec<usize> = (0..decoded.len()).collect();
    order.sort_by(|&a, &b| {
        let left = &decoded[a];
        let right = &decoded[b];
        right
            .height
            .cmp(&left.height)
            .then(right.width.cmp(&left.width))
            .then(left.id.cmp(&right.id))
    });

    let (dimension, placements) = choose_packing(&sprites_by_order(&decoded, &order), &limits)?;
    let dimensions = dimension as usize;
    let pixels = dimensions
        .checked_mul(dimensions)
        .ok_or_else(|| AtlasError::Limit("atlas dimension overflow".to_owned()))?;
    let bytes = pixels
        .checked_mul(CHANNELS)
        .ok_or_else(|| AtlasError::Limit("atlas byte size overflow".to_owned()))?;
    let mut rgba = vec![0u8; bytes];
    let mut rects = BTreeMap::new();

    for (index, placement) in order.iter().zip(placements) {
        let sprite = &decoded[*index];
        copy_with_gutter(sprite, &mut rgba, dimension, placement);
        rects.insert(
            sprite.id.clone(),
            SpriteRect {
                x: placement.0 + GUTTER,
                y: placement.1 + GUTTER,
                width: sprite.width,
                height: sprite.height,
                has_alpha: sprite.has_alpha,
                has_partial_alpha: sprite.has_partial_alpha,
            },
        );
    }

    Ok(Atlas {
        width: dimension,
        height: dimension,
        rgba,
        sprites: rects,
    })
}

fn validate_limits(limits: &AtlasLimits) -> Result<(), AtlasError> {
    if limits.max_dimension == 0 {
        return Err(AtlasError::Limit(
            "max atlas dimension must be at least 1".to_owned(),
        ));
    }
    if limits.max_dimension > MAX_SOURCE_DIMENSION {
        return Err(AtlasError::Limit(format!(
            "max atlas dimension {} exceeds the {MAX_SOURCE_DIMENSION} pixel bound",
            limits.max_dimension
        )));
    }
    Ok(())
}

struct Sprite {
    id: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    has_alpha: bool,
    has_partial_alpha: bool,
}

/// Padded size of one sprite: the source plus a gutter on every side.
fn padded(sprite: &Sprite) -> Result<(u32, u32), AtlasError> {
    let gutter = GUTTER
        .checked_mul(2)
        .ok_or_else(|| AtlasError::Limit("gutter overflow".to_owned()))?;
    Ok((
        sprite
            .width
            .checked_add(gutter)
            .ok_or_else(|| AtlasError::Limit(format!("sprite `{}` width overflow", sprite.id)))?,
        sprite
            .height
            .checked_add(gutter)
            .ok_or_else(|| AtlasError::Limit(format!("sprite `{}` height overflow", sprite.id)))?,
    ))
}

/// A stable, already size-ordered list, so the packer never sees input order.
fn sprites_by_order<'a>(decoded: &'a [Sprite], order: &[usize]) -> Vec<&'a Sprite> {
    order.iter().map(|&index| &decoded[index]).collect()
}

type Placement = (u32, u32);

/// Try shelf packing at every power-of-two dimension up to the limit and take
/// the first that fits. A failed fit is not an error; only running out of
/// dimensions is.
fn choose_packing(
    ordered: &[&Sprite],
    limits: &AtlasLimits,
) -> Result<(u32, Vec<Placement>), AtlasError> {
    let mut dimension = 1u32;
    loop {
        if let Some(placements) = pack(ordered, dimension)? {
            return Ok((dimension, placements));
        }
        if dimension > limits.max_dimension / 2 {
            return Err(AtlasError::Limit(format!(
                "sprites do not fit in a power-of-two square up to {}x{}",
                limits.max_dimension, limits.max_dimension
            )));
        }
        dimension *= 2;
    }
}

/// Next-fit shelves in the given order. `None` means this dimension is too
/// small; an overflow of the fixed bounds is a hard error.
fn pack(ordered: &[&Sprite], dimension: u32) -> Result<Option<Vec<Placement>>, AtlasError> {
    let mut placements = Vec::with_capacity(ordered.len());
    let mut shelf_y = 0u32;
    let mut shelf_height = 0u32;
    let mut used_x = 0u32;
    let mut placed = false;

    for sprite in ordered {
        let (width, height) = padded(sprite)?;
        if width > dimension || height > dimension {
            return Ok(None);
        }
        if !placed {
            shelf_y = 0;
            shelf_height = height;
            used_x = width;
            placements.push((0, 0));
            placed = true;
            continue;
        }
        let next_x = used_x
            .checked_add(width)
            .ok_or_else(|| AtlasError::Limit("shelf width overflow".to_owned()))?;
        if height <= shelf_height && next_x <= dimension {
            placements.push((used_x, shelf_y));
            used_x = next_x;
            continue;
        }
        let next_y = shelf_y
            .checked_add(shelf_height)
            .ok_or_else(|| AtlasError::Limit("shelf height overflow".to_owned()))?;
        let bottom = next_y
            .checked_add(height)
            .ok_or_else(|| AtlasError::Limit("shelf bottom overflow".to_owned()))?;
        if bottom > dimension {
            return Ok(None);
        }
        shelf_y = next_y;
        shelf_height = height;
        used_x = width;
        placements.push((0, shelf_y));
    }

    Ok(Some(placements))
}

/// Copy the source exactly and repeat its edge pixels, corners included, into
/// the gutter.
fn copy_with_gutter(sprite: &Sprite, rgba: &mut [u8], dimension: u32, at: Placement) {
    let (padded_width, padded_height) =
        padded(sprite).expect("padded sprite dimensions were checked during packing");
    let atlas_width = dimension as usize;
    let source_width = sprite.width as usize;
    for row in 0..padded_height {
        let source_y = row.saturating_sub(GUTTER).min(sprite.height - 1);
        let target_y = (at.1 + row) as usize;
        for column in 0..padded_width {
            let source_x = column.saturating_sub(GUTTER).min(sprite.width - 1);
            let source = (source_y as usize * source_width + source_x as usize) * CHANNELS;
            let target = (target_y * atlas_width + (at.0 + column) as usize) * CHANNELS;
            rgba[target..target + CHANNELS]
                .copy_from_slice(&sprite.rgba[source..source + CHANNELS]);
        }
    }
}

/// Decode one sprite, charging its RGBA output against the remaining budget.
fn decode_sprite(
    files: &BTreeMap<String, Vec<u8>>,
    id: &str,
    remaining: usize,
) -> Result<Sprite, AtlasError> {
    let path = sprite_path(id);
    let bytes = files.get(&path).ok_or_else(|| {
        AtlasError::MissingBytes(format!(
            "sprite `{id}` has no verified PNG ({path} missing)"
        ))
    })?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(AtlasError::Limit(format!(
            "sprite `{id}` is {} bytes, over the {MAX_SOURCE_BYTES}-byte source limit",
            bytes.len()
        )));
    }
    refuse_animated_metadata(files, id, &path)?;

    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes.as_slice()));
    decoder.set_limits(png::Limits { bytes: remaining });
    // Expand palettes, tRNS and sub-8-bit gray to 8-bit samples; 16-bit sources
    // are refused after the header instead of being silently narrowed.
    decoder.set_transformations(png::Transformations::normalize_to_color8());

    {
        let header = decoder
            .read_header_info()
            .map_err(|error| decode_error(id, error))?;
        check_dimension(id, header.width, header.height)?;
        if header.bit_depth == png::BitDepth::Sixteen {
            return Err(AtlasError::Unsupported(format!(
                "sprite `{id}` is a 16-bit PNG; only 8-bit samples are supported"
            )));
        }
    }

    let mut reader = decoder
        .read_info()
        .map_err(|error| decode_error(id, error))?;
    let info = reader.info();
    if info.animation_control.is_some() {
        return Err(AtlasError::Unsupported(format!(
            "sprite `{id}` is an APNG animation; this static atlas does not take frame 0"
        )));
    }
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err(AtlasError::Unsupported(format!(
            "sprite `{id}` decodes to {depth:?} samples; only 8-bit samples are supported"
        )));
    }
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(AtlasError::Unsupported(format!(
                "sprite `{id}` kept an indexed output colour type after expansion"
            )))
        }
    };
    let (width, height) = info.size();
    check_dimension(id, width, height)?;

    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| AtlasError::Limit(format!("sprite `{id}` pixel count overflow")))?;
    let expected = pixels
        .checked_mul(channels)
        .ok_or_else(|| AtlasError::Limit(format!("sprite `{id}` decoded size overflow")))?;
    let rgba_bytes = pixels
        .checked_mul(CHANNELS)
        .ok_or_else(|| AtlasError::Limit(format!("sprite `{id}` RGBA size overflow")))?;
    if rgba_bytes > remaining {
        return Err(AtlasError::Limit(format!(
            "sprite `{id}` needs {rgba_bytes} decoded bytes, over the remaining {remaining}"
        )));
    }
    if reader.output_buffer_size() != Some(expected) {
        return Err(AtlasError::Decode(format!(
            "sprite `{id}` reports an inconsistent decoded buffer size"
        )));
    }

    let mut raw = vec![0u8; expected];
    reader
        .next_frame(&mut raw)
        .map_err(|error| decode_error(id, error))?;
    let rgba = to_rgba(id, &raw, color, pixels)?;

    let mut has_alpha = false;
    let mut has_partial_alpha = false;
    for pixel in rgba.as_chunks::<CHANNELS>().0 {
        match pixel[3] {
            255 => {}
            0 => has_alpha = true,
            _ => {
                has_alpha = true;
                has_partial_alpha = true;
            }
        }
    }

    Ok(Sprite {
        id: id.to_owned(),
        width,
        height,
        rgba,
        has_alpha,
        has_partial_alpha,
    })
}

fn check_dimension(id: &str, width: u32, height: u32) -> Result<(), AtlasError> {
    if width == 0 || height == 0 {
        return Err(AtlasError::Decode(format!(
            "sprite `{id}` has an empty {width}x{height} image"
        )));
    }
    if width > MAX_SOURCE_DIMENSION || height > MAX_SOURCE_DIMENSION {
        return Err(AtlasError::Limit(format!(
            "sprite `{id}` is {width}x{height}, over the {MAX_SOURCE_DIMENSION}-pixel source bound"
        )));
    }
    Ok(())
}

fn decode_error(id: &str, error: png::DecodingError) -> AtlasError {
    AtlasError::Decode(format!("sprite `{id}` is not a decodable PNG: {error}"))
}

/// Expand one decoded frame into straight RGBA8, without gamma, colour-space
/// or premultiplication changes.
fn to_rgba(
    id: &str,
    raw: &[u8],
    color: png::ColorType,
    pixels: usize,
) -> Result<Vec<u8>, AtlasError> {
    let mut rgba = Vec::with_capacity(
        pixels
            .checked_mul(CHANNELS)
            .ok_or_else(|| AtlasError::Limit(format!("sprite `{id}` RGBA size overflow")))?,
    );
    match color {
        png::ColorType::Grayscale => {
            for &gray in raw {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in raw.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Rgb => {
            for pixel in raw.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(raw),
        png::ColorType::Indexed => {
            return Err(AtlasError::Unsupported(format!(
                "sprite `{id}` kept an indexed output colour type after expansion"
            )))
        }
    }
    if rgba.len() != pixels * CHANNELS {
        return Err(AtlasError::Decode(format!(
            "sprite `{id}` decoded a short or ragged frame"
        )));
    }
    Ok(rgba)
}

/// A sidecar that describes an animation is refused, and unreadable sidecar
/// metadata is refused too. A sidecar without an `animation` key only carries
/// static texture settings, which do not change these pixels.
fn refuse_animated_metadata(
    files: &BTreeMap<String, Vec<u8>>,
    id: &str,
    path: &str,
) -> Result<(), AtlasError> {
    let sidecar = format!("{path}.mcmeta");
    let Some(bytes) = files.get(&sidecar) else {
        return Ok(());
    };
    if bytes.len() > MAX_MCMETA_BYTES {
        return Err(AtlasError::Limit(format!(
            "sprite `{id}` animation metadata is {} bytes, over the {MAX_MCMETA_BYTES}-byte limit",
            bytes.len()
        )));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        AtlasError::Unsupported(format!(
            "sprite `{id}` has unreadable animation metadata ({sidecar}): {error}"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        AtlasError::Unsupported(format!(
            "sprite `{id}` animation metadata ({sidecar}) is not a JSON object"
        ))
    })?;
    if object.contains_key("animation") {
        return Err(AtlasError::Unsupported(format!(
            "sprite `{id}` is animated by {sidecar}; this static atlas does not take frame 0"
        )));
    }
    Ok(())
}

/// `namespace:path` to `assets/<namespace>/textures/<path>.png`.
fn sprite_path(id: &str) -> String {
    let (namespace, path) = id
        .split_once(':')
        .expect("canonical sprite ids always contain a namespace");
    format!("assets/{namespace}/textures/{path}.png")
}

/// Normalize and check a sprite id: lowercase `[a-z0-9_.-]` namespace and a
/// `[a-z0-9/._-]` relative path with no empty, `.` or `..` segment.
fn canonical_id(raw: &str) -> Result<String, AtlasError> {
    if raw.is_empty() {
        return Err(AtlasError::InvalidId(
            "sprite id must not be empty".to_owned(),
        ));
    }
    if raw.len() > MAX_SPRITE_ID_LEN {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` is longer than {MAX_SPRITE_ID_LEN} bytes"
        )));
    }
    if raw.contains('\\') {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` must not contain a backslash"
        )));
    }
    let (namespace, path) = raw.split_once(':').unwrap_or(("minecraft", raw));
    if namespace.is_empty() {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` has an empty namespace"
        )));
    }
    if namespace.len() > MAX_NAMESPACE_LEN {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` has a namespace longer than {MAX_NAMESPACE_LEN} bytes"
        )));
    }
    if !namespace.bytes().all(is_namespace_byte) || namespace == "." || namespace == ".." {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` has an unsafe namespace"
        )));
    }
    if path.is_empty() {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` has an empty path"
        )));
    }
    if path.len() > MAX_PATH_LEN {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` has a path longer than {MAX_PATH_LEN} bytes"
        )));
    }
    if !path.bytes().all(is_path_byte)
        || path.starts_with('/')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(AtlasError::InvalidId(format!(
            "sprite id `{raw}` is not a safe relative resource path"
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
    use std::io::Cursor;

    fn encode(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        palette: Option<&[u8]>,
        trns: Option<&[u8]>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(Cursor::new(&mut out), width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if let Some(palette) = palette {
                encoder.set_palette(palette);
            }
            if let Some(trns) = trns {
                encoder.set_trns(trns);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
            writer.finish().unwrap();
        }
        out
    }

    fn rgba_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        encode(
            width,
            height,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            None,
            None,
            pixels,
        )
    }

    fn assets(entries: &[(&str, Vec<u8>)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(id, bytes)| (sprite_path(&canonical_id(id).unwrap()), bytes.clone()))
            .collect()
    }

    fn build(files: &BTreeMap<String, Vec<u8>>, sprites: &[&str]) -> Atlas {
        let sprites: Vec<String> = sprites.iter().map(|id| (*id).to_owned()).collect();
        build_static_atlas(files, &sprites, AtlasLimits::default()).unwrap()
    }

    fn pixels(atlas: &Atlas, rect: &SpriteRect) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let offset = ((y as usize * atlas.width as usize) + x as usize) * 4;
                out.push([
                    atlas.rgba[offset],
                    atlas.rgba[offset + 1],
                    atlas.rgba[offset + 2],
                    atlas.rgba[offset + 3],
                ]);
            }
        }
        out
    }

    fn pixel(atlas: &Atlas, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y as usize * atlas.width as usize) + x as usize) * 4;
        [
            atlas.rgba[offset],
            atlas.rgba[offset + 1],
            atlas.rgba[offset + 2],
            atlas.rgba[offset + 3],
        ]
    }

    /// Straight source pixels reach the atlas unchanged for every supported
    /// eight-bit colour type.
    #[test]
    fn eight_bit_forms_expand_to_exact_straight_rgba() {
        let files = assets(&[
            (
                "minecraft:block/rgba",
                rgba_png(
                    2,
                    2,
                    &[
                        1, 2, 3, 4, 5, 6, 7, 8, //
                        9, 10, 11, 12, 13, 14, 15, 255,
                    ],
                ),
            ),
            (
                "minecraft:block/rgb",
                encode(
                    2,
                    1,
                    png::ColorType::Rgb,
                    png::BitDepth::Eight,
                    None,
                    None,
                    &[10, 20, 30, 200, 210, 220],
                ),
            ),
            (
                "minecraft:block/gray",
                encode(
                    2,
                    1,
                    png::ColorType::Grayscale,
                    png::BitDepth::Eight,
                    None,
                    None,
                    &[7, 200],
                ),
            ),
            (
                "minecraft:block/gray_alpha",
                encode(
                    2,
                    1,
                    png::ColorType::GrayscaleAlpha,
                    png::BitDepth::Eight,
                    None,
                    None,
                    &[7, 0, 200, 128],
                ),
            ),
        ]);
        let atlas = build(
            &files,
            &[
                "minecraft:block/rgba",
                "minecraft:block/rgb",
                "minecraft:block/gray",
                "minecraft:block/gray_alpha",
            ],
        );

        let rgba = atlas.sprites["minecraft:block/rgba"];
        assert_eq!(
            pixels(&atlas, &rgba),
            [
                [1, 2, 3, 4],
                [5, 6, 7, 8],
                [9, 10, 11, 12],
                [13, 14, 15, 255]
            ]
        );
        assert!(rgba.has_alpha);
        assert!(rgba.has_partial_alpha);

        let rgb = atlas.sprites["minecraft:block/rgb"];
        assert_eq!(
            pixels(&atlas, &rgb),
            [[10, 20, 30, 255], [200, 210, 220, 255]]
        );
        assert!(!rgb.has_alpha);
        assert!(!rgb.has_partial_alpha);

        let gray = atlas.sprites["minecraft:block/gray"];
        assert_eq!(
            pixels(&atlas, &gray),
            [[7, 7, 7, 255], [200, 200, 200, 255]]
        );

        let gray_alpha = atlas.sprites["minecraft:block/gray_alpha"];
        assert_eq!(
            pixels(&atlas, &gray_alpha),
            [[7, 7, 7, 0], [200, 200, 200, 128]]
        );
        assert!(gray_alpha.has_alpha);
        assert!(gray_alpha.has_partial_alpha);
    }

    /// Palettes, `tRNS` and `tRNS` on RGB expand to straight alpha; nothing is
    /// premultiplied.
    #[test]
    fn palette_and_trns_expand_without_premultiplication() {
        let files = assets(&[
            (
                "minecraft:block/indexed",
                encode(
                    2,
                    2,
                    png::ColorType::Indexed,
                    png::BitDepth::Eight,
                    Some(&[200, 10, 20, 30, 220, 40, 50, 60, 230, 1, 2, 3]),
                    Some(&[255, 128, 0]),
                    &[0, 1, 2, 0],
                ),
            ),
            (
                "minecraft:block/indexed_opaque",
                encode(
                    1,
                    2,
                    png::ColorType::Indexed,
                    png::BitDepth::Eight,
                    Some(&[10, 20, 30, 40, 50, 60]),
                    None,
                    &[0, 1],
                ),
            ),
            (
                "minecraft:block/rgb_trns",
                encode(
                    2,
                    1,
                    png::ColorType::Rgb,
                    png::BitDepth::Eight,
                    None,
                    Some(&[0, 200, 0, 10, 0, 20]),
                    &[200, 10, 20, 1, 2, 3],
                ),
            ),
        ]);
        let atlas = build(
            &files,
            &[
                "minecraft:block/indexed",
                "minecraft:block/indexed_opaque",
                "minecraft:block/rgb_trns",
            ],
        );

        let indexed = atlas.sprites["minecraft:block/indexed"];
        assert_eq!(
            pixels(&atlas, &indexed),
            [
                [200, 10, 20, 255],
                [30, 220, 40, 128],
                [50, 60, 230, 0],
                [200, 10, 20, 255]
            ]
        );
        assert!(indexed.has_alpha);
        assert!(indexed.has_partial_alpha);

        let opaque = atlas.sprites["minecraft:block/indexed_opaque"];
        assert_eq!(
            pixels(&atlas, &opaque),
            [[10, 20, 30, 255], [40, 50, 60, 255]]
        );
        assert!(!opaque.has_alpha);

        let trns = atlas.sprites["minecraft:block/rgb_trns"];
        assert_eq!(pixels(&atlas, &trns), [[200, 10, 20, 0], [1, 2, 3, 255]]);
        assert!(trns.has_alpha);
        assert!(!trns.has_partial_alpha);
    }

    /// Input order, aliases and canonical repeats do not change the result.
    #[test]
    fn packing_is_deterministic_across_input_order_and_aliases() {
        let files = assets(&[
            (
                "minecraft:block/a",
                rgba_png(2, 2, &[1, 2, 3, 255].repeat(4)),
            ),
            (
                "minecraft:block/b",
                rgba_png(3, 1, &[4, 5, 6, 128].repeat(3)),
            ),
            (
                "minecraft:block/c",
                rgba_png(1, 4, &[7, 8, 9, 255].repeat(4)),
            ),
        ]);
        let first = build(
            &files,
            &["block/a", "minecraft:block/b", "minecraft:block/c"],
        );
        let second = build(
            &files,
            &[
                "minecraft:block/c",
                "minecraft:block/b",
                "minecraft:block/a",
                "block/a",
                "block/c",
            ],
        );
        assert_eq!(first, second);
        assert_eq!(first.sprites.len(), 3);
        assert!(first.sprites.contains_key("minecraft:block/a"));
        assert_eq!(first.sprites["minecraft:block/a"].width, 2);
        assert_eq!(first.sprites["minecraft:block/b"].height, 1);
    }

    /// Edges and corners repeat into the two-pixel gutter; the rest of the
    /// atlas stays zero.
    #[test]
    fn gutters_extrude_edges_and_corners_and_unused_space_is_zero() {
        let files = assets(&[(
            "minecraft:block/quad",
            rgba_png(
                2,
                2,
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, //
                    0, 0, 255, 255, 255, 255, 0, 255,
                ],
            ),
        )]);
        let atlas = build(&files, &["minecraft:block/quad"]);
        let rect = atlas.sprites["minecraft:block/quad"];
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (2, 2, 2, 2));
        let tl = [255, 0, 0, 255];
        let tr = [0, 255, 0, 255];
        let bl = [0, 0, 255, 255];
        let br = [255, 255, 0, 255];
        // Both gutter rings, corners included.
        for gutter in 1..=2 {
            assert_eq!(pixel(&atlas, rect.x - gutter, rect.y - gutter), tl);
            assert_eq!(pixel(&atlas, rect.x + 1 + gutter, rect.y - gutter), tr);
            assert_eq!(pixel(&atlas, rect.x - gutter, rect.y + 1 + gutter), bl);
            assert_eq!(pixel(&atlas, rect.x + 1 + gutter, rect.y + 1 + gutter), br);
        }
        assert_eq!(pixels(&atlas, &rect), [tl, tr, bl, br]);

        let padded_left = rect.x - GUTTER;
        let padded_top = rect.y - GUTTER;
        let padded_right = rect.x + rect.width + GUTTER;
        let padded_bottom = rect.y + rect.height + GUTTER;
        for y in 0..atlas.height {
            for x in 0..atlas.width {
                let inside =
                    x >= padded_left && x < padded_right && y >= padded_top && y < padded_bottom;
                if !inside {
                    assert_eq!(pixel(&atlas, x, y), [0, 0, 0, 0], "pixel {x}x{y}");
                }
            }
        }
    }

    /// Missing bytes, animations, unsupported formats and every bound fail the
    /// whole build; a refused sprite never becomes a placeholder.
    #[test]
    fn missing_animated_oversized_and_over_budget_inputs_are_refused() {
        let png = rgba_png(2, 2, &[1, 2, 3, 255].repeat(4));

        let mut files = assets(&[("minecraft:block/static", png.clone())]);
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/absent".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::MissingBytes(_))
        ));

        files.insert(
            "assets/minecraft/textures/block/static.png.mcmeta".to_owned(),
            br#"{"animation":{"frametime":2}}"#.to_vec(),
        );
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/static".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Unsupported(message)) if message.contains("animated")
        ));
        files.insert(
            "assets/minecraft/textures/block/static.png.mcmeta".to_owned(),
            br#"{"texture":{"blur":true}}"#.to_vec(),
        );
        build_static_atlas(
            &files,
            &["minecraft:block/static".to_owned()],
            AtlasLimits::default(),
        )
        .unwrap();
        files.insert(
            "assets/minecraft/textures/block/static.png.mcmeta".to_owned(),
            b"not json".to_vec(),
        );
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/static".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Unsupported(_))
        ));

        let apng = apng_with_one_frame(&png);
        let files = assets(&[("minecraft:block/animated", apng)]);
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/animated".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Unsupported(message)) if message.contains("APNG")
        ));

        let sixteen_bit = encode(
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            None,
            None,
            &[0x12, 0x34],
        );
        let files = assets(&[("minecraft:block/deep", sixteen_bit)]);
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/deep".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Unsupported(message)) if message.contains("16-bit")
        ));

        let mut huge = BTreeMap::new();
        huge.insert(
            sprite_path("minecraft:block/huge"),
            vec![0u8; MAX_SOURCE_BYTES + 1],
        );
        assert!(matches!(
            build_static_atlas(
                &huge,
                &["minecraft:block/huge".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Limit(_))
        ));

        let wide = encode(
            4097,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            None,
            None,
            &[0u8; 4097 * 3],
        );
        let files = assets(&[("minecraft:block/wide", wide)]);
        assert!(matches!(
            build_static_atlas(
                &files,
                &["minecraft:block/wide".to_owned()],
                AtlasLimits::default()
            ),
            Err(AtlasError::Limit(_))
        ));

        let files = assets(&[
            ("minecraft:block/one", png.clone()),
            ("minecraft:block/two", png.clone()),
        ]);
        assert!(matches!(
            build_static_atlas(
                &files,
                &[
                    "minecraft:block/one".to_owned(),
                    "minecraft:block/two".to_owned()
                ],
                AtlasLimits {
                    max_sprites: 1,
                    ..AtlasLimits::default()
                }
            ),
            Err(AtlasError::Limit(_))
        ));
        assert!(matches!(
            build_static_atlas(
                &files,
                &["block/one".to_owned()],
                AtlasLimits {
                    max_dimension: 4,
                    ..AtlasLimits::default()
                }
            ),
            Err(AtlasError::Limit(_))
        ));
        assert!(matches!(
            build_static_atlas(
                &files,
                &["block/one".to_owned()],
                AtlasLimits {
                    max_decoded_bytes: 8,
                    ..AtlasLimits::default()
                }
            ),
            Err(AtlasError::Limit(_))
        ));
        assert!(matches!(
            build_static_atlas(
                &files,
                &["block/one".to_owned()],
                AtlasLimits {
                    max_dimension: MAX_SOURCE_DIMENSION + 1,
                    ..AtlasLimits::default()
                }
            ),
            Err(AtlasError::Limit(_))
        ));
    }

    /// Sprite ids are canonical resource ids; uppercase, traversal and empty
    /// segments are refused before any bytes are read.
    #[test]
    fn unsafe_sprite_ids_are_refused() {
        let files = assets(&[("minecraft:block/ok", rgba_png(1, 1, &[1, 2, 3, 255]))]);
        for id in [
            "",
            "Minecraft:block/ok",
            "minecraft:Block/ok",
            "minecraft:",
            ":block/ok",
            "minecraft:/block/ok",
            "minecraft:block/",
            "minecraft:block//ok",
            "minecraft:block/./ok",
            "minecraft:block/../ok",
            "minecraft:block\\ok",
            "../block/ok",
        ] {
            assert!(
                matches!(
                    build_static_atlas(&files, &[id.to_owned()], AtlasLimits::default()),
                    Err(AtlasError::InvalidId(_))
                ),
                "id {id:?} was accepted"
            );
        }
        let long = "block/".to_owned() + &"a".repeat(MAX_PATH_LEN);
        assert!(matches!(
            build_static_atlas(
                &files,
                &[format!("minecraft:{long}")],
                AtlasLimits::default()
            ),
            Err(AtlasError::InvalidId(_))
        ));
    }

    /// An empty request is an empty one-pixel atlas, not a placeholder texture.
    #[test]
    fn an_empty_request_has_no_sprites_and_no_placeholders() {
        let files = BTreeMap::new();
        let atlas = build_static_atlas(&files, &[], AtlasLimits::default()).unwrap();
        assert_eq!((atlas.width, atlas.height), (1, 1));
        assert_eq!(atlas.rgba, [0, 0, 0, 0]);
        assert!(atlas.sprites.is_empty());
    }

    /// Build a nominal APNG by inserting an `acTL` chunk after `IHDR`.
    fn apng_with_one_frame(png: &[u8]) -> Vec<u8> {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let ihdr_length = u32::from_be_bytes(png[8..12].try_into().unwrap()) as usize;
        let split = 8 + 4 + 4 + ihdr_length + 4;
        let mut out = Vec::from(&png[..split]);
        let mut actl = Vec::new();
        actl.extend_from_slice(b"acTL");
        actl.extend_from_slice(&1u32.to_be_bytes());
        actl.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&8u32.to_be_bytes());
        out.extend_from_slice(&actl);
        out.extend_from_slice(&crc32(&actl).to_be_bytes());
        out.extend_from_slice(&png[split..]);
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
