use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Try monorepo path first, then local path (for standalone SDK repo)
    let api_path = Path::new(&manifest_dir).join("../../api/openapi/api.json");
    let local_api_path = Path::new(&manifest_dir).join("api.json");

    let openapi_path = if api_path.exists() {
        api_path
    } else if local_api_path.exists() {
        local_api_path
    } else {
        panic!(
            "OpenAPI spec not found. Expected at {:?} or {:?}",
            api_path, local_api_path
        );
    };

    // Re-run build script if OpenAPI spec changes
    println!("cargo:rerun-if-changed={}", openapi_path.display());

    // Read OpenAPI spec
    let openapi_content = fs::read_to_string(&openapi_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", openapi_path, e));
    let openapi: serde_json::Value = serde_json::from_str(&openapi_content)
        .unwrap_or_else(|e| panic!("Failed to parse OpenAPI JSON: {}", e));

    // Extract schemas from components
    let schemas = openapi
        .get("components")
        .and_then(|c| c.get("schemas"))
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    // Convert OpenAPI refs to JSON Schema $defs refs
    let schemas = convert_refs(schemas);

    // Build JSON Schema document
    let schema_names: Vec<_> = schemas
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let json_schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ModelRelayAPI",
        "description": "Generated types from ModelRelay OpenAPI spec",
        "oneOf": schema_names.iter().map(|name| {
            serde_json::json!({ "$ref": format!("#/$defs/{}", name) })
        }).collect::<Vec<_>>(),
        "$defs": schemas
    });

    // Generate Rust types using typify
    let mut type_space = typify::TypeSpace::default();

    let schema: schemars::schema::RootSchema =
        serde_json::from_value(json_schema).expect("Failed to parse as JSON Schema");

    type_space
        .add_root_schema(schema)
        .expect("Failed to add schema to TypeSpace");

    let tokens = type_space.to_stream();

    // Output without attributes - they're in mod.rs
    let output = tokens.to_string();

    // Write to OUT_DIR
    let output_path = Path::new(&out_dir).join("generated_types.rs");
    fs::write(&output_path, output)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", output_path, e));
}

fn convert_refs(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if k == "$ref" {
                    if let serde_json::Value::String(s) = &v {
                        new_map.insert(
                            k,
                            serde_json::Value::String(
                                s.replace("#/components/schemas/", "#/$defs/"),
                            ),
                        );
                    } else {
                        new_map.insert(k, convert_refs(v));
                    }
                } else {
                    new_map.insert(k, convert_refs(v));
                }
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(convert_refs).collect())
        }
        other => other,
    }
}
