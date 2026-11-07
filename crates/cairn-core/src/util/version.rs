//! Loose version comparison, for dependency de-duplication and loader version sorting.
//! Segments are split at `.`, `-`, `_` and `+`. Numeric segments compare by value and sort after
//! alphabetic ones. Other segments compare lexicographically.

use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
enum Token<'a> {
    Num(u64),
    Text(&'a str),
}

fn tokens(s: &str) -> impl Iterator<Item = Token<'_>> {
    s.split(['.', '-', '_', '+', ' ']).filter(|t| !t.is_empty()).map(|t| match t.parse::<u64>() {
        Ok(n) => Token::Num(n),
        Err(_) => Token::Text(t),
    })
}

pub fn compare_loose(a: &str, b: &str) -> Ordering {
    let mut ta = tokens(a);
    let mut tb = tokens(b);
    loop {
        match (ta.next(), tb.next()) {
            (None, None) => return Ordering::Equal,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(x), Some(y)) => {
                let ord = match (&x, &y) {
                    (Token::Num(m), Token::Num(n)) => m.cmp(n),
                    (Token::Num(_), Token::Text(_)) => Ordering::Greater,
                    (Token::Text(_), Token::Num(_)) => Ordering::Less,
                    (Token::Text(s), Token::Text(t)) => s.cmp(t),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// Parses Java version strings such as `1.8.0_392`, `17.0.9`, `21` or `25-ea`.
pub fn parse_java_version(raw: &str) -> Option<(u32, u32, u32, u32)> {
    let raw = raw.trim().trim_matches('"');
    let mut nums = raw.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<u32>().unwrap_or(0));
    let first = nums.next()?;
    if first == 1 {
        // Old format such as `1.8.0_392`: the major version is the second segment.
        let major = nums.next()?;
        let minor = nums.next().unwrap_or(0);
        let patch = nums.next().unwrap_or(0);
        Some((major, minor, patch, 0))
    } else {
        let minor = nums.next().unwrap_or(0);
        let patch = nums.next().unwrap_or(0);
        let build = nums.next().unwrap_or(0);
        Some((first, minor, patch, build))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_segments_compare_by_value() {
        assert_eq!(compare_loose("2.9.4", "2.10.0"), Ordering::Less);
        assert_eq!(compare_loose("1.20.1", "1.20"), Ordering::Greater);
        assert_eq!(compare_loose("31.1.0-jre", "31.1.0-jre"), Ordering::Equal);
    }

    #[test]
    fn java_versions() {
        assert_eq!(parse_java_version("1.8.0_392"), Some((8, 0, 392, 0)));
        assert_eq!(parse_java_version("17.0.9"), Some((17, 0, 9, 0)));
        assert_eq!(parse_java_version("\"21\""), Some((21, 0, 0, 0)));
    }
}
