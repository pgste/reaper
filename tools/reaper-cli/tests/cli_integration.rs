//! CLI integration suite (round-3 Plan 05 §4.5, Testing T3 / Code R3-03).
//!
//! `reaper-cli` is the customer CI/CD entry point: pipelines gate on
//! `reaper test --expect deny` returning a non-zero exit. Until this suite,
//! nothing spawned the real binary to defend that contract — the only tests
//! were inline units, and the exit-code behavior customers script against
//! could silently invert.
//!
//! Every test here runs the ACTUAL binary (via Cargo's `CARGO_BIN_EXE_`,
//! zero extra dependencies) over checked-in fixtures and asserts the two
//! things a pipeline sees: the exit code and the stable stdout markers.
//! Cosmetic output may change freely; `PASS`/`FAIL`, `Decision:`,
//! `VALIDATION PASSED`, and the exit codes are the contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Run the built `reaper-cli` binary with `args`, cwd = fixtures/.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_reaper-cli"))
        .args(args)
        .current_dir(fixtures_dir())
        .output()
        .expect("spawn reaper-cli binary")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// `test` — THE exit-code contract customers gate CI on.
// ---------------------------------------------------------------------------

#[test]
fn test_met_allow_expectation_exits_zero() {
    let out = run(&[
        "test",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "alice",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "allow",
    ]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}; stderr: {}",
        out.status.code(),
        stderr_of(&out)
    );
    assert!(stdout_of(&out).contains("PASS"), "stdout must say PASS");
}

#[test]
fn test_met_deny_expectation_exits_zero() {
    let out = run(&[
        "test",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "bob",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "deny",
    ]);
    assert!(out.status.success(), "met deny expectation must exit 0");
    assert!(stdout_of(&out).contains("PASS"));
}

#[test]
fn test_violated_expectation_exits_nonzero() {
    // bob is a viewer: expecting allow must FAIL with a non-zero exit —
    // this is the line every customer pipeline scripts against.
    let out = run(&[
        "test",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "bob",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "allow",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "violated expectation must exit 1 (customer CI contract)"
    );
    let stdout = stdout_of(&out);
    assert!(stdout.contains("FAIL"), "stdout must say FAIL: {stdout}");
    assert!(
        stdout.contains("expected Allow"),
        "failure line must name the expectation: {stdout}"
    );
}

#[test]
fn test_deny_overrides_allow_for_suspended_admin() {
    // mallory holds the admin role AND is suspended: the deny rule must win.
    // Pins deny-overrides through the real binary, not just the engine.
    let out = run(&[
        "test",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "mallory",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "deny",
    ]);
    assert!(
        out.status.success(),
        "suspended admin must be denied (deny-overrides); stderr: {}",
        stderr_of(&out)
    );
}

#[test]
fn test_invalid_expect_value_is_an_error() {
    let out = run(&[
        "test",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "alice",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "maybe",
    ]);
    assert!(!out.status.success(), "invalid --expect must exit non-zero");
    assert!(
        stderr_of(&out).contains("Invalid expected decision"),
        "error must name the bad value"
    );
}

#[test]
fn test_missing_policy_file_is_an_error_not_a_pass() {
    let out = run(&[
        "test",
        "--policy",
        "no-such-policy.reap",
        "--data",
        "entities.json",
        "--principal",
        "alice",
        "--action",
        "read",
        "--resource",
        "doc-1",
        "--expect",
        "allow",
    ]);
    assert!(!out.status.success(), "missing policy must exit non-zero");
    assert!(stderr_of(&out).contains("not found"));
}

// ---------------------------------------------------------------------------
// `test-suite` — batch YAML runner.
// ---------------------------------------------------------------------------

#[test]
fn test_suite_all_passing_exits_zero() {
    let out = run(&["test-suite", "--file", "suite-pass.yaml"]);
    assert!(
        out.status.success(),
        "all-green suite must exit 0; stderr: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(stdout.contains("3 passed, 0 failed"), "summary: {stdout}");
    assert!(stdout.contains("All tests passed!"));
}

#[test]
fn test_suite_with_failure_exits_nonzero_and_names_the_test() {
    let out = run(&["test-suite", "--file", "suite-fail.yaml"]);
    assert_eq!(out.status.code(), Some(1), "failing suite must exit 1");
    let stdout = stdout_of(&out);
    assert!(stdout.contains("1 passed, 1 failed"), "summary: {stdout}");
    assert!(
        stdout.contains("viewer wrongly expected to be allowed"),
        "the failing test must be named: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// `eval` — reports the decision; exit 0 for BOTH allow and deny (a deny is a
// successful evaluation — only `test`/`check` turn decisions into exit codes).
// ---------------------------------------------------------------------------

#[test]
fn eval_reports_allow() {
    let out = run(&[
        "eval",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "alice",
        "--action",
        "read",
        "--resource",
        "doc-1",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(stdout_of(&out).contains("Decision: ALLOW"));
}

#[test]
fn eval_reports_deny_and_still_exits_zero() {
    let out = run(&[
        "eval",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "bob",
        "--action",
        "read",
        "--resource",
        "doc-1",
    ]);
    assert!(
        out.status.success(),
        "eval of a denied request is still a successful evaluation"
    );
    assert!(stdout_of(&out).contains("Decision: DENY"));
}

#[test]
fn eval_unknown_principal_is_an_error() {
    let out = run(&[
        "eval",
        "--policy",
        "rbac.reap",
        "--data",
        "entities.json",
        "--principal",
        "nobody",
        "--action",
        "read",
        "--resource",
        "doc-1",
    ]);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("not found in data"));
}

// ---------------------------------------------------------------------------
// `validate` — syntax gate.
// ---------------------------------------------------------------------------

#[test]
fn validate_good_policy_exits_zero() {
    let out = run(&["validate", "rbac.reap"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(stdout_of(&out).contains("VALIDATION PASSED"));
}

#[test]
fn validate_bad_syntax_exits_nonzero() {
    let out = run(&["validate", "bad-syntax.reap"]);
    assert!(!out.status.success(), "bad syntax must exit non-zero");
    assert!(stdout_of(&out).contains("SYNTAX ERROR"));
}

// ---------------------------------------------------------------------------
// `compile` + `bundle info` — the .rbb round-trip, no services needed.
// ---------------------------------------------------------------------------

#[test]
fn compile_then_bundle_info_round_trips() {
    let out_dir = std::env::temp_dir().join(format!("reaper-cli-it-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    let bundle_path = out_dir.join("cli_rbac.rbb");
    let bundle_str = bundle_path.to_str().expect("utf-8 temp path");

    let compiled = run(&["compile", "rbac.reap", "--output", bundle_str, "--info"]);
    assert!(
        compiled.status.success(),
        "compile must succeed; stderr: {}",
        stderr_of(&compiled)
    );
    let stdout = stdout_of(&compiled);
    assert!(stdout.contains("Bundle written"), "stdout: {stdout}");
    assert!(
        stdout.contains("Policy: cli_rbac"),
        "--info must show the policy name: {stdout}"
    );
    assert!(bundle_path.exists(), "bundle file must exist on disk");
    assert!(
        std::fs::metadata(&bundle_path)
            .expect("bundle metadata")
            .len()
            > 0,
        "bundle must be non-empty"
    );

    let info = run(&["bundle", "info", bundle_str]);
    assert!(
        info.status.success(),
        "bundle info must read the bundle back; stderr: {}",
        stderr_of(&info)
    );
    assert!(stdout_of(&info).contains("cli_rbac"));

    std::fs::remove_dir_all(&out_dir).ok();
}

#[test]
fn compile_bad_syntax_exits_nonzero_and_writes_nothing() {
    let out_dir = std::env::temp_dir().join(format!("reaper-cli-it-bad-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    let bundle_path = out_dir.join("never.rbb");

    let out = run(&[
        "compile",
        "bad-syntax.reap",
        "--output",
        bundle_path.to_str().expect("utf-8 temp path"),
    ]);
    assert!(!out.status.success(), "bad policy must not compile");
    assert!(
        !bundle_path.exists(),
        "no bundle may be written for a policy that failed to parse"
    );

    std::fs::remove_dir_all(&out_dir).ok();
}
