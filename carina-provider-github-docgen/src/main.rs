use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use carina_docgen::example::{self, DocExample, ExampleKind};
use carina_docgen::{
    Declaration, DeclarationKind, DocLink, Heading, PageBuilder, Resolved, TableCell, TypeDocHooks,
    TypeEdge, TypeRegistry, render_attribute_type, render_field_groups,
};
use carina_plugin_sdk::core::schema::{
    Compiled, CompiledAttributeType, DslMap, IdentityMode, RawShape, TypeIdentity,
    ValidatedResourceSchema,
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
    let mut types = SchemaTypes::collect(schema)?;
    let mut registry = TypeRegistry::default();
    // Discover every declaration before writing sections, preserving the existing
    // intern order even when a type is only used by a later definition.
    for id in 0..types.types.len() {
        let ty = types.types[id];
        render_attribute_type(ty, &(), &mut types, &mut registry, None)?;
        if let RawShape::Enum {
            identity,
            base,
            values: None,
            dsl_aliases,
            validate,
            to_dsl,
        } = ty.raw_shape()
        {
            // The shared renderer displays open enums as scalars. Keep their
            // existing definition sections, including validator/transform prose.
            types.register(
                &(),
                Declaration {
                    id,
                    ty,
                    kind: DeclarationKind::Enum {
                        identity,
                        base,
                        values: &[],
                        dsl_aliases,
                        validate,
                        to_dsl,
                    },
                },
            )?;
            types.open_enums.insert(id);
        }
    }
    let mut page = PageBuilder::new();
    page.heading(Heading::plain(
        1,
        format!("{PROVIDER}.{}", resource.resource_type()),
    ))?;
    if let Some(description) = resource.description() {
        page.push_str(&format!("{}\n\n", description_text(description)));
    }
    if resource.prevents_destroy_by_default() {
        page.push_str("Destruction protection is enabled by default. To opt out, set ");
        page.push_str(&inline_code("directives { prevent_destroy = false }")?);
        page.push_str(".\n\n");
    }
    if let Some(example) = example {
        let text = example.text_for(PROVIDER, resource.resource_type(), ExampleKind::Resource)?;
        page.heading(Heading::plain(2, "Example"))?;
        page.push_str(&format!("```crn\n{}\n```\n\n", text.trim_end_matches('\n')));
    }
    for (read_only, title) in [(false, "Argument Reference"), (true, "Attribute Reference")] {
        page.heading(Heading::plain(2, title))?;
        if !read_only {
            render_field_groups(&mut page, resource.field_groups());
        }
        for attribute in resource
            .attributes()
            .values()
            .filter(|a| a.is_read_only() == read_only)
        {
            let name = attribute.name();
            let ty =
                render_attribute_type(attribute.attr_type(), &(), &mut types, &mut registry, None)
                    .with_context(|| format!("cannot render type at attribute `{name}`"))?;
            page.heading(Heading::code(3, name))?;
            page.push_str("- **Type:** ");
            ty.write_to(&mut page);
            page.push_str("\n");
            if read_only {
                page.push_str("- **Read-only**\n");
            } else {
                page.push_str(&format!(
                    "- **Required:** {}\n",
                    if attribute.is_required() { "Yes" } else { "No" }
                ));
                if let Some(default) = attribute.default_value() {
                    let code = inline_code(&format_value(default))
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
            page.push_str("\n");
            if let Some(description) = attribute.description() {
                page.push_str(&format!("{}\n\n", description_text(description)));
            }
        }
    }
    if !resource.defs().is_empty() {
        page.heading(Heading::plain(2, "Type Definitions"))?;
        let rows = resource
            .defs()
            .iter()
            .map(|(name, ty)| {
                Ok([
                    TableCell::code(name),
                    render_attribute_type(ty, &(), &mut types, &mut registry, None)?.table_cell(),
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        page.table(["Name", "Type"], rows)?;
        page.push_str("\n");
    }
    for section in ["Enum Values", "Struct Definitions", "Union Definitions"] {
        let mut started = false;
        for id in 0..types.types.len() {
            let target = match (section, types.definitions.get(&id)) {
                ("Enum Values", Some(DeclarationKind::Enum { .. })) => TypeTarget::Enum(id),
                ("Struct Definitions", Some(DeclarationKind::Struct { .. })) => {
                    TypeTarget::Struct(id)
                }
                ("Union Definitions", Some(DeclarationKind::Union { .. })) => TypeTarget::Union(id),
                _ => continue,
            };
            let Some(definition) = types.definitions.remove(&id) else {
                continue;
            };
            if !started {
                page.heading(Heading::plain(2, section))?;
                started = true;
            }
            page.target_heading(target, Heading::code(3, &types.labels[id]))?;
            match definition {
                DeclarationKind::Enum {
                    identity,
                    base,
                    values,
                    dsl_aliases,
                    validate,
                    to_dsl,
                } => {
                    page.push_str("- **Base type:** ");
                    render_attribute_type(base, &(), &mut types, &mut registry, None)?
                        .write_to(&mut page);
                    page.push_str("\n\n");
                    if validate.is_some() {
                        page.push_str("Values are also checked by a custom validator.\n\n");
                    }
                    if let Some(transform) = to_dsl {
                        let code = inline_code(&format!("{transform:?}")).with_context(|| {
                            format!("cannot render transform at attribute `{}`", types.paths[id])
                        })?;
                        page.push_str(&format!("DSL transform: {code}.\n\n"));
                    }
                    if !types.open_enums.contains(&id) {
                        let mapping = DslMap::new(dsl_aliases, to_dsl);
                        page.table(
                            ["DSL Identifier", "Value"],
                            values.iter().map(|value| {
                                [
                                    TableCell::code(format!(
                                        "{identity}.{}",
                                        mapping.dsl_for(value)
                                    )),
                                    TableCell::code(value),
                                ]
                            }),
                        )
                        .with_context(|| {
                            format!("cannot render enum at attribute `{}`", types.paths[id])
                        })?;
                        page.push_str("\n");
                    } else {
                        page.push_str("This enum's values are not enumerated by the schema.\n\n");
                    }
                }
                DeclarationKind::Struct {
                    fields,
                    field_groups,
                    name: _name,
                } => {
                    let rows = fields
                        .iter()
                        .map(|field| {
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
                                TableCell::code(field.name()),
                                render_attribute_type(
                                    field.field_type(),
                                    &(),
                                    &mut types,
                                    &mut registry,
                                    None,
                                )?
                                .table_cell(),
                                TableCell::text(if mode.is_required() { "Yes" } else { "No" }),
                                TableCell::text(details.join(" ")),
                            ])
                        })
                        .collect::<Result<Vec<_>>>()?;
                    page.table(["Field", "Type", "Required", "Description"], rows)?;
                    page.push_str("\n");
                    render_field_groups(&mut page, field_groups);
                }
                DeclarationKind::Union { members } => {
                    page.push_str("A value must match one of these member types.\n\n");
                    let rows = members
                        .iter()
                        .enumerate()
                        .map(|(index, member)| {
                            Ok([
                                TableCell::text((index + 1).to_string()),
                                render_attribute_type(
                                    member,
                                    &(),
                                    &mut types,
                                    &mut registry,
                                    None,
                                )?
                                .table_cell(),
                            ])
                        })
                        .collect::<Result<Vec<_>>>()?;
                    page.table(["Member", "Type"], rows)?;
                    page.push_str("\n");
                }
                DeclarationKind::Recursive => unreachable!("recursive definitions are separate"),
            }
        }
    }
    if !types.recursive.is_empty() {
        page.heading(Heading::plain(2, "Recursive Type Definitions"))?;
        for id in types.recursive.clone() {
            page.target_heading(
                TypeTarget::Recursive(id),
                Heading::code(3, &types.labels[id]),
            )?;
            page.push_str("- **Type:** ");
            render_attribute_type(types.types[id], &(), &mut types, &mut registry, None)?
                .write_to(&mut page);
            page.push_str("\n\n");
        }
    }
    Ok(page.finish()?)
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

/// Traverse owned declarations without expanding Ref edges. Debug preserves the
/// SDK's declaration metadata; callback addresses distinguish its closure placeholders.
fn declaration_key(root: &CompiledAttributeType) -> Result<String> {
    let mut key = format!("{root:?}");
    let mut pending = vec![root];
    while let Some(ty) = pending.pop() {
        let validator = match ty.raw_shape() {
            RawShape::String {
                identity: _identity,
                pattern: _pattern,
                length: _length,
                validate,
                to_dsl: _to_dsl,
            } => validate,
            RawShape::Int {
                identity: _identity,
                range: _range,
                validate,
            } => validate,
            RawShape::Float {
                identity: _identity,
                range: _range,
                validate,
            } => validate,
            RawShape::Enum {
                identity: _identity,
                base: _base,
                values: _values,
                dsl_aliases: _dsl_aliases,
                validate,
                to_dsl: _to_dsl,
            } => validate,
            RawShape::Reference {
                identity: _identity,
                pattern: _pattern,
                length: _length,
            } => None,
            RawShape::List {
                element_type: _element_type,
                ordered: _ordered,
                constraints: _constraints,
            } => None,
            RawShape::Map {
                key: _key,
                value: _value,
                constraints: _constraints,
            } => None,
            RawShape::Struct {
                name: _name,
                fields: _fields,
                field_groups: _field_groups,
            } => None,
            RawShape::Bool | RawShape::Duration => None,
            RawShape::Union(_members) => None,
            RawShape::Ref(_name) => None,
            RawShape::UnloadedSchemaType(unloaded) => bail!("unloaded schema type: {unloaded:?}"),
        };
        key.push_str(&format!("{:?};", validator.map(std::sync::Arc::as_ptr)));
        pending.extend(children(ty)?.into_iter().map(|(_, child)| child));
    }
    Ok(key)
}

fn children(ty: &CompiledAttributeType) -> Result<Vec<(String, &CompiledAttributeType)>> {
    Ok(match ty.raw_shape() {
        RawShape::UnloadedSchemaType(unloaded) => bail!("unloaded schema type: {unloaded:?}"),
        RawShape::Enum {
            identity: _identity,
            base,
            values: _values,
            dsl_aliases: _dsl_aliases,
            validate: _validate,
            to_dsl: _to_dsl,
        } => vec![("base".into(), base)],
        RawShape::List {
            element_type,
            ordered: _ordered,
            constraints: _constraints,
        } => vec![("item".into(), element_type)],
        RawShape::Map {
            key,
            value,
            constraints: _constraints,
        } => vec![("key".into(), key), ("value".into(), value)],
        RawShape::Struct {
            name: _name,
            fields,
            field_groups: _field_groups,
        } => fields
            .iter()
            .map(|field| (field.name().into(), field.field_type()))
            .collect(),
        RawShape::Union(members) => members
            .iter()
            .enumerate()
            .map(|(index, member)| (format!("member_{}", index + 1), member))
            .collect(),
        RawShape::Reference {
            identity: _identity,
            pattern: _pattern,
            length: _length,
        } => vec![],
        RawShape::String {
            identity: _identity,
            pattern: _pattern,
            length: _length,
            validate: _validate,
            to_dsl: _to_dsl,
        } => vec![],
        RawShape::Int {
            identity: _identity,
            range: _range,
            validate: _validate,
        } => vec![],
        RawShape::Float {
            identity: _identity,
            range: _range,
            validate: _validate,
        } => vec![],
        RawShape::Bool | RawShape::Duration => vec![],
        RawShape::Ref(_name) => vec![],
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TypeTarget {
    Enum(usize),
    Struct(usize),
    Union(usize),
    Recursive(usize),
}

struct SchemaTypes<'a> {
    schema: &'a ValidatedResourceSchema,
    types: Vec<&'a CompiledAttributeType>,
    labels: Vec<String>,
    paths: Vec<String>,
    ids: HashMap<*const CompiledAttributeType, usize>,
    declarations: HashMap<String, usize>,
    definitions: BTreeMap<usize, DeclarationKind<'a, Compiled>>,
    open_enums: BTreeSet<usize>,
    recursive: BTreeSet<usize>,
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
            definitions: BTreeMap::new(),
            open_enums: BTreeSet::new(),
            recursive: BTreeSet::new(),
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
                RawShape::Enum {
                    identity,
                    base: _base,
                    values: _values,
                    dsl_aliases: _dsl_aliases,
                    validate: _validate,
                    to_dsl: _to_dsl,
                } => identity.to_string(),
                RawShape::Struct {
                    name,
                    fields: _fields,
                    field_groups: _field_groups,
                } => name.to_string(),
                RawShape::List {
                    element_type: _element_type,
                    ordered: _ordered,
                    constraints: _constraints,
                } => types.paths[id].clone(),
                RawShape::Map {
                    key: _key,
                    value: _value,
                    constraints: _constraints,
                } => types.paths[id].clone(),
                RawShape::Union(_members) => types.paths[id].clone(),
                RawShape::Reference {
                    identity: _identity,
                    pattern: _pattern,
                    length: _length,
                } => continue,
                RawShape::String {
                    identity: _identity,
                    pattern: _pattern,
                    length: _length,
                    validate: _validate,
                    to_dsl: _to_dsl,
                } => continue,
                RawShape::Int {
                    identity: _identity,
                    range: _range,
                    validate: _validate,
                } => continue,
                RawShape::Float {
                    identity: _identity,
                    range: _range,
                    validate: _validate,
                } => continue,
                RawShape::Bool | RawShape::Duration => continue,
                RawShape::Ref(_name) => continue,
                RawShape::UnloadedSchemaType(unloaded) => {
                    bail!("unloaded schema type: {unloaded:?}")
                }
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
}

impl<'a> TypeDocHooks<'a, Compiled> for SchemaTypes<'a> {
    type NodeId = usize;
    type Target = TypeTarget;
    // Paths and labels are allocated during interning, before shared traversal.
    type Scope = ();

    fn resolve(&self, ty: &'a CompiledAttributeType) -> Result<Resolved<'a, Compiled, usize>> {
        let id = self.id(ty)?;
        // Use canonical children as well as canonical IDs: equivalent cloned
        // declarations need not have independently interned child addresses.
        Ok(Resolved {
            id,
            ty: self.types[id],
        })
    }

    fn scalar_name(&self, identity: &TypeIdentity) -> String {
        identity.to_string()
    }

    fn child_scope(&self, _parent: &(), _edge: TypeEdge<'_>) {}

    fn register(
        &mut self,
        _scope: &(),
        declaration: Declaration<'a, Compiled, usize>,
    ) -> Result<DocLink<TypeTarget>> {
        let id = declaration.id;
        let target = match &declaration.kind {
            DeclarationKind::Enum { .. } => TypeTarget::Enum(id),
            DeclarationKind::Struct { .. } => TypeTarget::Struct(id),
            DeclarationKind::Union { .. } => TypeTarget::Union(id),
            DeclarationKind::Recursive => TypeTarget::Recursive(id),
        };
        if matches!(declaration.kind, DeclarationKind::Recursive) {
            self.recursive.insert(id);
        } else {
            self.definitions.insert(id, declaration.kind);
        }
        Ok(DocLink {
            target,
            label: self.labels[id].clone(),
        })
    }
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

/// The shared builder exposes code literals through headings and table cells.
/// Use its code heading for standalone inline literals too, stripping only the
/// heading wrapper so defaults and transform prose use the same shared escaper.
fn inline_code(literal: &str) -> Result<String> {
    let mut fragment = PageBuilder::<()>::new();
    fragment.heading(Heading::code(1, literal))?;
    let markdown = fragment.finish()?;
    Ok(markdown
        .strip_prefix("# ")
        .and_then(|code| code.strip_suffix("\n\n"))
        .context("shared code heading has no heading wrapper")?
        .to_string())
}
