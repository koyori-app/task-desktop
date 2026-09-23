use std::{env, fs, path::PathBuf};

fn main() {
    let spec_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("openapi.json");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let raw = fs::read_to_string(&spec_path).expect("read openapi.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&raw).expect("openapi.json is valid JSON");

    let schemas = doc
        .pointer_mut("/components/schemas")
        .and_then(|v| v.as_object_mut())
        .map(std::mem::take)
        .expect("components.schemas");

    let mut definitions = serde_json::Map::new();
    for (name, mut schema) in schemas {
        rewrite_refs(&mut schema);
        definitions.insert(name, schema);
    }

    let root = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": definitions,
    });

    let root: schemars::schema::RootSchema = serde_json::from_value(root).expect("schema document");

    let mut type_space = typify::TypeSpace::default();
    type_space.add_root_schema(root).expect("generate types");

    let tokens = type_space.to_stream();
    let ast = syn::parse2(tokens).expect("generated tokens parse");
    let content = prettyplease::unparse(&ast);

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("types.rs");
    fs::write(out, content).unwrap();
}

/// `#/components/schemas/X` → `#/definitions/X`
fn rewrite_refs(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(r)) = map.get_mut("$ref") {
                *r = r.replace("#/components/schemas/", "#/definitions/");
            }
            for value in map.values_mut() {
                rewrite_refs(value);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}
