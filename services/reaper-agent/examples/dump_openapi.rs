//! Emit the generated agent OpenAPI 3.1 contract as JSON to stdout.
//!
//! Used by the `api-contract` CI job to validate the document and publish it as
//! a build artifact:
//!
//! ```bash
//! cargo run -p reaper-agent --example dump_openapi > openapi-agent.json
//! openapi-spec-validator openapi-agent.json
//! ```
fn main() {
    let spec = reaper_agent::api::build_openapi();
    match spec.to_json() {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("failed to serialize OpenAPI document: {e}");
            std::process::exit(1);
        }
    }
}
