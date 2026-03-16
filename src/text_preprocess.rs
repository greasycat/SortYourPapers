use std::collections::{HashMap, HashSet};

const MAX_PHRASES: usize = 64;
const MAX_WORDS: usize = 128;
const MIN_TERM_LEN: usize = 3;

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

fn normalize_text(raw: &str) -> String {
    let replaced = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', " ");

    let chars = replaced.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(replaced.len());
    let mut index = 0usize;

    while index < chars.len() {
        let current = chars[index];

        if current == '\u{00ad}' {
            index += 1;
            continue;
        }

        if current == '-'
            && chars.get(index + 1) == Some(&'\n')
            && chars
                .get(index.wrapping_sub(1))
                .copied()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && chars
                .get(index + 2)
                .copied()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            index += 2;
            continue;
        }

        if current.is_whitespace() {
            if current == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
        } else {
            out.push(current);
        }

        index += 1;
    }

    out
}

fn prepare_lines(input: &str) -> Vec<String> {
    let raw_lines = input.lines().map(clean_line).collect::<Vec<_>>();

    let counts = raw_lines.iter().fold(HashMap::new(), |mut acc, line| {
        if !line.is_empty() {
            *acc.entry(normalize_line_for_match(line)).or_insert(0usize) += 1;
        }
        acc
    });

    let mut filtered = Vec::new();
    let mut saw_references = false;

    for line in raw_lines {
        if saw_references {
            break;
        }

        if line.is_empty() {
            if filtered
                .last()
                .is_some_and(|last: &String| !last.is_empty())
            {
                filtered.push(String::new());
            }
            continue;
        }

        let normalized = normalize_line_for_match(&line);
        if should_drop_line(
            &line,
            &normalized,
            counts.get(&normalized).copied().unwrap_or(0),
        ) {
            continue;
        }

        if is_reference_heading(&normalized) {
            saw_references = true;
            continue;
        }

        filtered.push(line);
    }

    merge_wrapped_lines(filtered)
}

fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last_was_space = false;

    for ch in line.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }

    out
}

fn normalize_line_for_match(line: &str) -> String {
    line.chars()
        .filter_map(|c| {
            if c.is_ascii_alphabetic() || c.is_ascii_whitespace() {
                Some(c.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn should_drop_line(line: &str, normalized: &str, occurrences: usize) -> bool {
    if normalized.is_empty() {
        return true;
    }

    if is_page_number_line(line) {
        return true;
    }

    if contains_boilerplate(normalized) {
        return true;
    }

    occurrences >= 2 && looks_like_repeated_header_footer(line, normalized)
}

fn is_page_number_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let compact = lower
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();

    compact.chars().all(|c| c.is_ascii_digit())
        || compact
            .strip_prefix("page")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        || compact
            .chars()
            .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'))
}

fn contains_boilerplate(normalized: &str) -> bool {
    normalized.contains("all rights reserved")
        || normalized.contains("downloaded from")
        || normalized.contains("creative commons")
        || normalized.contains("preprint")
        || normalized.contains("copyright")
}

fn looks_like_repeated_header_footer(line: &str, normalized: &str) -> bool {
    let word_count = normalized.split_whitespace().count();
    let char_count = line.chars().count();
    let has_digit = line.chars().any(|c| c.is_ascii_digit());
    let has_header_keyword = normalized.contains("conference")
        || normalized.contains("proceedings")
        || normalized.contains("journal")
        || normalized.contains("arxiv")
        || normalized.contains("doi");

    (has_digit || has_header_keyword) && char_count <= 80 && word_count <= 8
}

fn is_reference_heading(normalized: &str) -> bool {
    matches!(
        normalized,
        "references" | "bibliography" | "works cited" | "reference"
    )
}

fn merge_wrapped_lines(lines: Vec<String>) -> Vec<String> {
    let mut merged = Vec::new();
    let mut current = String::new();

    for line in lines {
        if line.is_empty() {
            if !current.is_empty() {
                merged.push(current.trim().to_string());
                current.clear();
            }
            continue;
        }

        if current.is_empty() {
            current = line;
            continue;
        }

        if should_join_lines(&current, &line) {
            current.push(' ');
            current.push_str(&line);
        } else {
            merged.push(current.trim().to_string());
            current = line;
        }
    }

    if !current.is_empty() {
        merged.push(current.trim().to_string());
    }

    merged
}

fn should_join_lines(current: &str, next: &str) -> bool {
    if looks_like_heading(current) || looks_like_heading(next) {
        return false;
    }

    let ends_with_terminal = current
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | ':'));
    let next_starts_lower = next.chars().next().is_some_and(|c| c.is_ascii_lowercase());

    !ends_with_terminal || next_starts_lower
}

fn looks_like_heading(line: &str) -> bool {
    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() || words.len() > 8 {
        return false;
    }

    if line
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | ',' | ';'))
    {
        return false;
    }

    let alpha_words = words
        .iter()
        .filter(|word| word.chars().any(|c| c.is_ascii_alphabetic()))
        .count();
    if alpha_words == 0 {
        return false;
    }

    let heading_like = words
        .iter()
        .filter(|word| {
            let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            cleaned
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
                || cleaned.chars().all(|c| c.is_ascii_uppercase())
        })
        .count();

    heading_like * 2 >= alpha_words
}

fn extract_terms_from_line(
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

fn alpha_tokens(input: &str) -> Vec<String> {
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

fn format_terms(phrases: &[String], words: &[String]) -> String {
    let mut sections = Vec::new();

    if !phrases.is_empty() {
        sections.push(phrases.join(", "));
    }
    if !words.is_empty() {
        sections.push(words.join(", "));
    }

    sections.join("\n")
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "about"
            | "above"
            | "after"
            | "again"
            | "against"
            | "all"
            | "also"
            | "am"
            | "an"
            | "and"
            | "any"
            | "are"
            | "as"
            | "at"
            | "be"
            | "because"
            | "been"
            | "before"
            | "being"
            | "below"
            | "between"
            | "both"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "doing"
            | "down"
            | "during"
            | "each"
            | "few"
            | "for"
            | "from"
            | "further"
            | "had"
            | "has"
            | "have"
            | "having"
            | "he"
            | "her"
            | "here"
            | "hers"
            | "herself"
            | "him"
            | "himself"
            | "his"
            | "how"
            | "however"
            | "i"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "itself"
            | "let"
            | "me"
            | "more"
            | "most"
            | "my"
            | "myself"
            | "nor"
            | "of"
            | "on"
            | "once"
            | "only"
            | "or"
            | "other"
            | "our"
            | "ours"
            | "ourselves"
            | "out"
            | "over"
            | "own"
            | "same"
            | "she"
            | "should"
            | "so"
            | "some"
            | "such"
            | "than"
            | "that"
            | "the"
            | "their"
            | "theirs"
            | "them"
            | "themselves"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "through"
            | "to"
            | "too"
            | "under"
            | "until"
            | "up"
            | "very"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "who"
            | "whom"
            | "why"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
            | "yours"
            | "yourself"
            | "yourselves"
    )
}

#[cfg(test)]
mod tests {
    use super::preprocess_for_llm;

    #[test]
    fn removes_numbers_symbols_and_stopwords() {
        let text = "Graph neural network 2024 improves accuracy by 12.5% on Cora.";
        let processed = preprocess_for_llm(text);

        assert!(processed.contains("graph neural network"));
        assert!(processed.contains("improves"));
        assert!(processed.contains("accuracy"));
        assert!(!processed.contains("2024"));
        assert!(!processed.contains('%'));
        assert!(!processed.contains(" by "));
    }

    #[test]
    fn repairs_hyphenation_and_deduplicates_terms() {
        let text = "multi-\nmodal learning enables multi-\nmodal learning";
        let processed = preprocess_for_llm(text);

        assert!(processed.contains("multimodal learning enables"));
        assert_eq!(processed.matches("multimodal").count(), 1);
    }

    #[test]
    fn removes_repeated_headers_and_page_numbers() {
        let text =
            "Conference 2024\n1\nGraph Neural Networks\nConference 2024\n2\nGraph Neural Networks";
        let processed = preprocess_for_llm(text);

        assert!(!processed.contains("conference"));
        assert!(!processed.contains("\n1"));
        assert!(processed.contains("graph neural networks"));
    }

    #[test]
    fn drops_references_section() {
        let text = "Abstract\nGraph neural networks for molecules.\n\nReferences\n[1] Smith 2020";
        let processed = preprocess_for_llm(text);

        assert!(processed.contains("graph neural networks"));
        assert!(!processed.contains("smith"));
    }

    #[test]
    fn discards_terms_with_two_or_fewer_characters() {
        let text = "AI for CV in 3D object detection and rl agents";
        let processed = preprocess_for_llm(text);

        assert!(processed.contains("object"));
        assert!(processed.contains("detection"));
        assert!(processed.contains("agents"));
        assert!(!processed.contains("ai"));
        assert!(!processed.contains("cv"));
        assert!(!processed.contains("rl"));
    }
}
