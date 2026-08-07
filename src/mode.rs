//! Execution posture — the structural non-blocking guarantee.
//!
//! `Shadow` observes, renders, and logs but **never** blocks; `Enforce` can deny.
//! The default is `Shadow`, and the `Shadow` arm of [`resolve_output`] does not
//! branch on `blocked` at all — so no future edit to the decision logic can make
//! Shadow emit a deny without changing that one obvious match arm. That is the
//! guarantee behind "the gate cannot break your workflow": it is a property of
//! the type-level mapping, not a runtime `if` that could regress.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Shadow,
    Enforce,
}

impl Mode {
    /// Fail-safe parse: only an exact (trimmed, case-insensitive) `"enforce"`
    /// yields `Enforce`. `None`, `""`, typos, `"ENFORCE!"` all stay `Shadow`.
    pub fn resolve(v: Option<&str>) -> Mode {
        match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("enforce") => Mode::Enforce,
            _ => Mode::Shadow,
        }
    }

    /// Resolve from `GLASSBOX_MODE`. Unset / anything-but-`enforce` → `Shadow`.
    pub fn from_env() -> Mode {
        Mode::resolve(std::env::var("GLASSBOX_MODE").ok().as_deref())
    }

    /// Per-rail resolution: `GLASSBOX_{RAIL}_MODE` (e.g. `GLASSBOX_SAFETY_MODE`)
    /// if set, else the global `GLASSBOX_MODE` default. Lets one rail (the
    /// pattern-based, well-tested `safety` rail) move to `Enforce` while
    /// others (`values`, `envelope`) stay in `Shadow` until they have a
    /// comparable false-positive track record — see Phase A follow-up,
    /// ~/.claude/plans/vectorized-nibbling-snowglobe.md.
    pub fn for_rail(rail: &str) -> Mode {
        let key = format!("GLASSBOX_{}_MODE", rail.to_ascii_uppercase());
        match std::env::var(&key) {
            Ok(v) => Mode::resolve(Some(&v)),
            Err(_) => Mode::from_env(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Shadow => "shadow",
            Mode::Enforce => "enforce",
        }
    }
}

/// What the Claude Code PreToolUse hook prints on stdout. `Defer` => `{}` (allow
/// / defer to normal permission flow); `Deny` => the deny-protocol JSON.
pub enum HookOutput {
    Defer,
    Deny(String),
}

impl HookOutput {
    pub fn render(&self) -> String {
        match self {
            HookOutput::Defer => "{}".to_string(),
            HookOutput::Deny(reason) => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": format!("Glass Box governance — {}", reason),
                }
            })
            .to_string(),
        }
    }
}

/// The decision→output mapping. **`Shadow` can only ever `Defer`** — its arm
/// never inspects `blocked`. The non-blocking guarantee lives here, in one place.
pub fn resolve_output(mode: Mode, blocked: bool, reason: &str) -> HookOutput {
    match mode {
        Mode::Shadow => HookOutput::Defer,
        Mode::Enforce if blocked => HookOutput::Deny(reason.to_string()),
        Mode::Enforce => HookOutput::Defer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set/clear a batch of vars, run `f`, always restore the prior values —
    /// even on panic, via `catch_unwind` — since these tests share
    /// process-global env under `ENV_LOCK` and a leaked var would poison
    /// later tests. Takes ALL vars in one call (not nested calls to this
    /// same helper): `std::sync::Mutex` is not reentrant, so locking
    /// `ENV_LOCK` twice on one thread — which a nested call would do —
    /// deadlocks instead of erroring.
    fn with_env<R>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> R) -> R {
        let _guard = crate::test_support::ENV_LOCK.lock().unwrap();
        let prev: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (k, p) in prev {
            match p {
                Some(val) => std::env::set_var(&k, val),
                None => std::env::remove_var(&k),
            }
        }
        result.unwrap_or_else(|e| std::panic::resume_unwind(e))
    }

    #[test]
    fn for_rail_falls_back_to_global_mode_when_unset() {
        with_env(
            &[
                ("GLASSBOX_SAFETY_MODE", None),
                ("GLASSBOX_MODE", Some("enforce")),
            ],
            || {
                assert_eq!(Mode::for_rail("safety"), Mode::Enforce);
                assert_eq!(Mode::for_rail("values"), Mode::Enforce);
            },
        );
        with_env(
            &[("GLASSBOX_SAFETY_MODE", None), ("GLASSBOX_MODE", None)],
            || {
                assert_eq!(Mode::for_rail("safety"), Mode::Shadow);
            },
        );
    }

    #[test]
    fn for_rail_override_wins_over_global_default() {
        with_env(
            &[
                ("GLASSBOX_MODE", Some("shadow")),
                ("GLASSBOX_SAFETY_MODE", Some("enforce")),
            ],
            || {
                assert_eq!(
                    Mode::for_rail("safety"),
                    Mode::Enforce,
                    "override enforces safety"
                );
                assert_eq!(
                    Mode::for_rail("values"),
                    Mode::Shadow,
                    "values untouched, uses global"
                );
                assert_eq!(
                    Mode::for_rail("envelope"),
                    Mode::Shadow,
                    "envelope untouched, uses global"
                );
            },
        );
    }

    #[test]
    fn for_rail_is_case_insensitive_on_rail_name() {
        with_env(
            &[
                ("GLASSBOX_MODE", None),
                ("GLASSBOX_SAFETY_MODE", Some("enforce")),
            ],
            || {
                assert_eq!(Mode::for_rail("SAFETY"), Mode::Enforce);
                assert_eq!(Mode::for_rail("Safety"), Mode::Enforce);
            },
        );
    }

    #[test]
    fn resolve_is_fail_safe() {
        // Only an exact, trimmed, case-insensitive "enforce" flips the mode.
        assert_eq!(Mode::resolve(None), Mode::Shadow);
        assert_eq!(Mode::resolve(Some("")), Mode::Shadow);
        assert_eq!(Mode::resolve(Some("shadow")), Mode::Shadow);
        assert_eq!(Mode::resolve(Some("shdw")), Mode::Shadow); // typo → safe default
        assert_eq!(Mode::resolve(Some("ENFORCE!")), Mode::Shadow); // not exact → safe
        assert_eq!(Mode::resolve(Some("enforce")), Mode::Enforce);
        assert_eq!(Mode::resolve(Some("Enforce")), Mode::Enforce);
        assert_eq!(Mode::resolve(Some("  ENFORCE  ")), Mode::Enforce); // trimmed + lowered
    }

    #[test]
    fn shadow_never_denies() {
        // The headline invariant: across the full product of {blocked} × reasons,
        // Shadow always renders exactly "{}" and never a deny payload.
        let reasons = [
            "all rails clean",
            "safety rail: contains forbidden substring '--force'",
            "values rail: forbid when extracts(value())",
            "",
        ];
        for &blocked in &[true, false] {
            for reason in reasons {
                let out = resolve_output(Mode::Shadow, blocked, reason).render();
                assert_eq!(
                    out, "{}",
                    "shadow must defer (blocked={blocked}, reason={reason:?})"
                );
                assert!(
                    !out.contains("deny"),
                    "shadow output must never contain a deny"
                );
                assert!(!out.contains("permissionDecision"));
            }
        }
    }

    #[test]
    fn enforce_denies_only_when_blocked() {
        let denied = resolve_output(Mode::Enforce, true, "safety rail: rm -rf").render();
        assert!(denied.contains("\"permissionDecision\":\"deny\""));
        assert!(denied.contains("Glass Box governance — safety rail: rm -rf"));

        let allowed = resolve_output(Mode::Enforce, false, "all rails clean").render();
        assert_eq!(allowed, "{}");
    }
}
