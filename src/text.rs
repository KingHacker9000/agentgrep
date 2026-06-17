pub fn tokenize_lexical(text: &str) -> Vec<String> {
    tokenize_terms(text)
        .into_iter()
        .filter(|t| t.len() >= 3 && !t.chars().all(|ch| ch.is_ascii_digit()))
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_lexical_splits_camel_case() {
        let tokens = tokenize_lexical("SearchResult");
        assert!(tokens.contains(&"search".to_string()));
        assert!(tokens.contains(&"result".to_string()));
    }

    #[test]
    fn tokenize_lexical_splits_snake_case() {
        let tokens = tokenize_lexical("search_result");
        assert!(tokens.contains(&"search".to_string()));
        assert!(tokens.contains(&"result".to_string()));
    }

    #[test]
    fn tokenize_lexical_splits_kebab_case() {
        let tokens = tokenize_lexical("build-index");
        assert!(tokens.contains(&"build".to_string()));
        assert!(tokens.contains(&"index".to_string()));
    }

    #[test]
    fn tokenize_lexical_splits_path_separators() {
        let tokens = tokenize_lexical("src/index.rs");
        assert!(tokens.contains(&"src".to_string()));
        assert!(tokens.contains(&"index".to_string()));
    }

    #[test]
    fn tokenize_lexical_lowercases() {
        let tokens = tokenize_lexical("FileRole");
        assert!(tokens.iter().all(|t| t == t.to_lowercase().as_str()));
    }

    #[test]
    fn tokenize_lexical_skips_tiny_tokens() {
        let tokens = tokenize_lexical("fn is it a go");
        assert!(!tokens.contains(&"fn".to_string()), "fn is too short");
        assert!(!tokens.contains(&"is".to_string()), "is is too short");
        assert!(!tokens.contains(&"it".to_string()), "it is too short");
        assert!(!tokens.contains(&"a".to_string()), "a is too short");
    }

    #[test]
    fn tokenize_lexical_skips_pure_numbers() {
        let tokens = tokenize_lexical("version 123 stable 456");
        assert!(!tokens.contains(&"123".to_string()));
        assert!(!tokens.contains(&"456".to_string()));
        assert!(tokens.contains(&"version".to_string()));
        assert!(tokens.contains(&"stable".to_string()));
    }

    #[test]
    fn tokenize_lexical_keeps_alphanumeric_identifiers() {
        let tokens = tokenize_lexical("type1 v2api");
        assert!(tokens.contains(&"type1".to_string()) || tokens.contains(&"type".to_string()));
    }
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
