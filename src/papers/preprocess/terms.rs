use std::collections::HashSet;

use super::{MAX_PHRASES, MAX_WORDS, MIN_TERM_LEN, stopwords::is_stopword};

pub(super) fn extract_terms_from_line(
    line: &str,
    seen_phrases: &mut HashSet<String>,
    phrases: &mut Vec<String>,
    used_words: &mut HashSet<String>,
    seen_words: &mut HashSet<String>,
    words: &mut Vec<String>,
) {
    let tokens = alpha_tokens(line);
    let mut run = Vec::new();

    for token in tokens {
        if is_stopword(&token) {
            flush_run(
                &mut run,
                seen_phrases,
                phrases,
                used_words,
                seen_words,
                words,
            );
        } else {
            run.push(token);
        }
    }

    flush_run(
        &mut run,
        seen_phrases,
        phrases,
        used_words,
        seen_words,
        words,
    );
}

fn flush_run(
    run: &mut Vec<String>,
    seen_phrases: &mut HashSet<String>,
    phrases: &mut Vec<String>,
    used_words: &mut HashSet<String>,
    seen_words: &mut HashSet<String>,
    words: &mut Vec<String>,
) {
    if run.is_empty() {
        return;
    }

    let phrase_len = if run.len() >= 3 {
        3
    } else if run.len() >= 2 {
        2
    } else {
        1
    };

    if phrase_len >= 2 {
        let slice = &run[..phrase_len];
        if slice.iter().all(|word| word.len() >= MIN_TERM_LEN) && phrases.len() < MAX_PHRASES {
            let phrase = slice.join(" ");
            if seen_phrases.insert(phrase.clone()) {
                for word in slice {
                    used_words.insert(word.clone());
                }
                phrases.push(phrase);
            }
        }
    }

    for word in run.iter().skip(phrase_len.min(run.len())) {
        if word.len() >= MIN_TERM_LEN
            && !used_words.contains(word)
            && seen_words.insert(word.clone())
            && words.len() < MAX_WORDS
        {
            words.push(word.clone());
        }
    }

    run.clear();
}

pub(super) fn alpha_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        if ch.is_ascii_alphabetic() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

pub(super) fn format_terms(phrases: &[String], words: &[String]) -> String {
    let mut sections = Vec::new();

    if !phrases.is_empty() {
        sections.push(phrases.join(", "));
    }
    if !words.is_empty() {
        sections.push(words.join(", "));
    }

    sections.join("\n")
}
