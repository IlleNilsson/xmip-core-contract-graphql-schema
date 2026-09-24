//! The GraphQL lexer, specification section 2: punctuators, names, numbers,
//! strings and block strings; commas, comments and whitespace are
//! insignificant and dropped.

use codec::char_reader::CharReader;

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

/// Tokenize `text`. GraphQL's white space is tab and space only, and a name
/// is ASCII: any other character, U+00A0 among them, is refused with its
/// line.
///
/// # Errors
/// An unterminated string, a character GraphQL has no token for, or a
/// number that does not end where a number ends.
pub fn lex(text: &str) -> Result<Vec<Token>, String> {
    let mut reader = CharReader::new(text);
    let mut tokens = Vec::new();
    while let Some(c) = reader.peek() {
        let line = reader.line();
        let kind = match c {
            '\n' | ' ' | '\t' | '\r' | ',' | '\u{feff}' => {
                reader.bump();
                continue;
            }
            '#' => {
                reader.take_while(|c| c != '\n');
                continue;
            }
            '.' if reader.eat_str("...") => Kind::Spread,
            '.' => return Err(format!("line {line}: a lone dot")),
            '!' | '$' | '&' | '(' | ')' | ':' | '=' | '@' | '[' | ']' | '{' | '|' | '}' => {
                reader.bump();
                Kind::Punct(c)
            }
            '"' => string(&mut reader)?,
            c if c == '_' || c.is_ascii_alphabetic() => Kind::Name(
                reader
                    .take_while(|c| c == '_' || c.is_ascii_alphanumeric())
                    .to_string(),
            ),
            c if c == '-' || c.is_ascii_digit() => number(&mut reader)?,
            other => return Err(format!("line {line}: {other:?} is not GraphQL")),
        };
        tokens.push(Token { kind, line });
    }
    Ok(tokens)
}

/// A string or block string starting at the quote the reader is on, its raw
/// content.
fn string(reader: &mut CharReader<'_>) -> Result<Kind, String> {
    let line = reader.line();
    if reader.eat_str("\"\"\"") {
        let start = reader.offset();
        loop {
            if reader.eat_str("\\\"\"\"") {
                continue;
            }
            let end = reader.offset();
            if reader.eat_str("\"\"\"") {
                let content = reader.text().get(start..end).unwrap_or_default();
                return Ok(Kind::Str(content.to_string()));
            }
            if reader.bump().is_none() {
                return Err(format!("line {line}: a block string that never closes"));
            }
        }
    }
    reader.bump();
    let start = reader.offset();
    loop {
        let end = reader.offset();
        match reader.bump() {
            Some('\\') => {
                reader.bump();
            }
            Some('"') => {
                let content = reader.text().get(start..end).unwrap_or_default();
                return Ok(Kind::Str(content.to_string()));
            }
            Some('\n') => {
                return Err(format!(
                    "line {line}: a string that runs onto the next line"
                ));
            }
            Some(_) => {}
            None => return Err(format!("line {line}: a string that never closes")),
        }
    }
}

fn number(reader: &mut CharReader<'_>) -> Result<Kind, String> {
    let line = reader.line();
    let start = reader.offset();
    reader.eat('-');
    if reader.take_while(|c| c.is_ascii_digit()).is_empty() {
        return Err(format!("line {line}: a minus with no digits"));
    }
    let mut float = false;
    if reader.eat('.') {
        float = true;
        if reader.take_while(|c| c.is_ascii_digit()).is_empty() {
            return Err(format!("line {line}: a decimal point with no digits"));
        }
    }
    if reader.eat('e') || reader.eat('E') {
        float = true;
        let _ = reader.eat('+') || reader.eat('-');
        if reader.take_while(|c| c.is_ascii_digit()).is_empty() {
            return Err(format!("line {line}: an exponent with no digits"));
        }
    }
    if reader
        .peek()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic() || c == '.')
    {
        return Err(format!("line {line}: a number that runs into a name"));
    }
    let text = reader.since(start).to_string();
    Ok(if float {
        Kind::Float(text)
    } else {
        Kind::Int(text)
    })
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

    #[test]
    fn multibyte_text_lexes_in_strings_and_is_refused_elsewhere() {
        let tokens =
            lex("a(x: \"Zoë 名前\u{a0}\") # é\n\"\"\"größe\n\u{1f600}\"\"\" b").expect("lex");
        let kinds: Vec<&Kind> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(kinds[4], &Kind::Str("Zoë 名前\u{a0}".into()));
        assert_eq!(kinds[6], &Kind::Str("größe\n\u{1f600}".into()));
        assert_eq!(tokens.last().map(|t| t.line), Some(3));
        let error = lex("a\n\u{a0}b").expect_err("U+00A0 is not white space");
        assert!(error.starts_with("line 2:"), "{error}");
        assert!(lex("größe").is_err(), "a name is ASCII");
        assert!(lex("\"öpen\u{a0}").is_err(), "unterminated");
        assert!(lex("1é").is_err(), "a number into a letter");
    }
}
