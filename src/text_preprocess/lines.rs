use std::collections::HashMap;

pub(super) fn prepare_lines(input: &str) -> Vec<String> {
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
        .filter_map(|ch| {
            if ch.is_ascii_alphabetic() || ch.is_ascii_whitespace() {
                Some(ch.to_ascii_lowercase())
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
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();

    compact.chars().all(|ch| ch.is_ascii_digit())
        || compact
            .strip_prefix("page")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
        || compact
            .chars()
            .all(|ch| matches!(ch, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'))
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
    let has_digit = line.chars().any(|ch| ch.is_ascii_digit());
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
        .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | ':'));
    let next_starts_lower = next
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase());

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
        .is_some_and(|ch| matches!(ch, '.' | ',' | ';'))
    {
        return false;
    }

    let alpha_words = words
        .iter()
        .filter(|word| word.chars().any(|ch| ch.is_ascii_alphabetic()))
        .count();
    if alpha_words == 0 {
        return false;
    }

    let heading_like = words
        .iter()
        .filter(|word| {
            let cleaned = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
            cleaned
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
                || cleaned.chars().all(|ch| ch.is_ascii_uppercase())
        })
        .count();

    heading_like * 2 >= alpha_words
}
