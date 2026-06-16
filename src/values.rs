//! The values rail — refuses what is *wrong* (extractive, unfair, repricing a
//! loyal client). Governed by the Conscience markdown in the mind layer.
//!
//! Two design choices that make it safe to run on the live agent:
//!   1. A cheap in-process keyword pre-screen — the Tessera subprocess only runs
//!      when the action plausibly touches values, so the 99% of commands with
//!      nothing to do with money pay nothing.
//!   2. FAIL-OPEN on infra error/timeout — a rare Tessera hiccup logs and allows
//!      rather than bricking the agent. (Safety, which never fails, stays the
//!      hard floor.)
//!
//! The Tessera subprocess sits behind the [`ConscienceOracle`] trait so the
//! fail-open path can be exercised in tests without a live Tessera.

use crate::gate::Verdict;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::Duration;

const VALUE_KEYWORDS: &[&str] = &[
    "reprice",
    "gouge",
    "squeeze",
    "upcharge",
    "upcharg",
    "charge",
    "invoice",
    "deposit",
    "refund",
    "payment",
    "price",
    "pricing",
    "discount",
    "markup",
    "loyal client",
    "extract",
];

/// A refusal from the values rail: the reason and the policy that fired.
pub struct Refusal {
    pub reason: String,
    pub policy: String,
}

/// The values rail's dependency on an external governance oracle. The production
/// impl shells out to Tessera; tests inject fakes to exercise the fail-open path
/// without a live Tessera.
pub trait ConscienceOracle: Send + Sync {
    /// `Some(Refusal)` on refusal; `None` on a clean pass **or any infra failure**
    /// — the fail-open contract lives in the impl, not the caller.
    fn consult(&self, action: &str, target: &str) -> Option<Refusal>;

    /// Short label for diagnostics (which oracle is governing).
    fn name(&self) -> &'static str {
        "oracle"
    }
}

/// The native, dependency-free conscience. Encodes the same moral-foundations
/// judgment as the Tessera `conscience.t.md` (extraction, unfair markup on a
/// counterparty, repricing a loyal relationship, deception) directly in Rust, so
/// the values rail works on a fresh `git clone` with **no Tessera and no Python**.
/// It is the default oracle whenever a Tessera install isn't found on disk.
///
/// This runs only after the keyword pre-screen in [`check_with`], so it sees just
/// the small slice of actions that plausibly touch money or a relationship.
pub struct BuiltinOracle;

impl ConscienceOracle for BuiltinOracle {
    fn consult(&self, action: &str, target: &str) -> Option<Refusal> {
        let hay = format!("{action} {target}").to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| hay.contains(w));

        // care — deception/fraud is wrong regardless of who it targets.
        if has(&["defraud", "deceive", "scam", "mislead", "trick"]) {
            return Some(refusal("care", "deceiving the counterparty"));
        }
        // fairness — inherently extractive verbs are wrong on their face.
        if has(&["gouge", "price gouge", "exploit", "squeeze", "overcharge"]) {
            return Some(refusal("fairness", "extractive: takes more than is fair"));
        }
        // loyalty — don't reprice/raise on a loyal relationship.
        if has(&["loyal"])
            && has(&[
                "reprice", "raise", "increase", "hike", "bump", "rate", "price",
            ])
        {
            return Some(refusal("loyalty", "repricing a loyal relationship"));
        }
        // fairness — marking up a known counterparty (homeowner/customer/client).
        if has(&["homeowner", "customer", "client", "tenant"])
            && has(&["markup", "upcharge", "inflate", "pad the", "hidden fee"])
        {
            return Some(refusal("fairness", "unfair markup on the counterparty"));
        }
        None
    }

    fn name(&self) -> &'static str {
        "native"
    }
}

/// A refusal tagged with the moral foundation it offends. `NoExtraction` matches
/// the policy name the Tessera conscience reports, so provenance and cards read
/// identically whichever oracle fired.
fn refusal(foundation: &str, why: &str) -> Refusal {
    Refusal {
        reason: format!("forbid when extracts(value()) — {foundation}: {why}"),
        policy: "NoExtraction".into(),
    }
}

/// Pick the values oracle once, by what's actually installed: the richer Tessera
/// conscience if both its binary and the conscience file exist on disk, otherwise
/// the native [`BuiltinOracle`]. Zero-config — a contributor with Tessera gets it
/// automatically; everyone else gets a working values rail with no setup.
fn select_oracle() -> Box<dyn ConscienceOracle> {
    let t = TesseraOracle::from_env();
    if Path::new(&t.bin).exists() && Path::new(&t.agent).exists() {
        Box::new(t)
    } else {
        Box::new(BuiltinOracle)
    }
}

/// The process-wide active oracle, chosen once (the on-disk probe in
/// [`select_oracle`] does not repeat on the hot path).
pub fn active_oracle() -> &'static dyn ConscienceOracle {
    static ORACLE: OnceLock<Box<dyn ConscienceOracle>> = OnceLock::new();
    ORACLE.get_or_init(select_oracle).as_ref()
}

/// Name of the active oracle (`"native"` or `"Tessera"`) for diagnostics.
pub fn active_oracle_name() -> &'static str {
    active_oracle().name()
}

/// Production oracle: spawn `tessera compile … --run Conscience` with a timeout.
pub struct TesseraOracle {
    pub bin: String,
    pub agent: String,
    pub timeout: Duration,
}

impl TesseraOracle {
    /// HOME-derived defaults, overridable via `GLASSBOX_TESSERA_BIN` /
    /// `GLASSBOX_CONSCIENCE`. The overrides also make the fail-open path reachable
    /// in tests (point the binary at a nonexistent path → spawn error → None).
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let bin = std::env::var("GLASSBOX_TESSERA_BIN")
            .unwrap_or_else(|_| format!("{home}/Projects/walt/tessera/.venv/bin/tessera"));
        let agent = std::env::var("GLASSBOX_CONSCIENCE")
            .unwrap_or_else(|_| format!("{home}/Projects/walt/mind/conscience.t.md"));
        TesseraOracle {
            bin,
            agent,
            timeout: Duration::from_secs(8),
        }
    }
}

impl ConscienceOracle for TesseraOracle {
    fn consult(&self, action: &str, target: &str) -> Option<Refusal> {
        let bin = self.bin.clone();
        let agent = self.agent.clone();
        let a = action.to_string();
        let t = target.to_string();
        let timeout = self.timeout;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let set_action = format!("action={a}");
            let set_target = format!("target={t}");
            let out = Command::new(&bin)
                .args([
                    "compile",
                    agent.as_str(),
                    "--run",
                    "Conscience",
                    "--set",
                    set_action.as_str(),
                    "--set",
                    set_target.as_str(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            let _ = tx.send(out);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(output)) => {
                let s = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                parse_refusal(&s).map(|(reason, policy)| Refusal { reason, policy })
            }
            _ => {
                eprintln!("glassbox: values rail unavailable — failing open");
                None
            }
        }
    }

    fn name(&self) -> &'static str {
        "Tessera"
    }
}

/// Command leaders that are dev / inspection / navigation. They author, inspect,
/// build, or commit code — they never MOVE money, even when the text mentions an
/// invoice or a payout. The values rail skips any action whose every shell segment
/// leads with one of these, so editing `payments.ts` or grepping `listPayouts` no
/// longer trips `action_class("payments")`. A real money action (`charge…`,
/// `reprice…`, a `curl` to a billing API) does not lead with a dev verb, so it is
/// still screened. This keys the rail on the *verb*, not a substring.
const DEV_LEADERS: &[&str] = &[
    // Edit/Write/Read/Grep/Glob adapters render actions that lead with these verbs.
    "edit",
    "write",
    "read",
    "grep",
    "glob",
    "notebookedit",
    // shell builtins / navigation / file ops
    "cd",
    "pwd",
    "echo",
    "printf",
    "export",
    "set",
    "env",
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "tree",
    "stat",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "diff",
    "find",
    "fd",
    "which",
    "type",
    "basename",
    "dirname",
    "realpath",
    "mkdir",
    "rmdir",
    "cp",
    "mv",
    "touch",
    "ln",
    "chmod",
    "tee",
    "xargs",
    "jq",
    "yq",
    // vcs / build / package / language tooling
    "git",
    "gh",
    "rg",
    "ag",
    "sed",
    "awk",
    "npm",
    "pnpm",
    "yarn",
    "npx",
    "cargo",
    "rustc",
    "rustup",
    "go",
    "python",
    "python3",
    "pip",
    "pip3",
    "node",
    "deno",
    "bun",
    "ruby",
    "java",
    "mvn",
    "gradle",
    "make",
    "cmake",
    "docker",
    "kubectl",
    "helm",
    "terraform",
    "brew",
    "vim",
    "nvim",
    "nano",
    "code",
    // walt tooling
    "tessera",
    "glassbox",
    "aeon",
    "vault",
];

/// True if every shell segment of `action` leads with a dev/inspection command —
/// i.e. this is code work, not a real-world money action. Splits on the shell
/// separators (`&& || ; |` and newlines) and checks each segment's leading token
/// (path-stripped), so a compound `cd … && git grep … invoice` is recognized as
/// all-dev. An empty action is not dev (let the keyword pre-screen decide).
fn is_dev_action(action: &str) -> bool {
    let mut saw_segment = false;
    for seg in action.split(['\n', ';', '|', '&']) {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        // Skip leading env-var assignments (`FOO=bar git commit`) to find the verb.
        let leader = seg
            .split_whitespace()
            .find(|tok| !is_env_assignment(tok))
            .unwrap_or("");
        if leader.is_empty() {
            continue; // a pure `FOO=bar` segment sets state; it isn't a money action
        }
        saw_segment = true;
        let leader = leader.rsplit('/').next().unwrap_or(leader); // /usr/bin/git → git
        if !DEV_LEADERS.contains(&leader.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    saw_segment
}

/// `KEY=VALUE` where KEY is a valid shell identifier.
fn is_env_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false)
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Keyword pre-screen, then consult the oracle. Callers reach this via
/// [`crate::gate::evaluate`] (production) or directly with a fake oracle (tests).
pub fn check_with(action: &str, target: &str, oracle: &dyn ConscienceOracle) -> Verdict {
    // Dev/inspection work never moves money — skip the values rail entirely so
    // payments-feature development doesn't false-positive. (Safety still runs.)
    if is_dev_action(action) {
        return clean();
    }
    let hay = format!("{action} {target}").to_lowercase();
    if !VALUE_KEYWORDS.iter().any(|k| hay.contains(k)) {
        return clean();
    }
    match oracle.consult(action, target) {
        Some(r) => Verdict {
            rail: "values".into(),
            refused: true,
            reason: r.reason,
            policy: r.policy,
        },
        None => clean(), // clean verdict OR fail-open on infra error
    }
}

fn clean() -> Verdict {
    Verdict {
        rail: "values".into(),
        refused: false,
        reason: "clean".into(),
        policy: String::new(),
    }
}

/// Parse `Refusal(reason=<q>...<q>, policy=<q>...<q>)` where <q> is ' or ".
/// Tessera's repr-style quoting guarantees the delimiter never appears inside,
/// so matching the next same-quote closes correctly.
pub(crate) fn parse_refusal(s: &str) -> Option<(String, String)> {
    let after_reason = &s[s.find("Refusal(reason=")? + "Refusal(reason=".len()..];
    let (reason, rest) = read_quoted(after_reason)?;
    let after_policy = &rest[rest.find("policy=")? + "policy=".len()..];
    let (policy, _) = read_quoted(after_policy)?;
    Some((reason, policy))
}

/// Read a quoted string at the start of `s`, returning (contents, remainder).
pub(crate) fn read_quoted(s: &str) -> Option<(String, &str)> {
    let q = s.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    let body = &s[q.len_utf8()..];
    let end = body.find(q)?;
    Some((body[..end].to_string(), &body[end + q.len_utf8()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Injectable fakes for the fail-open / pre-screen properties ----------

    struct AlwaysRefuse;
    impl ConscienceOracle for AlwaysRefuse {
        fn consult(&self, _a: &str, _t: &str) -> Option<Refusal> {
            Some(Refusal {
                reason: "forbid when extracts(value())".into(),
                policy: "NoExtraction".into(),
            })
        }
    }

    struct AlwaysClean;
    impl ConscienceOracle for AlwaysClean {
        fn consult(&self, _a: &str, _t: &str) -> Option<Refusal> {
            None
        }
    }

    /// Simulates a Tessera timeout/spawn error: the contract says return None.
    struct AlwaysFail;
    impl ConscienceOracle for AlwaysFail {
        fn consult(&self, _a: &str, _t: &str) -> Option<Refusal> {
            None
        }
    }

    /// Panics if consulted — proves the pre-screen short-circuits.
    struct PanicOracle;
    impl ConscienceOracle for PanicOracle {
        fn consult(&self, _a: &str, _t: &str) -> Option<Refusal> {
            panic!("oracle must not be consulted for a dev/non-values action");
        }
    }

    #[test]
    fn dev_commands_skip_the_values_rail() {
        // Editing/committing/grepping money-adjacent CODE is not moving money —
        // the oracle must never be consulted (PanicOracle would blow up). These are
        // the exact false-positives shadow dogfooding surfaced.
        assert!(
            !check_with(
                "git commit -m 'add charge + pricing endpoints'",
                "shell",
                &PanicOracle
            )
            .refused
        );
        assert!(!check_with("grep -rn listPayouts invoice src/", "shell", &PanicOracle).refused);
        assert!(
            !check_with(
                "cd ~/app && git grep deposit | head && git push",
                "shell",
                &PanicOracle
            )
            .refused
        );
        assert!(!check_with("edit src/payments.ts", "src/payments.ts", &PanicOracle).refused);
    }

    #[test]
    fn real_money_intent_is_still_screened() {
        // Leads with a non-dev verb → screened → reaches the oracle → refused.
        assert!(
            check_with(
                "charge the homeowner deposit invoice",
                "homeowner",
                &AlwaysRefuse
            )
            .refused
        );
        assert!(
            check_with(
                "reprice loyal client to market",
                "loyal-client",
                &AlwaysRefuse
            )
            .refused
        );
    }

    #[test]
    fn is_dev_action_distinguishes_code_from_intent() {
        assert!(is_dev_action("git commit -m 'pricing'"));
        assert!(is_dev_action("cd ~/x && grep invoice . | head"));
        assert!(is_dev_action("/usr/bin/git push")); // path-stripped leader
        assert!(is_dev_action("FOO=bar git commit -m x")); // env-prefix before the verb
        assert!(is_dev_action("RUST_LOG=debug cargo test"));
        assert!(!is_dev_action("charge the customer"));
        assert!(!is_dev_action("reprice loyal client"));
        assert!(!is_dev_action("")); // empty → not dev; let the keyword screen decide
        assert!(!is_dev_action("cd /tmp && curl https://api/charge")); // a non-dev segment disqualifies
    }

    #[test]
    fn refusal_is_surfaced() {
        let v = check_with("reprice loyal client", "loyal-client", &AlwaysRefuse);
        assert!(v.refused);
        assert_eq!(v.rail, "values");
        assert_eq!(v.policy, "NoExtraction");
    }

    #[test]
    fn clean_pass_allows() {
        let v = check_with("charge a fair rate", "stranger", &AlwaysClean);
        assert!(!v.refused);
    }

    #[test]
    fn builtin_oracle_matches_conscience_without_tessera() {
        // The dependency-free oracle must reproduce the conscience.t.md judgment:
        // refuse extraction / loyalty violations, pass fair business.
        let refuse = |a: &str, t: &str| check_with(a, t, &BuiltinOracle).refused;

        // Refuse — extractive or loyalty-violating.
        assert!(refuse(
            "reprice loyal client to market rate",
            "loyal-client"
        ));
        assert!(refuse(
            "gouge the homeowner on materials markup",
            "homeowner"
        ));
        assert!(refuse("squeeze the customer with a hidden fee", "customer"));
        assert!(refuse("defraud the client on the invoice", "client"));

        // Pass — fair business, not over-broad.
        assert!(!refuse("reprice the SaaS tier for new signups", "market"));
        assert!(!refuse("issue a refund to the customer", "customer"));
        assert!(!refuse("charge the standard deposit", "homeowner"));

        assert_eq!(BuiltinOracle.name(), "native");
    }

    #[test]
    fn fail_open_allows() {
        // A values-relevant action whose oracle errors must still pass (allow).
        let v = check_with("charge the deposit invoice", "homeowner", &AlwaysFail);
        assert!(!v.refused, "infra failure must fail OPEN, not block");
    }

    #[test]
    fn pre_screen_skips_oracle_for_non_values() {
        // PanicOracle would blow up if consulted; a non-values action must skip it.
        let v = check_with("git status", "shell", &PanicOracle);
        assert!(!v.refused);
    }

    #[test]
    fn parse_refusal_single_quoted() {
        let s = "Refusal(reason='forbid when extracts(value())', policy='NoExtraction')";
        let (reason, policy) = parse_refusal(s).unwrap();
        assert_eq!(reason, "forbid when extracts(value())");
        assert_eq!(policy, "NoExtraction");
    }

    #[test]
    fn parse_refusal_double_quoted_when_reason_has_apostrophe() {
        // Python !r flips to double quotes when the body contains a single quote.
        let s = "Refusal(reason=\"don't reprice a loyal client\", policy='NoExtraction')";
        let (reason, policy) = parse_refusal(s).unwrap();
        assert_eq!(reason, "don't reprice a loyal client");
        assert_eq!(policy, "NoExtraction");
    }

    #[test]
    fn parse_refusal_reason_with_comma_reads_to_closing_quote() {
        let s = "Refusal(reason='unfair, extractive', policy='NoExtraction')";
        let (reason, policy) = parse_refusal(s).unwrap();
        assert_eq!(reason, "unfair, extractive");
        assert_eq!(policy, "NoExtraction");
    }

    #[test]
    fn parse_refusal_clean_output_is_none() {
        let s = "PROPOSED ACTION: draft a fair estimate | TARGET: stranger lead";
        assert!(parse_refusal(s).is_none());
    }

    #[test]
    fn parse_refusal_truncated_without_policy_is_none() {
        let s = "Refusal(reason='something'"; // no policy
        assert!(parse_refusal(s).is_none());
    }

    #[test]
    fn read_quoted_handles_edge_cases() {
        assert!(read_quoted("not quoted").is_none());
        let (body, rest) = read_quoted("'', tail").unwrap();
        assert_eq!(body, "");
        assert_eq!(rest, ", tail");
        // Multibyte body must not split a char boundary.
        let (body, _) = read_quoted("'café'").unwrap();
        assert_eq!(body, "café");
    }
}
