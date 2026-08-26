use std::collections::{BTreeMap, HashSet};

use hegel::generators as gs;
use jsonschema::{
    canonical::{
        CanonicalSchema, Containment, ContainsView, Distinctness, ObjectViolationView,
        Satisfiability,
    },
    Draft,
};
use serde_json::{json, Number, Value};

use super::{
    fraction::Fraction, pool::arbitrary_scalar, size_ceiling, size_floor, Sampler, MAX_ATTEMPTS,
    MAX_SIZE,
};

/// The array constraints of one canonical node, in the view's own vocabulary.
pub(crate) struct ArrayFacets {
    pub(crate) min_items: Option<Number>,
    pub(crate) max_items: Option<Number>,
    pub(crate) distinctness: Distinctness,
    pub(crate) prefix_items: Vec<CanonicalSchema>,
    pub(crate) items: Option<CanonicalSchema>,
    pub(crate) contains: Vec<ContainsView>,
}

/// The object constraints of one canonical node, in the view's own vocabulary.
pub(crate) struct ObjectFacets {
    pub(crate) draft: Draft,
    pub(crate) min_properties: Option<Number>,
    pub(crate) max_properties: Option<Number>,
    pub(crate) required: Vec<String>,
    pub(crate) property_names: Option<CanonicalSchema>,
    pub(crate) properties: BTreeMap<String, CanonicalSchema>,
    pub(crate) pattern_properties: BTreeMap<String, CanonicalSchema>,
    pub(crate) additional_properties: Option<CanonicalSchema>,
    pub(crate) violations: Vec<ObjectViolationView>,
}

// What `uniqueItems` counts as one value: numbers compare by value across their forms.
fn uniqueness_key(value: &Value) -> String {
    match value {
        Value::Number(number) => match Fraction::from_number(number) {
            Some(fraction) => format!("{}/{}", fraction.numerator, fraction.denominator),
            None => value.to_string(),
        },
        _ => value.to_string(),
    }
}

fn node_validator(schema: &CanonicalSchema) -> Option<jsonschema::Validator> {
    jsonschema::options()
        .with_draft(schema.draft())
        .build(&schema.to_json_schema())
        .ok()
}

// The validator's own ECMA engine decides which names a pattern matches.
fn pattern_matcher(draft: Draft, pattern: &str) -> Option<jsonschema::Validator> {
    jsonschema::options()
        .with_draft(draft)
        .build(&json!({"type": "string", "pattern": pattern}))
        .ok()
}

struct Demand<'a> {
    schema: &'a CanonicalSchema,
    need: u64,
}

// A demanded element also answers to `items`.
fn demanded_element(demand: &CanonicalSchema, items: Option<&CanonicalSchema>) -> CanonicalSchema {
    match items {
        Some(tail) => match demand.intersect(tail) {
            Ok(joint) => joint,
            Err(_) => demand.clone(),
        },
        None => demand.clone(),
    }
}

fn plan_demands(contains: &[ContainsView]) -> Option<Vec<Demand<'_>>> {
    let mut demands = Vec::new();
    for demand in contains {
        let need = demand
            .min_contains
            .as_ref()
            .map_or(1, |bound| bound.as_u64().unwrap_or(u64::MAX));
        let cap = size_ceiling(demand.max_contains.as_ref());
        if need > cap || need > MAX_SIZE {
            return None;
        }
        demands.push(Demand {
            schema: &demand.schema,
            need,
        });
    }
    Some(demands)
}

/// Meet each demand inside the prefix where a position covers or can be narrowed toward it;
/// what remains comes back as element schemas to append.
fn carry_demands(
    demands: &[Demand<'_>],
    prefix: &mut [CanonicalSchema],
    items: Option<&CanonicalSchema>,
) -> Vec<CanonicalSchema> {
    let mut appends = Vec::new();
    for demand in demands {
        let mut missing = demand.need;
        // Prefix positions the demand already covers carry matches of their own.
        for entry in prefix.iter() {
            if missing > 0 && matches!(demand.schema.covers(entry), Ok(Containment::Yes)) {
                missing -= 1;
            }
        }
        // Narrowing a further prefix position toward the demand spends no appended slot.
        for entry in prefix.iter_mut() {
            if missing == 0 {
                break;
            }
            if matches!(demand.schema.covers(entry), Ok(Containment::Yes)) {
                continue;
            }
            if let Ok(narrowed) = entry.intersect(demand.schema) {
                if narrowed.satisfiability() != Satisfiability::No {
                    *entry = narrowed;
                    missing -= 1;
                }
            }
        }
        for _ in 0..missing {
            appends.push(demanded_element(demand.schema, items));
        }
    }
    appends
}

pub(crate) fn draw_array(sampler: &Sampler<'_>, facets: ArrayFacets) -> Option<Value> {
    let ArrayFacets {
        min_items,
        max_items,
        distinctness,
        prefix_items,
        items,
        contains,
    } = facets;
    let floor = size_floor(min_items.as_ref());
    if floor > MAX_SIZE {
        return None;
    }
    let ceiling = size_ceiling(max_items.as_ref());
    if ceiling < floor {
        return None;
    }
    let capped_ceiling = usize::try_from(ceiling.min(MAX_SIZE + 4)).expect("capped ceiling fits");
    // An array shorter than the prefix is still an instance, so the prefix stops at the
    // length ceiling.
    let mut prefix: Vec<CanonicalSchema> = prefix_items.into_iter().take(capped_ceiling).collect();
    let demands = plan_demands(&contains)?;
    let mut appends = carry_demands(&demands, &mut prefix, items.as_ref());
    let mut drawn = Vec::new();
    let mut truncated = false;
    for entry in &prefix {
        // An undrawable position truncates the array there rather than declining it.
        if let Some(value) = sampler.descend(entry) {
            drawn.push(value);
        } else {
            truncated = true;
            break;
        }
    }
    if truncated {
        // The carried counts assumed the whole prefix; behind a truncation every demand
        // appends its full count.
        appends.clear();
        for demand in &demands {
            for _ in 0..demand.need {
                appends.push(demanded_element(demand.schema, items.as_ref()));
            }
        }
    }
    if drawn.len() + appends.len() > capped_ceiling {
        return None;
    }
    for element in &appends {
        drawn.push(sampler.descend(element)?);
    }
    // Filler that provably cannot match a bounded demand holds its ceiling on its own.
    let steered = items.as_ref().and_then(|tail| {
        let mut narrowed = tail.clone();
        for demand in contains
            .iter()
            .filter(|demand| demand.max_contains.is_some())
        {
            narrowed = demand
                .schema
                .negate()
                .ok()
                .and_then(|complement| narrowed.intersect(&complement).ok())?;
        }
        (narrowed.satisfiability() != Satisfiability::No).then_some(narrowed)
    });
    let fill = || -> Option<Value> {
        match (&steered, &items) {
            (Some(steered), _) => sampler.descend(steered),
            (None, Some(tail)) => sampler.descend(tail),
            (None, None) => Some(sampler.tc.draw(arbitrary_scalar())),
        }
    };
    while (drawn.len() as u64) < floor {
        drawn.push(fill()?);
    }
    match distinctness {
        Distinctness::AllDistinct => {
            let mut seen = HashSet::new();
            drawn.retain(|item| seen.insert(uniqueness_key(item)));
            // Deduplication can fall below the floor; a few fresh draws may climb back.
            for _ in 0..4 {
                if drawn.len() as u64 >= floor {
                    break;
                }
                let fresh = fill()?;
                if seen.insert(uniqueness_key(&fresh)) {
                    drawn.push(fresh);
                }
            }
            if (drawn.len() as u64) < floor {
                return None;
            }
        }
        Distinctness::SomeRepeated => {
            if drawn.len() < 2 {
                return None;
            }
            let first = drawn[0].clone();
            if drawn.len() < capped_ceiling {
                drawn.push(first);
            } else {
                // No room to append, so the last position carries the repeat.
                let last = drawn.len() - 1;
                drawn[last] = first;
            }
        }
        Distinctness::Unconstrained => {}
    }
    Some(Value::Array(drawn))
}

struct ObjectDraw<'a> {
    sampler: &'a Sampler<'a>,
    facets: &'a ObjectFacets,
    // Pattern schemas paired with the validator deciding which names they match.
    claims: Vec<(jsonschema::Validator, &'a CanonicalSchema)>,
}

impl ObjectDraw<'_> {
    /// The value a key answers to: its named schema and every matching pattern schema at once.
    fn value_for(&self, key: &str) -> Option<Value> {
        let mut parts: Vec<&CanonicalSchema> = Vec::new();
        if let Some(entry) = self.facets.properties.get(key) {
            parts.push(entry);
        }
        for (matcher, entry) in &self.claims {
            if matcher.is_valid(&Value::String(key.to_owned())) {
                parts.push(entry);
            }
        }
        match parts.as_slice() {
            [] => match &self.facets.additional_properties {
                Some(additional) => self.sampler.descend(additional),
                None => Some(self.sampler.tc.draw(arbitrary_scalar())),
            },
            [only] => self.sampler.descend(only),
            [first, rest @ ..] => {
                let mut joint = (*first).clone();
                for part in rest {
                    joint = joint.intersect(part).ok()?;
                }
                self.sampler.descend(&joint)
            }
        }
    }

    /// A fresh key name honoring the name constraint, or `None` when this attempt has none.
    fn drawn_name(&self) -> Option<String> {
        match &self.facets.property_names {
            Some(names) => match self.sampler.descend(names) {
                Some(Value::String(name)) => Some(name),
                _ => None,
            },
            None => Some(self.sampler.tc.draw(gs::text().max_size(5))),
        }
    }

    fn inject_name_violation(
        &self,
        object: &mut serde_json::Map<String, Value>,
        bar: &CanonicalSchema,
    ) -> Option<()> {
        let refuses = node_validator(bar)?;
        for _ in 0..MAX_ATTEMPTS {
            let Some(name) = self.drawn_name() else {
                continue;
            };
            if object.contains_key(&name) || refuses.is_valid(&Value::String(name.clone())) {
                continue;
            }
            let Some(value) = self.value_for(&name) else {
                continue;
            };
            object.insert(name, value);
            return Some(());
        }
        None
    }

    fn inject_undeclared_violation(
        &self,
        object: &mut serde_json::Map<String, Value>,
        names: &[String],
        patterns: &[String],
        additional: &CanonicalSchema,
    ) -> Option<()> {
        let matchers: Vec<jsonschema::Validator> = patterns
            .iter()
            .filter_map(|pattern| pattern_matcher(self.facets.draft, pattern))
            .collect();
        if matchers.len() != patterns.len() {
            return None;
        }
        let complement = additional.negate().ok();
        let rejects = node_validator(additional)?;
        for _ in 0..MAX_ATTEMPTS {
            let name = self.sampler.tc.draw(gs::text().max_size(5));
            if object.contains_key(&name)
                || names.contains(&name)
                || matchers
                    .iter()
                    .any(|matcher| matcher.is_valid(&Value::String(name.clone())))
            {
                continue;
            }
            let value = if let Some(complement) = &complement {
                self.sampler.descend(complement)
            } else {
                let candidate = self.sampler.tc.draw(arbitrary_scalar());
                (!rejects.is_valid(&candidate)).then_some(candidate)
            };
            let Some(value) = value else {
                continue;
            };
            object.insert(name, value);
            return Some(());
        }
        None
    }
}

pub(crate) fn draw_object(sampler: &Sampler<'_>, facets: &ObjectFacets) -> Option<Value> {
    let floor = size_floor(facets.min_properties.as_ref());
    if floor > MAX_SIZE {
        return None;
    }
    let ceiling = size_ceiling(facets.max_properties.as_ref());
    let injected = facets.violations.len() as u64;
    if ceiling < floor || facets.required.len() as u64 + injected > ceiling {
        return None;
    }
    // Violation entries land last, each with a key of its own, so the base draw keeps
    // that much of the ceiling free.
    let budget = ceiling - injected;
    let claims: Vec<(jsonschema::Validator, &CanonicalSchema)> = facets
        .pattern_properties
        .iter()
        .filter_map(|(pattern, entry)| {
            pattern_matcher(facets.draft, pattern).map(|matcher| (matcher, entry))
        })
        .collect();
    let draw = ObjectDraw {
        sampler,
        facets,
        claims,
    };
    let mut object = serde_json::Map::new();
    for key in &facets.required {
        let value = draw.value_for(key)?;
        object.insert(key.clone(), value);
    }
    for key in facets.properties.keys() {
        if object.len() as u64 >= budget {
            break;
        }
        if !object.contains_key(key) && sampler.tc.draw(gs::booleans()) {
            // An optional entry whose value cannot be drawn is simply left out.
            if let Some(value) = draw.value_for(key) {
                object.insert(key.clone(), value);
            }
        }
    }
    for pattern in facets.pattern_properties.keys().take(2) {
        if object.len() as u64 >= budget {
            break;
        }
        if regex::Regex::new(pattern).is_err() {
            continue;
        }
        if sampler.tc.draw(gs::booleans()) {
            let key = sampler.tc.draw(gs::from_regex(pattern));
            if !object.contains_key(&key) {
                if let Some(value) = draw.value_for(&key) {
                    object.insert(key, value);
                }
            }
        }
    }
    // Keys past the named ones, drawn from the name constraint where one stands.
    for _ in 0..2 {
        if object.len() as u64 >= budget {
            break;
        }
        if !sampler.tc.draw(gs::booleans()) {
            continue;
        }
        let Some(name) = draw.drawn_name() else {
            continue;
        };
        if object.contains_key(&name) {
            continue;
        }
        if let Some(value) = draw.value_for(&name) {
            object.insert(name, value);
        }
    }
    // Keys filling the floor come from the name constraint where one stands; a closed
    // name set can run out, and the attempts cap turns that into a decline.
    let fill_target = floor.saturating_sub(injected).min(budget);
    for _ in 0..MAX_ATTEMPTS {
        if object.len() as u64 >= fill_target {
            break;
        }
        let name = match &facets.property_names {
            Some(_) => match draw.drawn_name() {
                Some(name) => name,
                None => continue,
            },
            None => format!("k{}", object.len()),
        };
        if object.contains_key(&name) {
            continue;
        }
        let Some(value) = draw.value_for(&name) else {
            continue;
        };
        object.insert(name, value);
    }
    if (object.len() as u64) < fill_target {
        return None;
    }
    for violation in &facets.violations {
        match violation {
            ObjectViolationView::NameFails(bar) => {
                draw.inject_name_violation(&mut object, bar)?;
            }
            ObjectViolationView::UndeclaredValueFails {
                names,
                patterns,
                additional,
            } => {
                draw.inject_undeclared_violation(&mut object, names, patterns, additional)?;
            }
        }
    }
    if (object.len() as u64) < floor {
        return None;
    }
    Some(Value::Object(object))
}
