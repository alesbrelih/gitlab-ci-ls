pub struct ParserUtils {}

impl ParserUtils {
    pub fn strip_quotes(value: &str) -> &str {
        value.trim_matches('\'').trim_matches('"')
    }

    pub fn extract_word(line: &str, char_index: usize) -> Option<&str> {
        if char_index >= line.len() {
            return None;
        }

        let start = line[..char_index]
            .rfind(|c: char| c.is_whitespace())
            .map_or(0, |index| index + 1);

        let end = line[char_index..]
            .find(|c: char| c.is_whitespace())
            .map_or(line.len(), |index| index + char_index);

        Some(&line[start..end])
    }

    pub fn word_before_cursor(
        line: &str,
        char_index: usize,
        predicate: fn(c: char) -> bool,
    ) -> &str {
        if char_index == 0 || char_index > line.len() {
            return "";
        }

        let start = line[..char_index]
            .rfind(predicate)
            .map_or(0, |index| index + 1);

        if start == char_index {
            return "";
        }

        &line[start..char_index]
    }

    pub fn word_after_cursor(line: &str, char_index: usize) -> &str {
        if char_index >= line.len() {
            return "";
        }

        let start = char_index;

        let end = line[start..]
            .char_indices()
            .find(|&(_, c)| c.is_whitespace())
            .map_or(line.len(), |(idx, _)| start + idx);

        &line[start..end]
    }
}
