//! The GraphQL lexer, specification section 2: punctuators, names, numbers,
//! strings and block strings; commas, comments and whitespace are
//! insignificant and dropped.

/// One lexical token, with the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `!`, `$`, `&`, `(`, `)`, `:`, `=`, `@`, `[`, `]`, `{`, `|`, `}`.
    Punct(char),
    /// `...`
    Spread,
    Name(String),
    Int(String),
    Float(String),
    /// A string or block string, its raw content.
    Str(String),
}

impl Kind {
    /// The name where this is one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Kind::Name(name) => Some(name),
            _ => None,
        }
    }

    /// Whether this is the punctuator `c`.
    #[must_use]
    pub fn is(&self, c: char) -> bool {
        matches!(self, Kind::Punct(p) if *p == c)
    }
}

/// Tokenize `text`.
///
/// # Errors
/// An unterminated string, a character GraphQL has no token for, or a
/// number that does not end where a number ends.
pub fn lex(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line = 1;
    while at < chars.len() {
        let c = chars[at];
        match c {
            '\n' => {
                line += 1;
                at += 1;
            }
            ' ' | '\t' | '\r' | ',' | '\u{feff}' => at += 1,
            '#' => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            '.' => {
                if chars.get(at + 1) == Some(&'.') && chars.get(at + 2) == Some(&'.') {
                    tokens.push(Token {
                        kind: Kind::Spread,
                        line,
                    });
                    at += 3;
                } else {
                    return Err(format!("line {line}: a lone dot"));
                }
            }
            '!' | '$' | '&' | '(' | ')' | ':' | '=' | '@' | '[' | ']' | '{' | '|' | '}' => {
                tokens.push(Token {
                    kind: Kind::Punct(c),
                    line,
                });
                at += 1;
            }
            '"' => {
                let (kind, next, lines) = string(&chars, at, line)?;
                tokens.push(Token { kind, line });
                line += lines;
                at = next;
            }
            c if c == '_' || c.is_ascii_alphabetic() => {
                let start = at;
                while at < chars.len() && (chars[at] == '_' || chars[at].is_ascii_alphanumeric()) {
                    at += 1;
                }
                tokens.push(Token {
                    kind: Kind::Name(chars[start..at].iter().collect()),
                    line,
                });
            }
            c if c == '-' || c.is_ascii_digit() => {
                let (kind, next) = number(&chars, at, line)?;
                tokens.push(Token { kind, line });
                at = next;
            }
            other => return Err(format!("line {line}: {other:?} is not GraphQL")),
        }
    }
    Ok(tokens)
}

/// A string or block string starting at the quote at `at`; the token, the
/// index after it, and the lines it spanned.
fn string(chars: &[char], at: usize, line: usize) -> Result<(Kind, usize, usize), String> {
    if chars.get(at + 1) == Some(&'"') && chars.get(at + 2) == Some(&'"') {
        let mut i = at + 3;
        let mut lines = 0;
        while i < chars.len() {
            if chars[i] == '\\' && chars.get(i + 1..i + 4) == Some(&['"', '"', '"']) {
                i += 4;
                continue;
            }
            if chars.get(i..i + 3) == Some(&['"', '"', '"']) {
                let content: String = chars[at + 3..i].iter().collect();
                return Ok((Kind::Str(content), i + 3, lines));
            }
            if chars[i] == '\n' {
                lines += 1;
            }
            i += 1;
        }
        return Err(format!("line {line}: a block string that never closes"));
    }
    let mut i = at + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => {
                let content: String = chars[at + 1..i].iter().collect();
                return Ok((Kind::Str(content), i + 1, 0));
            }
            '\n' => {
                return Err(format!(
                    "line {line}: a string that runs onto the next line"
                ));
            }
            _ => i += 1,
        }
    }
    Err(format!("line {line}: a string that never closes"))
}

fn number(chars: &[char], at: usize, line: usize) -> Result<(Kind, usize), String> {
    let mut i = at;
    if chars[i] == '-' {
        i += 1;
    }
    let digits_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return Err(format!("line {line}: a minus with no digits"));
    }
    let mut float = false;
    if chars.get(i) == Some(&'.') {
        float = true;
        i += 1;
        let fraction = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == fraction {
            return Err(format!("line {line}: a decimal point with no digits"));
        }
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        float = true;
        i += 1;
        if matches!(chars.get(i), Some('+' | '-')) {
            i += 1;
        }
        let exponent = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == exponent {
            return Err(format!("line {line}: an exponent with no digits"));
        }
    }
    if chars
        .get(i)
        .is_some_and(|c| *c == '_' || c.is_ascii_alphabetic() || *c == '.')
    {
        return Err(format!("line {line}: a number that runs into a name"));
    }
    let text: String = chars[at..i].iter().collect();
    Ok((
        if float {
            Kind::Float(text)
        } else {
            Kind::Int(text)
        },
        i,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_kind_lexes_and_the_insignificant_is_dropped() {
        let text = "query Q($id: ID!, $n: Float = -1.5e3) { # comment\n  user(id: $id) { ...F } }\n\"\"\"block\n\\\"\"\" text\"\"\" \"plain \\\" str\" 42";
        let tokens = lex(text).expect("lex");
        let kinds: Vec<&Kind> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(kinds[0], &Kind::Name("query".into()));
        assert_eq!(kinds[2], &Kind::Punct('('));
        assert_eq!(kinds[3], &Kind::Punct('$'));
        assert!(kinds.contains(&&Kind::Float("-1.5e3".into())));
        assert!(kinds.contains(&&Kind::Spread));
        assert!(kinds.contains(&&Kind::Str("block\n\\\"\"\" text".into())));
        assert!(kinds.contains(&&Kind::Str("plain \\\" str".into())));
        assert_eq!(kinds.last(), Some(&&Kind::Int("42".into())));
        let last = tokens.last().expect("last");
        assert_eq!(last.line, 4);
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, Kind::Name(n) if n == "comment"))
        );
    }

    #[test]
    fn what_is_not_graphql_is_refused_with_its_line() {
        assert!(lex("a . b").is_err(), "lone dot");
        assert!(lex("\"open").is_err(), "unterminated");
        assert!(lex("\"\"\"open").is_err(), "unterminated block");
        assert!(lex("\"two\nlines\"").is_err(), "newline in string");
        assert!(lex("1.").is_err(), "no fraction");
        assert!(lex("1e").is_err(), "no exponent");
        assert!(lex("-").is_err(), "no digits");
        assert!(lex("12abc").is_err(), "number into name");
        assert!(lex("a ~ b").is_err(), "tilde");
        let error = lex("ok\nok\n\"bad").expect_err("line");
        assert!(error.starts_with("line 3:"), "{error}");
    }
}
