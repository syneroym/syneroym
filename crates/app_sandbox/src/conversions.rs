use anyhow::Result;
use wasmtime::component::{Val, types::Type};

/// Convert JSON parameters to wasmtime Val vector based on function signature
pub fn json_to_wasm_params<'a>(
    params_iter: impl Iterator<Item = (&'a str, Type)>,
    json_params: Vec<serde_json::Value>,
) -> Result<Vec<Val>> {
    let mut wasm_params = Vec::new();

    // TODO: Instead of positional params, better to use named params. Since we use WIT, the names should be available in the loaded component
    for (i, (_param_name, ty)) in params_iter.enumerate() {
        let val = json_params.get(i).unwrap_or(&serde_json::Value::Null);
        match ty {
            Type::String => {
                let s: String = match val {
                    serde_json::Value::String(s) => s.clone(),
                    _ => val.to_string(),
                };
                wasm_params.push(Val::String(s));
            }
            Type::U32 => {
                let n = val.as_u64().unwrap_or(0) as u32;
                wasm_params.push(Val::U32(n));
            }
            Type::Bool => {
                let b = val.as_bool().unwrap_or(false);
                wasm_params.push(Val::Bool(b));
            }
            // Handle basic cases
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported parameter type in Wasm component. Add conversion logic."
                ));
            }
        }
    }

    Ok(wasm_params)
}

/// Convert wasmtime result values to JSON string
pub fn wasm_results_to_json_string(wasm_results: &[Val]) -> Result<String> {
    if wasm_results.is_empty() {
        Ok(String::new())
    } else {
        match &wasm_results[0] {
            Val::String(s) => Ok(s.to_string()),
            Val::Result(Ok(Some(v))) => match &**v {
                Val::String(s) => Ok(s.to_string()),
                _ => Ok(format!("{:?}", v)),
            },
            Val::Result(Ok(None)) => Ok(String::new()),
            Val::Result(Err(Some(e))) => Err(anyhow::anyhow!("Component returned error: {:?}", e)),
            Val::Result(Err(None)) => Err(anyhow::anyhow!("Component returned empty error")),
            other => Ok(format!("{:?}", other)),
        }
    }
}
