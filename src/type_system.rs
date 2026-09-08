//! The type system half of the document grammar: schema, scalar, type,
//! interface, input, enum, union and directive definitions, their `extend`
//! forms, and the field, argument and value shapes inside them. Read by the
//! same parser as the executable half in `document.rs`; split from it on
//! 2026-09-08 when one file held both and passed 400 lines.

use crate::document::{Definition, Parser};
use crate::lexer::Kind;

impl Parser {
    pub(crate) fn type_system(&mut self, keyword: String) -> Result<Definition, String> {
        match keyword.as_str() {
            "schema" => {
                self.directives()?;
                let roots = self.roots()?;
                Ok(Definition::Schema { roots })
            }
            "scalar" => {
                let name = self.expect_name("a scalar name")?;
                self.directives()?;
                Ok(Definition::Type {
                    keyword,
                    name,
                    fields: Vec::new(),
                })
            }
            "type" | "interface" | "input" => {
                let name = self.expect_name("a type name")?;
                if self.peek().and_then(Kind::name) == Some("implements") {
                    self.at += 1;
                    self.eat('&');
                    self.expect_name("an interface")?;
                    while self.eat('&') {
                        self.expect_name("an interface")?;
                    }
                }
                self.directives()?;
                let fields = if self.peek().is_some_and(|k| k.is('{')) {
                    self.fields(true)?
                } else {
                    Vec::new()
                };
                Ok(Definition::Type {
                    keyword,
                    name,
                    fields,
                })
            }
            "enum" => {
                let name = self.expect_name("an enum name")?;
                self.directives()?;
                let fields = if self.peek().is_some_and(|k| k.is('{')) {
                    self.fields(false)?
                } else {
                    Vec::new()
                };
                Ok(Definition::Type {
                    keyword,
                    name,
                    fields,
                })
            }
            "union" => {
                let name = self.expect_name("a union name")?;
                self.directives()?;
                let mut fields = Vec::new();
                if self.eat('=') {
                    self.eat('|');
                    fields.push(self.expect_name("a member type")?);
                    while self.eat('|') {
                        fields.push(self.expect_name("a member type")?);
                    }
                }
                Ok(Definition::Type {
                    keyword,
                    name,
                    fields,
                })
            }
            "directive" => {
                if !self.eat('@') {
                    return Err(self.error("expected @ after directive"));
                }
                let name = self.expect_name("a directive name")?;
                if self.peek().is_some_and(|k| k.is('(')) {
                    self.balanced('(', ')')?;
                }
                if self.peek().and_then(Kind::name) == Some("repeatable") {
                    self.at += 1;
                }
                if self.expect_name("on")? != "on" {
                    return Err(self.error("expected on"));
                }
                self.eat('|');
                self.expect_name("a location")?;
                while self.eat('|') {
                    self.expect_name("a location")?;
                }
                Ok(Definition::Directive { name })
            }
            other => Err(self.error(&format!("{other:?} does not begin a definition"))),
        }
    }

    /// `@name[(args)]`, any number of them.
    pub(crate) fn directives(&mut self) -> Result<(), String> {
        while self.eat('@') {
            self.expect_name("a directive name")?;
            if self.peek().is_some_and(|k| k.is('(')) {
                self.balanced('(', ')')?;
            }
        }
        Ok(())
    }

    /// The `{ query: Q mutation: M }` of a schema definition.
    pub(crate) fn roots(&mut self) -> Result<Vec<(String, String)>, String> {
        if !self.eat('{') {
            return Err(self.error("expected { after schema"));
        }
        let mut roots = Vec::new();
        while !self.eat('}') {
            let operation = self.expect_name("an operation kind")?;
            if !self.eat(':') {
                return Err(self.error("expected : in a schema definition"));
            }
            roots.push((operation, self.expect_name("a root type")?));
        }
        Ok(roots)
    }

    /// A `{ ... }` body of field or value definitions: the names defined at
    /// its top level, arguments and types skipped. A `typed` body — a type,
    /// interface or input — gives every field a type; an enum body does not.
    pub(crate) fn fields(&mut self, typed: bool) -> Result<Vec<String>, String> {
        if !self.eat('{') {
            return Err(self.error("expected {"));
        }
        let mut names = Vec::new();
        loop {
            if matches!(self.peek(), Some(Kind::Str(_))) {
                self.at += 1;
            }
            if self.eat('}') {
                return Ok(names);
            }
            names.push(self.expect_name("a field name")?);
            if self.peek().is_some_and(|k| k.is('(')) {
                self.balanced('(', ')')?;
            }
            if self.eat(':') {
                self.type_reference()?;
                if self.eat('=') {
                    self.value()?;
                }
            } else if typed {
                return Err(self.error("expected : after a field name"));
            }
            self.directives()?;
        }
    }

    /// `Name`, `[Type]`, either followed by `!`.
    pub(crate) fn type_reference(&mut self) -> Result<(), String> {
        if self.eat('[') {
            self.type_reference()?;
            if !self.eat(']') {
                return Err(self.error("expected ]"));
            }
        } else {
            self.expect_name("a type")?;
        }
        self.eat('!');
        Ok(())
    }

    /// One value: a scalar token, a list, or an object.
    pub(crate) fn value(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(Kind::Punct('[')) => self.balanced('[', ']'),
            Some(Kind::Punct('{')) => self.balanced('{', '}'),
            Some(Kind::Punct('$')) => {
                self.at += 1;
                self.expect_name("a variable").map(|_| ())
            }
            Some(Kind::Name(_) | Kind::Int(_) | Kind::Float(_) | Kind::Str(_)) => {
                self.at += 1;
                Ok(())
            }
            _ => Err(self.error("expected a value")),
        }
    }
}
