pub(super) fn normalize_text(raw: &str) -> String {
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
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            && chars
                .get(index + 2)
                .copied()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
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
