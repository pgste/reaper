//! Emit the generated control-plane OpenAPI 3.1 contract as JSON to stdout.
//!
//! Used by the `api-contract` CI job to feed an external spec validator
//! (`openapi-spec-validator`) and to publish the document as a build artifact:
//!
//! ```bash
//! cargo run -p reaper-management --example dump_openapi > openapi-management.json
//! openapi-spec-validator openapi-management.json
//! ```
fn main() {
    let spec = reaper_management::api::build_openapi();
    match spec.to_json() {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("failed to serialize OpenAPI document: {e}");
            std::process::exit(1);
        }
    }
}
