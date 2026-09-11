//! Lossless, self-contained storage of repeated Record reference maps.
//! This module never changes logical Records or their identity rules.

use crate::{canonical_digest, canonical_json};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const STORAGE_FORMAT: &str = "adf-record-refmaps-v1";
pub const MAX_STORED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Result,
    Evidence,
}

impl RecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Result => "result",
            Self::Evidence => "evidence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoragePolicy {
    #[default]
    Plain,
    Adaptive,
}

impl StoragePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain-json-v1",
            Self::Adaptive => "adaptive-refmaps-v1",
        }
    }
}

fn canonical(value: &Value) -> Result<String, String> {
    canonical_json(value).map_err(|error| error.to_string())
}

fn digest(value: &Value) -> Result<String, String> {
    canonical_digest(value).map_err(|error| error.to_string())
}

fn slots(record: &Value, kind: RecordKind) -> Vec<String> {
    let mut paths = Vec::new();
    for key in ["input_refs", "freshness_refs"] {
        if (kind == RecordKind::Result || key == "input_refs") && record.get(key).is_some() {
            paths.push(format!("/{key}"));
        }
    }
    if kind == RecordKind::Result
        && let Some(outcomes) = record
            .pointer("/payload/outcomes")
            .and_then(Value::as_array)
    {
        for (i, outcome) in outcomes.iter().enumerate() {
            for key in ["input_refs", "freshness_refs"] {
                if outcome.get(key).is_some() {
                    paths.push(format!("/payload/outcomes/{i}/{key}"));
                }
            }
        }
    }
    paths
}

fn location(path: &str, kind: RecordKind) -> Result<(&str, &str), String> {
    if path == "/input_refs" || (kind == RecordKind::Result && path == "/freshness_refs") {
        return Ok(("", &path[1..]));
    }
    if kind == RecordKind::Result
        && let Some(rest) = path.strip_prefix("/payload/outcomes/")
        && let Some((index, field)) = rest.split_once('/')
        && matches!(field, "input_refs" | "freshness_refs")
        && index.parse::<usize>().is_ok_and(|i| i.to_string() == index)
    {
        return Ok((&path[..path.len() - field.len() - 1], field));
    }
    Err(format!("invalid reference map location: {path}"))
}

fn reference_map(value: &Value) -> Result<(), String> {
    if value
        .as_object()
        .is_some_and(|map| map.values().all(Value::is_string))
    {
        Ok(())
    } else {
        Err("reference map must be a string-to-string object".to_owned())
    }
}

/// Caller validates the logical Record with the pinned Schema before writing.
pub fn encode(record: &Value, kind: RecordKind, policy: StoragePolicy) -> Result<Vec<u8>, String> {
    let plain = canonical(record)? + "\n";
    if plain.len() > MAX_EXPANDED_BYTES {
        return Err("expanded Record exceeds storage limit".to_owned());
    }
    if policy == StoragePolicy::Plain {
        return Ok(plain.into_bytes());
    }
    let mut counts = BTreeMap::<String, usize>::new();
    let mut values = BTreeMap::<String, Value>::new();
    let mut bindings = Map::new();
    for path in slots(record, kind) {
        let value = record.pointer(&path).expect("slot exists");
        reference_map(value)?;
        let hash = digest(value)?;
        if let Some(previous) = values.get(&hash)
            && previous != value
        {
            return Err("reference map hash collision".to_owned());
        }
        values.entry(hash.clone()).or_insert_with(|| value.clone());
        *counts.entry(hash.clone()).or_default() += 1;
        bindings.insert(path, Value::String(hash));
    }
    bindings.retain(|_, hash| counts[hash.as_str().expect("hash string")] > 1);
    if bindings.is_empty() {
        return Ok(plain.into_bytes());
    }
    values.retain(|hash, _| counts[hash] > 1);
    let mut body = record.clone();
    for path in bindings.keys() {
        let (parent, field) = location(path, kind)?;
        body.pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .ok_or("reference map parent is not an object")?
            .remove(field);
    }
    let envelope = json!({
        "storage_format": STORAGE_FORMAT,
        "record_kind": kind.as_str(),
        "record_digest": digest(record)?,
        "record": body,
        "reference_maps": values,
        "map_bindings": bindings,
    });
    let packed = canonical(&envelope)? + "\n";
    if packed.len() < plain.len() && packed.len() <= MAX_STORED_BYTES {
        Ok(packed.into_bytes())
    } else {
        Ok(plain.into_bytes())
    }
}

/// Decode before Schema validation, hashing, cache projection or identity checks.
pub fn decode(bytes: &[u8], kind: RecordKind) -> Result<Value, String> {
    decode_with_limit(bytes, kind, MAX_EXPANDED_BYTES)
}

fn decode_with_limit(bytes: &[u8], kind: RecordKind, limit: usize) -> Result<Value, String> {
    if bytes.len() > MAX_EXPANDED_BYTES {
        return Err("Record exceeds storage limit".to_owned());
    }
    let envelope = parse_strict(bytes)?;
    let object = envelope.as_object().ok_or("Record must be an object")?;
    if !object.contains_key("storage_format") {
        if bytes.len() > limit {
            return Err("expanded Record exceeds storage limit".to_owned());
        }
        return Ok(envelope);
    }
    if bytes.len() > MAX_STORED_BYTES {
        return Err("stored Record exceeds storage limit".to_owned());
    }
    let expected = [
        "map_bindings",
        "record",
        "record_digest",
        "record_kind",
        "reference_maps",
        "storage_format",
    ];
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected.into_iter().collect()
        || envelope["storage_format"] != STORAGE_FORMAT
        || envelope["record_kind"] != kind.as_str()
    {
        return Err("unsupported or invalid Record storage envelope".to_owned());
    }
    let maps = envelope["reference_maps"]
        .as_object()
        .ok_or("reference_maps must be an object")?;
    let bindings = envelope["map_bindings"]
        .as_object()
        .ok_or("map_bindings must be an object")?;
    if maps.is_empty() || bindings.is_empty() {
        return Err("storage envelope must contain reference maps and bindings".to_owned());
    }
    let mut sizes = BTreeMap::new();
    for (hash, value) in maps {
        reference_map(value)?;
        if hash != &digest(value)? {
            return Err("reference map digest mismatch".to_owned());
        }
        sizes.insert(hash.as_str(), canonical(value)?.len());
    }
    let mut body = envelope["record"].clone();
    if !body.is_object() {
        return Err("stored record must be an object".to_owned());
    }
    // Budget all insertions before cloning any shared map into its destinations.
    let mut expanded = canonical(&body)?.len();
    let mut used = BTreeSet::new();
    for (path, hash) in bindings {
        let hash = hash
            .as_str()
            .ok_or("reference map binding must be a string")?;
        let size = sizes.get(hash).ok_or("missing reference map")?;
        used.insert(hash);
        let (parent, field) = location(path, kind)?;
        let parent = body
            .pointer(parent)
            .and_then(Value::as_object)
            .ok_or("reference map parent is missing")?;
        if parent.contains_key(field) {
            return Err("reference map binding would overwrite an inline field".to_owned());
        }
        expanded = expanded
            .checked_add(*size)
            .and_then(|n| n.checked_add(field.len() + 4))
            .filter(|n| *n <= limit)
            .ok_or("expanded Record exceeds storage limit")?;
    }
    if used.len() != maps.len() {
        return Err("unused reference map".to_owned());
    }
    for (path, hash) in bindings {
        let (parent, field) = location(path, kind)?;
        body.pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .expect("validated location")
            .insert(
                field.to_owned(),
                maps[hash.as_str().expect("validated hash")].clone(),
            );
    }
    if envelope["record_digest"].as_str() != Some(digest(&body)?.as_str()) {
        return Err("restored Record digest mismatch".to_owned());
    }
    Ok(body)
}

/// Duplicate object keys cannot silently change a reference table or binding.
pub fn parse_strict(bytes: &[u8]) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer).map_err(|e| e.to_string())?;
    deserializer.end().map_err(|e| e.to_string())?;
    Ok(value.0)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(StrictValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, StrictValue(value))) =
                    map.next_entry::<String, StrictValue>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        let refs: Map<String, Value> = (0..30)
            .map(|i| {
                (
                    format!("code.日本語.{i}"),
                    json!(format!("sha256:{}", "a".repeat(64))),
                )
            })
            .collect();
        json!({"id":"result.example", "input_refs":refs, "freshness_refs":refs,
            "execution":{"context_bytes":30}, "payload":{"outcomes":[
                {"input_refs": refs}, {"freshness_refs":{}, "summary":"keep empty and missing distinct"}],
                "custom":{"input_refs":refs}}})
    }

    #[test]
    fn preserves_exact_logical_record_and_opaque_payload() {
        let record = example();
        let packed = encode(&record, RecordKind::Result, StoragePolicy::Adaptive).unwrap();
        let envelope = parse_strict(&packed).unwrap();
        assert_eq!(envelope["storage_format"], STORAGE_FORMAT);
        let schema: Value = serde_json::from_str(include_str!(
            "../schemas/storage/record-refmaps-v1.schema.json"
        ))
        .unwrap();
        crate::schema::validate_json_document(&envelope, &schema).unwrap();
        assert_eq!(
            envelope["record"]["payload"]["custom"],
            record["payload"]["custom"]
        );
        let restored = decode(&packed, RecordKind::Result).unwrap();
        assert_eq!(restored, record);
        assert_eq!(
            encode(&restored, RecordKind::Result, StoragePolicy::Adaptive).unwrap(),
            packed
        );
        assert!(packed.len() < canonical(&record).unwrap().len());
        assert!(decode_with_limit(&packed, RecordKind::Result, 100).is_err());
    }

    #[test]
    fn rejects_corrupt_ambiguous_and_unknown_envelopes() {
        let original =
            parse_strict(&encode(&example(), RecordKind::Result, StoragePolicy::Adaptive).unwrap())
                .unwrap();
        for pointer in ["/storage_format", "/record_kind", "/record_digest"] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = json!("bad");
            assert!(decode(&serde_json::to_vec(&value).unwrap(), RecordKind::Result).is_err());
        }
        let hash = original["reference_maps"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap();
        for path in [
            "/payload/custom/input_refs",
            "/payload/outcomes/00/input_refs",
            "/payload/outcomes/99/input_refs",
        ] {
            let mut value = original.clone();
            value["map_bindings"][path] = json!(hash);
            assert!(decode(&serde_json::to_vec(&value).unwrap(), RecordKind::Result).is_err());
        }
        let mut overwrite = original.clone();
        overwrite["record"]["input_refs"] = json!({});
        assert!(decode(&serde_json::to_vec(&overwrite).unwrap(), RecordKind::Result).is_err());
        let mut missing = original.clone();
        missing["reference_maps"] = json!({});
        assert!(decode(&serde_json::to_vec(&missing).unwrap(), RecordKind::Result).is_err());
        let mut tampered = original;
        tampered["record"]["execution"]["context_bytes"] = json!(31);
        assert!(decode(&serde_json::to_vec(&tampered).unwrap(), RecordKind::Result).is_err());
        assert!(parse_strict(br#"{"a":1,"a":2}"#).is_err());
    }

    #[test]
    fn python_storage_fixture_preserves_schema_and_identity() {
        let logical =
            parse_strict(include_bytes!("../testdata/storage/v1/result.logical.json")).unwrap();
        let stored = include_bytes!("../testdata/storage/v1/result.stored.json");
        assert_eq!(decode(stored, RecordKind::Result).unwrap(), logical);
        assert_eq!(
            encode(&logical, RecordKind::Result, StoragePolicy::Adaptive).unwrap(),
            stored
        );
        let registry = crate::schema::SchemaRegistry::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"),
        )
        .unwrap();
        registry.validate("result", &logical).unwrap();
        let mut body = logical.clone();
        body.as_object_mut().unwrap().remove("id");
        body.as_object_mut().unwrap().remove("execution");
        assert_eq!(
            logical["id"],
            format!("result.{}", &digest(&body).unwrap()[7..27])
        );
    }

    #[test]
    fn small_records_stay_plain() {
        let record = json!({"input_refs":{"code.a":"sha256:ABC"}});
        let bytes = encode(&record, RecordKind::Evidence, StoragePolicy::Adaptive).unwrap();
        assert_eq!(decode(&bytes, RecordKind::Evidence).unwrap(), record);
        assert!(
            !parse_strict(&bytes)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("storage_format")
        );
    }
}
