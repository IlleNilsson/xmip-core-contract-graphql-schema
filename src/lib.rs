#![forbid(unsafe_code)]

//! The GraphQL schema content contract — a technology of `xmip-core-contract`.
//!
//! Two claims, decided 2026-09-07 (ADR-0042): **well-formedness is a given**
//! and **conformance is a given once a contract is named**.
//!
//! Well-formed here is a *sound document*, the GraphQL specification's
//! grammar: it lexes, every definition is one the language has, every
//! selection set and body closes and is not empty. That holds for an
//! executable document — a request's operations and fragments — and for a
//! schema written in the definition language alike.
//!
//! Conformance is the *schema*: a Location that names this contract with a
//! schema file bound has every operation held to it at the root — the
//! operation's root type exists, and every field it selects there is a field
//! of that type — and every fragment's type condition is a type the schema
//! defines. Holding the selection below the root, field by field through the
//! types, is the next layer here; the root is where a request that was sent
//! to the wrong service, or against the wrong version, shows first.

pub mod document;
pub mod lexer;

use std::collections::BTreeMap;

use contract::{
    Contract, ContractDescriptor, ContractError, ContractFactory, ContractId, ValidationIssue,
    ValidationResult,
};
use document::{Definition, parse};
use stream::Stream;

/// What a bound schema knows: its root operation types and the fields of
/// every type, `extend`s merged in.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    roots: BTreeMap<String, String>,
    types: BTreeMap<String, Vec<String>>,
}

impl Schema {
    /// Read `text` as a schema in the definition language.
    ///
    /// # Errors
    /// Not a sound document, or one with no type in it.
    pub fn parse(text: &str) -> Result<Self, ContractError> {
        let definitions = parse(text).map_err(|message| ContractError { message })?;
        let mut schema = Self::default();
        for (operation, root) in [
            ("query", "Query"),
            ("mutation", "Mutation"),
            ("subscription", "Subscription"),
        ] {
            schema.roots.insert(operation.to_string(), root.to_string());
        }
        for definition in definitions {
            match definition {
                Definition::Schema { roots } => schema.roots.extend(roots),
                Definition::Type { name, fields, .. } => {
                    schema.types.entry(name).or_default().extend(fields);
                }
                _ => {}
            }
        }
        if schema.types.is_empty() {
            return Err(ContractError {
                message: "a schema that defines no type".to_string(),
            });
        }
        Ok(schema)
    }

    /// Why `document`'s definitions do not fit this schema at the root.
    fn issues(&self, definitions: &[Definition]) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        for (ordinal, definition) in definitions.iter().enumerate() {
            match definition {
                Definition::Operation { kind, name, fields } => {
                    let root = self.roots.get(kind).map_or("", String::as_str);
                    let label = name
                        .clone()
                        .unwrap_or_else(|| format!("{kind} {}", ordinal + 1));
                    let Some(known) = self.types.get(root) else {
                        issues.push(issue(format!("the schema has no {kind} root type"), &label));
                        continue;
                    };
                    for field in fields {
                        if !known.contains(field) && !field.starts_with("__") {
                            issues.push(issue(format!("{root} has no field {field}"), &label));
                        }
                    }
                }
                Definition::Fragment { name, on } => {
                    if !self.types.contains_key(on) {
                        issues.push(issue(
                            format!("the schema has no type {on}"),
                            &format!("fragment {name}"),
                        ));
                    }
                }
                _ => {}
            }
        }
        issues
    }
}

fn issue(message: String, path: &str) -> ValidationIssue {
    ValidationIssue {
        code: "schema".to_string(),
        message,
        path: Some(path.to_string()),
    }
}

/// The GraphQL contract, bare or bound to a schema.
pub struct GraphqlSchema {
    descriptor: ContractDescriptor,
    schema: Option<Schema>,
}

impl GraphqlSchema {
    /// A sound document, of any shape.
    #[must_use]
    pub fn new() -> Self {
        Self {
            descriptor: descriptor("graphql-schema"),
            schema: None,
        }
    }

    /// A sound document held to `schema` at the root.
    #[must_use]
    pub fn against(schema: Schema, name: &str) -> Self {
        Self {
            descriptor: descriptor(&format!("graphql-schema:{name}")),
            schema: Some(schema),
        }
    }

    /// Whether a schema is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.schema.is_some()
    }
}

impl Default for GraphqlSchema {
    fn default() -> Self {
        Self::new()
    }
}

fn descriptor(id: &str) -> ContractDescriptor {
    ContractDescriptor {
        id: ContractId(id.to_string()),
        version: "1".to_string(),
        representation: "application/graphql".to_string(),
    }
}

const OPENERS: [&str; 12] = [
    "query",
    "mutation",
    "subscription",
    "fragment",
    "schema",
    "type",
    "interface",
    "input",
    "enum",
    "union",
    "scalar",
    "directive",
];

impl Contract for GraphqlSchema {
    fn descriptor(&self) -> &ContractDescriptor {
        &self.descriptor
    }

    fn identify(&self, stream: &Stream) -> Result<bool, ContractError> {
        if stream.media_type().is_some_and(|m| {
            m.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/graphql")
        }) {
            return Ok(true);
        }
        let Ok(text) = std::str::from_utf8(stream.bytes()) else {
            return Ok(false);
        };
        let head = text.trim_start();
        Ok(head.starts_with('{')
            || head.starts_with("\"\"\"")
            || OPENERS.iter().any(|opener| {
                head.strip_prefix(opener)
                    .is_some_and(|rest| rest.starts_with(|c: char| !c.is_alphanumeric()))
            }))
    }

    fn validate(&self, stream: &Stream) -> Result<ValidationResult, ContractError> {
        let malformed = |message: String| ValidationResult {
            valid: false,
            issues: vec![ValidationIssue {
                code: "malformed".to_string(),
                message,
                path: None,
            }],
        };
        let text = match std::str::from_utf8(stream.bytes()) {
            Ok(text) => text,
            Err(error) => return Ok(malformed(format!("not text: {error}"))),
        };
        let definitions = match parse(text) {
            Ok(definitions) => definitions,
            Err(message) => return Ok(malformed(message)),
        };
        let issues = self
            .schema
            .as_ref()
            .map_or_else(Vec::new, |schema| schema.issues(&definitions));
        Ok(ValidationResult {
            valid: issues.is_empty(),
            issues,
        })
    }
}

/// Loads the contract a Location names: an empty reference is the bare
/// contract, anything else the path of a schema file in the definition
/// language.
pub struct GraphqlSchemaFactory;

impl ContractFactory for GraphqlSchemaFactory {
    fn technology(&self) -> &'static str {
        "graphql-schema"
    }

    fn load(&self, reference: &str) -> Result<Box<dyn Contract>, ContractError> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Ok(Box::new(GraphqlSchema::new()));
        }
        let text = std::fs::read_to_string(reference).map_err(|error| ContractError {
            message: format!("cannot read schema {reference}: {error}"),
        })?;
        Ok(Box::new(GraphqlSchema::against(
            Schema::parse(&text)?,
            reference,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::StreamId;

    const SCHEMA: &str = r"
        type Query { order(id: ID!): Order orders: [Order!]! }
        type Order { id: ID! lines: [Line!]! }
        type Line { sku: String! }
        extend type Query { me: Node }
        interface Node { id: ID! }
    ";

    fn stream(text: &str, media_type: Option<&str>) -> Stream {
        Stream::new(
            StreamId::new(1),
            text.as_bytes().to_vec(),
            media_type.map(str::to_string),
        )
    }

    #[test]
    fn a_sound_document_holds_bare_and_against_its_schema() {
        let bare = GraphqlSchema::new();
        let request = "query Q { orders { id lines { sku } } me { id } __typename }";
        assert!(bare.identify(&stream(request, None)).expect("identify"));
        assert!(
            bare.identify(&stream("x", Some("application/graphql; charset=utf-8")))
                .expect("identify")
        );
        assert!(!bare.identify(&stream("SELECT 1", None)).expect("identify"));
        assert!(!bare.identify(&stream("typed", None)).expect("identify"));
        assert!(
            bare.validate(&stream(request, None))
                .expect("validate")
                .valid
        );
        assert!(
            bare.validate(&stream(SCHEMA, None))
                .expect("validate")
                .valid
        );
        let bound = GraphqlSchema::against(Schema::parse(SCHEMA).expect("schema"), "orders");
        assert!(bound.is_bound());
        assert_eq!(bound.descriptor().id.0, "graphql-schema:orders");
        assert!(
            bound
                .validate(&stream(request, None))
                .expect("validate")
                .valid
        );
        let with_fragment = "fragment F on Order { id } { orders { ...F } }";
        assert!(
            bound
                .validate(&stream(with_fragment, None))
                .expect("validate")
                .valid
        );
    }

    #[test]
    fn a_request_that_does_not_fit_the_schema_is_named_at_the_root() {
        let bound = GraphqlSchema::against(Schema::parse(SCHEMA).expect("schema"), "orders");
        let result = bound
            .validate(&stream("query Q { orders { id } customers { id } }", None))
            .expect("validate");
        assert!(!result.valid);
        assert_eq!(result.issues[0].code, "schema");
        assert_eq!(result.issues[0].message, "Query has no field customers");
        assert_eq!(result.issues[0].path.as_deref(), Some("Q"));
        let result = bound
            .validate(&stream("mutation { placeOrder { id } }", None))
            .expect("validate");
        assert_eq!(
            result.issues[0].message,
            "the schema has no mutation root type"
        );
        assert_eq!(result.issues[0].path.as_deref(), Some("mutation 1"));
        let result = bound
            .validate(&stream(
                "fragment F on Customer { id } { orders { ...F } }",
                None,
            ))
            .expect("validate");
        assert_eq!(result.issues[0].message, "the schema has no type Customer");
        let renamed = "schema { query: Root } type Root { ping: String }";
        let bound = GraphqlSchema::against(Schema::parse(renamed).expect("schema"), "r");
        assert!(
            bound
                .validate(&stream("{ ping }", None))
                .expect("validate")
                .valid
        );
    }

    #[test]
    fn what_is_unsound_does_not_hold_and_the_factory_reads_a_file() {
        let result = GraphqlSchema::new()
            .validate(&stream("query { }", None))
            .expect("validate");
        assert!(!result.valid);
        assert_eq!(result.issues[0].code, "malformed");
        assert!(result.issues[0].message.starts_with("line 1:"));
        let binary = Stream::new(StreamId::new(1), vec![0xff], None);
        assert!(
            !GraphqlSchema::new()
                .validate(&binary)
                .expect("validate")
                .valid
        );
        assert!(Schema::parse("query { a }").is_err(), "no types");
        let dir = std::env::temp_dir().join("xmip-graphql-schema-test");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("orders.graphql");
        std::fs::write(&path, SCHEMA).expect("write");
        let loaded = GraphqlSchemaFactory
            .load(path.to_str().expect("path"))
            .expect("load");
        assert!(loaded.descriptor().id.0.ends_with("orders.graphql"));
        assert!(GraphqlSchemaFactory.load("/no/such/file.graphql").is_err());
        assert!(
            !GraphqlSchemaFactory
                .load("")
                .expect("bare")
                .descriptor()
                .id
                .0
                .contains(':')
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
