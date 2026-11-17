//! Selection of vanilla blockstate definitions.
//!
//! A definition is the JSON body of a block's `blockstates/<id>.json`. The
//! caller supplies the block's property map and receives one [`Part`] per
//! matching multipart entry (or exactly one for `variants`), each preserving all
//! weighted model alternatives in definition order.
//!
//! # Documented semantics and limits
//!
//! - `variants` keys are comma-separated `property=value` conjunctions.
//!   Multipart property values support `|` alternatives and `!` negation. The empty key matches every state. Exactly one
//!   variant key must match: zero matches and several matches are errors.
//! - `multipart` entries apply when `when` is absent or its condition holds.
//!   Conditions are `AND`/`OR` arrays (nesting at most
//!   [`MAX_CONDITION_DEPTH`]) or a plain object of property clauses combined
//!   with AND. An empty `AND` is true, an empty `OR` is false. Mixing `AND` or
//!   `OR` with other keys, or using both, is rejected. Zero matching entries is
//!   a valid, empty selection.
//! - Every definition is validated in full, including parts and variant keys
//!   that do not match the supplied state.
//! - Bounds: [`MAX_VARIANTS`] variant keys, [`MAX_PARTS`] multipart entries,
//!   [`MAX_OPTIONS_PER_PART`] model options per entry, and
//!   [`MAX_CONDITION_DEPTH`] nested condition operators.
//! - Property names and values are 1..=[`MAX_PROPERTY_NAME_LEN`] bytes, contain
//!   no whitespace or control characters, and no `,`, `|` or `=`.
//! - Model ids are normalized to `namespace:path`: a missing namespace becomes
//!   `minecraft`, namespaces are 1..=[`MAX_NAMESPACE_LEN`] bytes of
//!   `[a-z0-9_.-]`, paths are 1..=[`MAX_PATH_LEN`] bytes of `[a-z0-9/._-]`
//!   with no empty, `.` or `..` segment and no leading or trailing `/`.
//!   Backslashes, extra colons, absolute paths and uppercase characters are
//!   rejected.
//! - Model options accept only `model`, `x`, `y`, `uvlock` and `weight`.
//!   Rotations are 0, 90, 180 or 270; weights are positive `u32` values.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

/// Maximum number of keys in a `variants` map.
pub const MAX_VARIANTS: usize = 512;
/// Maximum number of entries in a `multipart` array.
pub const MAX_PARTS: usize = 512;
/// Maximum number of model options in one `apply` or variant value.
pub const MAX_OPTIONS_PER_PART: usize = 64;
/// Maximum nesting of `AND`/`OR` operators in one condition.
pub const MAX_CONDITION_DEPTH: usize = 32;
/// Maximum length of a raw model id in bytes.
pub const MAX_MODEL_ID_LEN: usize = 512;
/// Maximum length of a model namespace in bytes.
pub const MAX_NAMESPACE_LEN: usize = 64;
/// Maximum length of a model path in bytes.
pub const MAX_PATH_LEN: usize = 256;
/// Maximum length of a block property name in bytes.
pub const MAX_PROPERTY_NAME_LEN: usize = 64;
/// Maximum length of a block property value in bytes.
pub const MAX_PROPERTY_VALUE_LEN: usize = 64;

/// One multipart entry (or the single `variants` result) with all of its
/// weighted model alternatives, in definition order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Arrays consume a weighted RNG draw even when they contain one option.
    pub weighted: bool,
    /// Model alternatives; never empty, never randomly reduced.
    pub options: Vec<ModelOption>,
}

/// One model reference from a blockstate definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// Normalized `namespace:path` model id.
    pub model: String,
    /// Rotation around the x axis, in degrees.
    pub x: u16,
    /// Rotation around the y axis, in degrees.
    pub y: u16,
    /// Whether texture coordinates stay locked to the block.
    pub uvlock: bool,
    /// Positive selection weight; alternatives are all kept, never sampled.
    pub weight: u32,
}

/// Readable selection error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Select the model parts for one block state.
///
/// `properties` is the block's property map; unknown or missing properties make
/// a clause fail to match. See the module documentation for the accepted shape.
pub fn select(
    definition: &Value,
    properties: &BTreeMap<String, String>,
) -> Result<Vec<Part>, Error> {
    let definition = expect_object(definition, "definition")?;
    for key in definition.keys() {
        if key != "variants" && key != "multipart" {
            return Err(Error::new(format!(
                "definition: unknown top-level field `{key}`"
            )));
        }
    }
    match (definition.get("variants"), definition.get("multipart")) {
        (Some(variants), None) => select_variants(variants, properties),
        (None, Some(parts)) => select_multipart(parts, properties),
        (Some(_), Some(_)) => Err(Error::new(
            "definition: `variants` and `multipart` are mutually exclusive",
        )),
        (None, None) => Err(Error::new(
            "definition: expected either `variants` or `multipart`",
        )),
    }
}

fn select_variants(
    value: &Value,
    properties: &BTreeMap<String, String>,
) -> Result<Vec<Part>, Error> {
    let variants = expect_object(value, "variants")?;
    if variants.len() > MAX_VARIANTS {
        return Err(Error::new(format!(
            "variants: {} keys exceed the limit of {MAX_VARIANTS}",
            variants.len()
        )));
    }

    let mut parsed: Vec<(&str, Part, bool)> = Vec::new();
    for (key, value) in variants {
        let clauses = parse_variant_key(key)?;
        let options = parse_apply(value, &format!("variants[{key:?}]"))?;
        let matched = clauses_match(&clauses, properties);
        parsed.push((
            key,
            Part {
                weighted: value.is_array(),
                options,
            },
            matched,
        ));
    }

    let matched: Vec<&str> = parsed
        .iter()
        .filter(|(_, _, matched)| *matched)
        .map(|(key, _, _)| *key)
        .collect();

    match matched.as_slice() {
        [] => Err(Error::new(format!(
            "variants: no variant matches the supplied properties ({})",
            describe_properties(properties)
        ))),
        [key] => {
            let options = parsed
                .iter()
                .find(|(candidate, _, _)| candidate == key)
                .map(|(_, options, _)| options.clone())
                .ok_or_else(|| Error::new(format!("variants: key `{key}` disappeared")))?;
            Ok(vec![options])
        }
        keys => Err(Error::new(format!(
            "variants: {} variants match the supplied properties ({}); exactly one is required",
            keys.len(),
            keys.join(", ")
        ))),
    }
}

fn select_multipart(
    value: &Value,
    properties: &BTreeMap<String, String>,
) -> Result<Vec<Part>, Error> {
    let entries = value
        .as_array()
        .ok_or_else(|| Error::new("multipart: expected a JSON array"))?;
    if entries.len() > MAX_PARTS {
        return Err(Error::new(format!(
            "multipart: {} entries exceed the limit of {MAX_PARTS}",
            entries.len()
        )));
    }

    let mut parts = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let context = format!("multipart[{index}]");
        let entry = expect_object(entry, &context)?;
        for key in entry.keys() {
            if key != "when" && key != "apply" {
                return Err(Error::new(format!(
                    "{context}: unknown field `{key}` (expected `when` or `apply`)"
                )));
            }
        }
        let apply = entry
            .get("apply")
            .ok_or_else(|| Error::new(format!("{context}: missing `apply`")))?;
        let options = parse_apply(apply, &format!("{context}.apply"))?;
        let is_match = match entry.get("when") {
            None => true,
            Some(condition) => {
                parse_condition(condition, 0, &format!("{context}.when"))?.matches(properties)
            }
        };
        if is_match {
            parts.push(Part {
                weighted: apply.is_array(),
                options,
            });
        }
    }
    Ok(parts)
}

fn describe_properties(properties: &BTreeMap<String, String>) -> String {
    if properties.is_empty() {
        return "no properties".to_string();
    }
    properties
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_apply(value: &Value, context: &str) -> Result<Vec<ModelOption>, Error> {
    match value {
        Value::Object(_) => Ok(vec![parse_model_option(value, context)?]),
        Value::Array(items) => {
            if items.is_empty() {
                return Err(Error::new(format!(
                    "{context}: a weighted model array must not be empty"
                )));
            }
            if items.len() > MAX_OPTIONS_PER_PART {
                return Err(Error::new(format!(
                    "{context}: {} model options exceed the limit of {MAX_OPTIONS_PER_PART}",
                    items.len()
                )));
            }
            let options: Vec<ModelOption> = items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_model_option(item, &format!("{context}[{index}]")))
                .collect::<Result<_, _>>()?;
            options.iter().try_fold(0i32, |total, option| {
                total
                    .checked_add(option.weight as i32)
                    .ok_or_else(|| Error::new("weighted variant total exceeds signed 32-bit bound"))
            })?;
            Ok(options)
        }
        _ => Err(Error::new(format!(
            "{context}: expected a model object or a non-empty weighted array"
        ))),
    }
}

fn parse_model_option(value: &Value, context: &str) -> Result<ModelOption, Error> {
    let entry = expect_object(value, context)?;
    let mut model = None;
    let mut x = 0;
    let mut y = 0;
    let mut uvlock = false;
    let mut weight = 1;

    for (field, field_value) in entry {
        match field.as_str() {
            "model" => {
                model =
                    Some(field_value.as_str().ok_or_else(|| {
                        Error::new(format!("{context}: `model` must be a string"))
                    })?);
            }
            "x" => x = parse_rotation(field_value, "x", context)?,
            "y" => y = parse_rotation(field_value, "y", context)?,
            "uvlock" => {
                uvlock = field_value
                    .as_bool()
                    .ok_or_else(|| Error::new(format!("{context}: `uvlock` must be a boolean")))?;
            }
            "weight" => weight = parse_weight(field_value, context)?,
            other => {
                return Err(Error::new(format!(
                    "{context}: unknown model field `{other}` (expected `model`, `x`, `y`, `uvlock` or `weight`)"
                )));
            }
        }
    }

    let model = model.ok_or_else(|| Error::new(format!("{context}: missing `model`")))?;
    Ok(ModelOption {
        model: normalize_model(model, context)?,
        x,
        y,
        uvlock,
        weight,
    })
}

fn parse_rotation(value: &Value, field: &str, context: &str) -> Result<u16, Error> {
    let degrees = value.as_u64().ok_or_else(|| {
        Error::new(format!(
            "{context}: `{field}` must be a non-negative integer"
        ))
    })?;
    if degrees > 270 || degrees % 90 != 0 {
        return Err(Error::new(format!(
            "{context}: `{field}` must be 0, 90, 180 or 270"
        )));
    }
    Ok(degrees as u16)
}

fn parse_weight(value: &Value, context: &str) -> Result<u32, Error> {
    let weight = value
        .as_u64()
        .ok_or_else(|| Error::new(format!("{context}: `weight` must be a positive integer")))?;
    if weight == 0 || weight > i32::MAX as u64 {
        return Err(Error::new(format!(
            "{context}: `weight` must be a positive integer that fits in i32"
        )));
    }
    Ok(weight as u32)
}

fn normalize_model(raw: &str, context: &str) -> Result<String, Error> {
    if raw.is_empty() {
        return Err(Error::new(format!("{context}: `model` must not be empty")));
    }
    if raw.len() > MAX_MODEL_ID_LEN {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` is longer than {MAX_MODEL_ID_LEN} bytes"
        )));
    }
    if raw.contains('\\') {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` must not contain a backslash"
        )));
    }
    let (namespace, path) = match raw.split_once(':') {
        Some((namespace, path)) => (namespace, path),
        None => ("minecraft", raw),
    };
    if namespace.is_empty() {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has an empty namespace"
        )));
    }
    if namespace.len() > MAX_NAMESPACE_LEN {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has a namespace longer than {MAX_NAMESPACE_LEN} bytes"
        )));
    }
    if path.is_empty() {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has an empty path"
        )));
    }
    if path.len() > MAX_PATH_LEN {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has a path longer than {MAX_PATH_LEN} bytes"
        )));
    }
    if !namespace.bytes().all(is_namespace_byte) {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has characters outside `[a-z0-9_.-]` in its namespace"
        )));
    }
    if namespace == "." || namespace == ".." {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has a `.` or `..` namespace"
        )));
    }
    if !path.bytes().all(is_path_byte) {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has characters outside `[a-z0-9/._-]` in its path"
        )));
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` must be a relative resource path"
        )));
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(Error::new(format!(
            "{context}: `model` `{raw}` has an empty, `.` or `..` path segment"
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

fn parse_variant_key(key: &str) -> Result<Vec<(String, String)>, Error> {
    if key.is_empty() {
        return Ok(Vec::new());
    }
    let mut clauses = Vec::new();
    for clause in key.split(',') {
        let (name, value) = clause
            .split_once('=')
            .ok_or_else(|| Error::new("variant clause must be property=value"))?;
        validate_property_name(name).map_err(Error::new)?;
        validate_property_value(value).map_err(Error::new)?;
        if clauses.iter().any(|(existing, _)| existing == name) {
            return Err(Error::new("variant repeats property"));
        }
        clauses.push((name.to_owned(), value.to_owned()));
    }
    Ok(clauses)
}

fn validate_property_name(name: &str) -> Result<(), &'static str> {
    validate_property_text(name, MAX_PROPERTY_NAME_LEN)
}

fn validate_property_value(value: &str) -> Result<(), &'static str> {
    validate_property_text(value, MAX_PROPERTY_VALUE_LEN)
}

fn validate_property_text(text: &str, limit: usize) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("must not be empty");
    }
    if text.len() > limit {
        return Err("is too long");
    }
    for character in text.chars() {
        if character.is_whitespace() || character.is_control() {
            return Err("must not contain whitespace or control characters");
        }
        if matches!(character, ',' | '|' | '=') {
            return Err("must not contain `,`, `|` or `=`");
        }
    }
    Ok(())
}

fn clauses_match(clauses: &[(String, String)], properties: &BTreeMap<String, String>) -> bool {
    clauses.iter().all(|(name, expected)| {
        properties
            .get(name)
            .is_some_and(|actual| actual == expected)
    })
}

enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Clauses(Vec<(String, Vec<(String, bool)>)>),
}

impl Condition {
    fn matches(&self, properties: &BTreeMap<String, String>) -> bool {
        match self {
            Condition::All(children) => children.iter().all(|child| child.matches(properties)),
            Condition::Any(children) => children.iter().any(|child| child.matches(properties)),
            Condition::Clauses(clauses) => clauses.iter().all(|(name, terms)| {
                properties.get(name).is_some_and(|actual| {
                    terms
                        .iter()
                        .any(|(expected, negated)| (actual == expected) ^ negated)
                })
            }),
        }
    }
}

fn parse_condition(value: &Value, depth: usize, context: &str) -> Result<Condition, Error> {
    let condition = expect_object(value, context)?;
    let has_and = condition.contains_key("AND");
    let has_or = condition.contains_key("OR");

    if has_and && has_or {
        return Err(Error::new(format!(
            "{context}: `AND` and `OR` cannot be combined in one condition"
        )));
    }

    if has_and || has_or {
        if condition.len() != 1 {
            return Err(Error::new(format!(
                "{context}: `AND`/`OR` must be the only key of its condition object"
            )));
        }
        let next_depth = depth + 1;
        if next_depth > MAX_CONDITION_DEPTH {
            return Err(Error::new(format!(
                "{context}: `AND`/`OR` nesting exceeds the limit of {MAX_CONDITION_DEPTH}"
            )));
        }
        let (operator, children) = if has_and {
            ("AND", condition.get("AND"))
        } else {
            ("OR", condition.get("OR"))
        };
        let children = children.and_then(Value::as_array).ok_or_else(|| {
            Error::new(format!(
                "{context}: `{operator}` must be an array of conditions"
            ))
        })?;
        let mut parsed = Vec::with_capacity(children.len());
        for (index, child) in children.iter().enumerate() {
            parsed.push(parse_condition(
                child,
                next_depth,
                &format!("{context}.{operator}[{index}]"),
            )?);
        }
        return Ok(if has_and {
            Condition::All(parsed)
        } else {
            Condition::Any(parsed)
        });
    }

    let mut clauses = Vec::new();
    for (name, value) in condition {
        validate_property_name(name).map_err(|why| {
            Error::new(format!(
                "{context}: property name `{name}` is invalid: {why}"
            ))
        })?;
        let value = value.as_str().ok_or_else(|| {
            Error::new(format!(
                "{context}: property `{name}` must map to a string value"
            ))
        })?;
        let mut terms = Vec::new();
        for raw in value.split('|') {
            let negated = raw.starts_with('!');
            let term = raw.strip_prefix('!').unwrap_or(raw);
            validate_property_value(term).map_err(Error::new)?;
            terms.push((term.to_owned(), negated));
            if terms.len() > 64 {
                return Err(Error::new("too many property alternatives"));
            }
        }
        if clauses.iter().any(|(existing, _)| existing == name) {
            return Err(Error::new(format!(
                "{context}: property `{name}` is repeated"
            )));
        }
        clauses.push((name.clone(), terms));
    }

    if clauses.is_empty() {
        return Err(Error::new("empty property condition"));
    }
    Ok(Condition::Clauses(clauses))
}

fn expect_object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, Error> {
    value
        .as_object()
        .ok_or_else(|| Error::new(format!("{context}: expected a JSON object")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn properties(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn option(model: &str) -> ModelOption {
        ModelOption {
            model: model.to_string(),
            x: 0,
            y: 0,
            uvlock: false,
            weight: 1,
        }
    }

    fn expect_error(definition: &Value, properties: &BTreeMap<String, String>) -> String {
        match select(definition, properties) {
            Ok(parts) => panic!("expected an error, got {parts:?}"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn variants_pick_the_single_matching_key_and_defaults() {
        let definition = json!({
            "variants": {
                "facing=north": {"model": "block/stone"},
                "facing=south": {"model": "block/dirt", "x": 90, "y": 180, "uvlock": true}
            }
        });

        let parts = select(&definition, &properties(&[("facing", "north")])).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: false,
                options: vec![option("minecraft:block/stone")]
            }]
        );

        let parts = select(&definition, &properties(&[("facing", "south")])).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: false,
                options: vec![ModelOption {
                    model: "minecraft:block/dirt".to_string(),
                    x: 90,
                    y: 180,
                    uvlock: true,
                    weight: 1,
                }]
            }]
        );
    }

    #[test]
    fn variants_do_not_accept_multipart_alternative_syntax() {
        let definition = json!({"variants":{"facing=north|east":{"model":"block/a"}}});
        assert!(select(&definition, &properties(&[("facing", "east")])).is_err());
    }

    #[test]
    fn multipart_value_alternatives_and_negation() {
        let definition = json!({"multipart":[
            {"when":{"east":"side|up","powered":"!true"},"apply":{"model":"block/a"}}
        ]});
        for east in ["side", "up"] {
            let p = select(
                &definition,
                &properties(&[("east", east), ("powered", "false")]),
            )
            .unwrap();
            assert_eq!(p[0].options[0].model, "minecraft:block/a");
        }
        assert!(select(
            &definition,
            &properties(&[("east", "none"), ("powered", "false")])
        )
        .unwrap()
        .is_empty());
        assert!(select(
            &definition,
            &properties(&[("east", "up"), ("powered", "true")])
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn singleton_weighted_array_stays_distinct_from_plain_model() {
        let single = json!({"variants":{"":{"model":"block/a"}}});
        let array = json!({"variants":{"": [{"model":"block/a"}]}});
        assert!(!select(&single, &BTreeMap::new()).unwrap()[0].weighted);
        assert!(select(&array, &BTreeMap::new()).unwrap()[0].weighted);
    }

    #[test]
    fn weighted_prefix_total_cannot_overflow_vanilla_random_bound() {
        let definition = json!({"variants":{"": [
            {"model":"block/a","weight":2147483647},
            {"model":"block/b","weight":1}
        ]}});
        assert!(select(&definition, &BTreeMap::new()).is_err());
    }

    #[test]
    fn variants_empty_key_is_universal_but_ambiguous_with_another_match() {
        let universal = json!({"variants": {"": {"model": "block/air"}}});
        let parts = select(&universal, &properties(&[("anything", "goes")])).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: false,
                options: vec![option("minecraft:block/air")]
            }]
        );

        let both = json!({
            "variants": {
                "": {"model": "block/air"},
                "lit=true": {"model": "block/lamp"}
            }
        });
        let message = expect_error(&both, &properties(&[("lit", "true")]));
        assert!(message.contains("variants match"), "{message}");
        assert!(message.contains("exactly one"), "{message}");
    }

    #[test]
    fn variants_keep_every_weighted_alternative_in_order() {
        let definition = json!({
            "variants": {
                "power=1": [
                    {"model": "custom:block/a", "weight": 3, "x": 90},
                    {"model": "block/b", "y": 270, "uvlock": true}
                ]
            }
        });
        let parts = select(&definition, &properties(&[("power", "1")])).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: true,
                options: vec![
                    ModelOption {
                        model: "custom:block/a".to_string(),
                        x: 90,
                        y: 0,
                        uvlock: false,
                        weight: 3,
                    },
                    ModelOption {
                        model: "minecraft:block/b".to_string(),
                        x: 0,
                        y: 270,
                        uvlock: true,
                        weight: 1,
                    },
                ]
            }]
        );
    }

    #[test]
    fn multipart_combines_and_or_and_unconditional_entries() {
        let definition = json!({
            "multipart": [
                {
                    "when": {"OR": [
                        {"facing": "north"},
                        {"AND": [{"power": "2"}, {"lit": "true"}]}
                    ]},
                    "apply": {"model": "block/a"}
                },
                {
                    "when": {"facing": "north", "power": "2"},
                    "apply": {"model": "block/b"}
                },
                {"apply": {"model": "block/c"}}
            ]
        });

        let all = properties(&[("facing", "north"), ("power", "2"), ("lit", "true")]);
        let parts = select(&definition, &all).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].options, vec![option("minecraft:block/a")]);
        assert_eq!(parts[1].options, vec![option("minecraft:block/b")]);
        assert_eq!(parts[2].options, vec![option("minecraft:block/c")]);

        let or_second = properties(&[("facing", "south"), ("power", "2"), ("lit", "true")]);
        let parts = select(&definition, &or_second).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].options, vec![option("minecraft:block/a")]);
        assert_eq!(parts[1].options, vec![option("minecraft:block/c")]);
    }

    #[test]
    fn multipart_with_no_matching_entry_is_a_valid_empty_selection() {
        let definition = json!({
            "multipart": [
                {"when": {"lit": "true"}, "apply": {"model": "block/a"}},
                {"when": {"OR": [{"power": "1"}, {"facing": "up"}]}, "apply": {"model": "block/b"}}
            ]
        });
        let parts = select(&definition, &properties(&[("lit", "false")])).unwrap();
        assert!(parts.is_empty());
    }

    #[test]
    fn multipart_empty_and_is_true_and_empty_or_is_false() {
        let definition = json!({
            "multipart": [
                {"when": {"AND": []}, "apply": {"model": "block/always"}},
                {"when": {"OR": []}, "apply": {"model": "block/never"}}
            ]
        });
        let parts = select(&definition, &BTreeMap::new()).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: false,
                options: vec![option("minecraft:block/always")]
            }]
        );
    }

    #[test]
    fn multipart_keeps_all_weighted_alternatives() {
        let definition = json!({
            "multipart": [{
                "when": {"a": "b"},
                "apply": [
                    {"model": "block/a", "weight": 5},
                    {"model": "block/b", "weight": 1}
                ]
            }]
        });
        let parts = select(&definition, &properties(&[("a", "b")])).unwrap();
        assert_eq!(
            parts,
            vec![Part {
                weighted: true,
                options: vec![
                    ModelOption {
                        model: "minecraft:block/a".to_string(),
                        x: 0,
                        y: 0,
                        uvlock: false,
                        weight: 5,
                    },
                    ModelOption {
                        model: "minecraft:block/b".to_string(),
                        x: 0,
                        y: 0,
                        uvlock: false,
                        weight: 1,
                    },
                ]
            }]
        );
    }

    #[test]
    fn missing_properties_never_match() {
        let definition = json!({"variants": {"facing=north": {"model": "block/stone"}}});
        let message = expect_error(&definition, &BTreeMap::new());
        assert!(message.contains("no variant matches"), "{message}");
        assert!(message.contains("no properties"), "{message}");
    }

    #[test]
    fn model_ids_are_normalized() {
        for (raw, expected) in [
            ("block/stone", "minecraft:block/stone"),
            ("minecraft:block/stone", "minecraft:block/stone"),
            ("custom:block/stone", "custom:block/stone"),
            ("a.b-c_1:block/x.y-z_1", "a.b-c_1:block/x.y-z_1"),
        ] {
            let definition = json!({"multipart": [{"apply": {"model": raw}}]});
            let parts = select(&definition, &BTreeMap::new()).unwrap();
            assert_eq!(parts[0].options[0].model, expected);
        }
    }

    #[test]
    fn condition_nesting_accepts_thirty_two_operators_and_rejects_thirty_three() {
        let mut allowed = json!({"a": "1"});
        for _ in 0..32 {
            allowed = json!({"AND": [allowed]});
        }
        let definition = json!({"multipart": [{"when": allowed, "apply": {"model": "block/x"}}]});
        assert_eq!(
            select(&definition, &properties(&[("a", "1")]))
                .unwrap()
                .len(),
            1
        );

        let mut too_deep = json!({"a": "1"});
        for _ in 0..33 {
            too_deep = json!({"AND": [too_deep]});
        }
        let definition = json!({"multipart": [{"when": too_deep, "apply": {"model": "block/x"}}]});
        let message = expect_error(&definition, &properties(&[("a", "1")]));
        assert!(message.contains("nesting exceeds"), "{message}");
    }

    #[test]
    fn selection_bounds_are_enforced() {
        let mut variants = Map::new();
        for power in 0..=MAX_VARIANTS {
            variants.insert(format!("power={power}"), json!({"model": "block/x"}));
        }
        let definition = json!({"variants": variants});
        let message = expect_error(&definition, &properties(&[("power", "0")]));
        assert!(message.contains("exceed the limit"), "{message}");

        let mut options = Vec::new();
        for _ in 0..=MAX_OPTIONS_PER_PART {
            options.push(json!({"model": "block/x"}));
        }
        let definition = json!({"variants": {"power=1": options}});
        let message = expect_error(&definition, &properties(&[("power", "1")]));
        assert!(message.contains("exceed the limit"), "{message}");

        let mut parts = Vec::new();
        for _ in 0..=MAX_PARTS {
            parts.push(json!({"apply": {"model": "block/x"}}));
        }
        let definition = json!({"multipart": parts});
        let message = expect_error(&definition, &BTreeMap::new());
        assert!(message.contains("exceed the limit"), "{message}");
    }

    #[test]
    fn malformed_definitions_are_rejected() {
        let none = BTreeMap::new();
        let cases: Vec<(Value, &str)> = vec![
            (json!([]), "expected a JSON object"),
            (json!({}), "expected either `variants` or `multipart`"),
            (
                json!({"variants": {}, "multipart": []}),
                "mutually exclusive",
            ),
            (
                json!({"variants": {}, "ambientocclusion": true}),
                "unknown top-level field",
            ),
            (json!({"variants": []}), "expected a JSON object"),
            (
                json!({"variants": {"north": {"model": "block/x"}}}),
                "property=value",
            ),
            (
                json!({"variants": {"a=1|": {"model": "block/x"}}}),
                "must not contain",
            ),
            (
                json!({"variants": {"a=1,a=2": {"model": "block/x"}}}),
                "repeats property",
            ),
            (
                json!({"variants": {"=1": {"model": "block/x"}}}),
                "must not be empty",
            ),
            (
                json!({"variants": {"a b=1": {"model": "block/x"}}}),
                "whitespace",
            ),
            (json!({"variants": {"a=1": []}}), "must not be empty"),
            (
                json!({"variants": {"a=1": "block/x"}}),
                "model object or a non-empty weighted array",
            ),
            (json!({"variants": {"a=1": {}}}), "missing `model`"),
            (
                json!({"variants": {"a=1": {"model": "block/../x"}}}),
                "`..` path segment",
            ),
            (
                json!({"variants": {"a=1": {"model": "/block/x"}}}),
                "relative resource path",
            ),
            (
                json!({"variants": {"a=1": {"model": "block\\x"}}}),
                "backslash",
            ),
            (
                json!({"variants": {"a=1": {"model": "Block/x"}}}),
                "characters outside",
            ),
            (
                json!({"variants": {"a=1": {"model": "minecraft:"}}}),
                "empty path",
            ),
            (
                json!({"variants": {"a=1": {"model": "a:b:c"}}}),
                "characters outside",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "display": {}}}}),
                "unknown model field",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "x": 45}}}),
                "0, 90, 180 or 270",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "y": 360}}}),
                "0, 90, 180 or 270",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "x": -90}}}),
                "non-negative integer",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "x": 90.5}}}),
                "non-negative integer",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "weight": 0}}}),
                "positive integer",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "weight": 4294967296u64}}}),
                "fits in i32",
            ),
            (
                json!({"variants": {"a=1": {"model": "block/x", "uvlock": "true"}}}),
                "must be a boolean",
            ),
            (json!({"multipart": {}}), "expected a JSON array"),
            (json!({"multipart": [1]}), "expected a JSON object"),
            (
                json!({"multipart": [{"when": {"a": "1"}}]}),
                "missing `apply`",
            ),
            (
                json!({"multipart": [{"apply": {"model": "block/x"}, "weight": 2}]}),
                "unknown field",
            ),
            (
                json!({"multipart": [{"when": null, "apply": {"model": "block/x"}}]}),
                "expected a JSON object",
            ),
            (
                json!({"multipart": [{"when": [], "apply": {"model": "block/x"}}]}),
                "expected a JSON object",
            ),
            (
                json!({"multipart": [{"when": {"NOT": {"a": "1"}}, "apply": {"model": "block/x"}}]}),
                "must map to a string value",
            ),
            (
                json!({"multipart": [{"when": {"AND": {"a": "1"}}, "apply": {"model": "block/x"}}]}),
                "must be an array",
            ),
            (
                json!({"multipart": [{"when": {"AND": [], "a": "1"}, "apply": {"model": "block/x"}}]}),
                "only key",
            ),
            (
                json!({"multipart": [{"when": {"AND": [], "OR": []}, "apply": {"model": "block/x"}}]}),
                "cannot be combined",
            ),
            (
                json!({"multipart": [{"when": {"a": 1}, "apply": {"model": "block/x"}}]}),
                "must map to a string value",
            ),
        ];

        for (definition, fragment) in cases {
            let message = expect_error(&definition, &none);
            assert!(
                message.contains(fragment),
                "expected `{fragment}` in `{message}` for {definition}"
            );
        }
    }

    #[test]
    fn non_matching_entries_are_still_validated() {
        let variants = json!({
            "variants": {
                "a=1": {"model": "block/x"},
                "bad key": {"model": "block/x"}
            }
        });
        let message = expect_error(&variants, &properties(&[("a", "1")]));
        assert!(message.contains("property=value"), "{message}");

        let variants = json!({
            "variants": {
                "a=1": {"model": "block/x"},
                "a=2": {"model": "block/x", "x": 45}
            }
        });
        let message = expect_error(&variants, &properties(&[("a", "1")]));
        assert!(message.contains("0, 90, 180 or 270"), "{message}");

        let multipart = json!({
            "multipart": [{
                "when": {"a": "1"},
                "apply": {"model": "block/../escape"}
            }]
        });
        let message = expect_error(&multipart, &properties(&[("a", "2")]));
        assert!(message.contains("`..` path segment"), "{message}");
    }
}
