//! Structural complement of a canonical node.
use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Number, Value};

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        emptiness,
        ir::{
            type_set_schema, ArrayLeaf, AtLeastTwo, BoundCardinality, BoundInteger, BoundNumber,
            CanonicalJson, ContainsFacet, Discrete, Distinctness, Divisors, ExcludedDivisors,
            IntegerBounds, IntegerLeaf, LengthBounds, NumberLeaf, ObjectLeaf, ObjectViolation,
            Schema, SchemaKind, StringLeaf,
        },
        DefinitionMap, ROOT_DEFINITION_KEY,
    },
    JsonType, JsonTypeSet,
};

/// Resolutions one walk may spend before declining: a complement growing past this many inlined
/// targets costs more to spell than the caller can put to use, and the recursion it would take
/// runs ahead of the stack.
const RESOLUTION_BUDGET: usize = 1024;

/// Shared regions one choice's complement may spell: their number grows with the square of the
/// branch count, and a complement past this many costs more to assemble than the caller can put to
/// use.
const OVERLAP_BUDGET: usize = 64;

/// State of one resolving negation walk.
struct NegationWalk<'a> {
    definitions: &'a DefinitionMap,
    /// References being negated on the current path.
    active: Vec<Arc<str>>,
    /// Resolutions left before the walk declines.
    budget: usize,
    unspellable: Unspellable,
}

/// What the walk does where the complement has no structural form: either the exact `Not` re-wrap,
/// or nothing at all.
#[derive(Clone, Copy)]
enum Unspellable {
    /// The node keeps its complement symbolic under `Not`.
    Bar,
    /// The walk declines, so nothing it returns depends on which targets are already known.
    Decline,
}

/// The complement of a node, taking that node's place inside the document being canonicalized, or
/// `None` when the IR cannot spell it and the caller keeps the document `Raw`. Negation has no safe
/// default direction, so every arm is exact or declines.
///
/// A walk reaching every target it needs through `definitions` inlines their complements; one that
/// cannot keeps the whole node symbolic rather than resolving the part it reached, so the form
/// follows the document's reference graph and not the order its targets were canonicalized in.
pub(crate) fn negate_in_place(
    schema: &Schema,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let mut walk = NegationWalk {
        definitions,
        active: Vec::new(),
        budget: RESOLUTION_BUDGET,
        unspellable: Unspellable::Decline,
    };
    if let Some(complement) = negate_within(schema, &mut walk, ctx) {
        return Some(complement);
    }
    let detached = DefinitionMap::new();
    let mut walk = NegationWalk {
        definitions: &detached,
        active: Vec::new(),
        budget: RESOLUTION_BUDGET,
        unspellable: Unspellable::Bar,
    };
    negate_within(schema, &mut walk, ctx)
}

/// [`negate_in_place`], for a complement that replaces the document root instead. A walk that
/// reaches a reference already being negated on the current path has no finite complement and
/// declines; a complement still naming the root would name the wrong document once it takes the
/// root's place, and declines as well.
pub(crate) fn negate_with_definitions(
    schema: &Schema,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let mut walk = NegationWalk {
        definitions,
        active: Vec::new(),
        budget: RESOLUTION_BUDGET,
        unspellable: Unspellable::Bar,
    };
    let complement = negate_within(schema, &mut walk, ctx)?;
    let mut references = Vec::new();
    emptiness::collect_classified_references(
        &complement,
        emptiness::Position::InPlace,
        &mut references,
    );
    if references
        .iter()
        .any(|(uri, _)| uri.as_ref() == ROOT_DEFINITION_KEY)
    {
        return None;
    }
    Some(complement)
}

/// The exact `Not` re-wrap of a node the walk cannot open, for a walk that accepts one.
fn bar(schema: &Schema, walk: &NegationWalk<'_>) -> Option<Schema> {
    match walk.unspellable {
        Unspellable::Bar => Some(Schema::new(SchemaKind::Not(schema.clone()))),
        Unspellable::Decline => None,
    }
}

fn negate_within(
    schema: &Schema,
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    match schema.kind() {
        SchemaKind::True => Some(Schema::new(SchemaKind::False)),
        SchemaKind::False => Some(Schema::new(SchemaKind::True)),
        SchemaKind::MultiType(set) => negate_type_set(*set, ctx),
        SchemaKind::Const(value) => negate_finite_values(std::slice::from_ref(value), ctx),
        SchemaKind::Enum(values) => negate_finite_values(values.as_slice(), ctx),
        SchemaKind::Number(leaf) => negate_number_leaf(leaf.get(), ctx),
        SchemaKind::Integer(leaf) => negate_integer_leaf(leaf.get(), ctx),
        SchemaKind::String(leaf) => negate_string_leaf(leaf.get(), ctx),
        SchemaKind::Array(leaf) => negate_array_leaf(leaf.get(), walk, ctx),
        SchemaKind::Object(leaf) => negate_object_leaf(leaf.get(), walk, ctx),
        SchemaKind::Not(inner) => {
            // A target spelling its own complement leaves nothing consistent to return.
            if let SchemaKind::Reference(uri) = inner.kind() {
                if walk.active.iter().any(|name| name == uri) {
                    return None;
                }
            }
            Some(inner.clone())
        }
        // De Morgan: the complement of a union is the intersection of the branch complements, so
        // one inexpressible branch declines the whole node.
        SchemaKind::AnyOf(branches) => {
            let mut result = Schema::new(SchemaKind::True);
            for branch in branches.as_slice() {
                result = algebra::intersect(result, negate_within(branch, walk, ctx)?, ctx);
            }
            Some(result)
        }
        // De Morgan in the other direction restores a union when every branch has an exact
        // structural complement. Otherwise the whole conjunction stays opaque.
        SchemaKind::AllOf(branches) => {
            let mut complements = Vec::with_capacity(branches.as_slice().len());
            for branch in branches.as_slice() {
                let Some(complement) = negate_within(branch, walk, ctx) else {
                    return bar(schema, walk);
                };
                complements.push(complement);
            }
            Some(algebra::union(complements, ctx))
        }
        SchemaKind::OneOf(branches) => negate_one_of(branches, walk, ctx),
        SchemaKind::Reference(uri) => negate_reference(schema, uri, walk, ctx),
        SchemaKind::TypedGroup { ty, body } => negate_typed_group(*ty, body, walk, ctx),
        SchemaKind::Raw(_) => None,
    }
}

/// A choice holds where exactly one branch does, so its complement holds where no branch does and
/// where two branches share the value. Intersection is total, so every shared region is expressible
/// and only the half no branch matches rests on branch complements.
/// ```text
/// e.g.  {"not": {"oneOf": [{"$ref": "#/$defs/a"}, {"type": "integer"}]}}
///       =>  anyOf: [{"allOf": [{"type": "integer"}, {"$ref": "#/$defs/a"}]},
///                   {"allOf": [{"type": ["null", "boolean", "string", "array", "object"]},
///                              {"not": {"$ref": "#/$defs/a"}}]},
///                   {"allOf": [{"type": "number", "not": {"multipleOf": 1}},
///                              {"not": {"$ref": "#/$defs/a"}}]}]
/// ```
fn negate_one_of(
    branches: &[Schema],
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let mut regions = Vec::new();
    for (index, left) in branches.iter().enumerate() {
        for right in &branches[index + 1..] {
            let shared = algebra::intersect(left.clone(), right.clone(), ctx);
            if matches!(shared.kind(), SchemaKind::False) {
                continue;
            }
            if regions.len() == OVERLAP_BUDGET {
                return None;
            }
            regions.push(shared);
        }
    }
    let depth = walk.active.len();
    let budget = walk.budget;
    let mut matched_by_none = Schema::new(SchemaKind::True);
    for branch in branches {
        matched_by_none =
            algebra::intersect(matched_by_none, negate_within(branch, walk, ctx)?, ctx);
    }
    // The branch complements resolve references through the same walk, so they leave the path they
    // found and can only have spent budget on it.
    debug_assert_eq!(
        walk.active.len(),
        depth,
        "a branch complement left the negation path unbalanced"
    );
    debug_assert!(
        walk.budget <= budget,
        "a branch complement refilled the resolution budget"
    );
    regions.push(matched_by_none);
    Some(algebra::union(regions, ctx))
}

/// The complement of the reference's target. A target already being negated on the current path
/// admits no finite complement, and a target this map does not name leaves the reference opaque.
/// A complement that merely re-wraps the target hands the caller back the problem it asked about,
/// so it declines instead.
fn negate_reference(
    schema: &Schema,
    uri: &Arc<str>,
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    if walk.active.iter().any(|name| name == uri) {
        return None;
    }
    let Some(target) = walk.definitions.get(uri.as_ref()) else {
        return bar(schema, walk);
    };
    walk.budget = walk.budget.checked_sub(1)?;
    // Every active entry is a distinct key of the map, so the walk is bounded by its size.
    debug_assert!(
        walk.active.len() < walk.definitions.len(),
        "more active negations than definitions"
    );
    walk.active.push(Arc::clone(uri));
    let complement = negate_within(target, walk, ctx);
    let finished = walk.active.pop();
    debug_assert_eq!(finished.as_ref(), Some(uri), "unbalanced negation path");
    let complement = complement?;
    if matches!(complement.kind(), SchemaKind::Not(inner) if inner == target) {
        return None;
    }
    Some(complement)
}

/// De Morgan over the conjunction a typed group spells: the values off the type, and the values of
/// the type that the body rejects.
/// ```text
/// e.g.  draft 4: {"not": {"type": "integer", "enum": [1, 2]}}
///       =>  anyOf: [<non-integer types>, {"type": "integer", "maximum": 0},
///                   {"type": "integer", "minimum": 3}, {"type": "number", "not": {"type": "integer"}}]
/// ```
fn negate_typed_group(
    ty: JsonType,
    body: &Schema,
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let off_type = negate_type_set(JsonTypeSet::from(ty), ctx)?;
    let off_body = negate_within(body, walk, ctx)?;
    let within = algebra::intersect(type_set_schema(JsonTypeSet::from(ty)), off_body, ctx);
    Some(algebra::union(vec![off_type, within], ctx))
}

/// Complement of a finite value set: the untouched types stay whole, an unpaired boolean leaves the
/// other one, the numeric members carve rays and gaps out of the number line, the string members
/// become exclusions on the strings, and an empty container leaves the sizes above it.
/// ```text
/// e.g.  {"not": {"const": null}}  =>  {"type": ["boolean", "number", "string", "array", "object"]}
/// e.g.  {"not": {"const": []}}
///       =>  anyOf: [<non-array types>, {"type": "array", "minItems": 1}]
/// e.g.  {"not": {"const": [1]}}  =>  unchanged: array inequality is inexpressible
/// ```
fn negate_finite_values(values: &[CanonicalJson], ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut remaining = JsonTypeSet::all();
    let mut booleans = Vec::new();
    let mut numbers: Vec<Number> = Vec::new();
    let mut strings: Vec<Arc<str>> = Vec::new();
    let mut empty_array = false;
    let mut empty_object = false;
    for value in values {
        match value.as_value() {
            Value::Null => remaining = remaining.remove(JsonType::Null),
            Value::Bool(member) => {
                remaining = remaining.remove(JsonType::Boolean);
                booleans.push(*member);
            }
            Value::Number(number) => {
                remaining = remaining.remove(JsonType::Number).remove(JsonType::Integer);
                numbers.push(number.clone());
            }
            Value::String(text) => {
                remaining = remaining.remove(JsonType::String);
                strings.push(Arc::from(text.as_str()));
            }
            // An empty container is the only one of its size, so the sizes above it are the rest of
            // its type. Any other one needs a value to differ somewhere, which no facet spells.
            Value::Array(items) if items.is_empty() => {
                remaining = remaining.remove(JsonType::Array);
                empty_array = true;
            }
            Value::Object(entries) if entries.is_empty() => {
                remaining = remaining.remove(JsonType::Object);
                empty_object = true;
            }
            Value::Array(_) | Value::Object(_) => return None,
        }
    }
    let mut branches = vec![type_set_schema(remaining)];
    if empty_array {
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: above_empty(),
                distinctness: Distinctness::Unconstrained,
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    if empty_object {
        branches.push(object_branch(
            above_empty(),
            Vec::new(),
            BTreeMap::new(),
            ctx,
        ));
    }
    if let [member] = booleans.as_slice() {
        branches.push(Schema::new(SchemaKind::Const(CanonicalJson::from_value(
            &Value::Bool(!member),
        ))));
    }
    branches.extend(number_gaps(&numbers, ctx)?);
    if !strings.is_empty() {
        strings.sort();
        strings.dedup();
        branches.push(algebra::string_leaf(
            StringLeaf {
                lengths: LengthBounds::default(),
                patterns: Vec::new(),
                excluded_patterns: Vec::new(),
                formats: Vec::new(),
                excluded_formats: Vec::new(),
                content_media_types: Vec::new(),
                content_encodings: Vec::new(),
                excluded: strings,
            },
            ctx,
        ));
    }
    Some(algebra::union(branches, ctx))
}

/// The number-line complement of a finite set of numbers: the outer rays and the open gaps
/// between neighbours. Empty input adds nothing - the whole `number` type then stays remaining.
/// A gap the integers cannot spell declines the whole complement; dropping that one branch would
/// narrow the union.
fn number_gaps(numbers: &[Number], ctx: &CanonicalizationContext) -> Option<Vec<Schema>> {
    if numbers.is_empty() {
        return Some(Vec::new());
    }
    let mut ends: Vec<BoundNumber> = numbers
        .iter()
        .map(|number| BoundNumber::new(number, false))
        .collect();
    ends.sort();
    let mut branches = Vec::with_capacity(ends.len() + 1);
    let mut lower: Option<BoundNumber> = None;
    for end in ends {
        branches.push(number_window(lower.take(), Some(end.clone()), ctx)?);
        lower = Some(end);
    }
    branches.push(number_window(lower, None, ctx)?);
    Some(branches)
}

/// A window over the reals, or `None` when the integers it admits fall outside this build's range.
/// Such a window can still meet `type: integer`, where an integer window is the only form left to
/// carry it; clamping its ends into range would drop integers the window keeps.
fn number_window(
    minimum: Option<BoundNumber>,
    maximum: Option<BoundNumber>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let leaf = NumberLeaf {
        minimum,
        maximum,
        multiple_of: Divisors::default(),
        not_multiple_of: ExcludedDivisors::default(),
        excludes_integers: false,
    };
    algebra::integer_bounds_within(&leaf)?;
    Some(algebra::number_leaf(leaf, ctx))
}

/// Complement of a number window: the values of every other type plus the outer rays, each
/// endpoint's inclusivity flipped. A value escapes a run of divisors as soon as it misses one, and
/// a run of exclusions as soon as it lands on one, so each divisor flips into its dual on its own
/// branch. `None` where a flipped end leaves a ray the canonical form cannot spell.
/// ```text
/// e.g.  {"not": {"type": "number", "minimum": 5}}
///       =>  anyOf: [<non-number types>, {"type": "number", "exclusiveMaximum": 5}]
/// e.g.  {"not": {"type": "number", "multipleOf": 0.5}}
///       =>  anyOf: [<non-number types>, {"type": "number", "not": {"multipleOf": 0.5}}]
/// ```
fn negate_number_leaf(leaf: &NumberLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut branches = vec![type_set_schema(
        JsonTypeSet::all()
            .remove(JsonType::Number)
            .remove(JsonType::Integer),
    )];
    if let Some(minimum) = &leaf.minimum {
        branches.push(number_window(None, Some(flipped(minimum)), ctx)?);
    }
    if let Some(maximum) = &leaf.maximum {
        branches.push(number_window(Some(flipped(maximum)), None, ctx)?);
    }
    if leaf.excludes_integers {
        branches.push(type_set_schema(JsonTypeSet::from(JsonType::Integer)));
    }
    branches.extend(leaf.multiple_of.as_slice().iter().map(|step| {
        algebra::number_leaf(
            NumberLeaf {
                not_multiple_of: ExcludedDivisors::one(step.clone()),
                ..NumberLeaf::default()
            },
            ctx,
        )
    }));
    branches.extend(leaf.not_multiple_of.as_slice().iter().map(|step| {
        algebra::number_leaf(
            NumberLeaf {
                multiple_of: Divisors::one(step.clone()),
                ..NumberLeaf::default()
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// Complement of an integer window: every other type, the non-integer numbers, and one branch per
/// facet violation, or `None` where the canonical form cannot spell it exactly.
/// ```text
/// e.g.  {"not": {"type": "integer", "minimum": 0}}
///       =>  anyOf: [<non-number types>,
///                   {"type": "integer", "maximum": -1},
///                   {"type": "number", "not": {"multipleOf": 1}}]
/// ```
fn negate_integer_leaf(leaf: &IntegerLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut branches = vec![type_set_schema(
        JsonTypeSet::all()
            .remove(JsonType::Number)
            .remove(JsonType::Integer),
    )];
    branches.push(non_integer_number(ctx));
    // An end at the edge of this build's integer range leaves the ray beyond it unspellable.
    if let Some(minimum) = &leaf.bounds.minimum {
        let below = minimum.clone().checked_decrement()?;
        branches.push(integer_window(None, Some(below), ctx));
    }
    if let Some(maximum) = &leaf.bounds.maximum {
        let above = maximum.clone().checked_increment()?;
        branches.push(integer_window(Some(above), None, ctx));
    }
    branches.extend(leaf.multiple_of.as_slice().iter().map(|step| {
        algebra::integer_leaf(
            IntegerLeaf {
                not_multiple_of: ExcludedDivisors::one(step.clone()),
                ..IntegerLeaf::default()
            },
            ctx,
        )
    }));
    branches.extend(leaf.not_multiple_of.as_slice().iter().map(|step| {
        algebra::integer_leaf(
            IntegerLeaf {
                multiple_of: Divisors::one(step.clone()),
                ..IntegerLeaf::default()
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// The numbers outside the draft's integers.
fn non_integer_number(ctx: &CanonicalizationContext) -> Schema {
    algebra::number_leaf(
        NumberLeaf {
            excludes_integers: true,
            ..NumberLeaf::default()
        },
        ctx,
    )
}

fn integer_window(
    minimum: Option<BoundInteger>,
    maximum: Option<BoundInteger>,
    ctx: &CanonicalizationContext,
) -> Schema {
    algebra::integer_leaf(
        IntegerLeaf {
            bounds: IntegerBounds { minimum, maximum },
            ..IntegerLeaf::default()
        },
        ctx,
    )
}

/// The sizes a container holding something can take.
fn above_empty() -> LengthBounds {
    LengthBounds {
        minimum: Some(BoundCardinality::from(1)),
        maximum: None,
    }
}

/// A value set holding exactly these strings.
fn finite_strings(values: &[Arc<str>]) -> Schema {
    let members: Vec<CanonicalJson> = values
        .iter()
        .map(|value| CanonicalJson::from_value(&Value::String(value.to_string())))
        .collect();
    match AtLeastTwo::new(members) {
        Ok(set) => Schema::new(SchemaKind::Enum(set)),
        Err(mut single) => Schema::new(SchemaKind::Const(
            single.pop().expect("a non-empty exclusion list"),
        )),
    }
}

/// The same limit admitting exactly the values the original end rejects.
fn flipped(bound: &BoundNumber) -> BoundNumber {
    BoundNumber::new(&bound.to_number(), !bound.is_inclusive())
}

/// Complements of a count window: the ray below the floor and the ray above the ceiling. A floor
/// of zero excludes nothing below it; a ceiling with no successor in this build declines.
pub(crate) fn length_windows(lengths: &LengthBounds) -> Option<Vec<LengthBounds>> {
    let mut windows = Vec::new();
    if let Some(below) = lengths
        .minimum
        .as_ref()
        .and_then(|minimum| minimum.clone().checked_decrement())
    {
        windows.push(LengthBounds {
            minimum: None,
            maximum: Some(below),
        });
    }
    if let Some(maximum) = &lengths.maximum {
        let above = maximum.clone().checked_increment()?;
        windows.push(LengthBounds {
            minimum: Some(above),
            maximum: None,
        });
    }
    Some(windows)
}

/// A demanded pattern inverts into a barred one and a barred pattern inverts back into a demanded
/// one, so each pattern the leaf names contributes its own branch - exactly as formats do.
/// ```text
/// e.g.  {"not": {"type": "string", "minLength": 3}}
///       =>  anyOf: [<non-string types>, {"type": "string", "maxLength": 2}]
/// e.g.  {"not": {"type": "string", "pattern": "^a"}}
///       =>  anyOf: [<non-string types>,
///                   {"type": "string", "allOf": [{"not": {"pattern": "^a"}}]}]
/// ```
fn negate_string_leaf(leaf: &StringLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if !leaf.excluded.is_empty() {
        // The dual of the arm above: a leaf that only excludes values complements to those values.
        let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::String))];
        branches.push(finite_strings(&leaf.excluded));
        let positive = StringLeaf {
            excluded: Vec::new(),
            ..leaf.clone()
        };
        branches.push(negate_string_leaf(&positive, ctx)?);
        return Some(algebra::union(branches, ctx));
    }
    if !leaf.content_media_types.is_empty() || !leaf.content_encodings.is_empty() {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::String))];
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::string_leaf(
            StringLeaf {
                lengths,
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    // A string fails a run of formats as soon as it fails one of them, so each gets its own branch
    // - and a branch barring one format says nothing about the length or the others.
    branches.extend(leaf.formats.iter().map(|format| {
        algebra::string_leaf(
            StringLeaf {
                excluded_formats: vec![Arc::clone(format)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    branches.extend(leaf.excluded_formats.iter().map(|format| {
        algebra::string_leaf(
            StringLeaf {
                formats: vec![Arc::clone(format)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    // A string fails a run of patterns as soon as it fails one of them, so each gets its own branch
    // - and a branch barring one pattern says nothing about the length or the others.
    branches.extend(leaf.patterns.iter().map(|pattern| {
        algebra::string_leaf(
            StringLeaf {
                excluded_patterns: vec![Arc::clone(pattern)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    branches.extend(leaf.excluded_patterns.iter().map(|pattern| {
        algebra::string_leaf(
            StringLeaf {
                patterns: vec![Arc::clone(pattern)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// An element schema fails on an array exactly when one element violates it, which is a `contains`
/// demand for its complement. A demand for one match fails exactly when every element violates it,
/// which is the same trade the other way round. A positional schema constrains only the arrays long
/// enough to reach its index, so its violation carries that length as a floor. Distinctness is its
/// own dual: a demand that every element differ fails exactly when two of them coincide.
/// ```text
/// e.g.  {"not": {"type": "array", "maxItems": 2}}
///       =>  anyOf: [<non-array types>, {"type": "array", "minItems": 3}]
/// e.g.  {"not": {"type": "array", "items": {"type": "string"}}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "contains": {"type": <every type but string>}}]
/// e.g.  {"not": {"type": "array", "contains": {"type": "string"}}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "items": {"type": <every type but string>}}]
/// e.g.  {"not": {"type": "array", "prefixItems": [{"type": "string"}]}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "prefixItems": [{"type": <every type but string>}],
///                    "minItems": 1}]
/// e.g.  {"not": {"type": "array", "uniqueItems": true}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "minItems": 2,
///                    "allOf": [{"not": {"type": "array", "uniqueItems": true}}]}]
/// ```
fn negate_array_leaf(
    leaf: &ArrayLeaf,
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    // A demand names no position, so it cannot ask for a violation past the prefix and leave the
    // positions in front of it alone.
    if !leaf.prefix.is_empty() && leaf.items.is_some() {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Array))];
    let flipped = match leaf.distinctness {
        Distinctness::Unconstrained => None,
        Distinctness::AllDistinct => Some(Distinctness::SomeRepeated),
        Distinctness::SomeRepeated => Some(Distinctness::AllDistinct),
    };
    if let Some(distinctness) = flipped {
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds::default(),
                distinctness,
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    debug_assert!(
        leaf.prefix.is_empty() || leaf.items.is_none(),
        "a positional leaf reached the position branches carrying a tail"
    );
    for (index, schema) in leaf.prefix.iter().enumerate() {
        let mut prefix = vec![Schema::new(SchemaKind::True); index];
        prefix.push(negate_within(schema, walk, ctx)?);
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds {
                    minimum: Some(BoundCardinality::from(index as u64 + 1)),
                    maximum: None,
                },
                distinctness: Distinctness::Unconstrained,
                prefix,
                items: None,
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    if let Some(items) = &leaf.items {
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds::default(),
                distinctness: Distinctness::Unconstrained,
                prefix: Vec::new(),
                items: None,
                contains: vec![ContainsFacet {
                    schema: negate_within(items, walk, ctx)?,
                    minimum: None,
                    maximum: None,
                }],
            },
            ctx,
        ));
    }
    for facet in &leaf.contains {
        // Missing a window on the count means landing anywhere else in it, and an element schema
        // holding for every element can only say "nowhere".
        if facet.maximum.is_some() || facet.effective_minimum() != BoundCardinality::from(1) {
            return None;
        }
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds::default(),
                distinctness: Distinctness::Unconstrained,
                prefix: Vec::new(),
                items: Some(negate_within(&facet.schema, walk, ctx)?),
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::array_leaf(
            ArrayLeaf {
                lengths,
                distinctness: Distinctness::Unconstrained,
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// The leaf is a conjunction of facets over objects, so its complement is the union of the
/// per-facet complements beside the non-object types: a size window flips into its outer rays, a
/// required key into its absence, a property schema into the key held with a violating value, and a
/// key constraint into a demand for a key that breaks it.
/// ```text
/// e.g.  {"not": {"type": "object", "required": ["a"], "minProperties": 2}}
///       =>  anyOf: [<non-object types>,
///                   {"type": "object", "properties": {"a": false}},
///                   {"type": "object", "maxProperties": 1}]
/// e.g.  {"not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}
///       =>  anyOf: [<non-object types>, {"type": "object", "not": {"propertyNames": {"enum": ["a", "b"]}}}]
/// ```
fn negate_object_leaf(
    leaf: &ObjectLeaf,
    walk: &mut NegationWalk<'_>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    if !leaf.pattern_properties.is_empty() {
        return None;
    }
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Object))];
    for sizes in length_windows(&leaf.sizes)? {
        branches.push(object_branch(sizes, Vec::new(), BTreeMap::new(), ctx));
    }
    for key in &leaf.required {
        let absent = BTreeMap::from([(key.clone(), Schema::new(SchemaKind::False))]);
        branches.push(object_branch(
            LengthBounds::default(),
            Vec::new(),
            absent,
            ctx,
        ));
    }
    for (key, schema) in &leaf.properties {
        let violating = negate_within(schema, walk, ctx)?;
        let held = BTreeMap::from([(key.clone(), violating)]);
        branches.push(object_branch(
            LengthBounds::default(),
            vec![key.clone()],
            held,
            ctx,
        ));
    }
    // A key constraint fails on an object exactly when some key breaks it: that is exactly the
    // demand recorded below.
    if let Some(names) = &leaf.property_names {
        branches.push(algebra::object_leaf(
            ObjectLeaf {
                violations: vec![ObjectViolation::NameFails(names.clone())],
                ..ObjectLeaf::default()
            },
            ctx,
        ));
    }
    // A value shield fails on an object exactly when some key outside `properties` and
    // `patternProperties` holds a value it rejects: the demand records that declared key set so
    // the shield's reach stays exact once reinstated.
    if let Some(shield) = &leaf.additional {
        branches.push(algebra::object_leaf(
            ObjectLeaf {
                violations: vec![ObjectViolation::UndeclaredValueFails {
                    names: leaf.properties.keys().cloned().collect(),
                    patterns: leaf.pattern_properties.keys().cloned().collect(),
                    additional: shield.clone(),
                }],
                ..ObjectLeaf::default()
            },
            ctx,
        ));
    }
    for violation in &leaf.violations {
        match violation {
            ObjectViolation::NameFails(violated) => {
                branches.push(algebra::object_leaf(
                    ObjectLeaf {
                        property_names: Some(violated.clone()),
                        ..ObjectLeaf::default()
                    },
                    ctx,
                ));
            }
            ObjectViolation::UndeclaredValueFails {
                names,
                patterns,
                additional,
            } => {
                branches.push(algebra::object_leaf(
                    ObjectLeaf {
                        properties: names
                            .iter()
                            .map(|name| (name.clone(), Schema::new(SchemaKind::True)))
                            .collect(),
                        pattern_properties: patterns
                            .iter()
                            .map(|pattern| (pattern.clone(), Schema::new(SchemaKind::True)))
                            .collect(),
                        additional: Some(additional.clone()),
                        ..ObjectLeaf::default()
                    },
                    ctx,
                ));
            }
        }
    }
    Some(algebra::union(branches, ctx))
}

fn object_branch(
    sizes: LengthBounds,
    required: Vec<std::sync::Arc<str>>,
    properties: BTreeMap<std::sync::Arc<str>, Schema>,
    ctx: &CanonicalizationContext,
) -> Schema {
    algebra::object_leaf(
        ObjectLeaf {
            sizes,
            required,
            property_names: None,
            properties,
            pattern_properties: BTreeMap::new(),
            additional: None,
            violations: Vec::new(),
        },
        ctx,
    )
}

/// Complement of a type set over the value space. A set admitting `integer` but not `number`
/// leaves the non-integer numbers to a numeric facet no type set can name.
/// ```text
/// e.g.  {"not": {"type": "string"}}  =>  {"type": ["null", "boolean", "number", "array", "object"]}
/// e.g.  {"not": {"type": "integer"}}
///       =>  anyOf: [<non-number types>, {"type": "number", "not": {"multipleOf": 1}}]
/// ```
fn negate_type_set(set: JsonTypeSet, ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut complement = JsonTypeSet::empty();
    for ty in [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ] {
        if !set.contains(ty) {
            complement = complement.insert(ty);
        }
    }
    if set.contains(JsonType::Integer) && !set.contains(JsonType::Number) {
        let mut branches = vec![non_integer_number(ctx)];
        if !complement.is_empty() {
            branches.push(type_set_schema(complement));
        }
        return Some(algebra::union(branches, ctx));
    }
    // A set carrying `number` admits every number, so its complement admits none; a set carrying
    // neither numeric type admits no number, so its complement admits all of them.
    if !set.contains(JsonType::Number) {
        complement = complement.insert(JsonType::Number);
    }
    if complement.is_empty() {
        return Some(Schema::new(SchemaKind::False));
    }
    // The shared constructor, so a complement spelling a lone `null` or `boolean` lands on the same
    // canonical node as the direct spelling.
    Some(type_set_schema(complement))
}

#[cfg(test)]
mod tests {
    use referencing::Draft;
    use serde_json::{json, Value};

    use super::*;
    use crate::{canonical::ir::BoundRational, options::PatternEngineOptions};

    fn context() -> CanonicalizationContext {
        CanonicalizationContext::new(Draft::Draft202012, PatternEngineOptions::default(), false)
    }

    const TYPES: [JsonType; 7] = [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::Integer,
        JsonType::Number,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ];

    // One value per equivalence class of the type vocabulary; `1` and `1.5` are distinct classes
    // because an integer satisfies both `integer` and `number` while a fraction satisfies only
    // `number`.
    fn representatives() -> [Value; 7] {
        [
            json!(null),
            json!(true),
            json!(1),
            json!(1.5),
            json!("x"),
            json!([]),
            json!({}),
        ]
    }

    fn admits(set: JsonTypeSet, value: &Value) -> bool {
        match value {
            Value::Null => set.contains(JsonType::Null),
            Value::Bool(_) => set.contains(JsonType::Boolean),
            Value::Number(number) if number.is_i64() => {
                set.contains(JsonType::Integer) || set.contains(JsonType::Number)
            }
            Value::Number(_) => set.contains(JsonType::Number),
            Value::String(_) => set.contains(JsonType::String),
            Value::Array(_) => set.contains(JsonType::Array),
            Value::Object(_) => set.contains(JsonType::Object),
        }
    }

    // Membership for the canonical shapes a complement can take: a type set, its boolean-schema
    // collapses, the value-set spellings of a lone `null` or `boolean` type, and the
    // non-integer-number leaf beside its union.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn complement_admits(schema: &Schema, value: &Value) -> bool {
        match schema.kind() {
            SchemaKind::True => true,
            SchemaKind::False => false,
            SchemaKind::MultiType(set) => admits(*set, value),
            SchemaKind::Const(constant) => {
                assert_eq!(constant.as_value(), &Value::Null);
                value.is_null()
            }
            SchemaKind::Enum(values) => {
                let members: Vec<&Value> = values
                    .as_slice()
                    .iter()
                    .map(CanonicalJson::as_value)
                    .collect();
                assert_eq!(members, [&Value::Bool(false), &Value::Bool(true)]);
                value.is_boolean()
            }
            SchemaKind::AnyOf(branches) => branches
                .as_slice()
                .iter()
                .any(|branch| complement_admits(branch, value)),
            SchemaKind::Number(leaf) => {
                assert!(leaf.get().minimum.is_none());
                assert!(leaf.get().maximum.is_none());
                assert!(leaf.get().multiple_of.is_empty());
                let barred: Vec<Number> = leaf
                    .get()
                    .not_multiple_of
                    .as_slice()
                    .iter()
                    .map(BoundRational::to_number)
                    .collect();
                assert_eq!(barred, [Number::from(1)]);
                matches!(value, Value::Number(number) if !number.is_i64() && !number.is_u64())
            }
            other => {
                panic!("scaffold complement of a type set is a type-set shape, got {other:?}")
            }
        }
    }

    // The scaffold's domain is finite, so the complement-membership law is proven exhaustively: for
    // every one of the 128 type sets, the complement admits a value exactly when the original does
    // not.
    #[test]
    fn type_set_complement_partitions_the_value_space() {
        let ctx = context();
        for mask in 0u8..128 {
            let mut set = JsonTypeSet::empty();
            for ty in TYPES {
                if mask & ty as u8 != 0 {
                    set = set.insert(ty);
                }
            }
            let schema = Schema::new(SchemaKind::MultiType(set));
            let complement = negate_in_place(&schema, &DefinitionMap::new(), &ctx)
                .expect("expressible complement");
            for value in &representatives() {
                assert_ne!(
                    admits(set, value),
                    complement_admits(&complement, value),
                    "membership not partitioned for set {set:?} on {value}"
                );
            }
        }
    }

    #[test]
    fn boolean_schemas_negate_to_each_other() {
        let ctx = context();
        assert!(matches!(
            negate_in_place(&Schema::new(SchemaKind::True), &DefinitionMap::new(), &ctx)
                .map(|s| s.kind().clone()),
            Some(SchemaKind::False)
        ));
        assert!(matches!(
            negate_in_place(&Schema::new(SchemaKind::False), &DefinitionMap::new(), &ctx)
                .map(|s| s.kind().clone()),
            Some(SchemaKind::True)
        ));
    }
}
