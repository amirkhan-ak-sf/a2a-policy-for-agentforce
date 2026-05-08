// Copyright 2026 Salesforce, Inc. All rights reserved.

// Common helpers shared across integration tests.

pub const POLICY_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/wasm32-wasip1/release");

pub const COMMON_CONFIG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/config");

// In case the project name changes, override this value with the actual policy name.
// To obtain the current name, run "make show-policy-ref-name", or read it from
// "target/policy-ref-name.txt" after building the project.
pub const POLICY_NAME: &str = "agentforce-api-to-a2a-flex-v1-0";
