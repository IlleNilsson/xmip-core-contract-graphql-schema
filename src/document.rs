//! A GraphQL document read into its definitions: operations and fragments
//! of an executable document, type system definitions of a schema — enough
//! to say it is sound, which types it defines with which fields, and which
//! fields each operation selects at its root.

use crate::lexer::{Kind, Token, lex};

/// One top-level definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Definition {
    /// `query`, `mutation` or `subscription`, named or not, and the fields
    /// it selects at its root.
    Operation {
        kind: String,
        name: Option<String>,
        fields: Vec<String>,
    },
    /// `fragment Name on Type`.
    Fragment { name: String, on: String },
    /// `schema { query: Q ... }`: the root operation types it names.
    Schema { roots: Vec<(String, String)> },
    /// `type`, `interface`, `input`, `enum`, `union`, `scalar`, and their
    /// `extend` forms; the fields where the body has any.
    Type {
        keyword: String,
        name: String,
        fields: Vec<String>,
    },
    /// `directive @name on ...`.
    Directive { name: String },
}

/// Read `text` as a document.
///
/// # Errors
/// A lexical error, or a definition that is not one GraphQL has.
pub fn parse(text: &str) -> Result<Vec<Definition>, String> {
    let tokens = lex(text)?;
    let mut parser = Parser { tokens, at: 0 };
    let mut definitions = Vec::new();
    while !parser.done() {
        definitions.push(parser.definition()?);
    }
    if definitions.is_empty() {
        return Err("a document with no definition".to_string());
    }
    Ok(definitions)
}

pub(crate) struct Parser {
    pub(crate) tokens: Vec<Token>,
    pub(crate) at: usize,
}

impl Parser {
    pub(crate) fn done(&self) -> bool {
        self.at >= self.tokens.len()
    }

    pub(crate) fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.at).map(|t| &t.kind)
    }

    pub(crate) fn line(&self) -> usize {
        self.tokens
            .get(self.at)
            .or(self.tokens.last())
            .map_or(0, |t| t.line)
    }

    pub(crate) fn take(&mut self) -> Option<Kind> {
        let kind = self.tokens.get(self.at).map(|t| t.kind.clone());
        self.at += 1;
        kind
    }

    pub(crate) fn error(&self, what: &str) -> String {
        format!("line {}: {what}", self.line())
    }

    pub(crate) fn expect_name(&mut self, what: &str) -> Result<String, String> {
        match self.take() {
            Some(Kind::Name(name)) => Ok(name),
            _ => Err(self.error(&format!("expected {what}"))),
        }
    }

    pub(crate) fn eat(&mut self, c: char) -> bool {
        if self.peek().is_some_and(|k| k.is(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn definition(&mut self) -> Result<Definition, String> {
        if matches!(self.peek(), Some(Kind::Str(_))) {
            self.at += 1; // a description
        }
        if self.peek().is_some_and(|k| k.is('{')) {
            let fields = self.selection_set()?;
            return Ok(Definition::Operation {
                kind: "query".to_string(),
                name: None,
                fields,
            });
        }
        let keyword = self.expect_name("a definition")?;
        match keyword.as_str() {
            "query" | "mutation" | "subscription" => self.operation(keyword),
            "fragment" => {
                let name = self.expect_name("a fragment name")?;
                if self.expect_name("on")? != "on" {
                    return Err(self.error("expected on"));
                }
                let on = self.expect_name("a type condition")?;
                self.directives()?;
                self.selection_set()?;
                Ok(Definition::Fragment { name, on })
            }
            "extend" => {
                let keyword = self.expect_name("what to extend")?;
                self.type_system(keyword)
            }
            other => self.type_system(other.to_string()),
        }
    }

    fn operation(&mut self, kind: String) -> Result<Definition, String> {
        let name = match self.peek() {
            Some(Kind::Name(name)) => {
                let name = name.clone();
                self.at += 1;
                Some(name)
            }
            _ => None,
        };
        if self.peek().is_some_and(|k| k.is('(')) {
            self.balanced('(', ')')?;
        }
        self.directives()?;
        let fields = self.selection_set()?;
        Ok(Definition::Operation { kind, name, fields })
    }

    /// A selection set: the field names selected at its top level. Aliases
    /// are looked through, spreads skipped, nested sets consumed.
    fn selection_set(&mut self) -> Result<Vec<String>, String> {
        if !self.eat('{') {
            return Err(self.error("expected a selection set"));
        }
        let mut fields = Vec::new();
        loop {
            match self.take() {
                Some(Kind::Punct('}')) => break,
                Some(Kind::Spread) => {
                    if self.peek().and_then(Kind::name) == Some("on") {
                        self.at += 1;
                        self.expect_name("a type condition")?;
                    } else if matches!(self.peek(), Some(Kind::Name(_))) {
                        self.at += 1;
                    }
                    self.directives()?;
                    if self.peek().is_some_and(|k| k.is('{')) {
                        self.selection_set()?;
                    }
                }
                Some(Kind::Name(mut name)) => {
                    if self.eat(':') {
                        name = self.expect_name("a field after the alias")?;
                    }
                    fields.push(name);
                    if self.peek().is_some_and(|k| k.is('(')) {
                        self.balanced('(', ')')?;
                    }
                    self.directives()?;
                    if self.peek().is_some_and(|k| k.is('{')) {
                        self.selection_set()?;
                    }
                }
                _ => return Err(self.error("expected a field, a spread or }")),
            }
        }
        if fields.is_empty() && !self.saw_spread_before(self.at) {
            return Err(self.error("an empty selection set"));
        }
        Ok(fields)
    }

    /// Whether the selection set that just closed at `end` held a spread —
    /// a set of only spreads is not empty.
    fn saw_spread_before(&self, end: usize) -> bool {
        let mut depth = 0;
        for token in self.tokens[..end].iter().rev() {
            match &token.kind {
                Kind::Punct('}') => depth += 1,
                Kind::Punct('{') => {
                    if depth == 1 {
                        return false;
                    }
                    depth -= 1;
                }
                Kind::Spread if depth == 1 => return true,
                _ => {}
            }
        }
        false
    }

    /// Consume from `open` through its matching `close`, any nesting inside.
    pub(crate) fn balanced(&mut self, open: char, close: char) -> Result<(), String> {
        if !self.eat(open) {
            return Err(self.error(&format!("expected {open}")));
        }
        let mut depth = 1;
        while depth > 0 {
            match self.take() {
                Some(Kind::Punct(c)) if c == open => depth += 1,
                Some(Kind::Punct(c)) if c == close => depth -= 1,
                Some(_) => {}
                None => return Err(self.error(&format!("{open} never closed by {close}"))),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SCHEMA: &str = r#"
        "The schema"
        schema { query: Query mutation: Mutation }
        scalar DateTime @specifiedBy(url: "https://example")
        interface Node { id: ID! }
        type Order implements Node @key(fields: "id") {
            id: ID!
            "lines"
            lines(first: Int = 10, after: String): [Line!]!
            status: Status
        }
        type Line { sku: String! qty: Int! }
        enum Status { NEW PAID }
        union Result = | Order | Line
        input OrderInput { customer: String! lines: [LineInput!]! = [] }
        input LineInput { sku: String! qty: Int! }
        type Query { order(id: ID!): Order orders: [Order!]! }
        type Mutation { placeOrder(input: OrderInput!): Order }
        directive @key(fields: String!) repeatable on OBJECT | INTERFACE
        extend type Query { me: Node }
    "#;

    #[test]
    fn a_schema_reads_into_its_types_and_fields() {
        let definitions = parse(SCHEMA).expect("parse");
        assert_eq!(definitions.len(), 13);
        assert_eq!(
            definitions[0],
            Definition::Schema {
                roots: vec![
                    ("query".into(), "Query".into()),
                    ("mutation".into(), "Mutation".into())
                ]
            }
        );
        let order = definitions
            .iter()
            .find(|d| matches!(d, Definition::Type { name, .. } if name == "Order"));
        let Some(Definition::Type { fields, .. }) = order else {
            panic!("Order");
        };
        assert_eq!(fields, &["id", "lines", "status"]);
        assert!(definitions.iter().any(|d| matches!(d,
            Definition::Type { keyword, fields, .. } if keyword == "union" && fields.len() == 2)));
        assert!(definitions.iter().any(|d| matches!(d,
            Definition::Directive { name } if name == "key")));
        let last = definitions.last().expect("extend");
        assert!(matches!(last, Definition::Type { name, fields, .. }
            if name == "Query" && fields == &["me"]));
    }

    #[test]
    fn an_executable_document_reads_into_its_operations() {
        let text = r#"
            query Orders($first: Int!) @cached { orders(first: $first) { id ...LineFields } }
            mutation { placeOrder(input: {customer: "a", lines: []}) { id } }
            { first: order(id: "1") { id } ... on Query { me { id } } }
            fragment LineFields on Order { lines { sku qty } }
            subscription S { orderPlaced { id } }
        "#;
        let definitions = parse(text).expect("parse");
        assert_eq!(definitions.len(), 5);
        assert_eq!(
            definitions[0],
            Definition::Operation {
                kind: "query".into(),
                name: Some("Orders".into()),
                fields: vec!["orders".into()]
            }
        );
        assert!(
            matches!(&definitions[1], Definition::Operation { kind, name: None, fields }
            if kind == "mutation" && fields == &["placeOrder"])
        );
        assert!(
            matches!(&definitions[2], Definition::Operation { name: None, fields, .. }
            if fields == &["order"])
        );
        assert_eq!(
            definitions[3],
            Definition::Fragment {
                name: "LineFields".into(),
                on: "Order".into()
            }
        );
        assert!(parse("{ ...F }").is_ok(), "only a spread is not empty");
    }

    #[test]
    fn what_is_not_a_document_is_refused_with_its_line() {
        for (text, why) in [
            ("", "empty"),
            ("query { }", "empty selection"),
            ("query Q {", "unclosed"),
            ("type Order { id ID }", "no colon"),
            ("fragment F { id }", "no on"),
            ("directive key on FIELD", "no @"),
            ("banana Order { id: ID }", "keyword"),
            ("type Order { id: [ID }", "list unclosed"),
            ("query { a(b: }", "argument"),
        ] {
            assert!(parse(text).is_err(), "{why}");
        }
        let error = parse("type A { id: ID }\ntype B { id ID }").expect_err("line");
        assert!(error.starts_with("line 2:"), "{error}");
    }
}
