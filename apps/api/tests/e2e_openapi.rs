//! The contract `barras-api` publishes (spec §7, §8.1): every operation of its OpenAPI document
//! carries the metadata of spec §7.2; the committed `openapi.json` is the document the code
//! generates, byte for byte; `.env.example` holds a configuration the binary accepts.
#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;

use barras_adapter_http::metadata_problems;
use barras_adapter_postgres::Pools;
use barras_api::{ApiConfig, Dependencies, build_app, openapi_json};
use barras_bootstrap::{AppEnv, Environment, LogLevel};
use barras_testkit::{env_file, repository_file};

#[test]
fn every_operation_of_the_document_has_the_metadata_of_the_contract() {
    let app = build_app(Dependencies::offline().unwrap()).unwrap();

    assert_eq!(metadata_problems(app.openapi()), Vec::<String>::new());
}

#[test]
fn the_committed_openapi_json_is_the_document_the_code_generates() {
    let committed = fs::read_to_string(repository_file("openapi.json")).unwrap();

    let generated = openapi_json().unwrap();

    assert!(
        committed == generated,
        "openapi.json is stale: run `just openapi`"
    );
}

// A runtime, since each pool starts a task on it; neither opens a connection.
#[tokio::test]
async fn the_configuration_loads_from_the_values_of_env_example() {
    let variables = env_file(repository_file(".env.example"));

    let config = ApiConfig::from_env(Environment::from_pairs(variables)).unwrap();

    assert_eq!(config.env, AppEnv::Local);
    assert_eq!(config.log_level, LogLevel::Info);
    assert_eq!(config.bind_addr, "127.0.0.1:8080".parse().unwrap());
    assert_eq!(config.internal_bind_addr, "127.0.0.1:9080".parse().unwrap());
    assert_eq!(config.database_pool_size.get(), 10);
    // A developer machine has no replica: the read URL is the primary's, so one pool serves both.
    let pools = Pools::new(
        "barras-api",
        config.database_url.expose().get(),
        config.database_read_url.expose().get(),
        config.database_pool_size,
    );
    assert!(pools.read_is_primary());
}

#[test]
fn every_variable_of_the_configuration_is_in_env_example() {
    let written: Vec<String> = env_file(repository_file(".env.example"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    for name in ApiConfig::VARIABLES {
        assert!(
            written.iter().any(|written| written == name),
            "{name} is not in .env.example"
        );
    }
}

#[test]
fn every_variable_in_env_example_is_read_by_this_binary_or_belongs_to_the_ingestor() {
    for (name, _) in env_file(repository_file(".env.example")) {
        assert!(
            ApiConfig::VARIABLES.contains(&name.as_str()) || name.starts_with("INGEST_"),
            "{name} is in .env.example, but barras-api does not read it, and it is not an \
             INGEST_ variable of barras-ingest: a variable no binary reads any more"
        );
    }
}
