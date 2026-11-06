use super::*;
use carina_plugin_sdk::core::FiniteF64;
use carina_plugin_sdk::core::resource::{ConcreteValue, Value};
use carina_plugin_sdk::core::schema::{
    AttributeSchema, AttributeType, DslTransform, ResourceSchema, StructField, TypeIdentity,
};
use comrak::nodes::NodeValue;

fn rendered_code(markdown: &str) -> Vec<String> {
    let arena = comrak::Arena::new();
    let mut options = comrak::Options::default();
    options.extension.table = true;
    comrak::parse_document(&arena, markdown, &options)
        .descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Code(code) => Some(code.literal.clone()),
            _ => None,
        })
        .collect()
}

fn choice(name: &str, values: &[&str]) -> AttributeType {
    AttributeType::enum_(
        TypeIdentity::bare(name),
        Some(values.iter().map(|s| (*s).into()).collect()),
        vec![],
        None,
        None,
    )
}

fn page(schema: ResourceSchema) -> String {
    render_page(&schema.validate().unwrap(), None).unwrap()
}

fn input(name: impl Into<String>, ty: AttributeType) -> AttributeSchema {
    AttributeSchema::optional(name, ty).build()
}
fn output(name: &str, ty: AttributeType) -> AttributeSchema {
    AttributeSchema::provider_populated(name, ty).build()
}
fn field(name: &str, ty: AttributeType) -> StructField {
    StructField::optional(name, ty).build()
}

#[test]
fn literals_round_trip_as_parsed_text_inline_and_in_tables() {
    for (literal, expected) in [
        (
            r"^[A-Za-z0-9-]+/(?:[a-z]+|\.[a-z]+|\.\.[a-z]+)$",
            r"^[A-Za-z0-9-]+/(?:[a-z]+|\.[a-z]+|\.\.[a-z]+)$",
        ),
        ("`a``b`|c", "`a``b`|c"),
        ("`", "`"),
        ("`both`", "`both`"),
        (
            r#""quoted" &amp; &#124; \n \u{5b} \\|"#,
            r#""quoted" &amp; &#124; \n \u{5b} \\|"#,
        ),
        (" leading and trailing ", " leading and trailing "),
        ("   ", "   "),
        (" \u{a0} ", " \u{a0} "),
        ("日本語 🦀\u{2028}\u{2029}", "日本語 🦀\u{2028}\u{2029}"),
        (
            "first\nsecond\tthird\r\0\u{1}\u{b}\u{1f}",
            r"first\nsecond\tthird\u{d}\u{0}\u{1}\u{b}\u{1f}",
        ),
    ] {
        let mut markdown = format!("# Literal\n\n{}\n\n", code_span(literal, false).unwrap());
        table(
            &mut markdown,
            ["Literal"],
            [Ok([code_span(literal, true).unwrap()])],
        )
        .unwrap();
        assert_eq!(rendered_code(&markdown), [expected, expected], "{markdown}");
        carina_docgen::check_page(&markdown).unwrap();
    }
    assert_eq!(code_span("`a``b`", false).unwrap(), "``` `a``b` ```");
    assert_eq!(code_span("a|b", false).unwrap(), "`a|b`");
    assert_eq!(code_span("a|b", true).unwrap(), r"`a\|b`");
    assert_eq!(rendered_code(&code_span(r"\|", false).unwrap()), [r"\|"]);
    assert!(
        code_span(r"\|", true)
            .unwrap_err()
            .to_string()
            .contains("cannot round-trip")
    );
    assert!(code_span("", false).is_err());
}

#[test]
fn schema_patterns_defaults_enum_values_and_transforms_keep_their_spelling() {
    let literal = "a|b \"quoted\" `tick``";
    let default = Value::Concrete(ConcreteValue::String(literal.into()));
    let transform = DslTransform::StripSuffix(literal.into());
    let schema = ResourceSchema::new("test.Literals")
        .attribute(
            AttributeSchema::optional("default", AttributeType::string())
                .with_default(default.clone())
                .build(),
        )
        .attribute(input("choice", choice("Choice", &[literal])))
        .with_def(
            "Pattern",
            AttributeType::refined_string(
                None,
                Some("^[a-z|`]+$".into()),
                None,
                Some(transform.clone()),
            ),
        )
        .attribute(output(
            "output",
            AttributeType::list(AttributeType::ref_("Pattern")),
        ))
        .attribute(input(
            "transformed_enum",
            AttributeType::enum_(
                TypeIdentity::new(Some("github"), ["test", "Literals"], "Transformed"),
                None,
                vec![],
                None,
                Some(transform.clone()),
            ),
        ));
    let page = page(schema);
    let code = rendered_code(&page);
    for expected in [
        literal.to_string(),
        format_value(&default),
        format!("{transform:?}"),
        format!("String (pattern: ^[a-z|`]+$, DSL transform: {transform:?})"),
    ] {
        assert!(code.contains(&expected), "missing {expected:?}: {page}");
    }
    assert!(page.contains("pattern: ^[a-z|`]+$"));
    assert!(page.contains(r"pattern: ^[a-z\|`]+$"));
    assert!(!page.contains("\\u{5b}"));
}

#[test]
fn link_shaped_literals_round_trip_as_code() {
    let literal = "^[a](b)$";
    let pattern = format!("String (pattern: {literal})");
    let default = Value::Concrete(ConcreteValue::String(literal.into()));
    let transform = DslTransform::StripSuffix(literal.into());
    for (attribute, expected) in [
        (
            input(
                "pattern",
                AttributeType::refined_string(None, Some(literal.into()), None, None),
            ),
            pattern.clone(),
        ),
        (
            AttributeSchema::optional("default", AttributeType::string())
                .with_default(default.clone())
                .build(),
            format_value(&default),
        ),
        (
            input("enum_value", choice("Choice", &[literal])),
            literal.to_string(),
        ),
        (
            input(
                "transform",
                AttributeType::enum_(
                    TypeIdentity::new(Some("github"), ["test", "Literals"], "Transformed"),
                    None,
                    vec![],
                    None,
                    Some(transform.clone()),
                ),
            ),
            format!("{transform:?}"),
        ),
        (
            input(
                "nested",
                AttributeType::struct_(
                    "Settings",
                    vec![field(
                        "pattern",
                        AttributeType::refined_string(None, Some(literal.into()), None, None),
                    )],
                ),
            ),
            pattern,
        ),
    ] {
        let name = attribute.name().to_string();
        let page = page(ResourceSchema::new("test.Literals").attribute(attribute));
        assert!(
            rendered_code(&page).contains(&expected),
            "attribute `{name}` lost literal {expected:?}: {page}"
        );
    }
}

#[test]
fn real_oidc_subject_page_matches_the_schema_and_registry_shape() {
    let pages = generate_pages(&examples_dir()).unwrap();
    assert_eq!(pages, generate_pages(&examples_dir()).unwrap());
    assert_eq!(pages.len(), resource_schemas().len());
    let page = &pages[Path::new("actions/oidc_subject.md")];
    let schemas = resource_schemas();
    let resource = schemas[0].as_schema();
    let RawShape::String {
        pattern: Some(pattern),
        ..
    } = resource.attributes()["repository"].attr_type().raw_shape()
    else {
        panic!("repository must declare its pattern")
    };
    assert!(page.contains(&format!("(pattern: {pattern})`")));
    assert!(rendered_code(page).contains(&format!(
        "github.actions.OidcSubject.Repository (pattern: {pattern})"
    )));
    assert!(page.contains(resource.description().unwrap()));
    assert!(page.contains(include_str!("../../examples/actions/oidc_subject/main.crn")));
    assert!(page.contains("Destruction protection is enabled by default."));
    let (arguments, attributes) = page.split_once("## Attribute Reference\n").unwrap();
    assert!(arguments.contains("- **Required:** Yes\n- **Create-only:** Yes\n- **Identity:** Required (omitting this value leaves identity incomplete)"));
    assert!(arguments.contains("List\\<[github.actions.OidcSubject.ClaimKey](#githubactionsoidcsubjectclaimkey)\\> (items: 1.., unique items, ordered)"));
    assert!(!arguments.contains("### `use_default`"));
    assert!(attributes.contains("### `use_default`\n\n- **Type:** `Bool`\n- **Read-only**"));
    assert!(page.contains("| `github.actions.OidcSubject.ClaimKey.repo` | `repo` |"));
    assert!(
        page.contains("| `github.actions.OidcSubject.ClaimKey.workflow_sha` | `workflow_sha` |")
    );
    assert_eq!(
        carina_docgen::heading_ids(page),
        [
            "githubactionsoidcsubject",
            "example",
            "argument-reference",
            "claim_keys",
            "repository",
            "attribute-reference",
            "use_default",
            "enum-values",
            "githubactionsoidcsubjectclaimkey"
        ]
    );
}

#[test]
fn equal_declarations_share_sections_and_distinct_collisions_keep_their_contents() {
    let shared = choice("A.B", &["shared"]);
    let shared_struct = AttributeType::struct_("Settings", vec![field("value", shared.clone())]);
    let mut schema =
        ResourceSchema::new("test.Declarations").with_def("Alias", shared_struct.clone());
    for (name, ty) in [
        ("a_first", shared.clone()),
        ("a_second", shared.clone()),
        ("settings", shared_struct.clone()),
        ("other_settings", shared_struct),
    ] {
        schema = schema.attribute(input(name, ty));
    }
    for (index, name) in ["A.B", "AB", "AB (2)"].into_iter().enumerate() {
        schema = schema.attribute(input(
            format!("distinct_{index}"),
            choice(name, &[&format!("value_{index}")]),
        ));
    }
    for (name, field) in [
        (
            "input",
            StructField::required("value", AttributeType::string())
                .with_description("Input.")
                .build(),
        ),
        (
            "observed",
            StructField::provider_populated("value", AttributeType::string())
                .with_description("Observed.")
                .build(),
        ),
    ] {
        schema = schema.attribute(input(name, AttributeType::struct_("Settings", vec![field])));
    }
    // Debug uses closure placeholders, but independent validators are distinct.
    for index in 0..2 {
        let validated = AttributeType::enum_(
            TypeIdentity::bare("Validated"),
            Some(vec!["value".into()]),
            vec![],
            Some(std::sync::Arc::new(|_| Ok(()))),
            None,
        );
        schema = schema.attribute(input(format!("validated_{index}"), validated));
    }
    let schema = schema.validate().unwrap();
    let page = render_page(&schema, None).unwrap();
    assert_eq!(page, render_page(&schema, None).unwrap());
    assert_eq!(page.matches("### `A.B`\n").count(), 1);
    assert_eq!(page.matches("| `A.B.shared` | `shared` |").count(), 1);
    assert_eq!(page.matches("[A.B](#ab)").count(), 3);
    for index in 0..3 {
        assert!(page.contains(&format!("`value_{index}`")));
    }
    assert!(page.contains("| `value` | `String` | Yes | Input. |"));
    assert!(page.contains("| `value` | `String` | No | **Read-only.** Observed. |"));
    assert_eq!(
        page.lines()
            .filter(|line| line.starts_with("### `Settings"))
            .count(),
        3
    );
    assert_eq!(page.matches("### `Validated").count(), 2);
}

#[test]
fn descriptions_only_escape_leading_heading_markers() {
    let description = "`code` and https://example.com/a-b\n# Heading\n   ## Indented\n###\n#hashtag\n####### Seven\n    # Code block\n*emphasis*\n";
    assert_eq!(
        description_text(description),
        "`code` and https://example.com/a-b\n\\# Heading\n   \\## Indented\n\\###\n#hashtag\n####### Seven\n    # Code block\n*emphasis*\n"
    );
}

#[test]
fn descriptions_and_nested_metadata_preserve_their_meaning() {
    let description = "# Details\n\n## Argument Reference\n\n`code` and https://example.com/a-b\n\n    # Indented";
    let mut schema = ResourceSchema::new("test.Metadata").with_description(description);
    let mut fields = vec![
        StructField::provider_populated("observed", AttributeType::duration())
            .with_description(description)
            .build(),
    ];
    for required in [true, false] {
        let name = if required {
            "required_id"
        } else {
            "optional_id"
        };
        let attribute = AttributeSchema::optional(name, AttributeType::string()).create_only();
        let field = StructField::optional(name, AttributeType::string()).create_only();
        schema = schema.attribute(if required {
            attribute.identity().build()
        } else {
            attribute.identity_when_present().build()
        });
        fields.push(if required {
            field.identity().build()
        } else {
            field.identity_when_present().build()
        });
    }
    fields.push(
        StructField::optional("token", AttributeType::string())
            .create_only()
            .write_only()
            .with_description(description)
            .build(),
    );
    schema = schema
        .attribute(
            AttributeSchema::required("token", AttributeType::string())
                .create_only()
                .write_only()
                .with_description(description)
                .build(),
        )
        .with_def("Settings", AttributeType::struct_("Settings", fields));
    let page = page(schema);
    for identity in [
        "Required (omitting this value leaves identity incomplete)",
        "When present (omission is recorded as absent)",
    ] {
        assert_eq!(
            page.matches(&format!("**Identity:** {identity}")).count(),
            2
        );
    }
    assert!(page.contains(
        "| `token` | `String` | No | **Create-only:** Yes. **Write-only:** Yes. \\# Details"
    ));
    assert!(page.contains("| `observed` | `Duration` | No | **Read-only.** \\# Details"));
    assert!(page.contains("- **Create-only:** Yes\n- **Write-only:** Yes"));
    assert_eq!(page.matches("\\# Details").count(), 4);
    assert!(
        !carina_docgen::heading_ids(&page)
            .iter()
            .any(|id| id == "details" || id == "indented")
    );
    assert_eq!(
        rendered_code(&page)
            .iter()
            .filter(|code| *code == "code")
            .count(),
        4
    );
    assert_eq!(page.matches("https://example.com/a-b").count(), 4);
    assert!(!page.contains("## Example"));
}

#[test]
fn field_groups_render_at_resource_and_struct_levels() {
    for (kind, wording) in [
        (FieldGroupKind::ExactlyOne, "Exactly one"),
        (FieldGroupKind::AtLeastOne, "At least one"),
        (FieldGroupKind::AtMostOne, "At most one"),
    ] {
        let group = FieldGroup::new(kind, ["first", "second"]).unwrap();
        let mut schema = ResourceSchema::new("test.Groups").field_group(group.clone());
        let mut fields = Vec::new();
        for name in ["first", "second"] {
            schema = schema.attribute(input(name, AttributeType::string()));
            fields.push(field(name, AttributeType::string()));
        }
        let page = page(schema.with_def(
            "Grouped",
            AttributeType::struct_with_field_groups("Grouped", fields, vec![group]),
        ));
        assert_eq!(
            page.matches(&format!("- {wording} of: `first`, `second`.\n"))
                .count(),
            2
        );
    }
}

#[test]
fn aliases_reject_cycles_and_preserve_recursive_definitions() {
    for mutual in [false, true] {
        for used in [false, true] {
            let mut schema = ResourceSchema::new("test.Cycle")
                .with_def("A", AttributeType::ref_(if mutual { "B" } else { "A" }));
            if mutual {
                schema = schema.with_def("B", AttributeType::ref_("A"));
            }
            if used {
                schema = schema.attribute(input("value", AttributeType::ref_("A")));
            }
            let error = render_page(&schema.validate().unwrap(), None).unwrap_err();
            assert!(format!("{error:#}").contains("cyclic schema alias:"));
        }
    }
    let schema = ResourceSchema::new("test.Recursive")
        .with_def("Alias", AttributeType::ref_("Node"))
        .with_def("Items", AttributeType::list(AttributeType::ref_("Items")))
        .with_def(
            "Node",
            AttributeType::struct_(
                "Node",
                vec![
                    field(
                        "children",
                        AttributeType::list(AttributeType::ref_("Alias")),
                    ),
                    StructField::required(
                        "value",
                        AttributeType::union(vec![
                            AttributeType::ref_("Node"),
                            AttributeType::refined_int(None, Some((Some(-3), Some(42)))),
                        ]),
                    )
                    .build(),
                    field(
                        "lookup",
                        AttributeType::map_with_key(
                            AttributeType::reference(
                                TypeIdentity::bare("Key"),
                                Some("^key$".into()),
                                Some((None, Some(8))),
                            ),
                            AttributeType::refined_float(
                                None,
                                Some((None, Some(FiniteF64::new(0.5).unwrap()))),
                            ),
                        ),
                    ),
                ],
            ),
        )
        .attribute(output("output", AttributeType::ref_("Alias")));
    let page = page(schema);
    for expected in [
        "| `Alias` | [Struct(Node)](#node) |",
        "| `children` | List\\<[Struct(Node)](#node)\\> (ordered)",
        "| 1 | [Struct(Node)](#node) |",
        "| 2 | `Int (range: -3..=42)` |",
        "Map\\<`Key (pattern: ^key$, len: ..=8)`, `Float (range: ..=0.5)`\\>",
        "## Recursive Type Definitions",
        "[Recursive type (Items)](#items)",
    ] {
        assert!(page.contains(expected), "missing {expected}: {page}");
    }
    assert_eq!(page.matches("### `Node`\n").count(), 1);
    assert_eq!(
        resource_path("test.HTTPSource").unwrap(),
        Path::new("test/http_source.md")
    );
    assert!(resource_path("actions../OidcSubject").is_err());
}
