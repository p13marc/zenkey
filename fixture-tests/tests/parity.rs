//! Generated-vs-interpretive parity (issue #7): the compiled `Subject::parse`
//! arms and the runtime `zenkey::pattern::best_match` must select the same
//! pattern for every tail — over the whole ZenSight registry corpus, with
//! cross-pattern samples (every pattern's tails tested against every other
//! pattern of the same class). This is the drift-lock: the two matchers share
//! precedence code since v1.5, and this test keeps them shared.

use zenkey::grammar::Class;
use zenkey::pattern::{PatternChunk, SubjectPattern, best_match};
use zenkey_fixture_tests::registry;

/// Sample concrete chunks a variable might bind — including tricky shapes
/// (metric-ish, ip-slug-ish, literal-colliding).
const VAR_SAMPLES: &[&str] = &["p95_ms", "10-0-0-7", "x1", "health", "self"];

fn synthesize_tails(pattern: &SubjectPattern) -> Vec<Vec<String>> {
    // One tail per var-sample choice (all vars share the sample per tail —
    // enough for precedence coverage without a combinatorial explosion),
    // and 1..=3 rest lengths.
    let mut tails = Vec::new();
    for sample in VAR_SAMPLES {
        for rest_len in 1..=3usize {
            let mut tail: Vec<String> = Vec::new();
            for c in pattern.chunks() {
                match c {
                    PatternChunk::Literal(l) => tail.push(l.clone()),
                    PatternChunk::Var(_) => tail.push((*sample).to_string()),
                    PatternChunk::Rest(_) => {
                        for i in 0..rest_len {
                            tail.push(format!("{sample}.{i}"));
                        }
                    }
                }
            }
            tails.push(tail);
            if !pattern
                .chunks()
                .iter()
                .any(|c| matches!(c, PatternChunk::Rest(_)))
            {
                break; // rest_len only varies rest patterns
            }
        }
    }
    tails.sort();
    tails.dedup();
    tails
}

#[test]
fn generated_parse_agrees_with_best_match_over_the_corpus() {
    let mut checked = 0usize;
    for (name, toml_src) in registry::REGISTRIES {
        let slice = zenkey::parse_slice(toml_src).unwrap();
        for class in [Class::Telemetry, Class::State, Class::Events] {
            let class_str = class.chunk();
            let patterns: Vec<SubjectPattern> = slice
                .subjects
                .iter()
                .filter(|s| s.class == class_str)
                .map(|s| SubjectPattern::parse(&s.path).unwrap())
                .collect();
            if patterns.is_empty() {
                continue;
            }
            // Every pattern's synthesized tails, tested against the whole set.
            for p in &patterns {
                for tail in synthesize_tails(p) {
                    let refs: Vec<&str> = tail.iter().map(String::as_str).collect();
                    let generated = registry::parse_subject(name, class, &refs)
                        .map(|s| s.pattern().to_string());
                    let interpretive =
                        best_match(&patterns, &refs).map(|(i, _)| patterns[i].as_str().to_string());
                    assert_eq!(
                        generated, interpretive,
                        "{name}/{class_str}: divergence on tail {refs:?}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 500, "corpus coverage collapsed: {checked} checks");
}
