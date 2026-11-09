use super::*;
use carina_plugin_sdk::core::FiniteF64;
use carina_plugin_sdk::core::resource::{ConcreteValue, Value};
use carina_plugin_sdk::core::schema::{
    AttributeSchema, AttributeType, CompiledAttributeSchema, DslTransform, FieldGroup,
    FieldGroupKind, PatternMatch, PatternReason, PatternSpec, ResourceSchema, StructField,
    TypeIdentity,
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

fn rendered_paragraphs(markdown: &str) -> Vec<String> {
    let arena = comrak::Arena::new();
    let mut options = comrak::Options::default();
    options.extension.table = true;
    comrak::parse_document(&arena, markdown, &options)
        .descendants()
        .filter(|node| matches!(node.data.borrow().value, NodeValue::Paragraph))
        .map(|paragraph| {
            paragraph
                .descendants()
                .filter_map(|node| match &node.data.borrow().value {
                    NodeValue::Text(text) => Some(text.to_string()),
                    NodeValue::Code(code) => Some(code.literal.clone()),
                    _ => None,
                })
                .collect()
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

fn page_with_closed_enum(
    schema: ResourceSchema,
    literal: &str,
    transform: &DslTransform,
) -> String {
    // The current protocol rejects closed-enum transforms during admission.
    // Add this compiled fixture afterwards to exercise definition rendering.
    let schema = schema.validate().unwrap().as_schema().clone().attribute(
        CompiledAttributeSchema::optional(
            "closed_transform",
            CompiledAttributeType::enum_(
                TypeIdentity::new(Some("github"), ["test", "Literals"], "ClosedTransformed"),
                Some(vec![format!("value{literal}")]),
                vec![],
                Some(std::sync::Arc::new(|_| Ok(()))),
                Some(transform.clone()),
            ),
        )
        .build(),
    );
    render_schema_page(&schema, None).unwrap()
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
        let mut markdown = PageBuilder::<()>::new();
        markdown.heading(Heading::plain(1, "Literal")).unwrap();
        markdown.inline(TableCell::code(literal));
        markdown.push_str("\n\n");
        markdown
            .table(["Literal"], [[TableCell::code(literal)]])
            .unwrap();
        let markdown = markdown.finish().unwrap();
        assert_eq!(rendered_code(&markdown), [expected, expected], "{markdown}");
        carina_docgen::check_page(&markdown).unwrap();
    }
    let mut inline = PageBuilder::<()>::new();
    inline.inline(TableCell::code("`a``b`"));
    inline.push_str("\n\n");
    inline.inline(TableCell::code("a|b"));
    inline.push_str("\n\n");
    inline.inline(TableCell::code(r"\|"));
    let inline = inline.finish().unwrap();
    assert_eq!(inline, "``` `a``b` ```\n\n`a|b`\n\n`\\|`");
    assert_eq!(rendered_code(&inline), ["`a``b`", "a|b", r"\|"]);
    let mut table = PageBuilder::<()>::new();
    table
        .table(
            ["Literal"],
            [[TableCell::code("a|b")], [TableCell::code(r"\|")]],
        )
        .unwrap();
    let table = table.finish().unwrap();
    assert!(table.contains(r"`a\|b`"));
    // The shared renderer preserves this literal by splitting it into adjacent
    // code spans instead of rejecting the odd backslash run before the pipe.
    assert_eq!(rendered_code(&table), ["a|b", r"\", "|"]);
    let mut invalid = PageBuilder::<()>::new();
    invalid.inline(TableCell::code(""));
    let error = invalid.finish().unwrap_err();
    assert!(format!("{error:#}").contains("cannot render inline code literal"));
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
                Some(PatternSpec::new("^[a-z|`]+$", PatternMatch::Full)),
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
    let page = page_with_closed_enum(schema, literal, &transform);
    let code = rendered_code(&page);
    for expected in [
        literal.to_string(),
        format_value(&default),
        format!("github.test.Literals.Transformed (DSL transform: {transform:?})"),
        format!("String (pattern: /^[a-z|`]+$/ (Full), DSL transform: {transform:?})"),
        format!("{transform:?}"),
        "github.test.Literals.ClosedTransformed.value".into(),
        format!("value{literal}"),
    ] {
        assert!(code.contains(&expected), "missing {expected:?}: {page}");
    }
    assert!(page.contains("pattern: /^[a-z|`]+$/ (Full)"));
    assert!(page.contains(r"pattern: /^[a-z\|`]+$/ (Full)"));
    assert!(!page.contains("\\u{5b}"));
    let paragraphs = rendered_paragraphs(&page);
    assert!(
        paragraphs.contains(&"Values are also checked by a custom validator.".into()),
        "{page}"
    );
    assert!(
        paragraphs.contains(&format!("DSL transform: {transform:?}.")),
        "{page}"
    );
}

#[test]
fn link_shaped_literals_round_trip_as_code() {
    for literal in ["^[a](b)$", "a|b", "`a``b`"] {
        let pattern = format!(
            "String (pattern: /{literal}/ (Substring: Preserve the fixture's original regex search semantics.))"
        );
        let spec = PatternSpec::new(
            literal,
            PatternMatch::Substring {
                reason: PatternReason::new(
                    "Preserve the fixture's original regex search semantics.",
                )
                .unwrap(),
            },
        );
        let default = Value::Concrete(ConcreteValue::String(literal.into()));
        let transform = DslTransform::StripSuffix(literal.into());
        for (attribute, expected) in [
            (
                input(
                    "pattern",
                    AttributeType::refined_string(None, Some(spec.clone()), None, None),
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
                format!("github.test.Literals.Transformed (DSL transform: {transform:?})"),
            ),
            (
                input(
                    "nested",
                    AttributeType::struct_(
                        "Settings",
                        vec![field(
                            "pattern",
                            AttributeType::refined_string(None, Some(spec.clone()), None, None),
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
        let page = page_with_closed_enum(ResourceSchema::new("test.Literals"), literal, &transform);
        assert!(
            rendered_code(&page).contains(&format!("{transform:?}")),
            "{page}"
        );
        let paragraphs = rendered_paragraphs(&page);
        assert!(
            paragraphs.contains(&"Values are also checked by a custom validator.".into()),
            "{page}"
        );
        assert!(
            paragraphs.contains(&format!("DSL transform: {transform:?}.")),
            "{page}"
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
        identity: _identity,
        pattern: Some(pattern),
        length: _length,
        validate: _validate,
        to_dsl: _to_dsl,
    } = resource.attributes()["repository"].attr_type().raw_shape()
    else {
        panic!("repository must declare its pattern")
    };
    assert_eq!(pattern.mode(), &PatternMatch::Full);
    let pattern = pattern.as_str();
    assert!(page.contains(&format!("(pattern: /{pattern}/ (Full))`")));
    assert!(rendered_code(page).contains(&format!(
        "github.actions.OidcSubject.Repository (pattern: /{pattern}/ (Full))"
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
fn map_entry_bounds_render_in_attributes_and_definition_tables() {
    let bounded = AttributeType::refined_map(
        AttributeType::string(),
        AttributeType::int(),
        Some((Some(2), Some(5))),
    );
    let page = page(
        ResourceSchema::new("test.MapBounds")
            .attribute(input("entries", bounded.clone()))
            .attribute(output("observed", bounded.clone()))
            .with_def("Entries", bounded.clone())
            .with_def(
                "Settings",
                AttributeType::struct_("Settings", vec![field("entries", bounded)]),
            ),
    );
    let expected = r"Map\<`String`, `Int`\> (entries: 2..=5)";
    assert!(page.contains(&format!("### `entries`\n\n- **Type:** {expected}\n")));
    assert!(page.contains(&format!("### `observed`\n\n- **Type:** {expected}\n")));
    assert!(page.contains(&format!("| `Entries` | {expected} |")));
    assert!(page.contains(&format!("| `entries` | {expected} | No |  |")));
}

#[test]
fn open_enum_attributes_render_as_scalar_names_without_declarations() {
    let transform = DslTransform::StripSuffix("suffix".into());
    let page = page(ResourceSchema::new("test.OpenEnum").attribute(input(
        "value",
        AttributeType::enum_(
            TypeIdentity::new(Some("github"), ["x"], "Y"),
            None,
            vec![],
            Some(std::sync::Arc::new(|_| Ok(()))),
            Some(transform.clone()),
        ),
    )));
    assert!(page.contains(&format!(
        "### `value`\n\n- **Type:** `github.x.Y (custom validator, DSL transform: {transform:?})`\n"
    )));
    assert!(!page.contains("## Enum Values"));
    assert!(!page.contains("### `github.x.Y`"));
}

#[test]
fn render_errors_include_the_attribute_path_and_source() {
    let nested = (0..65).fold(AttributeType::string(), |ty, _| AttributeType::list(ty));
    for (schema, path) in [
        (
            ResourceSchema::new("test.RenderError").attribute(input("deep", nested.clone())),
            "deep",
        ),
        (
            ResourceSchema::new("test.RenderError").with_def("Deep", nested.clone()),
            "Deep",
        ),
        (
            ResourceSchema::new("test.RenderError").attribute(input(
                "settings",
                AttributeType::struct_("Settings", vec![field("entries", nested.clone())]),
            )),
            "settings.entries",
        ),
        (
            ResourceSchema::new("test.RenderError").attribute(input(
                "choice",
                AttributeType::union(vec![AttributeType::bool(), nested]),
            )),
            "choice.member_2",
        ),
    ] {
        let error = render_page(&schema.validate().unwrap(), None).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains(&format!("cannot render type at attribute `{path}`")),
            "{message}"
        );
        assert!(
            message.contains("type nesting exceeds documentation limit (64)"),
            "{message}"
        );
        assert!(error.source().is_some(), "{message}");
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
                                Some(PatternSpec::new("^key$", PatternMatch::Full)),
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
        "Map\\<`Key (pattern: /^key$/ (Full), len: ..=8)`, `Float (range: ..=0.5)`\\>",
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
