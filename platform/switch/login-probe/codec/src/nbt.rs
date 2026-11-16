//! Bounded structural NBT scanning and borrowed views for Java's network form.
//!
//! The network root is `NbtIo.readAnyTag`: one tag type byte and that tag's
//! payload, with **no** file root name. [`skip_unnamed`] consumes exactly one
//! such value and reports how many bytes it used, so a caller can hand the
//! rest of a packet to the next field. [`RootView`] borrows the same value and
//! exposes its root compound's direct members.
//!
//! This module only *walks* bytes. It never builds a tree, never allocates and
//! never interprets text: names and string values stay raw Java modified UTF-8
//! bytes, so the overlong NUL (`c0 80`), unpaired surrogates and any other
//! sequence Java accepts survive untouched. Floats and doubles are skipped as
//! their IEEE bits, never converted or compared.
//!
//! All work is iterative over an explicit 128-entry stack and bounded by
//! [`Limits`], so no input can cause unbounded recursion, allocation or work
//! proportional to an array's declared size.
//!
//! ```
//! # use craftrustnx_login_codec::nbt::{skip_unnamed, Limits, RootView};
//! // Compound {"min_y": -64} in the network form: no root name.
//! let bytes = [0x0a, 0x03, 0x00, 0x05, b'm', b'i', b'n', b'_', b'y',
//!              0xff, 0xff, 0xff, 0xc0, 0x00];
//! assert_eq!(skip_unnamed(&bytes, Limits::default()), Ok(bytes.len()));
//! let view = RootView::parse(&bytes, Limits::default()).unwrap();
//! assert_eq!(view.find_i32(b"min_y"), Some(-64));
//! ```

/// Largest number of input bytes one value may consume: 8 MiB.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Deepest container nesting accepted: the root container is at depth 0.
pub const MAX_DEPTH: usize = 128;
/// Most tags one value may hold, list elements counted individually: 65,536.
pub const MAX_TAGS: usize = 65_536;

/// `End`: a compound's terminator, or an absent tag as a network root.
pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

/// What one scan may spend. Every field is clamped down to the hard caps
/// ([`MAX_BYTES`], [`MAX_DEPTH`], [`MAX_TAGS`]) by [`Limits::clamped`]; a
/// caller may ask for less, never for more.
///
/// `max_bytes` counts the bytes the value consumes from the start of the
/// supplied slice. Bytes after the value (the rest of a packet) are not
/// charged, so a small value inside a large packet is never rejected for the
/// packet's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_tags: usize,
}

impl Limits {
    /// The hard caps: 8 MiB, depth 128, 65,536 tags.
    pub fn clamped(self) -> Self {
        Self {
            max_bytes: self.max_bytes.min(MAX_BYTES),
            max_depth: self.max_depth.min(MAX_DEPTH),
            max_tags: self.max_tags.min(MAX_TAGS),
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_BYTES,
            max_depth: MAX_DEPTH,
            max_tags: MAX_TAGS,
        }
    }
}

/// Why a value was rejected.
///
/// The three cases are deliberately separate: `Truncated` means bytes were
/// missing, `Invalid` means the bytes present cannot be a well-formed tag, and
/// `Limit` means a well-formed prefix exhausted a caller budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NbtError {
    /// The value runs past the supplied bytes.
    Truncated { offset: usize },
    /// Not a well-formed tag: unknown type, negative collection length, a
    /// non-empty list without an element type, or a length arithmetic
    /// overflow.
    Invalid { offset: usize },
    /// A caller limit ([`Limits`]) is exhausted.
    Limit { offset: usize },
}

impl NbtError {
    /// Input offset the scan was at when it rejected the value.
    pub fn offset(&self) -> usize {
        match *self {
            Self::Truncated { offset } | Self::Invalid { offset } | Self::Limit { offset } => {
                offset
            }
        }
    }
}

/// Consumes exactly one network root tag — its type byte and payload, but no
/// root name — and returns how many leading bytes it used. Bytes after the
/// value are left untouched for the caller.
///
/// A root `End` tag is one byte. Duplicate members, nested names and every
/// string's bytes are not interpreted: the scan is structural only.
pub fn skip_unnamed(input: &[u8], limits: Limits) -> Result<usize, NbtError> {
    let mut cursor = Cursor::new(input, limits.clamped());
    cursor.scan_root()?;
    Ok(cursor.pos)
}

/// One direct member of a root compound.
#[derive(Clone, Copy, Debug)]
pub struct Field<'a> {
    /// The member's name exactly as the wire carries it, without a terminator
    /// and without any decoding.
    pub name: &'a [u8],
    /// The member's value.
    pub value: TagView<'a>,
}

/// A structurally validated tag's type and raw payload bytes borrowed from the
/// input: no type byte, no name. The payload's extent is exactly this value,
/// never a sibling's or a nested value's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagView<'a> {
    type_id: u8,
    payload: &'a [u8],
}

impl<'a> TagView<'a> {
    /// The tag type id the wire carries (1..=12; never `End`).
    pub fn type_id(&self) -> u8 {
        self.type_id
    }
    /// The raw payload, validated to lie inside the input slice.
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }
    /// A [`TAG_BYTE`] member's value; `None` for any other type.
    pub fn as_byte(&self) -> Option<i8> {
        if self.type_id != TAG_BYTE {
            return None;
        }
        self.payload.first().map(|&byte| byte as i8)
    }
    /// A [`TAG_INT`] member's value, big-endian on the wire; `None` otherwise.
    pub fn as_i32(&self) -> Option<i32> {
        if self.type_id != TAG_INT {
            return None;
        }
        let bytes: [u8; 4] = self.payload.get(..4)?.try_into().ok()?;
        Some(i32::from_be_bytes(bytes))
    }
}

/// A validated network root tag, borrowed from the caller's bytes.
///
/// Only direct members of a root compound are ever yielded or matched: a
/// nested member with the same name is never mistaken for a root member.
/// Iteration keeps duplicates in wire order; [`RootView::find`] resolves them
/// the way vanilla's map does, last one wins.
#[derive(Clone, Copy, Debug)]
pub struct RootView<'a> {
    input: &'a [u8],
    root_type: u8,
    body: usize,
    consumed: usize,
    limits: Limits,
}

impl<'a> RootView<'a> {
    /// Validates one complete network root tag (no name) inside `input` and
    /// borrows it. The whole value is validated, so every later view is inside
    /// the slice.
    pub fn parse(input: &'a [u8], limits: Limits) -> Result<Self, NbtError> {
        let limits = limits.clamped();
        let mut cursor = Cursor::new(input, limits);
        let root_type = cursor.scan_root()?;
        Ok(Self {
            input,
            root_type,
            body: 1,
            consumed: cursor.pos,
            limits,
        })
    }
    /// The root tag's type id (`End` for an empty root).
    pub fn root_type(&self) -> u8 {
        self.root_type
    }
    /// Whether the root is a compound, and therefore has members.
    pub fn is_compound(&self) -> bool {
        self.root_type == TAG_COMPOUND
    }
    /// Bytes the root value occupied; trailing packet bytes are not included.
    pub fn consumed(&self) -> usize {
        self.consumed
    }
    /// The root value's exact bytes, trailing packet data excluded.
    pub fn raw(&self) -> &'a [u8] {
        &self.input[..self.consumed]
    }
    /// The root compound's direct members in wire order, duplicates included.
    /// Empty for any other root type.
    pub fn fields(&self) -> Fields<'a> {
        Fields {
            input: self.input,
            pos: (self.root_type == TAG_COMPOUND).then_some(self.body),
            limits: self.limits,
        }
    }
    /// The last direct member named `name`, exactly as vanilla's map keeps a
    /// duplicate: `None` when no direct member has that name.
    pub fn find(&self, name: &[u8]) -> Option<TagView<'a>> {
        self.fields()
            .filter(|field| field.name == name)
            .last()
            .map(|field| field.value)
    }
    /// [`RootView::find`] as a [`TAG_INT`].
    pub fn find_i32(&self, name: &[u8]) -> Option<i32> {
        self.find(name)?.as_i32()
    }
    /// [`RootView::find`] as a [`TAG_BYTE`].
    pub fn find_byte(&self, name: &[u8]) -> Option<i8> {
        self.find(name)?.as_byte()
    }
}

/// The direct members of a validated root compound.
///
/// It re-walks only bytes [`RootView::parse`] already accepted with the same
/// limits, so a member cannot fail to be read; the tail of a packet is never
/// touched.
#[derive(Clone, Debug)]
pub struct Fields<'a> {
    input: &'a [u8],
    /// Offset of the next member, or `None` past the root's `End`.
    pos: Option<usize>,
    limits: Limits,
}

impl<'a> Iterator for Fields<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Field<'a>> {
        let start = self.pos?;
        let mut cursor = Cursor::at(self.input, start, self.limits);
        let id = cursor.u8().ok()?;
        if id == TAG_END {
            self.pos = None;
            return None;
        }
        let name_len = cursor.string_len().ok()?;
        let name = cursor.take(name_len).ok()?;
        let value_start = cursor.pos;
        cursor.scan_value(id, 1).ok()?;
        let payload = self.input.get(value_start..cursor.pos)?;
        self.pos = Some(cursor.pos);
        Some(Field {
            name,
            value: TagView {
                type_id: id,
                payload,
            },
        })
    }
}

/// One entry of the explicit nesting stack.
#[derive(Clone, Copy)]
enum Frame {
    /// Inside a compound payload: the next byte is a member type or `End`.
    Compound,
    /// Inside a list payload: `left` elements still to read.
    List { elem: u8, left: u32 },
}

/// Width in bytes of a fixed-width scalar payload (types 1..=6).
fn scalar_width(id: u8) -> Option<usize> {
    match id {
        TAG_BYTE => Some(1),
        TAG_SHORT => Some(2),
        TAG_INT => Some(4),
        TAG_LONG => Some(8),
        TAG_FLOAT => Some(4),
        TAG_DOUBLE => Some(8),
        _ => None,
    }
}

/// A read position with the caller's budgets.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    limits: Limits,
    visited: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], limits: Limits) -> Self {
        Self::at(bytes, 0, limits)
    }
    fn at(bytes: &'a [u8], pos: usize, limits: Limits) -> Self {
        Self {
            bytes,
            pos,
            limits,
            visited: 0,
        }
    }
    fn invalid(&self) -> NbtError {
        NbtError::Invalid { offset: self.pos }
    }
    fn limit(&self) -> NbtError {
        NbtError::Limit { offset: self.pos }
    }
    /// The next `len` bytes, bounds-checked and charged to `max_bytes`.
    fn take(&mut self, len: usize) -> Result<&'a [u8], NbtError> {
        let end = self.pos.checked_add(len).ok_or_else(|| self.invalid())?;
        if end > self.bytes.len() {
            return Err(NbtError::Truncated { offset: self.pos });
        }
        if end > self.limits.max_bytes {
            return Err(self.limit());
        }
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }
    fn be_i32(&mut self) -> Result<i32, NbtError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn string_len(&mut self) -> Result<usize, NbtError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]) as usize)
    }
    /// A collection's signed big-endian length times `width`, as bytes to skip.
    /// A negative length is malformed; an overflowing one cannot be skipped.
    fn length(&mut self, width: usize) -> Result<usize, NbtError> {
        let count = self.be_i32()?;
        if count < 0 {
            return Err(self.invalid());
        }
        (count as usize)
            .checked_mul(width)
            .ok_or_else(|| self.invalid())
    }
    /// Charges one tag to the tag budget and rejects a type that is not a value.
    fn entry(&mut self, id: u8) -> Result<(), NbtError> {
        if id == TAG_END || id > TAG_LONG_ARRAY {
            return Err(self.invalid());
        }
        if self.visited >= self.limits.max_tags {
            return Err(self.limit());
        }
        self.visited += 1;
        Ok(())
    }
    /// Charges `count` tags at once, before any of them is walked.
    fn reserve(&mut self, count: usize) -> Result<(), NbtError> {
        if count > self.limits.max_tags.saturating_sub(self.visited) {
            return Err(self.limit());
        }
        self.visited += count;
        Ok(())
    }
    /// A container at absolute depth `base_depth + depth` may have children, so
    /// it must be shallower than `max_depth` and fit the fixed stack.
    fn descend(&self, base_depth: usize, depth: usize) -> Result<(), NbtError> {
        if depth >= MAX_DEPTH || base_depth.saturating_add(depth) >= self.limits.max_depth {
            return Err(self.limit());
        }
        Ok(())
    }
    /// The next value's type id: a list's remaining element or a compound's
    /// next member type, closing finished frames on the way. `None` finishes
    /// the value.
    fn next_from_stack(
        &mut self,
        stack: &mut [Frame; MAX_DEPTH],
        sp: &mut usize,
    ) -> Result<Option<u8>, NbtError> {
        loop {
            let Some(top) = sp.checked_sub(1) else {
                return Ok(None);
            };
            match &mut stack[top] {
                Frame::List { elem, left } => {
                    if *left == 0 {
                        *sp = top;
                        continue;
                    }
                    *left -= 1;
                    return Ok(Some(*elem));
                }
                Frame::Compound => {
                    let id = self.u8()?;
                    if id == TAG_END {
                        *sp = top;
                        continue;
                    }
                    self.entry(id)?;
                    // The name is opaque: its length is checked, its bytes are not.
                    let name = self.string_len()?;
                    self.take(name)?;
                    return Ok(Some(id));
                }
            }
        }
    }
    /// Scans one network root value (type byte and payload, no name).
    fn scan_root(&mut self) -> Result<u8, NbtError> {
        let id = self.u8()?;
        if id == TAG_END {
            return Ok(TAG_END);
        }
        self.scan_value(id, 0)?;
        Ok(id)
    }
    /// Scans one value whose type byte has already been read. `base_depth` is
    /// the depth of the value itself; a root value is at depth 0.
    fn scan_value(&mut self, id: u8, base_depth: usize) -> Result<(), NbtError> {
        self.entry(id)?;
        let mut stack = [Frame::Compound; MAX_DEPTH];
        let mut sp = 0usize;
        let mut pending = Some(id);
        while let Some(value_id) = pending {
            match value_id {
                TAG_BYTE => {
                    self.take(1)?;
                }
                TAG_SHORT => {
                    self.take(2)?;
                }
                TAG_INT => {
                    self.take(4)?;
                }
                TAG_LONG => {
                    self.take(8)?;
                }
                TAG_FLOAT => {
                    self.take(4)?;
                }
                TAG_DOUBLE => {
                    self.take(8)?;
                }
                TAG_BYTE_ARRAY => {
                    let bytes = self.length(1)?;
                    self.take(bytes)?;
                }
                TAG_STRING => {
                    let bytes = self.string_len()?;
                    self.take(bytes)?;
                }
                TAG_LIST => {
                    let elem = self.u8()?;
                    let count = self.be_i32()?;
                    if count < 0 {
                        return Err(self.invalid());
                    }
                    let count = count as u32;
                    // Vanilla only requires an element type when there are
                    // elements, so an empty list may name any byte.
                    if count > 0 && (elem == TAG_END || elem > TAG_LONG_ARRAY) {
                        return Err(self.invalid());
                    }
                    self.reserve(count as usize)?;
                    // A list is a container at its own depth whether or not it
                    // holds elements, exactly as vanilla's depth accounting counts it.
                    self.descend(base_depth, sp)?;
                    if count > 0 {
                        match elem {
                            // A fixed-width primitive list is skipped in one
                            // bound-checked step, never element by element.
                            1..=6 => {
                                let width = scalar_width(elem).ok_or_else(|| self.invalid())?;
                                let bytes = (count as usize)
                                    .checked_mul(width)
                                    .ok_or_else(|| self.invalid())?;
                                self.take(bytes)?;
                            }
                            7 | 11 | 12 => {
                                let width = match elem {
                                    TAG_BYTE_ARRAY => 1,
                                    TAG_INT_ARRAY => 4,
                                    _ => 8,
                                };
                                for _ in 0..count {
                                    let bytes = self.length(width)?;
                                    self.take(bytes)?;
                                }
                            }
                            TAG_STRING => {
                                for _ in 0..count {
                                    let bytes = self.string_len()?;
                                    self.take(bytes)?;
                                }
                            }
                            _ => {
                                stack[sp] = Frame::List { elem, left: count };
                                sp += 1;
                            }
                        }
                    }
                }
                TAG_COMPOUND => {
                    self.descend(base_depth, sp)?;
                    stack[sp] = Frame::Compound;
                    sp += 1;
                }
                TAG_INT_ARRAY => {
                    let bytes = self.length(4)?;
                    self.take(bytes)?;
                }
                TAG_LONG_ARRAY => {
                    let bytes = self.length(8)?;
                    self.take(bytes)?;
                }
                _ => return Err(self.invalid()),
            }
            pending = self.next_from_stack(&mut stack, &mut sp)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// A root compound: `has_skylight` byte 1, `min_y` int -64, `height` 384.
    fn dimension_root() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(0x0a);
        bytes.extend_from_slice(&[0x01, 0x00, 0x0c]);
        bytes.extend_from_slice(b"has_skylight");
        bytes.push(0x01);
        bytes.extend_from_slice(&[0x03, 0x00, 0x05]);
        bytes.extend_from_slice(b"min_y");
        bytes.extend_from_slice(&[0xff, 0xff, 0xff, 0xc0]);
        bytes.extend_from_slice(&[0x03, 0x00, 0x06]);
        bytes.extend_from_slice(b"height");
        bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0x80]);
        bytes.push(0x00);
        bytes
    }

    /// `levels` compounds, each holding the next as its only member.
    fn nested_compounds(levels: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(TAG_COMPOUND);
        for _ in 1..levels {
            bytes.extend_from_slice(&[TAG_COMPOUND, 0x00, 0x00]);
        }
        bytes.extend(core::iter::repeat_n(TAG_END, levels));
        bytes
    }

    fn lcg(state: &mut u64) -> u8 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*state >> 33) as u8
    }

    #[test]
    fn skip_consumes_exactly_one_value_of_each_type() {
        // Hand-assembled network values: type byte, then the payload only.
        let cases: &[(&[u8], usize)] = &[
            (&[0x01, 0x80][..], 2),                                      // Byte -128
            (&[0x02, 0x12, 0x34][..], 3),                                // Short 4660
            (&[0x03, 0xff, 0xff, 0xff, 0xc0][..], 5),                    // Int -64
            (&[0x04, 0, 0, 0, 0, 0, 0, 0, 1][..], 9),                    // Long 1
            (&[0x05, 0x3f, 0x80, 0x00, 0x00][..], 5),                    // Float 1.0
            (&[0x06, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0][..], 9),              // Double 1.0
            (&[0x07, 0, 0, 0, 3, 1, 2, 3][..], 8),                       // [1,2,3]
            (&[0x08, 0, 2, b'h', b'i'][..], 5),                          // "hi"
            (&[0x09, 0x03, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2][..], 14), // <Int>[1,2]
            (&[0x0a, 0x03, 0, 2, b'h', b'i', 0, 0, 0, 7, 0][..], 11),    // {"hi":7}
            (&[0x0b, 0, 0, 0, 1, 0, 0, 0, 7][..], 9),                    // [7]
            (&[0x0c, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5][..], 13),       // [5L]
            (&[0x09, 0x03, 0, 0, 0, 0][..], 6),                          // empty list
            (&[0x09, 0xc8, 0, 0, 0, 0][..], 6),                          // empty list, unknown type
        ];
        for &(bytes, used) in cases {
            assert_eq!(
                skip_unnamed(bytes, Limits::default()),
                Ok(used),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn trailing_packet_bytes_are_left_alone() {
        let mut packet = dimension_root();
        let value_len = packet.len();
        // A well-formed member shape after the root: reading it would be a bug.
        packet.extend_from_slice(&[0x03, 0x00, 0x01, b'x', 0x00, 0x00, 0x00, 0x09]);
        assert_eq!(skip_unnamed(&packet, Limits::default()), Ok(value_len));
        let view = RootView::parse(&packet, Limits::default()).unwrap();
        assert_eq!(view.consumed(), value_len);
        assert_eq!(view.raw(), &packet[..value_len]);
        assert_eq!(view.fields().count(), 3);
        assert_eq!(view.find(b"x"), None);
        assert_eq!(view.find_i32(b"x"), None);
    }

    #[test]
    fn a_root_end_tag_is_one_byte() {
        assert_eq!(skip_unnamed(&[0x00], Limits::default()), Ok(1));
        assert_eq!(
            skip_unnamed(&[0x00, 0x03, 0x00, 0x00, 0x00, 0x07], Limits::default()),
            Ok(1)
        );
        let view = RootView::parse(&[0x00], Limits::default()).unwrap();
        assert_eq!(view.root_type(), TAG_END);
        assert!(!view.is_compound());
        assert_eq!(view.consumed(), 1);
        assert_eq!(view.fields().count(), 0);
        assert_eq!(view.find(b"min_y"), None);
    }

    #[test]
    fn every_strict_prefix_is_truncated() {
        let vectors = [
            dimension_root(),
            nested_compounds(3),
            Vec::from(&[0x09u8, 0x0a, 0, 0, 0, 2, 0x00, 0x00][..]),
            Vec::from(
                &[
                    0x0c, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2,
                ][..],
            ),
        ];
        for bytes in &vectors {
            for end in 0..bytes.len() {
                assert!(
                    matches!(
                        skip_unnamed(&bytes[..end], Limits::default()),
                        Err(NbtError::Truncated { .. })
                    ),
                    "prefix {end} of {bytes:02x?}"
                );
            }
        }
    }

    #[test]
    fn negative_lengths_are_invalid() {
        for bytes in [
            &[0x07, 0xff, 0xff, 0xff, 0xff][..],
            &[0x0b, 0xff, 0xff, 0xff, 0xff][..],
            &[0x0c, 0xff, 0xff, 0xff, 0xff][..],
        ] {
            assert_eq!(
                skip_unnamed(bytes, Limits::default()),
                Err(NbtError::Invalid { offset: 5 }),
                "{bytes:02x?}"
            );
        }
        // A list's negative count is malformed too, after its element type.
        assert_eq!(
            skip_unnamed(&[0x09, 0x01, 0xff, 0xff, 0xff, 0xff], Limits::default()),
            Err(NbtError::Invalid { offset: 6 })
        );
    }

    #[test]
    fn unknown_types_and_bad_list_headers_are_invalid() {
        assert_eq!(
            skip_unnamed(&[0x0d], Limits::default()),
            Err(NbtError::Invalid { offset: 1 })
        );
        assert_eq!(
            skip_unnamed(&[0xff], Limits::default()),
            Err(NbtError::Invalid { offset: 1 })
        );
        // A non-empty list must name a real element type; End is not one.
        assert_eq!(
            skip_unnamed(&[0x09, 0x00, 0, 0, 0, 1], Limits::default()),
            Err(NbtError::Invalid { offset: 6 })
        );
        assert_eq!(
            skip_unnamed(&[0x09, 0x0d, 0, 0, 0, 1], Limits::default()),
            Err(NbtError::Invalid { offset: 6 })
        );
    }

    #[test]
    fn huge_counts_are_refused_before_any_loop() {
        // A list of i32::MAX ints cannot fit the tag budget.
        assert_eq!(
            skip_unnamed(&[0x09, 0x01, 0x7f, 0xff, 0xff, 0xff], Limits::default()),
            Err(NbtError::Limit { offset: 6 })
        );
        // An array whose declared size is not present is truncated, not walked.
        assert_eq!(
            skip_unnamed(&[0x07, 0x7f, 0xff, 0xff, 0xff, 0, 0], Limits::default()),
            Err(NbtError::Truncated { offset: 5 })
        );
        assert_eq!(
            skip_unnamed(&[0x0c, 0x7f, 0xff, 0xff, 0xff], Limits::default()),
            Err(NbtError::Truncated { offset: 5 })
        );
    }

    #[test]
    fn byte_cap_counts_consumed_bytes_only() {
        let bytes = dimension_root();
        let len = bytes.len();
        assert_eq!(skip_unnamed(&bytes, Limits::default()), Ok(len));
        let exact = Limits {
            max_bytes: len,
            ..Limits::default()
        };
        assert_eq!(skip_unnamed(&bytes, exact), Ok(len));
        for cap in [0, 1, 5, len - 1] {
            let limits = Limits {
                max_bytes: cap,
                ..Limits::default()
            };
            assert!(
                matches!(skip_unnamed(&bytes, limits), Err(NbtError::Limit { .. })),
                "cap {cap}"
            );
        }
    }

    #[test]
    fn tag_cap_counts_list_elements_individually() {
        let ints = [0x09u8, 0x03, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        assert_eq!(skip_unnamed(&ints, Limits::default()), Ok(ints.len()));
        let four = Limits {
            max_tags: 4,
            ..Limits::default()
        };
        assert_eq!(skip_unnamed(&ints, four), Ok(ints.len()));
        let three = Limits {
            max_tags: 3,
            ..Limits::default()
        };
        assert_eq!(
            skip_unnamed(&ints, three),
            Err(NbtError::Limit { offset: 6 })
        );
        // Two members plus their compound is three tags.
        let bytes = dimension_root();
        let two = Limits {
            max_tags: 2,
            ..Limits::default()
        };
        assert!(matches!(
            skip_unnamed(&bytes, two),
            Err(NbtError::Limit { .. })
        ));
    }

    #[test]
    fn depth_cap_is_the_vanilla_depth_of_containers() {
        let deepest = nested_compounds(MAX_DEPTH);
        assert_eq!(
            skip_unnamed(&deepest, Limits::default()),
            Ok(4 * MAX_DEPTH - 2)
        );
        let over = nested_compounds(MAX_DEPTH + 1);
        assert_eq!(
            skip_unnamed(&over, Limits::default()),
            Err(NbtError::Limit { offset: 385 })
        );
        let shallow = Limits {
            max_depth: 2,
            ..Limits::default()
        };
        assert_eq!(
            skip_unnamed(&nested_compounds(2), shallow),
            Ok(6),
            "two containers are depths 0 and 1"
        );
        assert_eq!(
            skip_unnamed(&nested_compounds(3), shallow),
            Err(NbtError::Limit { offset: 7 })
        );
    }

    #[test]
    fn limits_are_clamped_to_the_hard_caps() {
        assert_eq!(Limits::default().clamped(), Limits::default());
        assert_eq!(
            Limits {
                max_bytes: usize::MAX,
                max_depth: 10_000,
                max_tags: usize::MAX,
            }
            .clamped(),
            Limits::default()
        );
    }

    #[test]
    fn views_expose_direct_members_with_big_endian_scalars() {
        let bytes = dimension_root();
        let view = RootView::parse(&bytes, Limits::default()).unwrap();
        assert!(view.is_compound());
        assert_eq!(view.root_type(), TAG_COMPOUND);
        assert_eq!(view.consumed(), bytes.len());
        assert_eq!(view.raw(), &bytes[..]);

        let fields: Vec<_> = view.fields().collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, b"has_skylight");
        assert_eq!(fields[0].value.type_id(), TAG_BYTE);
        assert_eq!(fields[1].name, b"min_y");
        assert_eq!(fields[1].value.type_id(), TAG_INT);
        assert_eq!(fields[1].value.payload(), &[0xff, 0xff, 0xff, 0xc0]);
        assert_eq!(fields[2].name, b"height");
        assert_eq!(fields[2].value.payload(), &[0x00, 0x00, 0x01, 0x80]);

        assert_eq!(view.find_i32(b"min_y"), Some(-64));
        assert_eq!(view.find_i32(b"height"), Some(384));
        assert_eq!(view.find_byte(b"has_skylight"), Some(1));
        assert_eq!(view.find_i32(b"has_skylight"), None, "wrong type");
        assert_eq!(view.find_byte(b"min_y"), None, "wrong type");
        assert_eq!(view.find(b"nope"), None);
    }

    #[test]
    fn duplicates_keep_wire_order_and_lookup_is_last_wins() {
        let bytes: &[u8] = &[
            0x0a, 0x03, 0x00, 0x01, b'a', 0, 0, 0, 1, 0x03, 0x00, 0x01, b'b', 0, 0, 0, 2, 0x03,
            0x00, 0x01, b'a', 0, 0, 0, 3, 0x00,
        ];
        let view = RootView::parse(bytes, Limits::default()).unwrap();
        let fields: Vec<_> = view.fields().collect();
        let names: Vec<_> = fields.iter().map(|field| field.name).collect();
        assert_eq!(names, [&b"a"[..], &b"b"[..], &b"a"[..]]);
        let values: Vec<_> = fields
            .iter()
            .map(|field| field.value.as_i32().unwrap())
            .collect();
        assert_eq!(values, [1, 2, 3]);
        assert_eq!(view.find_i32(b"a"), Some(3), "the map keeps the last");
        assert_eq!(view.find_i32(b"b"), Some(2));
    }

    #[test]
    fn nested_names_are_never_mistaken_for_root_members() {
        let bytes: &[u8] = &[
            0x0a, 0x0a, 0x00, 0x05, b'o', b'u', b't', b'e', b'r', 0x03, 0x00, 0x05, b'm', b'i',
            b'n', b'_', b'y', 0xff, 0xff, 0xff, 0xc0, 0x00, 0x00,
        ];
        let view = RootView::parse(bytes, Limits::default()).unwrap();
        assert_eq!(view.find_i32(b"min_y"), None);
        let outer = view.find(b"outer").unwrap();
        assert_eq!(outer.type_id(), TAG_COMPOUND);
        assert_eq!(
            outer.payload(),
            &[0x03, 0x00, 0x05, b'm', b'i', b'n', b'_', b'y', 0xff, 0xff, 0xff, 0xc0, 0x00]
        );
        assert_eq!(outer.as_i32(), None);
    }

    #[test]
    fn names_and_strings_stay_raw_modified_utf8() {
        // Overlong NUL in a name, a NUL byte name and an unpaired surrogate
        // string: all preserved byte for byte, none decoded or rejected.
        let bytes: &[u8] = &[
            0x0a, 0x01, 0x00, 0x02, 0xc0, 0x80, 0x01, 0x01, 0x00, 0x01, 0x00, 0x02, 0x08, 0x00,
            0x01, b's', 0x00, 0x03, 0xed, 0xa0, 0x80, 0x00,
        ];
        assert_eq!(skip_unnamed(bytes, Limits::default()), Ok(bytes.len()));
        let view = RootView::parse(bytes, Limits::default()).unwrap();
        let fields: Vec<_> = view.fields().collect();
        assert_eq!(fields[0].name, &[0xc0, 0x80]);
        assert_eq!(fields[0].value.as_byte(), Some(1));
        assert_eq!(view.find_byte(&[0x00]), Some(2));
        assert_eq!(view.find_byte(&[0xc0, 0x80]), Some(1));
        let text = view.find(b"s").unwrap();
        assert_eq!(text.type_id(), TAG_STRING);
        assert_eq!(text.payload(), &[0x00, 0x03, 0xed, 0xa0, 0x80]);
    }

    #[test]
    fn a_non_compound_root_has_no_members() {
        let int = [0x03, 0, 0, 0, 7];
        let view = RootView::parse(&int, Limits::default()).unwrap();
        assert_eq!(view.root_type(), TAG_INT);
        assert!(!view.is_compound());
        assert_eq!(view.consumed(), 5);
        assert_eq!(view.fields().count(), 0);
        assert_eq!(view.find(b"min_y"), None);

        let empty = [0x0a, 0x00];
        let view = RootView::parse(&empty, Limits::default()).unwrap();
        assert!(view.is_compound());
        assert_eq!(view.fields().count(), 0);
    }

    #[test]
    fn lists_of_arrays_strings_and_containers_are_scanned_structurally() {
        let cases: &[(&[u8], usize)] = &[
            // <List> of two empty int lists.
            (
                &[0x09, 0x09, 0, 0, 0, 2, 3, 0, 0, 0, 0, 3, 0, 0, 0, 0][..],
                16,
            ),
            // <ByteArray> of [5] and [7,8].
            (
                &[0x09, 0x07, 0, 0, 0, 2, 0, 0, 0, 1, 5, 0, 0, 0, 2, 7, 8][..],
                17,
            ),
            // <String> of "a" and "bc".
            (
                &[0x09, 0x08, 0, 0, 0, 2, 0, 1, b'a', 0, 2, b'b', b'c'][..],
                13,
            ),
            // <Compound> of two empty compounds.
            (&[0x09, 0x0a, 0, 0, 0, 2, 0x00, 0x00][..], 8),
            // <LongArray> of one [5].
            (
                &[0x09, 0x0c, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5][..],
                18,
            ),
        ];
        for &(bytes, used) in cases {
            assert_eq!(
                skip_unnamed(bytes, Limits::default()),
                Ok(used),
                "{bytes:02x?}"
            );
        }
        // The last one is one tag plus one element, not part of a bigger tree.
        let tags = Limits {
            max_tags: 1,
            ..Limits::default()
        };
        assert_eq!(
            skip_unnamed(&[0x09, 0x0a, 0, 0, 0, 2, 0x00, 0x00], tags),
            Err(NbtError::Limit { offset: 6 })
        );
    }

    #[test]
    fn mixed_list_wrappers_are_ordinary_compounds() {
        // 26.3's `List<Compound>` wrapper: each element is a one-member
        // compound with an empty name. Structurally it is plain NBT, so the
        // scanner adds no type and the payload keeps the wrapper bytes.
        let bytes: &[u8] = &[
            0x0a, 0x09, 0x00, 0x07, b'p', b'a', b'l', b'e', b't', b't', b'e', 0x0a, 0x00, 0x00,
            0x00, 0x02, 0x08, 0x00, 0x00, 0x00, 0x03, b'A', b'B', b'C', 0x00, 0x08, 0x00, 0x00,
            0x00, 0x02, b'D', b'E', 0x00, 0x00,
        ];
        assert_eq!(skip_unnamed(bytes, Limits::default()), Ok(bytes.len()));
        let view = RootView::parse(bytes, Limits::default()).unwrap();
        let palette = view.find(b"palette").unwrap();
        assert_eq!(palette.type_id(), TAG_LIST);
        assert_eq!(palette.payload().len(), 22);
        assert_eq!(&palette.payload()[..5], &[0x0a, 0x00, 0x00, 0x00, 0x02]);
        assert_eq!(
            &palette.payload()[5..14],
            &[0x08, 0x00, 0x00, 0x00, 0x03, b'A', b'B', b'C', 0x00]
        );
    }

    #[test]
    fn malformed_input_never_panics() {
        let mut state = 0x1234_5678_9abc_def0u64;
        for len in 0..=96usize {
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(lcg(&mut state));
            }
            let _ = skip_unnamed(&bytes, Limits::default());
            if let Ok(view) = RootView::parse(&bytes, Limits::default()) {
                for field in view.fields() {
                    let _ = field.value.as_i32();
                    let _ = field.value.as_byte();
                }
                let _ = view.find(b"min_y");
            }
        }
        let base = dimension_root();
        for index in 0..base.len() {
            for bit in 0..8 {
                let mut bytes = base.clone();
                bytes[index] ^= 1 << bit;
                let _ = skip_unnamed(&bytes, Limits::default());
                if let Ok(view) = RootView::parse(&bytes, Limits::default()) {
                    for field in view.fields() {
                        let _ = field.value.payload().len();
                    }
                }
            }
        }
    }
}
