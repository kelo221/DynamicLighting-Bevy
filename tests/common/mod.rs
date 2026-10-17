//! Helpers shared by the GPU test files.

use std::process::Command;

/// Runs `cargo run --example <name> -- <args>` with this test's features and asserts it exits
/// cleanly without logging render errors.
pub fn run_example(name: &str, args: &[&str]) -> String {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["run", "--example", name]);
    if cfg!(feature = "dev") {
        command.args(["--features", "dev"]);
    }
    let output = command.arg("--").args(args).output().expect("cargo runs");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{name} failed:\n{log}");
    assert!(!log.contains(" ERROR "), "{name} logged an error:\n{log}");
    log
}
