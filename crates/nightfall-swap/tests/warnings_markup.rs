//! J3 — both warnings must remain in the wallet markup.
//!
//! If someone deletes the warning frame and leaves a comment behind, this
//! fails. Mutation: blank either constant, or drop it from the view.
//!
//! The file this reads moved once already, when the swap view was split out
//! of `views.rs`. That was the test doing its job — it noticed markup had
//! gone from where it was watching. To keep it honest after a future move,
//! it searches the whole crate rather than one file, but still insists the
//! warnings appear in a *drawing* file: a constant referenced only from a
//! test would satisfy a naive grep while showing the user nothing.

use std::fs;
use std::path::Path;

fn view_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("nightfall-core/src");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).expect("nightfall-core/src must exist") {
        let path = entry.expect("readable entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name.starts_with("views") && name.ends_with(".rs") {
            out.push((name, fs::read_to_string(&path).expect("readable source")));
        }
    }
    assert!(
        !out.is_empty(),
        "no view sources found — did they move again?"
    );
    out
}

#[test]
fn core_wallet_swap_view_still_mentions_both_warnings() {
    let sources = view_sources();
    for needle in ["NOT_PRIVATE", "NO_NIGHT_REFUND"] {
        let found: Vec<&str> = sources
            .iter()
            .filter(|(_, body)| body.contains(needle))
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(
            !found.is_empty(),
            "{needle} appears in no view source. The warning is not on screen; \
             the files searched were: {:?}",
            sources.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }
}

#[test]
fn the_constants_themselves_still_say_the_thing() {
    assert!(nightfall_swap::warnings::NOT_PRIVATE.contains("not private"));
    assert!(
        nightfall_swap::warnings::NO_NIGHT_REFUND.contains("no NIGHT refund")
            || nightfall_swap::warnings::NO_NIGHT_REFUND.contains("stuck forever")
    );
}
