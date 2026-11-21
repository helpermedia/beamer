//! The model's types for a typed web UI.
//!
//! A package with `webview/package.json` gets a test from
//! `#[beamer::export]` that compares `webview/src/model-types.ts` with the
//! model's Rust code. [`write`] runs that test with
//! `BEAMER_WRITE_MODEL_TYPES` set, so that it writes the file instead. Then
//! it prints the notes the generator left in it. The bundle writes the types
//! before the web build, whose `check` script then typechecks the page
//! against them.

use std::path::Path;
use std::process::{Command, Stdio};

use beamer_utils::web_ui::{MODEL_TYPES_NOTE, MODEL_TYPES_PATH, WRITE_MODEL_TYPES_VAR};

/// The test that `#[beamer::export]` adds to a package with a typed web UI.
const TEST_NAME: &str = "__beamer_model_types::model_types_match_the_model";

/// Write the model's types of `package` into its page.
///
/// Builds the plugin crate's tests for the build machine, with the global
/// flags of a debug bundle (`flags`) and the features of the bundle's
/// first format (`features`), so that they share its artifacts. A package
/// without a typed web UI is left alone.
pub fn write(
    package: &str,
    package_dir: &Path,
    workspace_root: &Path,
    target: &str,
    features: Option<&str>,
    flags: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    if !beamer_utils::web_ui::has_typed_web_ui(package_dir) {
        return Ok(());
    }

    crate::status!("  Writing the model's types...");

    let mut cmd = Command::new("cargo");
    cmd.args(["test", "-p", package, "--lib", "--target", target])
        .env(WRITE_MODEL_TYPES_VAR, "1")
        .current_dir(workspace_root);
    if let Some(features) = features {
        cmd.arg("--features").arg(features);
    }
    if let Some(flags) = flags {
        cmd.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    crate::build::set_deployment_target(&mut cmd, target);
    // A substring filter: `#[beamer::export]` in a module puts the module's
    // path in front of the test's name.
    cmd.args(["--", TEST_NAME]);

    // The compiler's progress shows in verbose mode only. The test's own
    // output is read either way to count the tests that ran. A failed run
    // prints it: it names what the generator could not write.
    cmd.stdout(Stdio::piped());
    cmd.stderr(if verbose { Stdio::inherit() } else { Stdio::piped() });
    let output = cmd.output().map_err(|e| format!("Failed to run cargo test: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        print!("{}", stdout);
        return Err("writing the model's types failed".to_string());
    }
    match passed_tests(&stdout) {
        1 => {}
        0 => {
            return Err(format!(
                "No test wrote the model's types: {} has no test named {}. Is #[beamer::export] on \
                 the model? A build from before webview/package.json existed can hide it: change \
                 the plugin's lib.rs to expand the macro again.",
                package, TEST_NAME
            ))
        }
        count => {
            return Err(format!(
                "{} tests named {} ran in {}, one per #[beamer::export]: a plugin crate exports one model.",
                count, TEST_NAME, package
            ))
        }
    }

    let path = package_dir.join(MODEL_TYPES_PATH);
    let written = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "The test that writes the model's types ran, but {} cannot be read ({}). Is #[beamer::export] on the model?",
            path.display(),
            e
        )
    })?;
    for note in notes(&written) {
        crate::status!("    {}", note);
    }
    crate::verbose!(verbose, "    Model types: {}", path.display());
    Ok(())
}

/// The number of tests that passed, from the summary lines of a test run.
fn passed_tests(stdout: &str) -> usize {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("test result: ")?.split("; ").next()?.rsplit(' ').nth(1)?.parse::<usize>().ok())
        .sum()
}

/// The notes the generator left in the model's types, without their
/// comment prefix.
fn notes(written: &str) -> impl Iterator<Item = &str> {
    written.lines().filter_map(|line| line.trim_start().strip_prefix(MODEL_TYPES_NOTE))
}

/// How the page's `check` script counts in a bundle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckMode {
    /// An error fails the bundle: the page is embedded.
    Fail,
    /// Errors are reported and the bundle goes on: the editor loads the
    /// page from a dev server, so a half-edited page must not block it.
    Report,
}

/// The `check` script a typed web UI needs, or the error that says how to
/// add one.
pub fn check_script(package_json: &str) -> Result<(), String> {
    let manifest: serde_json::Value =
        serde_json::from_str(package_json).map_err(|e| format!("webview/package.json is no JSON: {}", e))?;
    if manifest.pointer("/scripts/check").and_then(|script| script.as_str()).is_some() {
        return Ok(());
    }
    Err("webview/package.json has no \"check\" script. The bundle typechecks the page with it \
         before vite, which strips the types without checking them. Add it to \"scripts\": \
         \"check\": \"tsc -b\" for a React page, \
         \"check\": \"svelte-check --tsconfig ./tsconfig.json\" for a Svelte page."
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notes_are_the_lines_with_the_note_prefix() {
        let written = "// Generated.\nexport interface A {}\n  // Note: `Tab` has no #[derive(TypeScript)].\n";
        assert_eq!(notes(written).collect::<Vec<_>>(), ["`Tab` has no #[derive(TypeScript)]."]);
    }

    #[test]
    fn test_passed_tests_are_counted_from_the_summary_lines() {
        let one = "running 1 test\ntest m::__beamer_model_types::model_types_match_the_model ... ok\n\n\
                   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.01s\n";
        assert_eq!(passed_tests(one), 1);
        let none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out\n";
        assert_eq!(passed_tests(none), 0);
        assert_eq!(passed_tests(""), 0);
    }

    #[test]
    fn test_a_typed_web_ui_needs_a_check_script() {
        assert_eq!(check_script(r#"{ "scripts": { "build": "vite build", "check": "tsc --noEmit" } }"#), Ok(()));
        let missing = check_script(r#"{ "scripts": { "build": "vite build" } }"#).unwrap_err();
        assert!(missing.contains("no \"check\" script"), "{missing}");
        assert!(check_script("not json").is_err());
    }
}
