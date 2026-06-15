pub fn tokenize_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();

    for (index, ch) in chars.iter().enumerate() {
        if is_separator(*ch) {
            push_term(&mut terms, &mut current);
            continue;
        }

        if should_split_before(&chars, index) {
            push_term(&mut terms, &mut current);
        }

        current.push(ch.to_ascii_lowercase());
    }

    push_term(&mut terms, &mut current);
    terms
}

pub fn normalize_phrase(text: &str) -> String {
    text.split_whitespace()
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn squash_identifier(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

pub fn shorten_snippet(text: &str, max_len: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > max_len {
        let keep = max_len.saturating_sub(3);
        let mut shortened = compact.chars().take(keep).collect::<String>();
        shortened.push_str("...");
        return shortened;
    }
    compact
}

fn push_term(terms: &mut Vec<String>, current: &mut String) {
    if current.len() >= 2 {
        terms.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '/' | '\\' | '.' | '_' | '-' | ':' | ' ')
}

fn should_split_before(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return false;
    }

    let prev = chars[index - 1];
    let current = chars[index];
    let next = chars.get(index + 1).copied();

    (prev.is_ascii_lowercase() && current.is_ascii_uppercase())
        || (prev.is_ascii_digit() && current.is_ascii_alphabetic())
        || (prev.is_ascii_alphabetic() && current.is_ascii_digit())
        || (prev.is_ascii_uppercase()
            && current.is_ascii_uppercase()
            && next.map(|ch| ch.is_ascii_lowercase()).unwrap_or(false))
}
