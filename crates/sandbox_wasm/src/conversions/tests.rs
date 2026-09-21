#![allow(clippy::too_many_lines, clippy::cognitive_complexity)]

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
  ;; Nominal types (record/variant/enum/flags) referenced by an exported
  ;; function must themselves be exported *by name*; a plain `(type $t ...)`
  ;; used inline is rejected ("func not valid to be used as export"). The fix
  ;; is a named export alias: `(export $alias "name" (type $t))`, then
  ;; reference `$alias` (not `$t`) in the function signature. `record`/
  ;; `variant`/`enum` are covered via the real data-layer-test component
  ;; instead (already has all three); `flags` isn't used by any real WIT
  ;; interface in the repo, so it is covered here via this technique.
  (type $flags-t (flags "read" "write" "exec"))
  (export $flags-alias "perm-flags" (type $flags-t))
  (func (export "take-flags") (param "x" $flags-alias) (result bool) (canon lift (core func $i "f_i32")))
  (func (export "take-tuple") (param "x" (tuple u32 s32)) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-option") (param "x" (option u32)) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-result") (param "x" (result u32 (error u32))) (result bool) (canon lift (core func $i "f2")))
  (func (export "take-result-unit-ok") (param "x" (result (error u32))) (result bool) (canon lift (core func $i "f2")))
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
fn param_type(engine: &Engine, ct: &types::Component, export: &str, param_index: usize) -> Type {
    let ext = ct.get_export(engine, export).unwrap_or_else(|| panic!("export {export} missing"));
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
    assert_roundtrip(Val::Flags(vec!["read".into(), "exec".into()]), &ty("take-flags"));
    assert_roundtrip(Val::Flags(vec![]), &ty("take-flags"));
}

#[test]
fn json_to_val_result_unit_arm_rejects_stray_payload() {
    let engine = sync_engine();
    let component = Component::new(&engine, FIXTURE_WAT).unwrap();
    let ct = component.component_type();
    let ty = param_type(&engine, &ct, "take-result-unit-ok", 0);

    // Correct shape: the unit `ok` arm carries `null`.
    assert_eq!(json_to_val(&json!({"ok": null}), &ty).unwrap(), Val::Result(Ok(None)));
    // A payload on a unit arm must be rejected, not silently dropped.
    assert!(json_to_val(&json!({"ok": 5}), &ty).is_err());
    // The `err` arm still carries its declared u32 payload.
    assert_eq!(
        json_to_val(&json!({"err": 9}), &ty).unwrap(),
        Val::Result(Err(Some(Box::new(Val::U32(9)))))
    );
}

#[test]
fn json_to_val_flags_rejects_unknown_and_dedups() {
    let engine = sync_engine();
    let component = Component::new(&engine, FIXTURE_WAT).unwrap();
    let ct = component.component_type();
    let ty = param_type(&engine, &ct, "take-flags", 0);

    assert!(json_to_val(&json!(["bogus"]), &ty).is_err());
    assert!(json_to_val(&json!([1]), &ty).is_err());
    // Duplicate entries collapse (set semantics).
    assert_eq!(
        json_to_val(&json!(["read", "read"]), &ty).unwrap(),
        Val::Flags(vec!["read".into()])
    );
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
    // A finite f64 outside f32 range would cast to inf (overflow); must error.
    assert!(json_to_val(&json!(1e40), &ty).is_err());
    // A finite, nonzero f64 smaller than f32's min subnormal would cast to
    // 0.0 (underflow) -- must error, not silently become zero.
    assert!(json_to_val(&json!(1e-50), &ty).is_err());
    // A genuine zero is not underflow and must succeed.
    assert_eq!(json_to_val(&json!(0.0), &ty).unwrap(), Val::Float32(0.0));
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
        wasm_results_to_json_string(&[Val::Result(Ok(Some(Box::new(Val::String("5".into())))))])
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
    // A `result::err` in the (structurally unreachable for WIT-derived
    // components, but handled for completeness) multi-result arm still
    // propagates as a transport error, matching the single-result case.
    assert!(
        wasm_results_to_json_string(&[
            Val::U32(1),
            Val::Result(Err(Some(Box::new(Val::String("boom".into()))))),
        ])
        .is_err()
    );
    assert_eq!(wasm_results_to_json_string(&[Val::U32(1), Val::U32(2)]).unwrap(), "[1,2]");
}

// ------------------------------------------------------------------
// wasm_results_to_json (typed counterpart).
// ------------------------------------------------------------------

#[test]
fn wasm_results_to_json_contract() {
    assert_eq!(wasm_results_to_json(&[]).unwrap(), Value::Null);
    assert_eq!(wasm_results_to_json(&[Val::Result(Ok(None))]).unwrap(), Value::Null);
    assert_eq!(
        wasm_results_to_json(&[Val::Result(Ok(Some(Box::new(Val::Record(vec![(
            "a".into(),
            Val::U32(1)
        )])))))])
        .unwrap(),
        json!({"a": 1})
    );
    assert!(
        wasm_results_to_json(&[Val::Result(Err(Some(Box::new(Val::String("denied".into())))))])
            .is_err()
    );
    assert_eq!(wasm_results_to_json(&[Val::U32(42)]).unwrap(), json!(42));
    // A plain string result is a real JSON string here (unlike the raw
    // boundary contract of `wasm_results_to_json_string`).
    assert_eq!(wasm_results_to_json(&[Val::String("hello".into())]).unwrap(), json!("hello"));
    assert_eq!(wasm_results_to_json(&[Val::U32(1), Val::U32(2)]).unwrap(), json!([1, 2]));
}

// ------------------------------------------------------------------
// json_to_val round-trip for heap composites, via real WIT `Type`s
// harvested from the prebuilt data-layer-test component (skips if the
// wasm artifact has not been built).
// ------------------------------------------------------------------

fn store_iface(engine: &Engine, ct: &types::Component) -> types::ComponentInstance {
    let import =
        ct.get_import(engine, "syneroym:data-layer/store@0.1.0").expect("data-layer store import");
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
    let bytes =
        fs::read(test_constants::data_layer_test_wasm_path()).expect("wasm artifact not built");
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
    let indexes_ty =
        schema_ty.unwrap_record().fields().find(|f| f.name == "indexes").expect("indexes field").ty;
    let index_def_ty = indexes_ty.unwrap_list().ty();
    let index_type_enum =
        index_def_ty.unwrap_record().fields().find(|f| f.name == "type").expect("type field").ty;
    assert_roundtrip(Val::Enum("numeric".into()), &index_type_enum);
    assert!(json_to_val(&json!("not-a-case"), &index_type_enum).is_err());
}
