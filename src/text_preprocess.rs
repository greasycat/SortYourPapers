mod lines;
mod normalization;
mod stopwords;
mod terms;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use lines::prepare_lines;
use normalization::normalize_text;
use terms::{alpha_tokens, extract_terms_from_line, format_terms};

const MAX_PHRASES: usize = 64;
const MAX_WORDS: usize = 128;
const MIN_TERM_LEN: usize = 3;

#[must_use]
pub fn preprocess_for_llm(raw: &str) -> String {
    let normalized = normalize_text(raw);
    let lines = prepare_lines(&normalized);
    let prose = lines.join("\n");

    let mut seen_phrases = HashSet::new();
    let mut phrases = Vec::new();
    let mut used_words = HashSet::new();
    let mut seen_words = HashSet::new();
    let mut words = Vec::new();

    for line in &lines {
        extract_terms_from_line(
            line,
            &mut seen_phrases,
            &mut phrases,
            &mut used_words,
            &mut seen_words,
            &mut words,
        );

        if phrases.len() >= MAX_PHRASES && words.len() >= MAX_WORDS {
            break;
        }
    }

    if phrases.is_empty() && words.is_empty() {
        for token in alpha_tokens(&prose) {
            if token.len() >= MIN_TERM_LEN && seen_words.insert(token.clone()) {
                words.push(token);
                if words.len() >= MAX_WORDS {
                    break;
                }
            }
        }
    }

    format_terms(&phrases, &words)
}
