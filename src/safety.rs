//! The safety rail — refuses irreversible operations.
//!
//! Runs fully in-process (no subprocess, no failure mode) so it is always on and
//! sub-millisecond — the part of the gate that must never be down. The forbidden
//! substrings are READ FROM the readable governance file (`agents/safety.t.md`),
//! so the markdown stays the source of truth even on the hot path.

use crate::gate::Verdict;
use std::fs;
use std::sync::OnceLock;

fn load_patterns() -> Vec<String> {
    // Resolve the governance file: explicit override first, then the repo-relative
    // default, then a co-located `agents/safety.t.md`. If none is found the hard
    // fallback below keeps the floor populated, so a fresh clone still works.
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("GLASSBOX_SAFETY_FILE")
        .unwrap_or_else(|_| format!("{home}/Projects/walt/glassbox/agents/safety.t.md"));
    let mut pats = Vec::new();
    if let Ok(text) = fs::read_to_string(&path) {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("forbid contains ") {
                let s = rest.trim().trim_matches('"');
                if !s.is_empty() {
                    pats.push(s.to_string());
                }
            }
        }
    }
    // Hard fallback: never let the safety rail run empty, even if the file moves.
    if pats.is_empty() {
        pats = [
            "rm -rf",
            "rm -fr",
            "--force",
            "push -f",
            "reset --hard",
            "drop table",
            "DROP TABLE",
            "truncate",
            "delete from",
            "DELETE FROM",
            "mkfs",
            "dd if=",
            "shutil.rmtree",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    pats
}

/// The forbidden substrings, parsed once and cached for the process lifetime.
/// The markdown is still the only place the patterns are declared (the source of
/// truth); it is simply read once per process instead of on every action. The
/// per-action hook is one process per call, so behavior is identical — and on a
/// long-running surface the floor is deliberately fixed for the run rather than
/// mutating mid-flight under a live agent.
fn patterns() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(load_patterns)
}

/// Structural rules catch irreversible operations that no single forbidden
/// substring can express without risking a false positive. `find … -delete`
/// removes every matched path with no rollback — squarely the recursive-delete
/// case this rail exists for — yet neither token is forbiddable alone: bare
/// `find` is read-only, and `-delete` also appears inside reversible flags like
/// `git branch --delete`. Requiring BOTH the `find` invocation AND the `-delete`
/// action keeps the match precise (no benign command pairs them) while
/// generalizing past the one obfuscated literal in the corpus.
fn structural_refusal(action: &str) -> Option<&'static str> {
    if action.contains("find ") && action.contains("-delete") {
        return Some("find with -delete (recursive irreversible delete)");
    }
    // Fork bomb: `:(){ :|:& };:` spawns processes recursively until the machine is
    // exhausted and must be hard-rebooted — an irreversible loss of running state.
    // No single token is forbiddable alone, but the `:` function definition
    // (`:(){`) paired with its recursive self-pipe (`:|:`) is unique to the fork
    // bomb and present in no benign command, so requiring BOTH keeps the match
    // precise while tolerating the common whitespace variant (`:(){ :|: & };:`).
    if action.contains(":(){") && action.contains(":|:") {
        return Some("fork bomb (recursive process exhaustion)");
    }
    None
}

pub fn check(action: &str) -> Verdict {
    for p in patterns() {
        if action.contains(p.as_str()) {
            return Verdict {
                rail: "safety".into(),
                refused: true,
                reason: format!("contains forbidden substring '{}'", p),
                policy: "Irreversible".into(),
            };
        }
    }
    if let Some(rule) = structural_refusal(action) {
        return Verdict {
            rail: "safety".into(),
            refused: true,
            reason: format!("matches structural rule: {rule}"),
            policy: "Irreversible".into(),
        };
    }
    Verdict {
        rail: "safety".into(),
        refused: false,
        reason: "clean".into(),
        policy: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_every_floor_pattern() {
        // Each declared pattern, embedded in a realistic command, must refuse.
        let cases = [
            "rm -rf /tmp/x",
            "rm -fr /tmp/x",
            "git push origin main --force",
            "git push -f origin main",
            "git reset --hard HEAD~3",
            "psql -c 'drop table users'",
            "psql -c 'DROP TABLE users'",
            "truncate -s 0 db.sql",
            "delete from orders",
            "DELETE FROM orders",
            "mkfs.ext4 /dev/sda1",
            "dd if=/dev/zero of=/dev/sda",
        ];
        for c in cases {
            let v = check(c);
            assert!(v.refused, "should refuse: {c}");
            assert_eq!(v.policy, "Irreversible");
        }
    }

    #[test]
    fn allows_benign_commands() {
        let benign = [
            "git status",
            "git commit -m 'wip'",
            "cat README.md",
            "ls -la",
            "cargo build",
            "npm test",
            "mkdir build",
            "grep -r TODO src",
        ];
        for c in benign {
            assert!(!check(c).refused, "should allow: {c}");
        }
    }

    #[test]
    fn documents_known_floor_gaps() {
        // The minimal floor is a deliberate choice (see plan: "keep it minimal").
        // These obfuscated / indirect destructive actions are NOT caught — pinned
        // here so the suite and the eval agree, and any future widening is a
        // conscious diff, not an accident.
        let known_misses = [
            "rm\u{200b} -rf /tmp/x", // zero-width char splits the "rm -rf" substring
            "> important.db",        // truncate-by-redirect
        ];
        for c in known_misses {
            assert!(!check(c).refused, "documented as a known miss: {c:?}");
        }
    }

    #[test]
    fn catches_python_shutil_rmtree() {
        // A recursive tree-delete smuggled inside a `python3 -c` one-liner is just as
        // irreversible as `rm -rf`. The `shutil.rmtree` token is unambiguously
        // destructive and appears in no benign command, so catching it adds reach
        // without risking a false positive.
        let v = check("python3 -c \"import shutil; shutil.rmtree('/tmp/x')\"");
        assert!(v.refused, "shutil.rmtree must be refused");
        assert_eq!(v.policy, "Irreversible");
    }

    #[test]
    fn catches_find_delete_via_structural_rule() {
        // `find … -delete` is a recursive, unrecoverable delete obfuscated past the
        // `rm` substrings. It is caught by a structural rule (find + -delete), not a
        // single substring, because neither token is safe to forbid on its own.
        let v = check("find . -type f -delete");
        assert!(v.refused, "find -delete must be refused");
        assert_eq!(v.policy, "Irreversible");
        // Generalizes beyond the corpus literal.
        assert!(check("find /var/log -name '*.log' -delete").refused);
        // And does not fire on the tokens in isolation (no false positive).
        assert!(!check("find . -name '*.rs'").refused, "read-only find is fine");
        assert!(
            !check("git branch --delete old-feature").refused,
            "--delete on a reversible op must not trip the rule"
        );
    }

    #[test]
    fn catches_fork_bomb_via_structural_rule() {
        // `:(){ :|:& };:` recursively spawns processes until the machine is wedged
        // and must be hard-rebooted — running state lost with no rollback. Caught by
        // a structural rule (`:(){` + `:|:`), not a single substring, because neither
        // token is safe to forbid on its own.
        let v = check(":(){ :|:& };:");
        assert!(v.refused, "fork bomb must be refused");
        assert_eq!(v.policy, "Irreversible");
        // Tolerates the common whitespace-before-background variant.
        assert!(check(":(){ :|: & };:").refused);
        // And does not fire on either token in isolation (no false positive).
        assert!(
            !check("echo ':|: pipes are fine in prose'").refused,
            "a lone self-pipe string must not trip the rule"
        );
        assert!(
            !check("greet(){ echo hi; }; greet").refused,
            "an ordinary shell function definition must not trip the rule"
        );
    }

    #[test]
    fn fallback_keeps_floor_populated_when_file_missing() {
        let _guard = crate::test_support::ENV_LOCK.lock().unwrap();
        let prev = std::env::var("HOME").ok();
        // Point HOME where no safety.t.md exists → patterns() uses the fallback.
        std::env::set_var("HOME", "/nonexistent-glassbox-test-home");
        let pats = load_patterns();
        match prev {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        assert!(pats.len() >= 12, "fallback must keep the floor populated");
        assert!(pats.iter().any(|p| p == "rm -rf"));
    }
}
