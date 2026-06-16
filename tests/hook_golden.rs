//! Black-box integration tests: pipe real PreToolUse payloads to the built
//! `glassbox hook` binary and assert the protocol end-to-end.
//!
//! Headline invariant: in the default **shadow** mode, stdout is exactly `{}`
//! and NEVER contains a deny — regardless of what the rails decided. One
//! enforce-mode case proves the seam works and is strictly opt-in.
//!
//! Hermetic: `HOME` points at a tmp dir (so the suite never writes to the real
//! `~/.glassbox`), and `GLASSBOX_TESSERA_BIN` points at a nonexistent path so the
//! values rail fails open without spawning a real Tessera.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_glassbox");

/// Run `glassbox hook` with the given stdin and optional mode; return (stdout, exit_code).
fn run_hook(stdin: &str, mode: Option<&str>) -> (String, i32) {
    let tmp = std::env::temp_dir().join("glassbox-itest-home");
    let _ = std::fs::create_dir_all(&tmp);

    let mut cmd = Command::new(BIN);
    cmd.arg("hook")
        .env("HOME", &tmp)
        .env("GLASSBOX_TESSERA_BIN", "/nonexistent/tessera")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match mode {
        Some(m) => {
            cmd.env("GLASSBOX_MODE", m);
        }
        None => {
            cmd.env_remove("GLASSBOX_MODE");
        }
    }

    let mut child = cmd.spawn().expect("spawn glassbox");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait glassbox");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read fixture {name}"))
}

#[test]
fn shadow_surfaces_the_decision_and_never_denies() {
    // The single most important invariant of the sprint: shadow shows the governed
    // decision (a `systemMessage` one-liner) but NEVER denies, regardless of what
    // the rails decided.
    for f in [
        "bash_rm_rf.json",
        "bash_force_push.json",
        "bash_safe_commit.json",
        "write_etc_passwd.json",
        "read_readme.json",
    ] {
        let (stdout, code) = run_hook(&fixture(f), None); // None ⇒ default shadow
        assert_eq!(code, 0, "{f}: hook must exit 0");
        assert!(
            !stdout.contains("deny"),
            "{f}: shadow output must never contain a deny"
        );
        assert!(
            !stdout.contains("permissionDecision"),
            "{f}: shadow must not emit a permission decision"
        );
        assert!(
            stdout.contains("systemMessage"),
            "{f}: shadow should surface the live one-liner"
        );
        assert!(
            stdout.contains("SHADOW"),
            "{f}: the one-liner names the posture"
        );
    }
}

#[test]
fn malformed_and_empty_input_defer() {
    let (stdout, code) = run_hook("not json", None);
    assert_eq!(code, 0);
    assert_eq!(stdout, "{}");

    let (stdout, code) = run_hook("", None);
    assert_eq!(code, 0);
    assert_eq!(stdout, "{}");
}

#[test]
fn enforce_mode_denies_destructive_proving_the_seam() {
    // Opt-in only: the seam works, but shadow (the default) is what ships live.
    let (stdout, code) = run_hook(&fixture("bash_rm_rf.json"), Some("enforce"));
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "enforce must deny rm -rf, got: {stdout}"
    );
    assert!(stdout.contains("safety rail"));
}

#[test]
fn enforce_mode_allows_benign() {
    let (stdout, _) = run_hook(&fixture("bash_safe_commit.json"), Some("enforce"));
    assert_eq!(stdout, "{}");
}
