//! Full WIT ⇄ JSON value conversion for the component-model dispatch boundary
//! (requirement `[PLT-DAT]`).
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
//! # JSON encoding conventions
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
//! | resource / future / stream / error-context | **unsupported** ⇒ error (see note below) |
//!
//! Known, *documented and deterministic* fidelity limitations (not worked
//! around):
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
//!   one is a hard error. Decoding is guarded symmetrically: a finite JSON
//!   number that would *overflow* `f32` (cast to `±inf`) or *underflow* it (a
//!   nonzero value that casts to `0.0`) is likewise a hard error, never a
//!   silent value change.
//!
//! `map<K,V>` requires wasmtime's unstable `wasm_component_model_map` engine
//! feature, which `AppSandboxEngine::build_wasm_engine` does not enable — so
//! `Type::Map`/`Val::Map` cannot occur for any component this substrate can
//! actually load today. The encode/decode arms exist for component-model
//! completeness (and are unit-tested on the encode side), but are not
//! reachable in practice. **This is not a capability gap**: a generic
//! string-keyed map is already fully expressible as `list<tuple<string, V>>`
//! (the standard, stable WIT idiom for "map", precisely because `map<K,V>`
//! itself is a newer, less-supported spec addition) — `list`/`tuple` are both
//! fully supported and encode to the same `[[k,v],...]` JSON shape as this
//! converter's own non-string-key `map` fallback. Separately, the JSON-RPC
//! wire is already a generic string→value map at the object level: named
//! parameter binding (`json_to_wasm_params`) and any WIT `record` both treat
//! a JSON object as exactly that.
//!
//! `resource`/`future`/`stream`/`error-context` values never actually reach
//! this converter. WIT `resource` is used in this codebase (the guest-side
//! streaming protocol in `syneroym:messaging/stream-types`, ADR-0014), but
//! its `stream-cursor`/`stream-sink` method calls go through a dedicated
//! native call path (`crates/sandbox_wasm/src/stream.rs`) that builds and
//! reads `Val::Resource` directly in Rust — never through the JSON-RPC
//! `execute_wasm` path this module serves. The native async component-model
//! `future<T>`/`stream<T>`/`error-context` primitives (distinct from the
//! `stream-cursor` *resource*, despite the name) aren't used by any `.wit`
//! file in the repo. These arms exist purely so the `match` is exhaustive.

use std::fmt;

use anyhow::Result;
use serde_json::{Map, Number, Value};
use wasmtime::component::{Val, types::Type};

/// Convert a wasmtime component [`Val`] to a JSON [`Value`].
///
/// Errors only for values that cannot be represented on a JSON wire: non-finite
/// floats (see the module docs) and resource/future/stream/error-context
/// handles.
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
            let original = json.as_f64().ok_or_else(|| type_error("float32", json))?;
            let f = original as f32;
            // Reject both overflow (finite f64 casts to ±inf) and underflow
            // (nonzero finite f64 casts to 0.0) — either silently changes the
            // value's meaning rather than losing only unrepresentable precision.
            if !f.is_finite() || (f == 0.0 && original != 0.0) {
                return Err(anyhow::anyhow!("float32 value is out of range: {json}"));
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
/// becomes a transport-level `Err`. [`wasm_results_to_json`] is the
/// fully-typed counterpart, with no string special-case.
pub fn wasm_results_to_json_string(wasm_results: &[Val]) -> Result<String> {
    match wasm_results {
        [] => Ok(String::new()),
        [single] => single_result_to_string(single),
        many => {
            // A WIT function can only ever declare a single top-level result
            // (a tuple for "multiple values" is one `Val::Tuple`), so this arm
            // is unreachable for any WIT-derived component today. Handled for
            // component-model completeness, with the same err-propagation
            // semantics as the single-result case for consistency.
            let mut arr = Vec::with_capacity(many.len());
            for val in many {
                if let Val::Result(Err(payload)) = val {
                    let detail = match payload {
                        Some(err) => val_to_json(err)?,
                        None => Value::Null,
                    };
                    return Err(anyhow::anyhow!("component returned error: {detail}"));
                }
                arr.push(val_to_json(val)?);
            }
            Ok(serde_json::to_string(&Value::Array(arr))?)
        }
    }
}

/// Typed counterpart of [`wasm_results_to_json_string`]: the
/// guest's results as a JSON [`Value`], with no string special-case. Used by
/// the Universal Proxy (`ProxyRouter::invoke_local`) and the inbound
/// `JsonRpcToWasm` route, which is deliberately left on the string-boundary
/// path (see that function's doc comment).
///
///   `[]`                       -> `Value::Null`
///   `[Result(Ok(None))]`       -> `Value::Null`
///   `[Result(Ok(Some(v)))]`    -> `val_to_json(v)`
///   `[Result(Err(payload))]`   -> `Err` (component returned an error)
///   `[other]`                  -> `val_to_json(other)`
///   many (structurally unreachable for WIT-derived components, handled for
///   completeness) -> `Value::Array`, err-propagating like the single case
pub fn wasm_results_to_json(wasm_results: &[Val]) -> Result<Value> {
    match wasm_results {
        [] => Ok(Value::Null),
        [Val::Result(Ok(None))] => Ok(Value::Null),
        [Val::Result(Ok(Some(inner)))] => val_to_json(inner),
        [Val::Result(Err(Some(err)))] => {
            Err(anyhow::anyhow!("component returned error: {}", val_to_json(err)?))
        }
        [Val::Result(Err(None))] => Err(anyhow::anyhow!("component returned an empty error")),
        [single] => val_to_json(single),
        many => {
            let mut arr = Vec::with_capacity(many.len());
            for val in many {
                if let Val::Result(Err(payload)) = val {
                    let detail = match payload {
                        Some(err) => val_to_json(err)?,
                        None => Value::Null,
                    };
                    return Err(anyhow::anyhow!("component returned error: {detail}"));
                }
                arr.push(val_to_json(val)?);
            }
            Ok(Value::Array(arr))
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
        None if json.is_null() => Ok(None),
        None => Err(anyhow::anyhow!(
            "result arm has no payload type but a non-null value was provided: {json}"
        )),
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
mod tests;
