//! Full WIT ⇄ JSON value conversion for the component-model dispatch boundary
//! (Slice A0′, requirement `[PLT-DAT]`).
//!
//! Two `Type`-directed primitives do all the work:
//!
//! - [`val_to_json`] turns a wasmtime component [`Val`] into a [`Value`]. A
//!   `Val` is self-describing, so no WIT [`Type`] is needed.
//! - [`json_to_val`] turns a [`Value`] into a [`Val`], *directed by the target
//!   WIT [`Type`]* — JSON is lossy, so the type is what disambiguates (a `null`
//!   could be `option::none`; a one-char string could be `char` or `string`; an
//!   object could be a record or a map).
//!
//! [`json_to_wasm_params`] (parameter binding, named or positional) and
//! [`wasm_results_to_json_string`] (result serialization) are thin adapters
//! over these two.
//!
//! # JSON encoding conventions (the "lossy-edge design note", Decision A.5)
//!
//! | WIT type | JSON encoding |
//! |---|---|
//! | `bool` | boolean |
//! | all integer widths | number |
//! | `f32`/`f64` | number (non-finite ⇒ **hard error**, never `null`) |
//! | `char` | one-scalar string |
//! | `string` | string |
//! | `list<T>` / `tuple<…>` | array |
//! | `record` | object keyed by WIT field name (kebab-case, verbatim) |
//! | `variant` | `{"tag": name[, "val": payload]}` |
//! | `enum` | string (case name) |
//! | `option<T>` | `null` \| encoded `T` |
//! | `result<T,E>` | `{"ok": …}` \| `{"err": …}` |
//! | `flags` | array of enabled flag names |
//! | `map<K,V>` | object if `K = string`, else array of `[k, v]` pairs |
//! | resource / future / stream / error-context | **unsupported** ⇒ error |
//!
//! Known, *documented and deterministic* fidelity limitations (not worked
//! around — see the M04A A0′ task section):
//!
//! - **`u64`/`s64` > 2^53.** `serde_json::Value::Number` stores `u64`/`i64`
//!   losslessly, so an in-process round-trip is exact for the full 64-bit
//!   range. The gap is *interop-only*: a consumer parsing the serialized JSON
//!   with IEEE-754 doubles (e.g. JavaScript `JSON.parse`) loses precision above
//!   `2^53`. We emit native JSON numbers; we do not stringify big integers.
//! - **`char` vs `string`.** Both encode to a one-character JSON string; at the
//!   JSON layer alone they are indistinguishable. Typed decode disambiguates
//!   via the WIT `Type`.
//! - **nested `option<option<T>>`.** JSON `null` collapses the two "empty"
//!   states: outer `none` and `some(none)` both serialize to `null`, and `null`
//!   decodes to outer `none`. So `some(none)` deterministically round-trips to
//!   `none` — a documented collapse, not silent corruption. Single-level
//!   `option<T>` is fully lossless.
//! - **non-finite floats.** `NaN`/`±Infinity` cannot be a JSON number; encoding
//!   one is a hard error, and decoding a finite-but-out-of-`f32`-range number
//!   into `f32` (which would cast to `±inf`) is likewise an error.

use std::fmt;

use anyhow::Result;
use serde_json::{Map, Number, Value};
use wasmtime::component::{Val, types::Type};

/// Convert a wasmtime component [`Val`] to a JSON [`Value`].
///
/// Errors only for values that cannot be represented on a JSON wire: non-finite
/// floats (§ module docs) and resource/future/stream/error-context handles.
pub fn val_to_json(val: &Val) -> Result<Value> {
    let json = match val {
        Val::Bool(b) => Value::Bool(*b),
        Val::S8(n) => Value::from(*n),
        Val::U8(n) => Value::from(*n),
        Val::S16(n) => Value::from(*n),
        Val::U16(n) => Value::from(*n),
        Val::S32(n) => Value::from(*n),
        Val::U32(n) => Value::from(*n),
        Val::S64(n) => Value::from(*n),
        Val::U64(n) => Value::from(*n),
        Val::Float32(f) => float_to_json(f64::from(*f), "float32")?,
        Val::Float64(f) => float_to_json(*f, "float64")?,
        Val::Char(c) => Value::String(c.to_string()),
        Val::String(s) => Value::String(s.clone()),
        Val::List(items) | Val::Tuple(items) => {
            Value::Array(items.iter().map(val_to_json).collect::<Result<_>>()?)
        }
        Val::Record(fields) => {
            let mut map = Map::with_capacity(fields.len());
            for (name, value) in fields {
                map.insert(name.clone(), val_to_json(value)?);
            }
            Value::Object(map)
        }
        Val::Variant(case, payload) => {
            let mut map = Map::with_capacity(2);
            map.insert("tag".to_string(), Value::String(case.clone()));
            if let Some(inner) = payload {
                map.insert("val".to_string(), val_to_json(inner)?);
            }
            Value::Object(map)
        }
        Val::Enum(case) => Value::String(case.clone()),
        Val::Option(opt) => match opt {
            Some(inner) => val_to_json(inner)?,
            None => Value::Null,
        },
        Val::Result(res) => {
            let (key, payload) = match res {
                Ok(payload) => ("ok", payload),
                Err(payload) => ("err", payload),
            };
            let encoded = match payload {
                Some(inner) => val_to_json(inner)?,
                None => Value::Null,
            };
            let mut map = Map::with_capacity(1);
            map.insert(key.to_string(), encoded);
            Value::Object(map)
        }
        Val::Flags(names) => Value::Array(names.iter().map(|n| Value::String(n.clone())).collect()),
        Val::Map(entries) => map_to_json(entries)?,
        Val::Resource(_) | Val::Future(_) | Val::Stream(_) | Val::ErrorContext(_) => {
            return Err(anyhow::anyhow!(
                "cannot convert WIT resource/future/stream/error-context to JSON: not \
                 representable on a JSON wire"
            ));
        }
    };
    Ok(json)
}

/// Convert a JSON [`Value`] into a wasmtime component [`Val`], directed by the
/// target WIT [`Type`].
pub fn json_to_val(json: &Value, ty: &Type) -> Result<Val> {
    let val = match ty {
        Type::Bool => Val::Bool(json.as_bool().ok_or_else(|| type_error("bool", json))?),
        Type::S8 => Val::S8(json_to_signed(json, "s8")?),
        Type::U8 => Val::U8(json_to_unsigned(json, "u8")?),
        Type::S16 => Val::S16(json_to_signed(json, "s16")?),
        Type::U16 => Val::U16(json_to_unsigned(json, "u16")?),
        Type::S32 => Val::S32(json_to_signed(json, "s32")?),
        Type::U32 => Val::U32(json_to_unsigned(json, "u32")?),
        Type::S64 => Val::S64(json.as_i64().ok_or_else(|| type_error("s64", json))?),
        Type::U64 => Val::U64(json.as_u64().ok_or_else(|| type_error("u64", json))?),
        Type::Float32 => {
            let f = json.as_f64().ok_or_else(|| type_error("float32", json))? as f32;
            if !f.is_finite() {
                return Err(anyhow::anyhow!("float32 value is out of range or non-finite: {json}"));
            }
            Val::Float32(f)
        }
        Type::Float64 => {
            // A JSON number is always finite, but guard the invariant explicitly.
            let f = json.as_f64().ok_or_else(|| type_error("float64", json))?;
            if !f.is_finite() {
                return Err(anyhow::anyhow!("float64 value is non-finite: {json}"));
            }
            Val::Float64(f)
        }
        Type::Char => {
            let s = json.as_str().ok_or_else(|| type_error("char", json))?;
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Val::Char(c),
                _ => {
                    return Err(anyhow::anyhow!(
                        "char must be a single-character string, got {s:?}"
                    ));
                }
            }
        }
        Type::String => {
            Val::String(json.as_str().ok_or_else(|| type_error("string", json))?.to_string())
        }
        Type::List(list) => {
            let arr = json.as_array().ok_or_else(|| type_error("list", json))?;
            let elem_ty = list.ty();
            Val::List(arr.iter().map(|v| json_to_val(v, &elem_ty)).collect::<Result<_>>()?)
        }
        Type::Tuple(tuple) => {
            let arr = json.as_array().ok_or_else(|| type_error("tuple", json))?;
            let types: Vec<Type> = tuple.types().collect();
            if arr.len() != types.len() {
                return Err(anyhow::anyhow!(
                    "tuple expects {} elements, got {}",
                    types.len(),
                    arr.len()
                ));
            }
            Val::Tuple(
                arr.iter().zip(&types).map(|(v, ty)| json_to_val(v, ty)).collect::<Result<_>>()?,
            )
        }
        Type::Record(record) => {
            let obj = json.as_object().ok_or_else(|| type_error("record", json))?;
            let mut fields = Vec::new();
            for field in record.fields() {
                let value = match obj.get(field.name) {
                    Some(v) => json_to_val(v, &field.ty)?,
                    None if matches!(field.ty, Type::Option(_)) => Val::Option(None),
                    None => {
                        return Err(anyhow::anyhow!(
                            "missing required record field '{}'",
                            field.name
                        ));
                    }
                };
                fields.push((field.name.to_string(), value));
            }
            Val::Record(fields)
        }
        Type::Variant(variant) => {
            let obj = json.as_object().ok_or_else(|| type_error("variant", json))?;
            let tag = obj
                .get("tag")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("variant requires a string 'tag' field: {json}"))?;
            let case = variant
                .cases()
                .find(|c| c.name == tag)
                .ok_or_else(|| anyhow::anyhow!("unknown variant case '{tag}'"))?;
            let payload = match case.ty {
                Some(payload_ty) => {
                    let inner = obj.get("val").ok_or_else(|| {
                        anyhow::anyhow!("variant case '{tag}' requires a 'val' payload")
                    })?;
                    Some(Box::new(json_to_val(inner, &payload_ty)?))
                }
                None => None,
            };
            Val::Variant(tag.to_string(), payload)
        }
        Type::Enum(en) => {
            let s = json.as_str().ok_or_else(|| type_error("enum", json))?;
            if en.names().any(|n| n == s) {
                Val::Enum(s.to_string())
            } else {
                return Err(anyhow::anyhow!("unknown enum case '{s}'"));
            }
        }
        Type::Option(opt) => match json {
            Value::Null => Val::Option(None),
            other => Val::Option(Some(Box::new(json_to_val(other, &opt.ty())?))),
        },
        Type::Result(result) => {
            let obj = json.as_object().ok_or_else(|| type_error("result", json))?;
            match (obj.get("ok"), obj.get("err")) {
                (Some(ok), None) => Val::Result(Ok(decode_result_arm(ok, result.ok())?)),
                (None, Some(err)) => Val::Result(Err(decode_result_arm(err, result.err())?)),
                _ => {
                    return Err(anyhow::anyhow!(
                        "result must have exactly one of 'ok' or 'err': {json}"
                    ));
                }
            }
        }
        Type::Flags(flags) => {
            let arr = json.as_array().ok_or_else(|| type_error("flags", json))?;
            let declared: Vec<&str> = flags.names().collect();
            let mut set: Vec<String> = Vec::new();
            for entry in arr {
                let name = entry
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("flags entries must be strings"))?;
                if !declared.contains(&name) {
                    return Err(anyhow::anyhow!("unknown flag '{name}'"));
                }
                if !set.iter().any(|n| n == name) {
                    set.push(name.to_string());
                }
            }
            Val::Flags(set)
        }
        Type::Map(map_ty) => {
            let key_ty = map_ty.key();
            let value_ty = map_ty.value();
            let entries = match json {
                Value::Object(obj) => obj
                    .iter()
                    .map(|(k, v)| {
                        let key = json_to_val(&Value::String(k.clone()), &key_ty)?;
                        Ok((key, json_to_val(v, &value_ty)?))
                    })
                    .collect::<Result<Vec<_>>>()?,
                Value::Array(arr) => arr
                    .iter()
                    .map(|pair| {
                        let elems = pair
                            .as_array()
                            .filter(|p| p.len() == 2)
                            .ok_or_else(|| anyhow::anyhow!("map entries must be [key, value]"))?;
                        Ok((json_to_val(&elems[0], &key_ty)?, json_to_val(&elems[1], &value_ty)?))
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => return Err(type_error("map", json)),
            };
            Val::Map(entries)
        }
        Type::Own(_) | Type::Borrow(_) | Type::Future(_) | Type::Stream(_) | Type::ErrorContext => {
            return Err(anyhow::anyhow!(
                "WIT resource/future/stream/error-context cannot be decoded from JSON"
            ));
        }
    };
    Ok(val)
}

/// Bind a JSON-RPC `params` payload to a function's typed parameter list.
///
/// A JSON **object** binds parameters **by name**; a JSON **array** binds them
/// **positionally**; `null` binds nothing; any other scalar binds as a single
/// positional argument (valid only for a one-parameter function). A missing
/// parameter is `option::none` when its type is `option<_>`, otherwise an
/// error.
pub fn json_to_wasm_params<'a>(
    params_iter: impl Iterator<Item = (&'a str, Type)>,
    json_params: &Value,
) -> Result<Vec<Val>> {
    let params: Vec<(&str, Type)> = params_iter.collect();
    match json_params {
        Value::Object(map) => params
            .iter()
            .map(|(name, ty)| match map.get(*name) {
                Some(v) => json_to_val(v, ty),
                None => default_for_missing(name, ty),
            })
            .collect(),
        Value::Array(arr) => params
            .iter()
            .enumerate()
            .map(|(i, (name, ty))| match arr.get(i) {
                Some(v) => json_to_val(v, ty),
                None => default_for_missing(name, ty),
            })
            .collect(),
        Value::Null => params.iter().map(|(name, ty)| default_for_missing(name, ty)).collect(),
        scalar => match params.as_slice() {
            [(_, ty)] => Ok(vec![json_to_val(scalar, ty)?]),
            [] => Err(anyhow::anyhow!("function takes no parameters but a value was provided")),
            _ => Err(anyhow::anyhow!(
                "function takes {} parameters but a single scalar was provided",
                params.len()
            )),
        },
    }
}

/// Convert a function's result values to the string carried in today's JSON-RPC
/// `result` field.
///
/// The boundary contract is preserved for backward compatibility (the caller in
/// `route_handler/dispatch.rs` wraps this as `Value::String`, and integration
/// tests parse the raw string): a `string`-typed result is returned **raw**
/// (not JSON-quoted); any other value is JSON-serialized; a WIT `result::err`
/// becomes a transport-level `Err`. Fully typing the `result` field is Slice
/// A1.
pub fn wasm_results_to_json_string(wasm_results: &[Val]) -> Result<String> {
    match wasm_results {
        [] => Ok(String::new()),
        [single] => single_result_to_string(single),
        many => {
            let arr = many.iter().map(val_to_json).collect::<Result<Vec<_>>>()?;
            Ok(serde_json::to_string(&Value::Array(arr))?)
        }
    }
}

fn single_result_to_string(val: &Val) -> Result<String> {
    match val {
        Val::Result(Ok(None)) => Ok(String::new()),
        Val::Result(Ok(Some(inner))) => stringify_boundary_value(inner),
        Val::Result(Err(Some(err))) => {
            Err(anyhow::anyhow!("component returned error: {}", val_to_json(err)?))
        }
        Val::Result(Err(None)) => Err(anyhow::anyhow!("component returned an empty error")),
        other => stringify_boundary_value(other),
    }
}

/// Raw string for a `string` value (backward compat), JSON text otherwise.
fn stringify_boundary_value(val: &Val) -> Result<String> {
    match val_to_json(val)? {
        Value::String(s) => Ok(s),
        other => Ok(serde_json::to_string(&other)?),
    }
}

fn default_for_missing(name: &str, ty: &Type) -> Result<Val> {
    if matches!(ty, Type::Option(_)) {
        Ok(Val::Option(None))
    } else {
        Err(anyhow::anyhow!("missing required parameter '{name}'"))
    }
}

fn float_to_json(f: f64, kind: &str) -> Result<Value> {
    Number::from_f64(f)
        .map(Value::Number)
        .ok_or_else(|| anyhow::anyhow!("non-finite {kind} value cannot be represented in JSON"))
}

fn map_to_json(entries: &[(Val, Val)]) -> Result<Value> {
    // A JSON object requires string keys; fall back to an array of pairs when
    // the map's key type is not `string`.
    if entries.iter().all(|(k, _)| matches!(k, Val::String(_))) {
        let mut map = Map::with_capacity(entries.len());
        for (k, v) in entries {
            if let Val::String(key) = k {
                map.insert(key.clone(), val_to_json(v)?);
            }
        }
        Ok(Value::Object(map))
    } else {
        let pairs = entries
            .iter()
            .map(|(k, v)| Ok(Value::Array(vec![val_to_json(k)?, val_to_json(v)?])))
            .collect::<Result<_>>()?;
        Ok(Value::Array(pairs))
    }
}

fn decode_result_arm(json: &Value, payload_ty: Option<Type>) -> Result<Option<Box<Val>>> {
    match payload_ty {
        Some(ty) => Ok(Some(Box::new(json_to_val(json, &ty)?))),
        None => Ok(None),
    }
}

fn type_error(expected: &str, got: &Value) -> anyhow::Error {
    anyhow::anyhow!("expected JSON value compatible with WIT {expected}, got {got}")
}

fn json_to_unsigned<T>(json: &Value, name: &str) -> Result<T>
where
    T: TryFrom<u64>,
    <T as TryFrom<u64>>::Error: fmt::Display,
{
    let n = json.as_u64().ok_or_else(|| type_error(name, json))?;
    T::try_from(n).map_err(|e| anyhow::anyhow!("value {n} out of range for {name}: {e}"))
}

fn json_to_signed<T>(json: &Value, name: &str) -> Result<T>
where
    T: TryFrom<i64>,
    <T as TryFrom<i64>>::Error: fmt::Display,
{
    let n = json.as_i64().ok_or_else(|| type_error(name, json))?;
    T::try_from(n).map_err(|e| anyhow::anyhow!("value {n} out of range for {name}: {e}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use syneroym_core::test_constants;
    use wasmtime::{
        Config, Engine,
        component::{Component, types, types::ComponentItem},
    };

    use super::*;

    // ------------------------------------------------------------------
    // val_to_json: exhaustive, hand-built `Val` -> exact JSON. No component
    // needed (a `Val` is self-describing).
    // ------------------------------------------------------------------

    #[test]
    fn val_to_json_scalars() {
        assert_eq!(val_to_json(&Val::Bool(true)).unwrap(), json!(true));
        assert_eq!(val_to_json(&Val::S8(-5)).unwrap(), json!(-5));
        assert_eq!(val_to_json(&Val::U8(200)).unwrap(), json!(200));
        assert_eq!(val_to_json(&Val::S16(-30000)).unwrap(), json!(-30000));
        assert_eq!(val_to_json(&Val::U16(60000)).unwrap(), json!(60000));
        assert_eq!(val_to_json(&Val::S32(-2_000_000_000)).unwrap(), json!(-2_000_000_000));
        assert_eq!(val_to_json(&Val::U32(4_000_000_000)).unwrap(), json!(4_000_000_000u32));
        assert_eq!(val_to_json(&Val::Char('λ')).unwrap(), json!("λ"));
        assert_eq!(val_to_json(&Val::String("hi".into())).unwrap(), json!("hi"));
    }

    #[test]
    fn val_to_json_u64_beyond_2_53_is_lossless_in_value() {
        // serde_json::Value::Number stores u64/i64 exactly; the documented gap
        // is interop-only (IEEE-754 consumers), not in-process.
        let big = (1u64 << 53) + 1;
        let v = val_to_json(&Val::U64(big)).unwrap();
        assert_eq!(v, json!(big));
        assert_eq!(v.as_u64().unwrap(), big);
        assert_eq!(val_to_json(&Val::U64(u64::MAX)).unwrap().as_u64().unwrap(), u64::MAX);
        assert_eq!(val_to_json(&Val::S64(i64::MIN)).unwrap().as_i64().unwrap(), i64::MIN);
    }

    #[test]
    fn val_to_json_finite_floats_and_nonfinite_error() {
        assert_eq!(val_to_json(&Val::Float64(1.5)).unwrap(), json!(1.5));
        assert_eq!(val_to_json(&Val::Float32(-2.25)).unwrap(), json!(-2.25));
        assert!(val_to_json(&Val::Float64(f64::NAN)).is_err());
        assert!(val_to_json(&Val::Float64(f64::INFINITY)).is_err());
        assert!(val_to_json(&Val::Float32(f32::NEG_INFINITY)).is_err());
    }

    #[test]
    fn val_to_json_compound() {
        assert_eq!(val_to_json(&Val::List(vec![Val::U32(1), Val::U32(2)])).unwrap(), json!([1, 2]));
        assert_eq!(
            val_to_json(&Val::Tuple(vec![Val::U32(1), Val::String("a".into())])).unwrap(),
            json!([1, "a"])
        );
        assert_eq!(
            val_to_json(&Val::Record(vec![
                ("creator-id".into(), Val::String("did".into())),
                ("count".into(), Val::U64(3)),
            ]))
            .unwrap(),
            json!({"creator-id": "did", "count": 3})
        );
        assert_eq!(val_to_json(&Val::Enum("beta".into())).unwrap(), json!("beta"));
        assert_eq!(
            val_to_json(&Val::Flags(vec!["read".into(), "write".into()])).unwrap(),
            json!(["read", "write"])
        );
    }

    #[test]
    fn val_to_json_variant_tagged() {
        assert_eq!(
            val_to_json(&Val::Variant("delete".into(), Some(Box::new(Val::String("id".into())))))
                .unwrap(),
            json!({"tag": "delete", "val": "id"})
        );
        assert_eq!(
            val_to_json(&Val::Variant("permission-denied".into(), None)).unwrap(),
            json!({"tag": "permission-denied"})
        );
    }

    #[test]
    fn val_to_json_result() {
        assert_eq!(
            val_to_json(&Val::Result(Ok(Some(Box::new(Val::U32(7)))))).unwrap(),
            json!({"ok": 7})
        );
        assert_eq!(val_to_json(&Val::Result(Ok(None))).unwrap(), json!({"ok": null}));
        assert_eq!(
            val_to_json(&Val::Result(Err(Some(Box::new(Val::String("boom".into())))))).unwrap(),
            json!({"err": "boom"})
        );
    }

    #[test]
    fn val_to_json_option_and_nested_collapse() {
        // Single-level option is lossless.
        assert_eq!(val_to_json(&Val::Option(Some(Box::new(Val::U32(9))))).unwrap(), json!(9));
        assert_eq!(val_to_json(&Val::Option(None)).unwrap(), Value::Null);
        // Documented collapse: outer `none` and `some(none)` both -> null.
        let some_none = Val::Option(Some(Box::new(Val::Option(None))));
        assert_eq!(val_to_json(&some_none).unwrap(), Value::Null);
        assert_eq!(val_to_json(&Val::Option(None)).unwrap(), Value::Null);
    }

    #[test]
    fn val_to_json_map_object_vs_pairs() {
        let string_keyed = Val::Map(vec![(Val::String("k".into()), Val::U32(1))]);
        assert_eq!(val_to_json(&string_keyed).unwrap(), json!({"k": 1}));
        let int_keyed = Val::Map(vec![(Val::U32(1), Val::String("a".into()))]);
        assert_eq!(val_to_json(&int_keyed).unwrap(), json!([[1, "a"]]));
    }

    // ------------------------------------------------------------------
    // json_to_val round-trip for flat (memory-free) types, via a hand-written
    // component-model WAT fixture. `Type` values can only come from a real
    // component, so we harvest them here.
    // ------------------------------------------------------------------

    const FIXTURE_WAT: &str = r#"
(component
  (core module $m
    (func (export "f_i32") (param i32) (result i32) i32.const 1)
    (func (export "f_i64") (param i64) (result i32) i32.const 1)
    (func (export "f_f32") (param f32) (result i32) i32.const 1)
    (func (export "f_f64") (param f64) (result i32) i32.const 1)
    (func (export "f2") (param i32 i32) (result i32) i32.const 1)
    (func (export "f3") (param i32 i32 i32) (result i32) i32.const 1)
  )
  (core instance $i (instantiate $m))
  (func (export "take-s8")   (param "x" s8)   (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-u8")   (param "x" u8)   (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-s16")  (param "x" s16)  (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-u16")  (param "x" u16)  (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-s32")  (param "x" s32)  (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-u32")  (param "x" u32)  (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-s64")  (param "x" s64)  (result bool) (canon lift (core func $i "f_i64")))
  (func (export "take-u64")  (param "x" u64)  (result bool) (canon lift (core func $i "f_i64")))
  (func (export "take-f32")  (param "x" f32)  (result bool) (canon lift (core func $i "f_f32")))
  (func (export "take-f64")  (param "x" f64)  (result bool) (canon lift (core func $i "f_f64")))
  (func (export "take-char") (param "x" char) (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-bool") (param "x" bool) (result bool) (canon lift (core func $i "f_i32")))
  ;; Structural (anonymous) composites are exportable at top level; nominal
  ;; types (record/variant/enum/flags) are covered via the real data-layer-test
  ;; component instead (the component model requires them to be named types on
  ;; an exported function, which is awkward to express in hand-written WAT).
  (func (export "take-tuple") (param "x" (tuple u32 s32)) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-option") (param "x" (option u32)) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-result") (param "x" (result u32 (error u32))) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-nested-option") (param "x" (option (option u32))) (result bool) (canon lift (core func $i "f3")))
  (func (export "take-two") (param "a" u32) (param "b" u32) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-req-opt") (param "a" u32) (param "b" (option u32)) (result bool) (canon lift (core func $i "f3")))
)
"#;

    fn sync_engine() -> Engine {
        let mut config = Config::new();
        config.wasm_component_model(true);
        Engine::new(&config).expect("engine")
    }

    /// Harvest the type of parameter `param_index` of a top-level exported
    /// function from a component's static type (no instantiation needed).
    fn param_type(
        engine: &Engine,
        ct: &types::Component,
        export: &str,
        param_index: usize,
    ) -> Type {
        let ext =
            ct.get_export(engine, export).unwrap_or_else(|| panic!("export {export} missing"));
        match ext.ty {
            ComponentItem::ComponentFunc(f) => {
                f.params().nth(param_index).unwrap_or_else(|| panic!("param {param_index}")).1
            }
            _ => panic!("{export} is not a function"),
        }
    }

    fn assert_roundtrip(val: Val, ty: &Type) {
        let encoded = val_to_json(&val).expect("encode");
        let decoded = json_to_val(&encoded, ty).expect("decode");
        assert_eq!(val, decoded, "round-trip mismatch (json={encoded})");
    }

    #[test]
    fn json_to_val_roundtrip_flat_types() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).expect("fixture compiles");
        let ct = component.component_type();
        let ty = |export: &str| param_type(&engine, &ct, export, 0);

        assert_roundtrip(Val::Bool(true), &ty("take-bool"));
        assert_roundtrip(Val::S8(-8), &ty("take-s8"));
        assert_roundtrip(Val::U8(250), &ty("take-u8"));
        assert_roundtrip(Val::S16(-16000), &ty("take-s16"));
        assert_roundtrip(Val::U16(64000), &ty("take-u16"));
        assert_roundtrip(Val::S32(-32000), &ty("take-s32"));
        assert_roundtrip(Val::U32(4_000_000_000), &ty("take-u32"));
        assert_roundtrip(Val::S64(i64::MIN), &ty("take-s64"));
        assert_roundtrip(Val::U64((1u64 << 53) + 1), &ty("take-u64"));
        assert_roundtrip(Val::U64(u64::MAX), &ty("take-u64"));
        assert_roundtrip(Val::Float32(-2.25), &ty("take-f32"));
        assert_roundtrip(Val::Float64(1234.5), &ty("take-f64"));
        assert_roundtrip(Val::Char('λ'), &ty("take-char"));
        assert_roundtrip(Val::Tuple(vec![Val::U32(1), Val::S32(-2)]), &ty("take-tuple"));
        assert_roundtrip(Val::Option(Some(Box::new(Val::U32(5)))), &ty("take-option"));
        assert_roundtrip(Val::Option(None), &ty("take-option"));
        assert_roundtrip(Val::Result(Ok(Some(Box::new(Val::U32(1))))), &ty("take-result"));
        assert_roundtrip(Val::Result(Err(Some(Box::new(Val::U32(9))))), &ty("take-result"));
    }

    #[test]
    fn json_to_val_nested_option_collapses_to_none() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).unwrap();
        let ct = component.component_type();
        let ty = param_type(&engine, &ct, "take-nested-option", 0);

        // some(some(v)) is lossless.
        let some_some = Val::Option(Some(Box::new(Val::Option(Some(Box::new(Val::U32(5)))))));
        let decoded = json_to_val(&val_to_json(&some_some).unwrap(), &ty).unwrap();
        assert_eq!(some_some, decoded);

        // Documented collapse: some(none) encodes to null and decodes to none.
        let some_none = Val::Option(Some(Box::new(Val::Option(None))));
        let encoded = val_to_json(&some_none).unwrap();
        assert_eq!(encoded, Value::Null);
        let decoded = json_to_val(&encoded, &ty).unwrap();
        assert_eq!(decoded, Val::Option(None));
    }

    #[test]
    fn json_to_val_float32_out_of_range_errors() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).unwrap();
        let ct = component.component_type();
        let ty = param_type(&engine, &ct, "take-f32", 0);
        // A finite f64 outside f32 range would cast to inf; must error.
        assert!(json_to_val(&json!(1e40), &ty).is_err());
        // Range-checked integers.
        let ty_u8 = param_type(&engine, &ct, "take-u8", 0);
        assert!(json_to_val(&json!(256), &ty_u8).is_err());
        assert!(json_to_val(&json!(-1), &ty_u8).is_err());
    }

    #[test]
    fn json_to_val_char_requires_single_char() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).unwrap();
        let ct = component.component_type();
        let ty = param_type(&engine, &ct, "take-char", 0);
        assert!(json_to_val(&json!("ab"), &ty).is_err());
        assert!(json_to_val(&json!(""), &ty).is_err());
        assert_eq!(json_to_val(&json!("x"), &ty).unwrap(), Val::Char('x'));
    }

    // ------------------------------------------------------------------
    // Named / positional parameter binding.
    // ------------------------------------------------------------------

    #[test]
    fn json_to_wasm_params_named_and_positional() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).unwrap();
        let ct = component.component_type();
        let two = ct.get_export(&engine, "take-two").unwrap();
        let func = match two.ty {
            ComponentItem::ComponentFunc(f) => f,
            _ => panic!("not a func"),
        };

        // Named (object) binding.
        let named = json_to_wasm_params(func.params(), &json!({"a": 1, "b": 2})).unwrap();
        assert_eq!(named, vec![Val::U32(1), Val::U32(2)]);

        // Positional (array) binding.
        let positional = json_to_wasm_params(func.params(), &json!([3, 4])).unwrap();
        assert_eq!(positional, vec![Val::U32(3), Val::U32(4)]);

        // Named binding ignores extra keys.
        let extra = json_to_wasm_params(func.params(), &json!({"a": 1, "b": 2, "c": 9})).unwrap();
        assert_eq!(extra, vec![Val::U32(1), Val::U32(2)]);

        // Missing required parameter is an error.
        assert!(json_to_wasm_params(func.params(), &json!({"a": 1})).is_err());
    }

    #[test]
    fn json_to_wasm_params_missing_option_becomes_none() {
        let engine = sync_engine();
        let component = Component::new(&engine, FIXTURE_WAT).unwrap();
        let ct = component.component_type();
        let f = ct.get_export(&engine, "take-req-opt").unwrap();
        let func = match f.ty {
            ComponentItem::ComponentFunc(f) => f,
            _ => panic!("not a func"),
        };

        // Missing option<u32> -> none.
        let bound = json_to_wasm_params(func.params(), &json!({"a": 1})).unwrap();
        assert_eq!(bound, vec![Val::U32(1), Val::Option(None)]);

        // Present option value.
        let bound = json_to_wasm_params(func.params(), &json!({"a": 1, "b": 7})).unwrap();
        assert_eq!(bound, vec![Val::U32(1), Val::Option(Some(Box::new(Val::U32(7))))]);

        // Single scalar into a one-param function is positional; here there are
        // two params, so a bare scalar is rejected.
        assert!(json_to_wasm_params(func.params(), &json!(5)).is_err());
    }

    // ------------------------------------------------------------------
    // Result-boundary string contract (backward compatibility).
    // ------------------------------------------------------------------

    #[test]
    fn wasm_results_to_json_string_contract() {
        // Empty -> empty string.
        assert_eq!(wasm_results_to_json_string(&[]).unwrap(), "");
        // Plain string result -> raw string (not JSON-quoted).
        assert_eq!(wasm_results_to_json_string(&[Val::String("hello".into())]).unwrap(), "hello");
        // result<string, _>::ok -> raw inner string.
        assert_eq!(
            wasm_results_to_json_string(&[Val::Result(Ok(Some(Box::new(Val::String(
                "5".into()
            )))))])
            .unwrap(),
            "5"
        );
        // result::err -> transport error.
        assert!(
            wasm_results_to_json_string(&[Val::Result(Err(Some(Box::new(Val::String(
                "denied".into()
            )))))])
            .is_err()
        );
        // result<_, _>::ok with no payload -> empty string.
        assert_eq!(wasm_results_to_json_string(&[Val::Result(Ok(None))]).unwrap(), "");
        // Non-string result -> proper JSON (guards the removed `{:?}` fallback).
        assert_eq!(wasm_results_to_json_string(&[Val::U32(42)]).unwrap(), "42");
        assert_eq!(
            wasm_results_to_json_string(&[Val::Record(vec![("a".into(), Val::U32(1))])]).unwrap(),
            r#"{"a":1}"#
        );
    }

    // ------------------------------------------------------------------
    // json_to_val round-trip for heap composites, via real WIT `Type`s
    // harvested from the prebuilt data-layer-test component (skips if the
    // wasm artifact has not been built).
    // ------------------------------------------------------------------

    fn store_iface(engine: &Engine, ct: &types::Component) -> types::ComponentInstance {
        let import = ct
            .get_import(engine, "syneroym:data-layer/store@0.1.0")
            .expect("data-layer store import");
        match import.ty {
            ComponentItem::ComponentInstance(i) => i,
            _ => panic!("store import is not an instance"),
        }
    }

    fn store_func(
        engine: &Engine,
        iface: &types::ComponentInstance,
        name: &str,
    ) -> types::ComponentFunc {
        for (fname, ext) in iface.exports(engine) {
            if fname == name
                && let ComponentItem::ComponentFunc(f) = ext.ty
            {
                return f;
            }
        }
        panic!("store function {name} not found");
    }

    #[test]
    fn json_to_val_roundtrip_heap_composites_via_data_layer() {
        let Ok(bytes) = fs::read(test_constants::data_layer_test_wasm_path()) else {
            eprintln!("skipping: data-layer-test wasm artifact not built");
            return;
        };
        let engine = sync_engine();
        let component = Component::new(&engine, &bytes).expect("load data-layer-test");
        let ct = component.component_type();
        let iface = store_iface(&engine, &ct);

        // `put(collection: string, value: record-write-value)` -> record with
        // a string and a list<u8>.
        let put = store_func(&engine, &iface, "put");
        let record_write_value = put.params().nth(1).unwrap().1;
        assert_roundtrip(
            Val::Record(vec![
                ("id".into(), Val::String("rec-1".into())),
                ("payload".into(), Val::List(vec![Val::U8(1), Val::U8(2), Val::U8(255)])),
            ]),
            &record_write_value,
        );

        // `query(collection: string, opts: query-options)` -> record of options.
        let query = store_func(&engine, &iface, "query");
        let query_options = query.params().nth(1).unwrap().1;
        assert_roundtrip(
            Val::Record(vec![
                ("filter".into(), Val::Option(Some(Box::new(Val::String("age>20".into()))))),
                ("limit".into(), Val::Option(Some(Box::new(Val::U32(10))))),
                ("cursor".into(), Val::Option(None)),
            ]),
            &query_options,
        );
        // Missing optional record fields decode to `none`.
        let decoded = json_to_val(&json!({}), &query_options).unwrap();
        assert_eq!(
            decoded,
            Val::Record(vec![
                ("filter".into(), Val::Option(None)),
                ("limit".into(), Val::Option(None)),
                ("cursor".into(), Val::Option(None)),
            ])
        );

        // `get(...) -> result<option<record-read-value>, data-layer-error>`:
        // harvest the result and its error variant.
        let get = store_func(&engine, &iface, "get");
        let get_result = get.results().next().unwrap();
        assert_roundtrip(Val::Result(Ok(Some(Box::new(Val::Option(None))))), &get_result);
        let data_layer_error = get_result.unwrap_result().err().expect("result has an error type");
        assert_roundtrip(Val::Variant("permission-denied".into(), None), &data_layer_error);
        assert_roundtrip(
            Val::Variant("internal".into(), Some(Box::new(Val::String("oops".into())))),
            &data_layer_error,
        );

        // `create-collection(schema: collection-schema)`: drill into
        // `collection-schema.indexes: list<index-definition>` ->
        // `index-definition.type: index-type` (an enum).
        let create = store_func(&engine, &iface, "create-collection");
        let schema_ty = create.params().next().unwrap().1;
        let indexes_ty = schema_ty
            .unwrap_record()
            .fields()
            .find(|f| f.name == "indexes")
            .expect("indexes field")
            .ty;
        let index_def_ty = indexes_ty.unwrap_list().ty();
        let index_type_enum = index_def_ty
            .unwrap_record()
            .fields()
            .find(|f| f.name == "type")
            .expect("type field")
            .ty;
        assert_roundtrip(Val::Enum("numeric".into()), &index_type_enum);
        assert!(json_to_val(&json!("not-a-case"), &index_type_enum).is_err());
    }
}
