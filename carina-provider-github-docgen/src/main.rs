use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use carina_docgen::example::{self, DocExample, ExampleKind};
use carina_plugin_sdk::core::schema::{
    CompiledAttributeType, DslMap, FieldGroup, FieldGroupKind, IdentityMode, ListElements,
    RawShape, ValidatedResourceSchema,
};
use carina_plugin_sdk::core::value::format_value;
use carina_provider_github::resource_schemas;

const PROVIDER: &str = "github";
const USAGE: &str = "usage: docgen --output-dir <dir>";

#[cfg(test)]
mod tests;

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(flag) = args.next() else {
        bail!(USAGE);
    };
    if flag == "--help" || flag == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    if flag != "--output-dir" {
        bail!(USAGE);
    }
    let output_dir = PathBuf::from(args.next().context(USAGE)?);
    if args.next().is_some() {
        bail!(USAGE);
    }

    for (relative, markdown) in generate_pages(&examples_dir())? {
        let path = output_dir.join(relative);
        fs::create_dir_all(path.parent().context("page has no parent directory")?)?;
        fs::write(&path, markdown)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

fn resource_path(resource_type: &str) -> Result<PathBuf> {
    let (service, name) = resource_type
        .split_once('.')
        .context("resource type must have the form <service>.<Name>")?;
    if service.is_empty()
        || name.is_empty()
        || !service
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        || !name.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        bail!("unsupported resource type: {resource_type}");
    }
    let chars: Vec<char> = name.chars().collect();
    let mut snake_name = String::new();
    for (index, &character) in chars.iter().enumerate() {
        if character.is_ascii_uppercase()
            && index > 0
            && (chars[index - 1].is_ascii_lowercase()
                || chars[index - 1].is_ascii_digit()
                || (chars[index - 1].is_ascii_uppercase()
                    && chars.get(index + 1).is_some_and(char::is_ascii_lowercase)))
        {
            snake_name.push('_');
        }
        snake_name.push(character.to_ascii_lowercase());
    }
    Ok(Path::new(service).join(snake_name).with_extension("md"))
}

fn generate_pages(examples_dir: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut pages = BTreeMap::new();
    for schema in resource_schemas() {
        let resource_type = schema.as_schema().resource_type();
        let relative = resource_path(resource_type)?;
        let directory = relative.with_extension("");
        let example_path = examples_dir.join(&directory).join("main.crn");
        let example = if example_path.try_exists()? {
            Some(example::load(
                examples_dir,
                directory.to_str().context("example path must be UTF-8")?,
                PROVIDER,
                resource_type,
                ExampleKind::Resource,
            )?)
        } else {
            None
        };
        let markdown = render_page(&schema, example.as_ref())
            .with_context(|| format!("failed to generate {PROVIDER}.{resource_type}"))?;
        if pages.insert(relative.clone(), markdown).is_some() {
            bail!("multiple schemas map to {}", relative.display());
        }
    }
    Ok(pages)
}

fn render_page(schema: &ValidatedResourceSchema, example: Option<&DocExample>) -> Result<String> {
    let resource = schema.as_schema();
    let types = SchemaTypes::collect(schema)?;
    let mut recursive = BTreeSet::new();
    let mut page = format!("# {PROVIDER}.{}\n\n", resource.resource_type());
    if let Some(description) = resource.description() {
        page.push_str(&format!("{}\n\n", description_text(description)));
    }
    if resource.prevents_destroy_by_default() {
        page.push_str("Destruction protection is enabled by default. To opt out, set `directives { prevent_destroy = false }`.\n\n");
    }
    if let Some(example) = example {
        let text = example.text_for(PROVIDER, resource.resource_type(), ExampleKind::Resource)?;
        page.push_str(&format!(
            "## Example\n\n```crn\n{}\n```\n\n",
            text.trim_end_matches('\n')
        ));
    }
    for (read_only, title) in [(false, "Argument Reference"), (true, "Attribute Reference")] {
        page.push_str(&format!("## {title}\n\n"));
        if !read_only {
            render_field_groups(&mut page, resource.field_groups())?;
        }
        for attribute in resource
            .attributes()
            .values()
            .filter(|a| a.is_read_only() == read_only)
        {
            let name = attribute.name();
            let ty = types.describe(attribute.attr_type(), false, &mut recursive)?;
            page.push_str(&format!(
                "### {}\n\n- **Type:** {ty}\n",
                code_span(name, false)?
            ));
            if read_only {
                page.push_str("- **Read-only**\n");
            } else {
                page.push_str(&format!(
                    "- **Required:** {}\n",
                    if attribute.is_required() { "Yes" } else { "No" }
                ));
                if let Some(default) = attribute.default_value() {
                    let code = code_span(&format_value(default), false)
                        .with_context(|| format!("cannot render default of attribute `{name}`"))?;
                    page.push_str(&format!("- **Default:** {code}\n"));
                }
                for metadata in field_metadata(
                    attribute.is_create_only(),
                    attribute.is_write_only(),
                    attribute.identity_mode(),
                ) {
                    page.push_str(&format!("- {metadata}\n"));
                }
            }
            page.push('\n');
            if let Some(description) = attribute.description() {
                page.push_str(&format!("{}\n\n", description_text(description)));
            }
        }
    }
    if !resource.defs().is_empty() {
        page.push_str("## Type Definitions\n\n");
        table(
            &mut page,
            ["Name", "Type"],
            resource.defs().iter().map(|(name, ty)| {
                Ok([
                    code_span(name, true)?,
                    types.describe(ty, true, &mut recursive)?,
                ])
            }),
        )?;
    }
    for section in ["Enum Values", "Struct Definitions", "Union Definitions"] {
        let mut started = false;
        for (id, ty) in types.types.iter().enumerate() {
            if !matches!(
                (section, ty.raw_shape()),
                ("Enum Values", RawShape::Enum { .. })
                    | ("Struct Definitions", RawShape::Struct { .. })
                    | ("Union Definitions", RawShape::Union(_))
            ) {
                continue;
            }
            if !started {
                page.push_str(&format!("## {section}\n\n"));
                started = true;
            }
            page.push_str(&format!("### {}\n\n", code_span(&types.labels[id], false)?));
            match ty.raw_shape() {
                RawShape::Enum {
                    identity,
                    base,
                    values,
                    dsl_aliases,
                    validate,
                    to_dsl,
                } => {
                    page.push_str(&format!(
                        "- **Base type:** {}\n\n",
                        types.describe(base, false, &mut recursive)?
                    ));
                    if validate.is_some() {
                        page.push_str("Values are also checked by a custom validator.\n\n");
                    }
                    if let Some(transform) = to_dsl {
                        let code =
                            code_span(&format!("{transform:?}"), false).with_context(|| {
                                format!(
                                    "cannot render transform at attribute `{}`",
                                    types.paths[id]
                                )
                            })?;
                        page.push_str(&format!("DSL transform: {code}.\n\n"));
                    }
                    if let Some(values) = values {
                        let mapping = DslMap::new(dsl_aliases, to_dsl);
                        table(
                            &mut page,
                            ["DSL Identifier", "Value"],
                            values.iter().map(|value| {
                                Ok([
                                    code_span(
                                        &format!("{identity}.{}", mapping.dsl_for(value)),
                                        true,
                                    )?,
                                    code_span(value, true)?,
                                ])
                            }),
                        )
                        .with_context(|| {
                            format!("cannot render enum at attribute `{}`", types.paths[id])
                        })?;
                    } else {
                        page.push_str("This enum's values are not enumerated by the schema.\n\n");
                    }
                }
                RawShape::Struct {
                    fields,
                    field_groups,
                    ..
                } => {
                    table(
                        &mut page,
                        ["Field", "Type", "Required", "Description"],
                        fields.iter().map(|field| {
                            // A definition records declared modes; read-only use sites keep their own mode.
                            let parts = field.parts();
                            let mode = parts.declared_input_mode;
                            let mut details = field_metadata(
                                parts.create_only,
                                parts.write_only,
                                parts.identity_mode,
                            )
                            .into_iter()
                            .map(|text| format!("{text}."))
                            .collect::<Vec<_>>();
                            if mode.is_provider_populated() {
                                details.insert(0, "**Read-only.**".into());
                            }
                            if let Some(description) = parts.description {
                                details.push(description_text(description));
                            }
                            Ok([
                                code_span(field.name(), true)?,
                                types.describe(field.field_type(), true, &mut recursive)?,
                                if mode.is_required() { "Yes" } else { "No" }.into(),
                                details.join(" ").replace('\n', " "),
                            ])
                        }),
                    )?;
                    render_field_groups(&mut page, field_groups)?;
                }
                RawShape::Union(members) => {
                    page.push_str("A value must match one of these member types.\n\n");
                    table(
                        &mut page,
                        ["Member", "Type"],
                        members.iter().enumerate().map(|(index, member)| {
                            Ok([
                                (index + 1).to_string(),
                                types.describe(member, true, &mut recursive)?,
                            ])
                        }),
                    )?;
                }
                _ => unreachable!("only selected declaration kinds"),
            }
        }
    }
    if !recursive.is_empty() {
        page.push_str("## Recursive Type Definitions\n\n");
        for id in recursive.clone() {
            page.push_str(&format!(
                "### {}\n\n- **Type:** {}\n\n",
                code_span(&types.labels[id], false)?,
                types.describe(types.types[id], false, &mut recursive)?
            ));
        }
    }
    carina_docgen::check_page(&page)?;
    Ok(page)
}

fn table<const N: usize>(
    page: &mut String,
    headers: [&str; N],
    rows: impl IntoIterator<Item = Result<[String; N]>>,
) -> Result<()> {
    page.push_str(&format!(
        "| {} |\n| {} |\n",
        headers.join(" | "),
        ["---"; N].join(" | ")
    ));
    for row in rows {
        page.push_str(&format!("| {} |\n", row?.join(" | ")));
    }
    page.push('\n');
    Ok(())
}

fn field_metadata(create_only: bool, write_only: bool, identity_mode: IdentityMode) -> Vec<String> {
    let mut metadata = Vec::new();
    if create_only {
        metadata.push("**Create-only:** Yes".into());
    }
    if write_only {
        metadata.push("**Write-only:** Yes".into());
    }
    match identity_mode {
        IdentityMode::None => {}
        IdentityMode::Required => metadata
            .push("**Identity:** Required (omitting this value leaves identity incomplete)".into()),
        IdentityMode::WhenPresent => {
            metadata.push("**Identity:** When present (omission is recorded as absent)".into())
        }
    }
    metadata
}

fn render_field_groups(page: &mut String, groups: &[FieldGroup]) -> Result<()> {
    if groups.is_empty() {
        return Ok(());
    }
    page.push_str("**Field groups:**\n\n");
    for group in groups {
        let cardinality = match group.kind() {
            FieldGroupKind::ExactlyOne => "Exactly one",
            FieldGroupKind::AtLeastOne => "At least one",
            FieldGroupKind::AtMostOne => "At most one",
        };
        page.push_str(&format!("- {cardinality} of: "));
        for (index, field) in group.fields().iter().enumerate() {
            if index > 0 {
                page.push_str(", ");
            }
            page.push_str(&code_span(field, false)?);
        }
        page.push_str(".\n");
    }
    page.push('\n');
    Ok(())
}

/// Traverse owned declarations without expanding Ref edges. Debug preserves the
/// SDK's declaration metadata; callback addresses distinguish its closure placeholders.
fn declaration_key(root: &CompiledAttributeType) -> Result<String> {
    let mut key = format!("{root:?}");
    let mut pending = vec![root];
    while let Some(ty) = pending.pop() {
        let validator = match ty.raw_shape() {
            RawShape::String { validate, .. }
            | RawShape::Int { validate, .. }
            | RawShape::Float { validate, .. }
            | RawShape::Enum { validate, .. } => validate,
            _ => None,
        };
        key.push_str(&format!("{:?};", validator.map(std::sync::Arc::as_ptr)));
        pending.extend(children(ty)?.into_iter().map(|(_, child)| child));
    }
    Ok(key)
}

fn children(ty: &CompiledAttributeType) -> Result<Vec<(String, &CompiledAttributeType)>> {
    Ok(match ty.raw_shape() {
        RawShape::UnloadedSchemaType(unloaded) => bail!("unloaded schema type: {unloaded:?}"),
        RawShape::Enum { base, .. } => vec![("base".into(), base)],
        RawShape::List { element_type, .. } => vec![("item".into(), element_type)],
        RawShape::Map { key, value, .. } => vec![("key".into(), key), ("value".into(), value)],
        RawShape::Struct { fields, .. } => fields
            .iter()
            .map(|field| (field.name().into(), field.field_type()))
            .collect(),
        RawShape::Union(members) => members
            .iter()
            .enumerate()
            .map(|(index, member)| (format!("member_{}", index + 1), member))
            .collect(),
        RawShape::Reference { .. }
        | RawShape::String { .. }
        | RawShape::Int { .. }
        | RawShape::Float { .. }
        | RawShape::Bool
        | RawShape::Duration
        | RawShape::Ref(_) => vec![],
    })
}

struct SchemaTypes<'a> {
    schema: &'a ValidatedResourceSchema,
    types: Vec<&'a CompiledAttributeType>,
    labels: Vec<String>,
    paths: Vec<String>,
    ids: HashMap<*const CompiledAttributeType, usize>,
    declarations: HashMap<String, usize>,
}

impl<'a> SchemaTypes<'a> {
    fn collect(schema: &'a ValidatedResourceSchema) -> Result<Self> {
        let mut types = Self {
            schema,
            types: Vec::new(),
            labels: Vec::new(),
            paths: Vec::new(),
            ids: HashMap::new(),
            declarations: HashMap::new(),
        };
        let resource = schema.as_schema();
        for attribute in resource.attributes().values() {
            types.intern(attribute.attr_type(), attribute.name().into())?;
        }
        for (name, ty) in resource.defs() {
            types.intern(ty, name.clone())?;
        }
        let mut index = 0;
        while index < types.types.len() {
            for (suffix, child) in children(types.types[index])? {
                types.intern(child, format!("{}.{suffix}", types.paths[index]))?;
            }
            index += 1;
        }
        // Reserve page structure and attribute headings before assigning declaration labels.
        let mut used: BTreeSet<_> = [
            "Example",
            "Argument Reference",
            "Attribute Reference",
            "Type Definitions",
            "Enum Values",
            "Struct Definitions",
            "Union Definitions",
            "Recursive Type Definitions",
        ]
        .into_iter()
        .chain(resource.attributes().keys().map(String::as_str))
        .map(carina_docgen::heading_slug)
        .collect();
        used.insert(carina_docgen::heading_slug(&format!(
            "{PROVIDER}.{}",
            resource.resource_type()
        )));
        for (id, ty) in types.types.iter().enumerate() {
            let name = match ty.raw_shape() {
                RawShape::Enum { identity, .. } => identity.to_string(),
                RawShape::Struct { name, .. } => name.to_string(),
                RawShape::Union(_) | RawShape::List { .. } | RawShape::Map { .. } => {
                    types.paths[id].clone()
                }
                _ => continue,
            };
            let mut label = name.clone();
            let mut suffix = 2;
            while !used.insert(carina_docgen::heading_slug(&label)) {
                label = format!("{name} ({suffix})");
                suffix += 1;
            }
            types.labels[id] = label;
        }
        Ok(types)
    }
    fn resolve_aliases(
        &self,
        mut ty: &'a CompiledAttributeType,
    ) -> Result<&'a CompiledAttributeType> {
        let mut visited = BTreeSet::new();
        while let RawShape::Ref(name) = ty.raw_shape() {
            if !visited.insert(name) {
                bail!("cyclic schema alias: {name}");
            }
            ty = self
                .schema
                .as_schema()
                .defs()
                .get(name)
                .with_context(|| format!("missing schema definition: {name}"))?;
        }
        Ok(ty)
    }
    fn intern(&mut self, ty: &'a CompiledAttributeType, path: String) -> Result<usize> {
        let ty = self
            .resolve_aliases(ty)
            .with_context(|| format!("cannot resolve type at {path}"))?;
        let address = std::ptr::from_ref(ty);
        if let Some(id) = self.ids.get(&address) {
            return Ok(*id);
        }
        let id = *self
            .declarations
            .entry(declaration_key(ty)?)
            .or_insert(self.types.len());
        self.ids.insert(address, id);
        if id == self.types.len() {
            self.types.push(ty);
            self.labels.push(path.clone());
            self.paths.push(path);
        }
        Ok(id)
    }
    fn id(&self, ty: &'a CompiledAttributeType) -> Result<usize> {
        self.ids
            .get(&std::ptr::from_ref(self.resolve_aliases(ty)?))
            .copied()
            .context("schema type was not collected")
    }
    fn link(&self, id: usize, prefix: &str, table: bool) -> String {
        let label = if prefix.is_empty() {
            self.labels[id].clone()
        } else {
            format!("{prefix}({})", self.labels[id])
        };
        let label = label.replace('[', r"\[").replace(']', r"\]");
        let label = if table {
            label.replace('|', r"\|")
        } else {
            label
        };
        format!(
            "[{label}](#{})",
            carina_docgen::heading_slug(&self.labels[id])
        )
    }
    fn describe(
        &self,
        ty: &'a CompiledAttributeType,
        table: bool,
        recursive: &mut BTreeSet<usize>,
    ) -> Result<String> {
        let id = self.id(ty)?;
        self.type_text(id, table, &mut Vec::new(), recursive)
            .with_context(|| format!("cannot render type at attribute `{}`", self.paths[id]))
    }
    fn type_text(
        &self,
        id: usize,
        table: bool,
        active: &mut Vec<usize>,
        recursive: &mut BTreeSet<usize>,
    ) -> Result<String> {
        if active.contains(&id) {
            recursive.insert(id);
            return Ok(self.link(id, "Recursive type ", table));
        }
        if active.len() >= 64 {
            bail!(
                "type nesting exceeds documentation limit at {}",
                self.paths[id]
            );
        }
        active.push(id);
        let nested = |ty, active: &mut Vec<usize>, recursive: &mut BTreeSet<usize>| {
            self.type_text(self.id(ty)?, table, active, recursive)
        };
        let scalar = |name: String, constraints: Vec<String>| {
            code_span(
                &if constraints.is_empty() {
                    name
                } else {
                    format!("{name} ({})", constraints.join(", "))
                },
                table,
            )
        };
        let text = match self.types[id].raw_shape() {
            RawShape::Reference {
                identity,
                pattern,
                length,
            } => scalar(identity.to_string(), string_constraints(pattern, length))?,
            RawShape::String {
                identity,
                pattern,
                length,
                validate,
                to_dsl,
            } => {
                let mut constraints = string_constraints(pattern, length);
                if validate.is_some() {
                    constraints.push("custom validator".into());
                }
                if let Some(transform) = to_dsl {
                    constraints.push(format!("DSL transform: {transform:?}"));
                }
                scalar(
                    identity.map_or_else(|| "String".into(), ToString::to_string),
                    constraints,
                )?
            }
            RawShape::Int {
                identity,
                range,
                validate,
            } => scalar(
                identity.map_or_else(|| "Int".into(), ToString::to_string),
                number_constraints(range, validate.is_some()),
            )?,
            RawShape::Float {
                identity,
                range,
                validate,
            } => scalar(
                identity.map_or_else(|| "Float".into(), ToString::to_string),
                number_constraints(range, validate.is_some()),
            )?,
            RawShape::Bool => scalar("Bool".into(), vec![])?,
            RawShape::Duration => scalar("Duration".into(), vec![])?,
            RawShape::Enum { .. } => self.link(id, "", table),
            RawShape::Struct { .. } => self.link(id, "Struct", table),
            RawShape::Union(_) => self.link(id, "Union", table),
            RawShape::List {
                element_type,
                ordered,
                constraints,
            } => {
                let mut refinements = Vec::new();
                if let Some(length) = constraints.length() {
                    refinements.push(format!("items: {}", bounds(length)));
                }
                if constraints.elements() == ListElements::Unique {
                    refinements.push("unique items".into());
                }
                refinements.push(if ordered { "ordered" } else { "unordered" }.into());
                format!(
                    "List\\<{}\\> ({})",
                    nested(element_type, active, recursive)?,
                    refinements.join(", ")
                )
            }
            // Map entry bounds are not rendered here: this local renderer is
            // slated for replacement by carina-docgen (carina-rs/carina-provider-github#20).
            RawShape::Map { key, value, .. } => format!(
                "Map\\<{}, {}\\>",
                nested(key, active, recursive)?,
                nested(value, active, recursive)?
            ),
            RawShape::UnloadedSchemaType(unloaded) => bail!("unloaded schema type: {unloaded:?}"),
            RawShape::Ref(_) => unreachable!("intern resolves aliases"),
        };
        active.pop();
        Ok(text)
    }
}

fn number_constraints<T: Display>(
    range: Option<(Option<T>, Option<T>)>,
    validate: bool,
) -> Vec<String> {
    let mut constraints = Vec::new();
    if let Some(range) = range {
        constraints.push(format!("range: {}", bounds(range)));
    }
    if validate {
        constraints.push("custom validator".into());
    }
    constraints
}

/// Preserve provider-authored Markdown without introducing ATX headings.
fn description_text(description: &str) -> String {
    description
        .split('\n')
        .map(|line| {
            let content = line.trim_start_matches(' ');
            let indent = line.len() - content.len();
            let after_hashes = content.trim_start_matches('#');
            let hashes = content.len() - after_hashes.len();
            if indent <= 3
                && (1..=6).contains(&hashes)
                && (after_hashes.trim_end_matches('\r').is_empty()
                    || after_hashes.starts_with([' ', '\t']))
            {
                format!("{}\\{content}", &line[..indent])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bounds<T: Display>((min, max): (Option<T>, Option<T>)) -> String {
    format!(
        "{}..{}",
        min.map(|value| value.to_string()).unwrap_or_default(),
        max.map(|value| format!("={value}")).unwrap_or_default()
    )
}

fn string_constraints(
    pattern: Option<&str>,
    length: Option<(Option<u64>, Option<u64>)>,
) -> Vec<String> {
    let mut constraints = Vec::new();
    if let Some(pattern) = pattern {
        constraints.push(format!("pattern: {pattern}"));
    }
    if let Some(length) = length {
        constraints.push(format!("len: {}", bounds(length)));
    }
    constraints
}

/// Literals retain their spelling in code spans. GFM removes the pipe escape
/// from table cells; controls must be visible because Markdown normalizes them.
fn code_span(literal: &str, table: bool) -> Result<String> {
    if literal.is_empty() {
        bail!("empty literal cannot be represented by a Markdown code span");
    }
    let mut code = String::new();
    let mut backslashes = 0;
    for character in literal.chars() {
        match character {
            '\n' => code.push_str(r"\n"),
            '\t' => code.push_str(r"\t"),
            c if c.is_control() => code.extend(c.escape_unicode()),
            '|' if table => {
                // GFM cannot round-trip a pipe preceded by an odd backslash run.
                if backslashes % 2 == 1 {
                    bail!("literal cannot round-trip in a GFM table cell: {literal:?}");
                }
                code.push_str(r"\|");
            }
            c => code.push(c),
        }
        backslashes = if character == '\\' {
            backslashes + 1
        } else {
            0
        };
    }
    let fence = "`".repeat(code.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1);
    let padding = if code.starts_with('`')
        || code.ends_with('`')
        || (code.starts_with(' ') && code.ends_with(' ') && !code.chars().all(|c| c == ' '))
    {
        " "
    } else {
        ""
    };
    let span = format!("{fence}{padding}{code}{padding}{fence}");
    carina_docgen::check_page(&span).context("literal rejected by carina-docgen")?;
    Ok(span)
}
