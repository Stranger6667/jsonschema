use crate::{paths::Location, ValidationError};
use ahash::AHashMap;
use referencing::{JsonPointerSegment, Uri};
use serde::{
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
    Serialize,
};
use std::{
    borrow::Cow,
    fmt,
    sync::{Arc, LazyLock, OnceLock},
};

/// Annotations associated with an output unit.
#[derive(Clone)]
pub struct Annotations(Arc<AnnotationsValue>);

enum AnnotationsValue {
    Value(serde_json::Value),
    /// An array of names the schema declares, built into a value when it is read.
    #[cfg(feature = "macros")]
    Names {
        names: Box<[&'static str]>,
        value: OnceLock<serde_json::Value>,
    },
}

#[cfg(feature = "macros")]
fn names_value(names: &[&'static str]) -> serde_json::Value {
    serde_json::Value::Array(
        names
            .iter()
            .map(|name| serde_json::Value::String((*name).to_owned()))
            .collect(),
    )
}

impl Annotations {
    #[must_use]
    pub(crate) fn new(v: serde_json::Value) -> Self {
        Annotations(Arc::new(AnnotationsValue::Value(v)))
    }

    /// A shared annotation of `value`; boolean annotations allocate nothing.
    #[must_use]
    pub(crate) fn boolean(value: bool) -> Self {
        static TRUE: LazyLock<Annotations> =
            LazyLock::new(|| Annotations::new(serde_json::Value::Bool(true)));
        static FALSE: LazyLock<Annotations> =
            LazyLock::new(|| Annotations::new(serde_json::Value::Bool(false)));
        if value {
            TRUE.clone()
        } else {
            FALSE.clone()
        }
    }

    /// An array of `names`.
    #[cfg(feature = "macros")]
    #[must_use]
    pub(crate) fn names(names: Box<[&'static str]>) -> Self {
        Annotations(Arc::new(AnnotationsValue::Names {
            names,
            value: OnceLock::new(),
        }))
    }

    /// Returns the inner [`serde_json::Value`] of the annotation.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> serde_json::Value {
        match Arc::try_unwrap(self.0) {
            Ok(AnnotationsValue::Value(value)) => value,
            #[cfg(feature = "macros")]
            Ok(AnnotationsValue::Names { names, value }) => {
                value.into_inner().unwrap_or_else(|| names_value(&names))
            }
            Err(shared) => Annotations(shared).value().clone(),
        }
    }

    /// The `serde_json::Value` of the annotation.
    #[must_use]
    pub fn value(&self) -> &serde_json::Value {
        match &*self.0 {
            AnnotationsValue::Value(value) => value,
            #[cfg(feature = "macros")]
            AnnotationsValue::Names { names, value } => value.get_or_init(|| names_value(names)),
        }
    }
}

impl fmt::Debug for Annotations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Annotations").field(self.value()).finish()
    }
}

impl PartialEq for Annotations {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl serde::Serialize for Annotations {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &*self.0 {
            AnnotationsValue::Value(value) => value.serialize(serializer),
            #[cfg(feature = "macros")]
            AnnotationsValue::Names { names, value } => match value.get() {
                Some(value) => value.serialize(serializer),
                None => names.serialize(serializer),
            },
        }
    }
}

/// Description of a validation error used within evaluation outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDescription {
    keyword: Cow<'static, str>,
    message: String,
}

impl ErrorDescription {
    /// A `keyword` error with `message`, for generated validators.
    #[doc(hidden)]
    #[inline]
    #[must_use]
    pub fn described(keyword: &'static str, message: String) -> Self {
        Self::new(keyword, message)
    }

    #[inline]
    #[must_use]
    pub(crate) fn new(keyword: &'static str, message: String) -> Self {
        Self {
            keyword: Cow::Borrowed(keyword),
            message,
        }
    }

    #[inline]
    #[must_use]
    pub(crate) fn from_validation_error(e: &ValidationError<'_>) -> Self {
        ErrorDescription {
            keyword: match e.kind().keyword_name() {
                Ok(keyword) => Cow::Borrowed(keyword),
                Err(custom) => Cow::Owned(custom.to_owned()),
            },
            message: render_message(e),
        }
    }

    /// Returns the keyword associated with this error.
    #[inline]
    #[must_use]
    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    /// Returns the inner [`String`] of the error description.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> String {
        self.message
    }

    /// Returns the message of the error description.
    #[inline]
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Formats `error` into a stack buffer first, so a message that fits `MESSAGE_BUFFER` is allocated
/// once at its final length instead of growing through every power of two below it.
fn render_message(error: &ValidationError<'_>) -> String {
    let mut writer = MessageWriter {
        buffer: [0; MESSAGE_BUFFER],
        len: 0,
        spilled: None,
    };
    fmt::write(&mut writer, format_args!("{error}")).expect("formatting into memory cannot fail");
    match writer.spilled {
        Some(message) => message,
        None => std::str::from_utf8(&writer.buffer[..writer.len])
            .expect("only whole `str` pieces are written")
            .to_owned(),
    }
}

const MESSAGE_BUFFER: usize = 128;

struct MessageWriter {
    buffer: [u8; MESSAGE_BUFFER],
    len: usize,
    spilled: Option<String>,
}

impl fmt::Write for MessageWriter {
    fn write_str(&mut self, piece: &str) -> fmt::Result {
        if let Some(message) = &mut self.spilled {
            message.push_str(piece);
        } else if let Some(slot) = self.buffer.get_mut(self.len..self.len + piece.len()) {
            slot.copy_from_slice(piece.as_bytes());
            self.len += piece.len();
        } else {
            let written = std::str::from_utf8(&self.buffer[..self.len])
                .expect("only whole `str` pieces are written");
            let mut message = String::with_capacity(self.len + piece.len());
            message.push_str(written);
            message.push_str(piece);
            self.spilled = Some(message);
        }
        Ok(())
    }
}

impl fmt::Display for ErrorDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) const NO_NODE: u32 = u32::MAX;

/// Set on a child link that names an entry of `EvaluationArena::runs` rather than a node.
const RUN_LINK: u32 = 1 << 31;

/// An instance position recorded during evaluation. Its JSON Pointer is rendered on first read,
/// since most positions are never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstancePosition(u32);

impl InstancePosition {
    /// The instance itself, implicit in every arena.
    pub const ROOT: InstancePosition = InstancePosition(0);
}

#[derive(Debug, Clone, Copy)]
struct Position {
    parent: u32,
    segment: PositionSegment,
}

#[derive(Debug, Clone, Copy)]
enum PositionSegment {
    /// A byte range of `InstancePositions::keys`.
    Key {
        start: u32,
        end: u32,
    },
    Index(usize),
    /// An index into `InstancePositions::prerendered`, for positions whose parent is unknown.
    Rendered(u32),
}

#[derive(Debug, Default)]
struct InstancePositions {
    /// Every position but the root, which is implicit. Parents come before their children, so one
    /// forward pass renders every position. Chunks are never resized, so growth copies nothing.
    entries: Vec<Vec<Position>>,
    len: usize,
    /// The names of every `Key` segment, concatenated, so recording a position allocates nothing
    /// of its own.
    keys: String,
    prerendered: Vec<Location>,
    /// Every position's pointer, rendered together on the first read.
    rendered: OnceLock<Box<[Location]>>,
}

impl InstancePositions {
    fn push(&mut self, position: Position) -> InstancePosition {
        debug_assert!(
            self.rendered.get().is_none(),
            "positions are recorded before any is read"
        );
        let id = u32::try_from(self.len + 1).expect("evaluation exceeded u32 positions");
        if self.len % POSITION_CHUNK == 0 {
            self.entries.push(Vec::with_capacity(POSITION_CHUNK));
        }
        self.entries
            .last_mut()
            .expect("a chunk was just reserved")
            .push(position);
        self.len += 1;
        InstancePosition(id)
    }

    fn below(&self, parent: &Location, segment: PositionSegment) -> Location {
        match segment {
            PositionSegment::Key { start, end } => parent.join_pointer_segment(
                &JsonPointerSegment::Key(Cow::Borrowed(&self.keys[start as usize..end as usize])),
            ),
            PositionSegment::Index(index) => {
                parent.join_pointer_segment(&JsonPointerSegment::Index(index))
            }
            PositionSegment::Rendered(index) => self.prerendered[index as usize].clone(),
        }
    }

    fn location(&self, position: InstancePosition) -> &Location {
        let rendered = self.rendered.get_or_init(|| {
            let mut rendered: Vec<Location> = Vec::with_capacity(self.len + 1);
            rendered.push(Location::new());
            for entry in self.entries.iter().flatten() {
                let location = match entry.segment {
                    PositionSegment::Rendered(index) => self.prerendered[index as usize].clone(),
                    segment => self.below(&rendered[entry.parent as usize], segment),
                };
                rendered.push(location);
            }
            rendered.into_boxed_slice()
        });
        &rendered[position.0 as usize]
    }

    /// Renders one position without caching it, for reads while positions are still recorded.
    fn render(&self, position: InstancePosition) -> Location {
        if position == InstancePosition::ROOT {
            return Location::new();
        }
        let index = position.0 as usize - 1;
        let entry = self.entries[index / POSITION_CHUNK][index % POSITION_CHUNK];
        match entry.segment {
            PositionSegment::Rendered(index) => self.prerendered[index as usize].clone(),
            segment => self.below(&self.render(InstancePosition(entry.parent)), segment),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct EvaluationNode {
    pub(crate) keyword_location: Location,
    pub(crate) absolute_keyword_location: Option<Arc<Uri<String>>>,
    pub(crate) schema_location: Arc<str>,
    pub(crate) instance_location: InstancePosition,
    pub(crate) valid: bool,
    pub(crate) annotations: Option<Annotations>,
    pub(crate) dropped_annotations: Option<Annotations>,
    pub(crate) errors: Vec<ErrorDescription>,
    pub(crate) first_child: u32,
    pub(crate) next_sibling: u32,
}

// Pinned at 96 bytes; under 128, moves stay inline copies rather than `memcpy` calls.
#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<EvaluationNode>() == 96);

/// Positions per chunk of `InstancePositions::entries`.
const POSITION_CHUNK: usize = 256;

/// Nodes per chunk. A power of two so the index split is a shift and a mask.
const CHUNK: usize = 32;

/// Consecutive items of one array, each valid under the same subschema with no unit reporting an
/// error or an annotation. Their output units are read from `units`, and their positions are not
/// recorded: each item's pointer is the array's followed by the item's index.
#[derive(Debug)]
struct ItemRun {
    /// The array's position.
    parent: InstancePosition,
    /// The first item's index in the array.
    start: usize,
    count: u32,
    /// Where the items' pointers start in `EvaluationArena::run_locations`.
    offset: u32,
    units: RunUnits,
    next_sibling: u32,
}

/// One output unit that every item of a run reports.
#[derive(Debug)]
pub(crate) struct RunUnit {
    pub(crate) evaluation_path: Location,
    pub(crate) schema_location: Arc<str>,
}

/// A range of `EvaluationArena::run_units`: the subschema's unit, then its keywords' in output
/// order.
#[derive(Debug, Clone, Copy)]
pub struct RunUnits {
    start: u32,
    len: u32,
}

/// The units of the latest runs, with what identifies them: the code that recorded them
/// (`owner`) and the subschema's evaluation path.
#[derive(Debug)]
struct LatestRunUnits {
    owner: usize,
    evaluation_path: Location,
    units: RunUnits,
}

/// Consecutive items gathered into one run, until an item that needs its own node ends it.
#[derive(Debug, Default, Clone, Copy)]
pub struct PendingRun {
    start: usize,
    count: u32,
}

impl PendingRun {
    /// Adds the item at `index`, which follows the previous item.
    #[inline]
    pub fn add(&mut self, index: usize) {
        if self.count == 0 {
            self.start = index;
        }
        debug_assert_eq!(index, self.start + self.count as usize);
        self.count += 1;
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Every node of one evaluation, in chunks that are never resized, so growth copies nothing.
///
/// Children are a sibling chain rather than a `Vec` per node: subtrees are finished in the order
/// they are evaluated, so a parent cannot know where its children sit until they all exist.
#[derive(Debug, Default)]
pub(crate) struct EvaluationArena {
    chunks: Vec<Vec<EvaluationNode>>,
    len: usize,
    positions: InstancePositions,
    /// Every run, by the index its child link carries.
    runs: Vec<ItemRun>,
    /// How many items all runs hold: the next run's offset into `run_locations`.
    run_items: u32,
    /// Every run item's pointer, rendered together on the first read.
    run_locations: OnceLock<Box<[Location]>>,
    /// The units each run's items share, one range per subschema and evaluation path.
    run_units: Vec<RunUnit>,
    latest_run_units: Option<LatestRunUnits>,
}

impl EvaluationArena {
    /// Records a position whose pointer is already rendered.
    pub(crate) fn rendered_position(&mut self, location: Location) -> InstancePosition {
        let index = u32::try_from(self.positions.prerendered.len())
            .expect("evaluation exceeded u32 positions");
        self.positions.prerendered.push(location);
        self.positions.push(Position {
            parent: NO_NODE,
            segment: PositionSegment::Rendered(index),
        })
    }

    /// Records the position one `segment` below `parent`.
    pub(crate) fn position_below(
        &mut self,
        parent: InstancePosition,
        segment: &JsonPointerSegment<'_>,
    ) -> InstancePosition {
        let segment = match segment {
            JsonPointerSegment::Key(key) => {
                let start = self.positions.keys.len();
                self.positions.keys.push_str(key);
                PositionSegment::Key {
                    start: u32::try_from(start).expect("evaluation exceeded u32 key bytes"),
                    end: u32::try_from(self.positions.keys.len())
                        .expect("evaluation exceeded u32 key bytes"),
                }
            }
            JsonPointerSegment::Index(index) => PositionSegment::Index(*index),
        };
        self.positions.push(Position {
            parent: parent.0,
            segment,
        })
    }

    /// The pointer of `position`, once every position is recorded.
    pub(crate) fn location(&self, position: InstancePosition) -> &Location {
        self.positions.location(position)
    }

    /// The pointer of `position`, while positions are still being recorded.
    pub(crate) fn render(&self, position: InstancePosition) -> Location {
        self.positions.render(position)
    }

    #[inline]
    pub(crate) fn push(&mut self, node: EvaluationNode) -> u32 {
        if self.len % CHUNK == 0 {
            self.reserve_chunk();
        }
        // Below `RUN_LINK`, which `reserve_chunk` checks at every chunk start.
        #[allow(clippy::cast_possible_truncation)]
        let index = self.len as u32;
        self.chunks
            .last_mut()
            .expect("a chunk was just reserved")
            .push(node);
        self.len += 1;
        index
    }

    /// Adds the chunk the next nodes go to. `RUN_LINK` is a multiple of `CHUNK`, so the node
    /// reaching it starts a chunk.
    #[cold]
    #[inline(never)]
    fn reserve_chunk(&mut self) {
        assert!(
            self.len < RUN_LINK as usize,
            "evaluation exceeded 2^31 nodes"
        );
        self.chunks.push(Vec::with_capacity(CHUNK));
    }

    #[inline]
    pub(crate) fn node(&self, index: u32) -> &EvaluationNode {
        let index = index as usize;
        &self.chunks[index / CHUNK][index % CHUNK]
    }

    #[inline]
    fn node_mut(&mut self, index: u32) -> &mut EvaluationNode {
        let index = index as usize;
        &mut self.chunks[index / CHUNK][index % CHUNK]
    }

    pub(crate) fn child_indices(&self, index: u32) -> ChildIndices<'_> {
        ChildIndices {
            arena: self,
            current: self.node(index).first_child,
        }
    }

    /// The units of the latest runs, if `owner` recorded them for a subschema at `evaluation_path`.
    #[inline]
    pub(crate) fn latest_run_units(
        &self,
        owner: usize,
        evaluation_path: &Location,
    ) -> Option<RunUnits> {
        self.latest_run_units
            .as_ref()
            .filter(|latest| latest.owner == owner && latest.evaluation_path == *evaluation_path)
            .map(|latest| latest.units)
    }

    /// Records `units`, the subschema's unit first, for the runs `owner` makes of that subschema's
    /// items.
    pub(crate) fn add_run_units(
        &mut self,
        owner: usize,
        units: impl IntoIterator<Item = RunUnit>,
    ) -> RunUnits {
        let start = self.run_units.len();
        self.run_units.extend(units);
        let units = RunUnits {
            start: u32::try_from(start).expect("evaluation exceeded u32 run units"),
            len: u32::try_from(self.run_units.len() - start)
                .expect("evaluation exceeded u32 run units"),
        };
        self.latest_run_units = Some(LatestRunUnits {
            owner,
            evaluation_path: self.run_units[start].evaluation_path.clone(),
            units,
        });
        units
    }

    fn shared_units(&self, units: RunUnits) -> &[RunUnit] {
        &self.run_units[units.start as usize..(units.start + units.len) as usize]
    }

    /// The link to a new run of the `pending` items of the array at `parent`.
    #[inline]
    fn push_run(&mut self, parent: InstancePosition, pending: PendingRun, units: RunUnits) -> u32 {
        let index = match u32::try_from(self.runs.len()) {
            Ok(index) if index < RUN_LINK => index,
            _ => panic!("evaluation exceeded 2^31 runs"),
        };
        self.runs.push(ItemRun {
            parent,
            start: pending.start,
            count: pending.count,
            offset: self.run_items,
            units,
            next_sibling: NO_NODE,
        });
        self.run_items = self
            .run_items
            .checked_add(pending.count)
            .expect("evaluation exceeded u32 run items");
        index | RUN_LINK
    }

    /// The pointer of the item at `item` in `run`, once every position is recorded.
    fn run_item_location(&self, run: &ItemRun, item: u32) -> &Location {
        let rendered = self.run_locations.get_or_init(|| self.render_run_items());
        &rendered[(run.offset + item) as usize]
    }

    fn render_run_items(&self) -> Box<[Location]> {
        let mut rendered = Vec::with_capacity(self.run_items as usize);
        for run in &self.runs {
            let parent = self.location(run.parent);
            rendered.extend(
                (run.start..run.start + run.count as usize)
                    .map(|index| parent.join_pointer_segment(&JsonPointerSegment::Index(index))),
            );
        }
        rendered.into_boxed_slice()
    }

    #[inline]
    fn run(&self, index: u32) -> &ItemRun {
        &self.runs[index as usize]
    }

    #[inline]
    fn next_sibling(&self, link: u32) -> u32 {
        if link & RUN_LINK == 0 {
            self.node(link).next_sibling
        } else {
            self.run(link & !RUN_LINK).next_sibling
        }
    }

    #[inline]
    fn set_next_sibling(&mut self, link: u32, next: u32) {
        if link & RUN_LINK == 0 {
            self.node_mut(link).next_sibling = next;
        } else {
            self.set_run_next_sibling(link, next);
        }
    }

    #[cold]
    #[inline(never)]
    fn set_run_next_sibling(&mut self, link: u32, next: u32) {
        self.runs[(link & !RUN_LINK) as usize].next_sibling = next;
    }
}

#[derive(Debug)]
pub(crate) struct ChildIndices<'a> {
    arena: &'a EvaluationArena,
    current: u32,
}

impl Iterator for ChildIndices<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.current == NO_NODE {
            return None;
        }
        let index = self.current;
        self.current = self.arena.next_sibling(index);
        Some(index)
    }
}

/// The children collected for one node, as a chain into the evaluation arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildList {
    first: u32,
    last: u32,
    len: u32,
    /// Whether every child pushed so far is valid.
    valid: bool,
}

impl Default for ChildList {
    fn default() -> Self {
        ChildList {
            first: NO_NODE,
            last: NO_NODE,
            len: 0,
            valid: true,
        }
    }
}

impl ChildList {
    #[inline]
    pub(crate) fn push(&mut self, arena: &mut EvaluationArena, node: EvaluationNode) {
        self.valid &= node.valid;
        let index = arena.push(node);
        self.link(arena, index);
        self.len += 1;
    }

    /// Moves the items `pending` holds, of the array at `parent`, onto the list as one run whose
    /// items share `units`.
    #[inline]
    pub(crate) fn push_run(
        &mut self,
        arena: &mut EvaluationArena,
        parent: InstancePosition,
        pending: &mut PendingRun,
        units: RunUnits,
    ) {
        let pending = std::mem::take(pending);
        let count = pending.count;
        let link = arena.push_run(parent, pending, units);
        self.link(arena, link);
        self.len += count;
    }

    #[inline]
    fn link(&mut self, arena: &mut EvaluationArena, link: u32) {
        if self.last == NO_NODE {
            self.first = link;
        } else {
            arena.set_next_sibling(self.last, link);
        }
        self.last = link;
    }

    /// Moves `other`'s nodes after this list's.
    pub(crate) fn append(&mut self, arena: &mut EvaluationArena, other: ChildList) {
        if other.first == NO_NODE {
            return;
        }
        if self.last == NO_NODE {
            *self = other;
            return;
        }
        arena.set_next_sibling(self.last, other.first);
        self.last = other.last;
        self.len += other.len;
        self.valid &= other.valid;
    }

    pub(crate) fn len(self) -> usize {
        self.len as usize
    }

    pub(crate) fn all_valid(self) -> bool {
        self.valid
    }

    /// A list of `nodes`, for tests that build the nodes first.
    #[cfg(test)]
    pub(crate) fn from_nodes(
        arena: &mut EvaluationArena,
        nodes: impl IntoIterator<Item = EvaluationNode>,
    ) -> Self {
        let mut list = ChildList::default();
        for node in nodes {
            list.push(arena, node);
        }
        list
    }

    /// A list holding `node` alone.
    #[inline]
    pub(crate) fn of(arena: &mut EvaluationArena, node: EvaluationNode) -> Self {
        let mut list = ChildList::default();
        list.push(arena, node);
        list
    }
}

impl EvaluationNode {
    pub(crate) fn valid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: impl Into<Arc<str>>,
        instance_location: InstancePosition,
        annotations: Option<Annotations>,
        children: ChildList,
    ) -> Self {
        let schema_location = schema_location.into();
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: true,
            annotations,
            dropped_annotations: None,
            errors: Vec::new(),
            first_child: children.first,
            next_sibling: NO_NODE,
        }
    }

    pub(crate) fn invalid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: impl Into<Arc<str>>,
        instance_location: InstancePosition,
        annotations: Option<Annotations>,
        errors: Vec<ErrorDescription>,
        children: ChildList,
    ) -> Self {
        let schema_location = schema_location.into();
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: false,
            annotations: None,
            dropped_annotations: annotations,
            errors,
            first_child: children.first,
            next_sibling: NO_NODE,
        }
    }
}

/// Result of evaluating a JSON instance against a schema.
///
/// Structured output formats of the
/// [JSON Schema specification](https://json-schema.org/draft/2020-12/json-schema-core#name-output-structure).
///
/// # Output Formats
///
/// The evaluation result can be accessed in three standard formats:
///
/// - **Flag**: Simple boolean validity indicator via [`flag()`](Self::flag)
/// - **List**: Flat list of all evaluation units via [`list()`](Self::list)
/// - **Hierarchical**: Nested tree structure via [`hierarchical()`](Self::hierarchical)
///
/// All formats are serializable to JSON using `serde_json`.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "minLength": 3});
/// let validator = jsonschema::validator_for(&schema)?;
///
/// let instance = json!("ab");
/// let evaluation = validator.evaluate(&instance);
///
/// let flag = evaluation.flag();
/// assert!(!flag.valid);
///
/// let list_output = serde_json::to_value(evaluation.list())?;
/// println!("{}", serde_json::to_string_pretty(&list_output)?);
///
/// for error in evaluation.iter_errors() {
///     println!("Error at {}: {}", error.instance_location, error.error);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Evaluation {
    arena: EvaluationArena,
    root: u32,
}

impl Evaluation {
    pub(crate) fn new(arena: EvaluationArena, root: u32) -> Self {
        Evaluation { arena, root }
    }

    pub(crate) fn with_root(mut arena: EvaluationArena, root: EvaluationNode) -> Self {
        let root = arena.push(root);
        Evaluation::new(arena, root)
    }

    fn root_node(&self) -> &EvaluationNode {
        self.arena.node(self.root)
    }
    /// Returns the root output unit, from which every other unit is reachable through
    /// [`OutputUnit::children`].
    ///
    /// [`Evaluation::hierarchical`] serializes this tree as is, and [`Evaluation::list`] serializes
    /// its units in pre-order.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let validator = jsonschema::validator_for(&json!({"items": {"type": "integer"}}))?;
    /// let evaluation = validator.evaluate(&json!([1, "oops"]));
    ///
    /// let root = evaluation.root();
    /// assert!(!root.valid());
    /// let items = root.children().next().expect("`items` unit");
    /// assert_eq!(items.evaluation_path().as_str(), "/items");
    /// let failed = items.children().nth(1).expect("unit for the second item");
    /// assert_eq!(failed.instance_location().as_str(), "/1");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn root(&self) -> OutputUnit<'_> {
        OutputUnit::node(&self.arena, self.root)
    }
    /// Returns the flag output format.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "number"});
    /// let validator = jsonschema::validator_for(&schema)?;
    ///
    /// let evaluation = validator.evaluate(&json!(42));
    /// let flag = evaluation.flag();
    /// assert!(flag.valid);
    ///
    /// let evaluation = validator.evaluate(&json!("not a number"));
    /// let flag = evaluation.flag();
    /// assert!(!flag.valid);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn flag(&self) -> FlagOutput {
        FlagOutput {
            valid: self.root_node().valid,
        }
    }
    /// Whether the instance is valid against the schema.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let validator = jsonschema::validator_for(&json!({"type": "number"}))?;
    ///
    /// assert!(validator.evaluate(&json!(42)).is_valid());
    /// assert!(!validator.evaluate(&json!("oops")).is_valid());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.root_node().valid
    }
    /// Returns the list output format.
    ///
    /// Every output unit, flattened in pre-order.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "array",
    ///     "prefixItems": [{"type": "string"}],
    ///     "items": {"type": "integer"}
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!(["hello", "oops"]));
    ///
    /// assert_eq!(
    ///     serde_json::to_value(evaluation.list())?,
    ///     json!({
    ///         "valid": false,
    ///         "details": [
    ///             {"evaluationPath": "", "instanceLocation": "", "schemaLocation": "", "valid": false},
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/type",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/type"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/items",
    ///                 "droppedAnnotations": true
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "/1",
    ///                 "schemaLocation": "/items"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items/type",
    ///                 "instanceLocation": "/1",
    ///                 "schemaLocation": "/items/type",
    ///                 "errors": {"type": "\"oops\" is not of type \"integer\""}
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/prefixItems",
    ///                 "annotations": 0
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems/0",
    ///                 "instanceLocation": "/0",
    ///                 "schemaLocation": "/prefixItems/0"
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems/0/type",
    ///                 "instanceLocation": "/0",
    ///                 "schemaLocation": "/prefixItems/0/type"
    ///             }
    ///         ]
    ///     })
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn list(&self) -> ListOutput<'_> {
        ListOutput {
            arena: &self.arena,
            root: self.root,
        }
    }
    /// Returns the hierarchical output format.
    ///
    /// Output units nested by evaluation: each unit holds those of the subschemas evaluated
    /// below it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "array",
    ///     "prefixItems": [{"type": "string"}],
    ///     "items": {"type": "integer"}
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!(["hello", "oops"]));
    ///
    /// assert_eq!(
    ///     serde_json::to_value(evaluation.hierarchical())?,
    ///     json!({
    ///         "valid": false,
    ///         "evaluationPath": "",
    ///         "schemaLocation": "",
    ///         "instanceLocation": "",
    ///         "details": [
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/type",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/type"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/items",
    ///                 "droppedAnnotations": true,
    ///                 "details": [
    ///                     {
    ///                         "valid": false,
    ///                         "evaluationPath": "/items",
    ///                         "instanceLocation": "/1",
    ///                         "schemaLocation": "/items",
    ///                         "details": [
    ///                             {
    ///                                 "valid": false,
    ///                                 "evaluationPath": "/items/type",
    ///                                 "instanceLocation": "/1",
    ///                                 "schemaLocation": "/items/type",
    ///                                 "errors": {"type": "\"oops\" is not of type \"integer\""}
    ///                             }
    ///                         ]
    ///                     }
    ///                 ]
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/prefixItems",
    ///                 "annotations": 0,
    ///                 "details": [
    ///                     {
    ///                         "valid": true,
    ///                         "evaluationPath": "/prefixItems/0",
    ///                         "instanceLocation": "/0",
    ///                         "schemaLocation": "/prefixItems/0",
    ///                         "details": [
    ///                             {
    ///                                 "valid": true,
    ///                                 "evaluationPath": "/prefixItems/0/type",
    ///                                 "instanceLocation": "/0",
    ///                                 "schemaLocation": "/prefixItems/0/type"
    ///                             }
    ///                         ]
    ///                     }
    ///                 ]
    ///             }
    ///         ]
    ///     })
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn hierarchical(&self) -> HierarchicalOutput<'_> {
        HierarchicalOutput {
            arena: &self.arena,
            root: self.root,
        }
    }
    /// Returns an iterator over all annotations produced during evaluation.
    ///
    /// Annotations are metadata emitted by keywords during successful validation. A failing
    /// schema produces no annotations, including those of its subschemas, so an invalid instance
    /// yields none.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "object",
    ///     "properties": {"name": {"type": "string"}, "age": {"type": "number", "minimum": 0}},
    ///     "required": ["name"]
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!({"name": "Alice", "age": 30}));
    ///
    /// let entries: Vec<_> = evaluation.iter_annotations().collect();
    /// assert_eq!(entries.len(), 1);
    /// assert_eq!(entries[0].schema_location, "/properties");
    /// assert_eq!(entries[0].instance_location.as_str(), "");
    ///
    /// let mut annotation_names: Vec<_> = entries[0]
    ///     .annotations
    ///     .value()
    ///     .as_array()
    ///     .expect("annotation should be an array")
    ///     .iter()
    ///     .map(|value| value.as_str().expect("annotation items should be strings"))
    ///     .collect();
    /// annotation_names.sort_unstable();
    /// assert_eq!(annotation_names, vec!["age", "name"]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn iter_annotations(&self) -> AnnotationIter<'_> {
        AnnotationIter::new(&self.arena, self.root)
    }
    /// Returns an iterator over all errors produced during evaluation.
    ///
    /// Each error entry contains information about a validation failure,
    /// including its location in both the schema and instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "object",
    ///     "required": ["name"],
    ///     "properties": {
    ///         "age": {"type": "number"}
    ///     }
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!({"name": "Bob", "age": "oops"}));
    ///
    /// let errors: Vec<_> = evaluation.iter_errors().collect();
    /// assert_eq!(errors.len(), 1);
    /// assert_eq!(errors[0].schema_location, "/properties/age/type");
    /// assert_eq!(errors[0].instance_location.as_str(), "/age");
    /// assert_eq!(errors[0].error.to_string(), "\"oops\" is not of type \"number\"");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn iter_errors(&self) -> ErrorIter<'_> {
        ErrorIter::new(&self.arena, self.root)
    }
}

/// Flag output format containing only a validity indicator.
///
/// # JSON Structure
///
/// ```json
/// {
///   "valid": true
/// }
/// ```
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "string"});
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!("hello"));
///
/// let flag = evaluation.flag();
/// assert_eq!(flag.valid, true);
///
/// let output = serde_json::to_value(flag)?;
/// assert_eq!(output, json!({"valid": true}));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Serialize)]
pub struct FlagOutput {
    /// Whether the instance is valid according to the schema.
    pub valid: bool,
}

/// List output format providing a flat list of evaluation units.
///
/// See [`Evaluation::list`] for an example JSON payload produced by this type.
#[derive(Debug)]
pub struct ListOutput<'a> {
    arena: &'a EvaluationArena,
    root: u32,
}

/// Hierarchical output format providing a tree structure of evaluation results.
///
/// See [`Evaluation::hierarchical`] for an example JSON payload produced by this type.
#[derive(Debug)]
pub struct HierarchicalOutput<'a> {
    arena: &'a EvaluationArena,
    root: u32,
}

impl Serialize for ListOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_list(self.arena, self.root, serializer)
    }
}

impl Serialize for HierarchicalOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_hierarchical(self.arena, self.root, serializer)
    }
}

fn serialize_list<S>(arena: &EvaluationArena, root: u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut state = serializer.serialize_struct("ListOutput", 2)?;
    state.serialize_field("valid", &arena.node(root).valid)?;
    let mut units = Vec::new();
    collect_list_entries(arena, root, &mut units);
    state.serialize_field("details", &ListEntries { arena, units })?;
    state.end()
}

fn serialize_hierarchical<S>(
    arena: &EvaluationArena,
    root: u32,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_unit(arena, root, serializer, true)
}

fn collect_list_entries(arena: &EvaluationArena, index: u32, out: &mut Vec<Unit>) {
    // Keeps units without errors or annotations, which the spec says SHOULD be excluded: the
    // official test suite expects them.
    out.push(Unit::Node(index));
    let mut link = arena.node(index).first_child;
    while link != NO_NODE {
        if link & RUN_LINK == 0 {
            collect_list_entries(arena, link, out);
            link = arena.node(link).next_sibling;
        } else {
            let run = link & !RUN_LINK;
            let entry = arena.run(run);
            for item in 0..entry.count {
                out.extend((0..entry.units.len).map(|unit| Unit::RunItem { run, item, unit }));
            }
            link = entry.next_sibling;
        }
    }
}

fn serialize_unit<S>(
    arena: &EvaluationArena,
    index: u32,
    serializer: S,
    include_children: bool,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let node = arena.node(index);
    let mut state = serializer.serialize_struct("OutputUnit", 7)?;
    state.serialize_field("valid", &node.valid)?;
    state.serialize_field("evaluationPath", node.keyword_location.as_str())?;
    state.serialize_field("schemaLocation", node.schema_location.as_ref())?;
    state.serialize_field(
        "instanceLocation",
        arena.location(node.instance_location).as_str(),
    )?;
    if let Some(annotations) = &node.annotations {
        state.serialize_field("annotations", annotations)?;
    }
    if let Some(annotations) = &node.dropped_annotations {
        state.serialize_field("droppedAnnotations", annotations)?;
    }
    if !node.errors.is_empty() {
        state.serialize_field("errors", &ErrorEntriesSerializer(&node.errors))?;
    }
    if include_children && node.first_child != NO_NODE {
        state.serialize_field("details", &DetailsSerializer { arena, index })?;
    }
    state.end()
}

/// The unit at `unit` among the shared units of `run`, for its item at `item`: valid, with
/// neither annotations nor errors.
fn serialize_run_unit<S>(
    arena: &EvaluationArena,
    run: u32,
    item: u32,
    unit: u32,
    serializer: S,
    include_children: bool,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let entry = arena.run(run);
    let units = arena.shared_units(entry.units);
    let shared = &units[unit as usize];
    let mut state = serializer.serialize_struct("OutputUnit", 7)?;
    state.serialize_field("valid", &true)?;
    state.serialize_field("evaluationPath", shared.evaluation_path.as_str())?;
    state.serialize_field("schemaLocation", shared.schema_location.as_ref())?;
    state.serialize_field(
        "instanceLocation",
        arena.run_item_location(entry, item).as_str(),
    )?;
    // A run item's subschema unit holds its keyword units.
    if include_children && unit == 0 && units.len() > 1 {
        state.serialize_field("details", &RunItemDetails { arena, run, item })?;
    }
    state.end()
}

/// `schemaLocation` of a keyword: its absolute URI when it has one, otherwise its JSON Pointer.
///
/// The URI's fragment counts from the root of the keyword's own resource, which a subschema
/// declaring its own `$id` places below the document root.
pub(crate) fn format_keyword_location(
    location: &Location,
    absolute: Option<&Arc<Uri<String>>>,
) -> Arc<str> {
    absolute.map_or_else(|| location.as_arc(), |uri| Arc::from(uri.as_str()))
}

/// One output unit of an [`Evaluation`]: a node of the hierarchical output, or one entry of the
/// list output.
#[derive(Clone, Copy, Debug)]
pub struct OutputUnit<'a> {
    arena: &'a EvaluationArena,
    unit: Unit,
}

/// Where an output unit's members are read from.
#[derive(Clone, Copy, Debug)]
enum Unit {
    Node(u32),
    /// The unit at `unit` among the shared units of run `run`, for its item at `item`.
    RunItem {
        run: u32,
        item: u32,
        unit: u32,
    },
}

impl<'a> OutputUnit<'a> {
    #[inline]
    fn node(arena: &'a EvaluationArena, index: u32) -> Self {
        OutputUnit {
            arena,
            unit: Unit::Node(index),
        }
    }

    /// The shared unit at `unit` of run `run`.
    #[inline]
    fn shared(&self, run: u32, unit: u32) -> &'a RunUnit {
        let arena = self.arena;
        &arena.shared_units(arena.run(run).units)[unit as usize]
    }
    /// Whether the instance is valid at this unit.
    #[inline]
    #[must_use]
    pub fn valid(&self) -> bool {
        match self.unit {
            Unit::Node(index) => self.arena.node(index).valid,
            Unit::RunItem { .. } => true,
        }
    }
    /// The path of keywords followed to reach this unit (`evaluationPath`).
    #[inline]
    #[must_use]
    pub fn evaluation_path(&self) -> &'a Location {
        match self.unit {
            Unit::Node(index) => &self.arena.node(index).keyword_location,
            Unit::RunItem { run, unit, .. } => &self.shared(run, unit).evaluation_path,
        }
    }
    /// The keyword's absolute URI when it has one, otherwise its JSON Pointer (`schemaLocation`).
    #[inline]
    #[must_use]
    pub fn schema_location(&self) -> &'a str {
        match self.unit {
            Unit::Node(index) => &self.arena.node(index).schema_location,
            Unit::RunItem { run, unit, .. } => &self.shared(run, unit).schema_location,
        }
    }
    /// The location in the instance this unit evaluated (`instanceLocation`).
    #[inline]
    #[must_use]
    pub fn instance_location(&self) -> &'a Location {
        let arena = self.arena;
        match self.unit {
            Unit::Node(index) => arena.location(arena.node(index).instance_location),
            Unit::RunItem { run, item, .. } => arena.run_item_location(arena.run(run), item),
        }
    }
    /// Annotations this unit produced (`annotations`).
    #[inline]
    #[must_use]
    pub fn annotations(&self) -> Option<&'a Annotations> {
        match self.unit {
            Unit::Node(index) => self.arena.node(index).annotations.as_ref(),
            Unit::RunItem { .. } => None,
        }
    }
    /// Annotations this unit produced and then dropped because it failed (`droppedAnnotations`).
    #[inline]
    #[must_use]
    pub fn dropped_annotations(&self) -> Option<&'a Annotations> {
        match self.unit {
            Unit::Node(index) => self.arena.node(index).dropped_annotations.as_ref(),
            Unit::RunItem { .. } => None,
        }
    }
    /// Errors this unit reported, in the order they were produced (`errors`).
    #[inline]
    #[must_use]
    pub fn errors(&self) -> &'a [ErrorDescription] {
        match self.unit {
            Unit::Node(index) => &self.arena.node(index).errors,
            Unit::RunItem { .. } => &[],
        }
    }
    /// Output units of the subschemas evaluated below this one (`details`).
    #[inline]
    #[must_use]
    pub fn children(&self) -> OutputUnitChildren<'a> {
        let next = match self.unit {
            Unit::Node(index) => NextChild::Chain {
                link: self.arena.node(index).first_child,
                item: 0,
            },
            // A run item's subschema unit holds its keyword units.
            Unit::RunItem { run, item, unit: 0 } => NextChild::Shared { run, item, unit: 1 },
            Unit::RunItem { .. } => NextChild::Chain {
                link: NO_NODE,
                item: 0,
            },
        };
        OutputUnitChildren {
            arena: self.arena,
            next,
        }
    }
}

/// Iterator over the children of an [`OutputUnit`].
#[derive(Debug)]
pub struct OutputUnitChildren<'a> {
    arena: &'a EvaluationArena,
    next: NextChild,
}

#[derive(Debug, Clone, Copy)]
enum NextChild {
    /// The sibling chain from `link`; at a run, the subschema unit of its item at `item`.
    Chain { link: u32, item: u32 },
    /// The shared units of run `run` from `unit` on, for its item at `item`.
    Shared { run: u32, item: u32, unit: u32 },
}

impl<'a> Iterator for OutputUnitChildren<'a> {
    type Item = OutputUnit<'a>;

    #[inline]
    fn next(&mut self) -> Option<OutputUnit<'a>> {
        let arena = self.arena;
        let unit = match self.next {
            NextChild::Chain { link, item } => {
                if link == NO_NODE {
                    return None;
                }
                if link & RUN_LINK == 0 {
                    self.next = NextChild::Chain {
                        link: arena.node(link).next_sibling,
                        item: 0,
                    };
                    Unit::Node(link)
                } else {
                    let index = link & !RUN_LINK;
                    let run = arena.run(index);
                    self.next = if item + 1 < run.count {
                        NextChild::Chain {
                            link,
                            item: item + 1,
                        }
                    } else {
                        NextChild::Chain {
                            link: run.next_sibling,
                            item: 0,
                        }
                    };
                    Unit::RunItem {
                        run: index,
                        item,
                        unit: 0,
                    }
                }
            }
            NextChild::Shared { run, item, unit } => {
                if unit == arena.run(run).units.len {
                    return None;
                }
                self.next = NextChild::Shared {
                    run,
                    item,
                    unit: unit + 1,
                };
                Unit::RunItem { run, item, unit }
            }
        };
        Some(OutputUnit { arena, unit })
    }
}

/// The list output's units, in pre-order.
struct ListEntries<'a> {
    arena: &'a EvaluationArena,
    units: Vec<Unit>,
}

impl Serialize for ListEntries<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.units.len()))?;
        for &unit in &self.units {
            seq.serialize_element(&ListEntry {
                arena: self.arena,
                unit,
            })?;
        }
        seq.end()
    }
}

struct ListEntry<'a> {
    arena: &'a EvaluationArena,
    unit: Unit,
}

impl Serialize for ListEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.unit {
            Unit::Node(index) => serialize_unit(self.arena, index, serializer, false),
            Unit::RunItem { run, item, unit } => {
                serialize_run_unit(self.arena, run, item, unit, serializer, false)
            }
        }
    }
}

struct DetailsSerializer<'a> {
    arena: &'a EvaluationArena,
    index: u32,
}

impl Serialize for DetailsSerializer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let arena = self.arena;
        let mut seq = serializer.serialize_seq(None)?;
        let mut link = arena.node(self.index).first_child;
        while link != NO_NODE {
            if link & RUN_LINK == 0 {
                seq.serialize_element(&SeqEntry { arena, index: link })?;
                link = arena.node(link).next_sibling;
            } else {
                let run = link & !RUN_LINK;
                let entry = arena.run(run);
                for item in 0..entry.count {
                    seq.serialize_element(&RunUnitEntry {
                        arena,
                        run,
                        item,
                        unit: 0,
                    })?;
                }
                link = entry.next_sibling;
            }
        }
        seq.end()
    }
}

struct SeqEntry<'a> {
    arena: &'a EvaluationArena,
    index: u32,
}

impl Serialize for SeqEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_unit(self.arena, self.index, serializer, true)
    }
}

/// The keyword units of a run item.
struct RunItemDetails<'a> {
    arena: &'a EvaluationArena,
    run: u32,
    item: u32,
}

impl Serialize for RunItemDetails<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(None)?;
        for unit in 1..self.arena.run(self.run).units.len {
            seq.serialize_element(&RunUnitEntry {
                arena: self.arena,
                run: self.run,
                item: self.item,
                unit,
            })?;
        }
        seq.end()
    }
}

struct RunUnitEntry<'a> {
    arena: &'a EvaluationArena,
    run: u32,
    item: u32,
    unit: u32,
}

impl Serialize for RunUnitEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_run_unit(self.arena, self.run, self.item, self.unit, serializer, true)
    }
}

/// Entry describing annotations emitted by a keyword during evaluation.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let instance = json!({"name": "Alice", "age": 30});
/// let evaluation = validator.evaluate(&instance);
/// let entry = evaluation.iter_annotations().next().unwrap();
/// assert_eq!(entry.schema_location, "/properties");
/// assert_eq!(entry.instance_location.as_str(), "");
///
/// let mut annotation_names: Vec<_> = entry
///     .annotations
///     .value()
///     .as_array()
///     .expect("annotation should be an array")
///     .iter()
///     .map(|value| value.as_str().expect("annotation items should be strings"))
///     .collect();
/// annotation_names.sort_unstable();
/// assert_eq!(annotation_names, vec!["age", "name"]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct AnnotationEntry<'a> {
    /// The keyword's absolute URI when it has one, otherwise its JSON Pointer.
    pub schema_location: &'a str,
    /// The absolute URI of the keyword location, if available.
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    /// The JSON Pointer to the instance location being validated.
    pub instance_location: &'a Location,
    /// The annotations produced by the keyword.
    pub annotations: &'a Annotations,
}

/// Entry describing errors emitted by a keyword during evaluation.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "required": ["name"],
///     "properties": {
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let instance = json!({"name": "test", "age": "oops"});
/// let evaluation = validator.evaluate(&instance);
/// let entry = evaluation.iter_errors().next().unwrap();
/// assert_eq!(entry.schema_location, "/properties/age/type");
/// assert_eq!(entry.instance_location.as_str(), "/age");
/// assert_eq!(entry.error.to_string(), "\"oops\" is not of type \"number\"");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ErrorEntry<'a> {
    /// The keyword's absolute URI when it has one, otherwise its JSON Pointer.
    pub schema_location: &'a str,
    /// The absolute URI of the keyword location, if available.
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    /// The JSON Pointer to the instance location that failed validation.
    pub instance_location: &'a Location,
    /// The error description.
    pub error: &'a ErrorDescription,
}

impl fmt::Display for ErrorEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.instance_location.is_empty() {
            self.error.fmt(f)
        } else {
            write!(f, "{}: {}", self.instance_location, self.error)
        }
    }
}

struct NodeIter<'a> {
    arena: &'a EvaluationArena,
    stack: Vec<u32>,
    /// Whether to walk into the subtrees of failing nodes.
    enter_invalid: bool,
}

impl<'a> NodeIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        NodeIter {
            arena,
            stack: vec![root],
            enter_invalid: true,
        }
    }

    /// Only nodes whose every ancestor, and the node itself, is valid.
    fn valid(arena: &'a EvaluationArena, root: u32) -> Self {
        NodeIter {
            arena,
            stack: vec![root],
            enter_invalid: false,
        }
    }
}

impl<'a> Iterator for NodeIter<'a> {
    type Item = &'a EvaluationNode;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let index = self.stack.pop()?;
            let node = self.arena.node(index);
            if !node.valid && !self.enter_invalid {
                continue;
            }
            let start = self.stack.len();
            // Runs hold neither errors nor annotations.
            self.stack.extend(
                self.arena
                    .child_indices(index)
                    .filter(|&link| link & RUN_LINK == 0),
            );
            self.stack[start..].reverse();
            return Some(node);
        }
    }
}

/// Iterator over annotations produced during evaluation.
///
/// Yields an [`AnnotationEntry`] for each unit that produced annotations.
///
/// A failing schema produces no annotations, including those of its subschemas.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!({"name": "Alice", "age": 30}));
///
/// let annotations: Vec<_> = evaluation.iter_annotations().collect();
/// assert_eq!(annotations.len(), 1);
/// assert_eq!(annotations[0].instance_location.as_str(), "");
///
/// let mut annotation_names: Vec<_> = annotations[0]
///     .annotations
///     .value()
///     .as_array()
///     .expect("annotation should be an array")
///     .iter()
///     .map(|value| value.as_str().expect("annotation items should be strings"))
///     .collect();
/// annotation_names.sort_unstable();
/// assert_eq!(annotation_names, vec!["age", "name"]);
/// # Ok(())
/// # }
/// ```
pub struct AnnotationIter<'a> {
    nodes: NodeIter<'a>,
}

impl<'a> AnnotationIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        AnnotationIter {
            nodes: NodeIter::valid(arena, root),
        }
    }
}

impl<'a> Iterator for AnnotationIter<'a> {
    type Item = AnnotationEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        for node in self.nodes.by_ref() {
            if let Some(annotations) = node.annotations.as_ref() {
                return Some(AnnotationEntry {
                    schema_location: &node.schema_location,
                    absolute_keyword_location: node.absolute_keyword_location.as_deref(),
                    instance_location: self.nodes.arena.location(node.instance_location),
                    annotations,
                });
            }
        }
        None
    }
}

/// Iterator over errors produced during evaluation.
///
/// Yields an [`ErrorEntry`] per error, depth-first.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "required": ["name"],
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number", "minimum": 0}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!({"age": -5}));
///
/// let errors: Vec<_> = evaluation.iter_errors().collect();
/// assert_eq!(errors.len(), 2);
/// assert_eq!(errors[0].schema_location, "/required");
/// assert_eq!(errors[0].instance_location.as_str(), "");
/// assert_eq!(errors[1].schema_location, "/properties/age/minimum");
/// assert_eq!(errors[1].instance_location.as_str(), "/age");
/// # Ok(())
/// # }
/// ```
pub struct ErrorIter<'a> {
    nodes: NodeIter<'a>,
    current: Option<(&'a EvaluationNode, usize)>,
}

impl<'a> ErrorIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        ErrorIter {
            nodes: NodeIter::new(arena, root),
            current: None,
        }
    }
}

impl<'a> Iterator for ErrorIter<'a> {
    type Item = ErrorEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((node, idx)) = self.current {
                if idx < node.errors.len() {
                    let entry = ErrorEntry {
                        schema_location: &node.schema_location,
                        absolute_keyword_location: node.absolute_keyword_location.as_deref(),
                        instance_location: self.nodes.arena.location(node.instance_location),
                        error: &node.errors[idx],
                    };
                    self.current = Some((node, idx + 1));
                    return Some(entry);
                }
                self.current = None;
            }

            {
                let node = self.nodes.next()?;
                if node.errors.is_empty() {
                    continue;
                }
                self.current = Some((node, 0));
            }
        }
    }
}

struct ErrorEntriesSerializer<'a>(&'a [ErrorDescription]);

impl<'a> Serialize for ErrorEntriesSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut grouped: Vec<(&'a str, Vec<&'a str>)> = Vec::new();
        let mut indexes: AHashMap<&'a str, usize> = AHashMap::new();

        for error in self.0 {
            let keyword = error.keyword();
            let msg = error.message();
            if let Some(&idx) = indexes.get(keyword) {
                grouped[idx].1.push(msg);
            } else {
                indexes.insert(keyword, grouped.len());
                grouped.push((keyword, vec![msg]));
            }
        }

        let mut map = serializer.serialize_map(Some(grouped.len()))?;
        for (keyword, messages) in grouped {
            if messages.len() == 1 {
                map.serialize_entry(keyword, messages[0])?;
            } else {
                map.serialize_entry(keyword, &messages)?;
            }
        }
        map.end()
    }
}

/// What generated validators build an [`Evaluation`] from.
#[cfg(feature = "macros")]
#[doc(hidden)]
pub mod generated {
    use super::{format_keyword_location, Evaluation, EvaluationArena, RunUnit, NO_NODE};
    pub use super::{
        Annotations, ChildList, ErrorDescription, EvaluationNode, InstancePosition, PendingRun,
        RunUnits,
    };
    use crate::{
        compiler::resolve_absolute_location,
        error::ValidationErrorKind,
        json::Array,
        paths::{capture_evaluation_path, write_location, LazyLocation, Location},
        properties::{scan_members, HASHMAP_THRESHOLD},
        validator::EVALUATION_PATH_CACHE_THRESHOLD,
        Json, Node, ValidationError,
    };
    use ahash::AHashMap;
    use referencing::{uri, Uri};
    use serde::Deserialize;
    use serde_json::Value;
    use std::{
        borrow::Cow,
        cell::{Cell, RefCell},
        collections::hash_map::Entry,
        sync::{Arc, OnceLock},
        thread::LocalKey,
    };

    /// The locations a generated `evaluate` reports at, as the generator writes them in JSON: one
    /// string literal costs the compiler far less than the same tables as Rust values.
    #[derive(Deserialize)]
    struct Tables {
        /// The root first.
        schemas: Vec<SchemaTable>,
        units: Vec<UnitTable>,
        /// The absolute URI the pointers resolve against, if the schema has one.
        #[serde(default)]
        base: Option<String>,
        /// Each resource a schema sits in, for references that resolve through the dynamic scope.
        #[serde(default)]
        resources: Vec<ResourceTable>,
        /// The code of each reference target a `$ref` enters.
        #[serde(default)]
        functions: Vec<Function>,
        /// The root's entry in `functions` when a `$ref` targets it; references back to the root
        /// enter its cycle guard.
        #[serde(default)]
        root_function: Option<usize>,
        /// References track the dynamic scope, since a `$dynamicRef`, a `$recursiveRef` or
        /// generated code reads it.
        #[serde(default)]
        keeps_scope: bool,
    }

    /// A reference target's generated code.
    #[derive(Deserialize)]
    struct Function {
        /// The function whose code the generator was emitting when it first emitted this one;
        /// `None` when that is the root's code.
        #[serde(default)]
        parent: Option<usize>,
    }

    /// A resource the dynamic scope may hold.
    #[derive(Deserialize)]
    struct ResourceTable {
        /// Its root, where a `$recursiveRef` may resolve to.
        root: Option<usize>,
        /// Its root has `$recursiveAnchor: true`.
        recursive: bool,
        /// Its `$dynamicAnchor` schemas, where a `$dynamicRef` may resolve to.
        anchors: Vec<(String, usize)>,
    }

    #[derive(Deserialize)]
    struct SchemaTable {
        pointer: String,
        /// Its index in `Tables::resources`.
        #[serde(default)]
        resource: usize,
        /// The resource it sits in, where that is not the root's.
        #[serde(default)]
        base: Option<String>,
        /// It sits in another resource than the root's, under the default base: its keywords
        /// have no absolute location.
        #[serde(default)]
        default_base: bool,
        /// Its pointer within that resource, where that is not `pointer`.
        #[serde(default)]
        at: Option<String>,
        /// Its units, a range of `Tables::units` in the runtime's priority order.
        first: usize,
        count: usize,
        /// The schema's annotation: its keywords the draft does not know.
        #[serde(default)]
        annotations: Option<Value>,
    }

    /// One output unit: a keyword, or a keyword with what `evaluate` needs to report it.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum UnitTable {
        Keyword(String),
        Detailed {
            keyword: String,
            #[serde(default)]
            aliases: Vec<String>,
            #[serde(default)]
            strings: Option<Value>,
            #[serde(default)]
            target: usize,
            /// That target's code, an index of `Tables::functions`.
            #[serde(default)]
            function: usize,
            /// That target is in another resource than the schema holding the reference: in another
            /// document, or embedded in the same one.
            #[serde(default)]
            leaves: bool,
        },
    }

    /// The static target of a reference unit.
    #[derive(Clone, Copy, Default)]
    struct Reference {
        target: usize,
        /// The target's code, an index of `Built::functions`.
        function: usize,
        /// The target is in another resource than the schema holding the reference: in another
        /// document, or embedded in the same one.
        leaves: bool,
        /// The resource holding the reference.
        caller: usize,
    }

    /// Where a unit or a schema sits, fixed when the validator is generated.
    struct UnitLocation {
        keyword: Location,
        schema: Arc<str>,
        absolute: Option<Arc<Uri<String>>>,
        /// Where else its errors are reported.
        aliases: Vec<Location>,
        /// Its index in `Built::units` followed by `Built::schemas`.
        site: usize,
        annotations: Option<Annotations>,
        reference: Reference,
    }

    /// With `enter`, pushes `mark` onto `marks`, or returns `true` without pushing when it is there
    /// already: the instance is being validated against the target. Without, pops the last mark.
    /// The generated code's own references to the target share its mark.
    #[must_use]
    pub fn guard(
        marks: &'static LocalKey<RefCell<Vec<(usize, usize)>>>,
        mark: (usize, usize),
        enter: bool,
    ) -> bool {
        marks.with_borrow_mut(|marks| {
            if !enter {
                marks.pop();
                false
            } else if marks.contains(&mark) {
                true
            } else {
                marks.push(mark);
                false
            }
        })
    }

    /// The `minContains` and `maxContains` beside a `contains`.
    #[derive(Clone, Copy)]
    struct ContainsBounds {
        min: Option<u64>,
        max: Option<u64>,
    }

    impl ContainsBounds {
        /// The error of an array with `count` items matching `contains`, if that count fails.
        fn error<F: Json>(self, count: usize, instance: &F::Node<'_>) -> Option<ErrorDescription> {
            let count = count as u64;
            let instance = instance.lazy_value();
            let error = match self.max {
                Some(max) if count > max => ValidationError::max_contains(
                    Location::new(),
                    Location::new(),
                    Location::new(),
                    instance,
                    max,
                ),
                _ if count >= self.min.unwrap_or(1) => return None,
                _ => match self.min {
                    Some(min) => ValidationError::min_contains(
                        Location::new(),
                        Location::new(),
                        Location::new(),
                        instance,
                        min,
                    ),
                    None => ValidationError::contains(
                        Location::new(),
                        Location::new(),
                        Location::new(),
                        instance,
                    ),
                },
            };
            Some(ErrorDescription::from_validation_error(&error))
        }
    }

    /// The error of the `false` schema at `site` for the instance at `location`.
    #[cold]
    #[inline(never)]
    fn rejection<F: Json>(
        site: &UnitLocation,
        value: &F::Node<'_>,
        location: &LazyLocation,
    ) -> ErrorDescription {
        let error = ValidationError::false_schema(
            site.keyword.clone(),
            capture_evaluation_path(None, &site.keyword),
            Location::from(location),
            value.lazy_value(),
        );
        ErrorDescription::from_validation_error(&error)
    }

    /// The instance as a JSON value, for an error message.
    #[inline]
    pub fn instance_value<'a, F: Json>(instance: &F::Node<'a>) -> Cow<'a, Value> {
        instance.to_value()
    }

    /// A leaf unit's errors as `evaluate` reports them.
    #[derive(Default)]
    pub struct Descriptions(Vec<ErrorDescription>);

    impl Descriptions {
        /// Records `error`; never stops the caller.
        // Takes `error` as `Errors::push` does, since the same generated code calls both.
        #[allow(clippy::needless_pass_by_value)]
        #[inline]
        #[must_use]
        pub fn push(&mut self, error: ValidationError<'_>) -> bool {
            self.0.push(ErrorDescription::from_validation_error(&error));
            false
        }
    }

    /// Put one resource's entries on the generated code's dynamic-scope stacks, and take them off:
    /// its `$dynamicAnchor` schemas, and its root for `$recursiveRef` when that root has
    /// `$recursiveAnchor: true`.
    #[derive(Clone, Copy)]
    pub struct ScopeStacks {
        pub enter: fn(usize),
        pub leave: fn(usize),
    }

    pub struct Built {
        /// Each unit and schema by the locations its errors are reported at.
        sites: AHashMap<Box<str>, Vec<usize>>,
        schemas: Box<[UnitLocation]>,
        /// Each entry of `Tables::functions`.
        functions: Box<[Function]>,
        /// `Tables::root_function`.
        root_function: Option<usize>,
        resources: Box<[ResourceTable]>,
        /// References track the dynamic scope, since a `$dynamicRef`, a `$recursiveRef` or
        /// generated code reads it.
        keeps_scope: bool,
        /// Each schema's units, a range of `units`.
        schema_units: Box<[(usize, usize)]>,
        units: Box<[UnitLocation]>,
    }

    /// # Panics
    ///
    /// The tables are not ones the generator writes.
    fn build(tables: &str) -> Built {
        let tables: Tables =
            serde_json::from_str(tables).expect("the generator writes valid tables");
        let base = tables
            .base
            .map(|base| Uri::parse(base).expect("the generator writes a URI base"));
        let mut buffer = uri::EncodedBuffer::new();
        let mut locate =
            |base: Option<&Uri<String>>, pointer: &str, at: &str, annotations: Option<Value>| {
                let keyword = Location::from_escaped(pointer);
                let absolute =
                    base.map(|base| Arc::new(resolve_absolute_location(base, at, &mut buffer)));
                UnitLocation {
                    schema: format_keyword_location(&keyword, absolute.as_ref()),
                    keyword,
                    absolute,
                    aliases: Vec::new(),
                    site: 0,
                    annotations: annotations.map(Annotations::new),
                    reference: Reference::default(),
                }
            };
        let mut units: Vec<Option<UnitLocation>> = tables.units.iter().map(|_| None).collect();
        let mut unit_tables: Vec<Option<UnitTable>> = tables.units.into_iter().map(Some).collect();
        let mut schema_units = Vec::with_capacity(tables.schemas.len());
        let mut schemas = Vec::with_capacity(tables.schemas.len());
        for schema in tables.schemas {
            let pointer = &schema.pointer;
            let at = schema.at.as_deref().unwrap_or(pointer);
            let own = schema
                .base
                .map(|base| Uri::parse(base).expect("the generator writes a URI base"));
            let base = if schema.default_base {
                None
            } else {
                own.as_ref().or(base.as_ref())
            };
            for index in schema.first..schema.first + schema.count {
                let caller = schema.resource;
                let (keyword, aliases, strings, reference) = match unit_tables[index]
                    .take()
                    .expect("every unit belongs to one schema")
                {
                    UnitTable::Keyword(keyword) => {
                        (keyword, Vec::new(), None, Reference::default())
                    }
                    UnitTable::Detailed {
                        keyword,
                        aliases,
                        strings,
                        target,
                        function,
                        leaves,
                    } => (
                        keyword,
                        aliases,
                        strings,
                        Reference {
                            target,
                            function,
                            leaves,
                            caller,
                        },
                    ),
                };
                units[index] = Some(UnitLocation {
                    aliases: aliases
                        .iter()
                        .map(|alias| Location::from_escaped(&format!("{pointer}/{alias}")))
                        .collect(),
                    reference,
                    ..locate(
                        base,
                        &format!("{pointer}/{keyword}"),
                        &format!("{at}/{keyword}"),
                        strings,
                    )
                });
            }
            schema_units.push((schema.first, schema.count));
            schemas.push(locate(base, pointer, at, schema.annotations));
        }
        let mut units: Box<[UnitLocation]> = units
            .into_iter()
            .map(|unit| unit.expect("every unit belongs to a schema"))
            .collect();
        let mut sites: AHashMap<Box<str>, Vec<usize>> = AHashMap::new();
        for (id, site) in units.iter_mut().chain(schemas.iter_mut()).enumerate() {
            site.site = id;
            for path in std::iter::once(&site.keyword).chain(&site.aliases) {
                sites.entry(path.as_str().into()).or_default().push(id);
            }
        }
        Built {
            sites,
            schemas: schemas.into_boxed_slice(),
            functions: tables.functions.into_boxed_slice(),
            root_function: tables.root_function,
            resources: tables.resources.into_boxed_slice(),
            keeps_scope: tables.keeps_scope,
            schema_units: schema_units.into_boxed_slice(),
            units,
        }
    }

    /// A unit's evaluation path below a `$ref`: its rebase, and the address of its keyword
    /// location in `Built`.
    #[derive(PartialEq, Eq, Hash)]
    struct RebasedPath {
        rebase: usize,
        keyword: usize,
    }

    /// A rebase, by the rebase it sits in (`0` for none) and the address of its `$ref` unit's
    /// keyword location in `Built`. Together they determine the dynamic scope, and with it the
    /// target.
    #[derive(PartialEq, Eq, Hash)]
    struct RebaseIdentity {
        outer: usize,
        site: usize,
    }

    /// Where the units of a `$ref` target are reported: their pointers below the target, after
    /// the evaluation path of the `$ref`.
    #[derive(Clone)]
    struct Rebase {
        /// The length of the target's pointer.
        strip: usize,
        prefix: Location,
        /// The address of the `$ref` unit's keyword location in `Built`.
        site: usize,
        /// Its identity, set on first use; `0` before.
        id: Cell<usize>,
    }

    /// The sites with errors at one of their schema locations, and each visit's errors by index
    /// into `errors`, whose evaluation paths are `paths`.
    fn index<'e>(
        built: &Built,
        errors: &[&'e ValidationError<'_>],
        paths: &[&'e str],
    ) -> (Vec<bool>, Visits<'e>) {
        let mut reported = Vec::new();
        if !errors.is_empty() {
            reported.resize(built.units.len() + built.schemas.len(), false);
        }
        let visits = errors
            .iter()
            .zip(paths)
            .map(|(error, evaluation_path)| Visit {
                evaluation_path,
                instance_location: error.instance_path().as_str(),
                string: error.instance().as_str(),
            });
        for error in errors {
            for &site in built
                .sites
                .get(error.schema_path().as_str())
                .into_iter()
                .flatten()
            {
                reported[site] = true;
            }
        }
        if errors.len() <= VISIT_SCAN {
            return (reported, Visits::Scan(visits.collect()));
        }
        let mut by_visit: AHashMap<Visit<'_>, Reported> = AHashMap::with_capacity(errors.len());
        for (index, visit) in visits.enumerate() {
            match by_visit.entry(visit) {
                Entry::Occupied(mut entry) => entry.get_mut().push(index),
                Entry::Vacant(entry) => {
                    entry.insert(Reported::One(index));
                }
            }
        }
        (reported, Visits::Keyed(by_visit))
    }

    /// Up to this many errors are scanned; more go into a map keyed by visit.
    const VISIT_SCAN: usize = 8;

    /// The visit of each error.
    enum Visits<'e> {
        /// In report order.
        Scan(Vec<Visit<'e>>),
        /// Indices into `Evaluator::errors` by visit.
        Keyed(AHashMap<Visit<'e>, Reported>),
    }

    /// `errors`, with the errors nested in a failing `anyOf`, a `oneOf` with no valid branch, or a
    /// `propertyNames` in place of the error holding them: the runtime reports the nested ones
    /// only.
    fn flatten<'e, 'i>(errors: &'e [ValidationError<'i>], into: &mut Vec<&'e ValidationError<'i>>) {
        for error in errors {
            match error.kind() {
                ValidationErrorKind::AnyOf { context }
                | ValidationErrorKind::OneOfNotValid { context } => {
                    for branch in context {
                        flatten(branch, into);
                    }
                }
                ValidationErrorKind::PropertyNames { error } => {
                    flatten(std::slice::from_ref(error.as_ref()), into);
                }
                _ => into.push(error),
            }
        }
    }

    /// Where the evaluation reaches a unit. The visits of one unit below a `$ref` cycle, or for each
    /// item of an array, share its schema location.
    #[derive(PartialEq, Eq, Hash)]
    struct Visit<'e> {
        evaluation_path: &'e str,
        instance_location: &'e str,
        /// A name is evaluated at its object's location; its value tells the names apart.
        string: Option<&'e str>,
    }

    /// The errors one visit reports, as indices into `errors` in report order. Most visits report
    /// one, which needs no allocation.
    enum Reported {
        One(usize),
        Many(Vec<usize>),
    }

    impl Reported {
        fn push(&mut self, index: usize) {
            match self {
                Reported::One(first) => *self = Reported::Many(vec![*first, index]),
                Reported::Many(all) => all.push(index),
            }
        }

        fn as_slice(&self) -> &[usize] {
            match self {
                Reported::One(index) => std::slice::from_ref(index),
                Reported::Many(all) => all,
            }
        }
    }

    /// The units and schemas a generated `evaluate` reports, located on first use.
    pub struct Sites {
        /// The JSON the generator writes.
        pub tables: &'static str,
        /// Set when the generated code resolves references through the dynamic scope.
        pub scope: Option<ScopeStacks>,
        pub built: OnceLock<Built>,
    }

    impl Sites {
        /// An evaluation whose units report their own errors, of which there are none when the
        /// instance is `valid`.
        pub fn checking(&'static self, valid: bool) -> Evaluator<'static, 'static> {
            Evaluator::new(self.built(), self.scope, !valid, false, Vec::new())
        }

        /// An evaluation whose units report `errors`, which a run of the generated `errors`
        /// function (the errors run) collected.
        pub fn reporting<'e, 'i>(
            &'static self,
            errors: &'e [ValidationError<'i>],
        ) -> Evaluator<'e, 'i> {
            let mut flat = Vec::with_capacity(errors.len());
            flatten(errors, &mut flat);
            Evaluator::new(self.built(), self.scope, false, true, flat)
        }

        fn built(&'static self) -> &'static Built {
            self.built.get_or_init(|| build(self.tables))
        }
    }

    /// Builds the output of a generated `evaluate`, for the code the generator emits per schema.
    pub struct Evaluator<'e, 'i> {
        built: &'static Built,
        /// Whether units report their own errors, as the instance fails.
        checks: bool,
        /// Whether units take their errors from the errors run instead.
        reporting: bool,
        stacks: Option<ScopeStacks>,
        errors: Vec<&'e ValidationError<'i>>,
        /// Whether each site, by `UnitLocation::site`, has errors at one of its schema locations, so
        /// one without builds no visit key; empty without errors.
        reported: Vec<bool>,
        /// Indices into `errors` by the visit that reports them.
        by_visit: Visits<'e>,
        arena: EvaluationArena,
        /// The rebases of the `$ref`s the evaluation is below, the outermost first.
        rebases: Vec<Rebase>,
        /// The code the evaluation is in, an index of `Built::functions`: the innermost static
        /// `$ref` target's, `Built::root_function` at the root, `None` below a reference through
        /// the dynamic scope.
        function: Option<usize>,
        /// Each schema a reference through the dynamic scope entered, with the address of its
        /// instance location.
        active: Vec<(usize, usize)>,
        /// The dynamic scope, outermost first: each resource a reference left, and each resource
        /// enclosing an embedded one the evaluation entered.
        scope: Vec<usize>,
        /// Evaluation paths below a `$ref`, built once per rebase and unit after the first
        /// `EVALUATION_PATH_CACHE_THRESHOLD` builds, which small instances do not repay.
        rebased_paths: RefCell<AHashMap<RebasedPath, Location>>,
        rebased_calls: Cell<u32>,
        rebase_ids: RefCell<AHashMap<RebaseIdentity, usize>>,
        /// Reused for the instance location of each visit with errors.
        instance_location: Cell<String>,
    }

    /// The schemas references through the dynamic scope entered, set aside while a name is
    /// evaluated at its object's location.
    pub struct ActiveTargets(Vec<(usize, usize)>);

    impl<'e, 'i> Evaluator<'e, 'i> {
        fn new(
            built: &'static Built,
            stacks: Option<ScopeStacks>,
            checks: bool,
            reporting: bool,
            errors: Vec<&'e ValidationError<'i>>,
        ) -> Self {
            let paths: Vec<&str> = errors
                .iter()
                .map(|error| error.evaluation_path().as_str())
                .collect();
            let (reported, by_visit) = index(built, &errors, &paths);
            Evaluator {
                built,
                checks,
                reporting,
                stacks,
                errors,
                reported,
                by_visit,
                arena: EvaluationArena::default(),
                rebases: Vec::new(),
                function: built.root_function,
                active: Vec::new(),
                scope: Vec::new(),
                rebased_paths: RefCell::default(),
                rebased_calls: Cell::new(0),
                rebase_ids: RefCell::default(),
                instance_location: Cell::default(),
            }
        }

        /// The evaluation whose root output unit is `root`.
        #[must_use]
        pub fn finish(self, root: EvaluationNode) -> Evaluation {
            Evaluation::with_root(self.arena, root)
        }

        /// Whether units report their own errors.
        #[inline]
        #[must_use]
        pub fn checks(&self) -> bool {
            self.checks
        }

        /// Records the position of the instance at `location`, one segment below `parent`.
        #[cfg_attr(debug_assertions, inline)]
        pub fn position_below(
            &mut self,
            parent: InstancePosition,
            location: &LazyLocation,
        ) -> InstancePosition {
            self.arena.position_below(parent, location.segment())
        }

        /// `list` with `node` added.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn push(&mut self, mut list: ChildList, node: EvaluationNode) -> ChildList {
            list.push(&mut self.arena, node);
            list
        }

        /// `units` with the node of the leaf unit at `unit` added, with its keyword's annotation
        /// when `annotated`.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn leaf(
            &mut self,
            mut units: ChildList,
            unit: usize,
            errors: Vec<ErrorDescription>,
            annotated: bool,
            position: InstancePosition,
        ) -> ChildList {
            let site = &self.built.units[unit];
            let annotations = if annotated {
                site.annotations.clone()
            } else {
                None
            };
            let node = self.node(site, errors, annotations, ChildList::default(), position);
            units.push(&mut self.arena, node);
            units
        }

        /// `units` with the node of the single `type` unit at `unit` added, with the error
        /// `check` words for `instance` when units check.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn not_of_type<'a, F: Json>(
            &mut self,
            units: ChildList,
            unit: usize,
            check: fn(&F::Node<'a>) -> Vec<ErrorDescription>,
            instance: &F::Node<'a>,
            position: InstancePosition,
        ) -> ChildList {
            let errors = if self.checks {
                check(instance)
            } else {
                Vec::new()
            };
            self.leaf(units, unit, errors, false, position)
        }

        /// `units` with the node of the leaf unit at `unit` added, with the errors its keyword's
        /// code `check` reports for `instance` when units check, and its keyword's annotation when
        /// `annotated`.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn checked<'a, F: Json>(
            &mut self,
            units: ChildList,
            unit: usize,
            check: fn(F::Node<'a>, &LazyLocation, &mut Descriptions) -> bool,
            instance: &F::Node<'a>,
            annotated: bool,
            position: InstancePosition,
        ) -> ChildList {
            let errors = if self.checks {
                let mut errors = Descriptions::default();
                let _ = check(instance.clone(), &LazyLocation::new(), &mut errors);
                errors.0
            } else {
                Vec::new()
            };
            self.leaf(units, unit, errors, annotated, position)
        }

        /// `units` with the node of the leaf unit at `unit` added, with the errors the errors run
        /// reports there for `instance` at `location`, and its keyword's annotation when
        /// `annotated`.
        #[inline]
        #[must_use]
        pub fn reported<F: Json>(
            &mut self,
            units: ChildList,
            unit: usize,
            instance: &F::Node<'_>,
            location: &LazyLocation,
            annotated: bool,
            position: InstancePosition,
        ) -> ChildList {
            let errors = self.errors_at::<F>(&self.built.units[unit], instance, location);
            self.leaf(units, unit, errors, annotated, position)
        }

        /// `units` with the node of the unit at `unit`, over `children`, added.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn unit(
            &mut self,
            mut units: ChildList,
            unit: usize,
            errors: Vec<ErrorDescription>,
            annotations: Option<Annotations>,
            children: ChildList,
            position: InstancePosition,
        ) -> ChildList {
            let node = self.node(
                &self.built.units[unit],
                errors,
                annotations,
                children,
                position,
            );
            units.push(&mut self.arena, node);
            units
        }

        /// The node of the schema at `schema`, over its `units`.
        #[cfg_attr(debug_assertions, inline)]
        #[must_use]
        pub fn schema(
            &self,
            schema: usize,
            errors: Vec<ErrorDescription>,
            units: ChildList,
            position: InstancePosition,
        ) -> EvaluationNode {
            let site = &self.built.schemas[schema];
            self.node(site, errors, site.annotations.clone(), units, position)
        }

        /// The errors the errors run reports at the unit at `unit` for the instance at `location`.
        #[must_use]
        pub fn unit_errors<F: Json>(
            &self,
            unit: usize,
            instance: &F::Node<'_>,
            location: &LazyLocation,
        ) -> Vec<ErrorDescription> {
            self.errors_at::<F>(&self.built.units[unit], instance, location)
        }

        /// The errors the errors run reports at the schema at `schema` itself.
        #[must_use]
        pub fn schema_errors<F: Json>(
            &self,
            schema: usize,
            instance: &F::Node<'_>,
            location: &LazyLocation,
        ) -> Vec<ErrorDescription> {
            self.errors_at::<F>(&self.built.schemas[schema], instance, location)
        }

        /// The error of the `false` schema at `schema` for `value` at `location`, when units check.
        #[must_use]
        pub fn rejection<F: Json>(
            &self,
            schema: usize,
            value: &F::Node<'_>,
            location: &LazyLocation,
        ) -> Vec<ErrorDescription> {
            if self.checks {
                vec![rejection::<F>(&self.built.schemas[schema], value, location)]
            } else {
                Vec::new()
            }
        }

        /// The `false` subschema at `schema` failing for `value` at `location`.
        #[must_use]
        pub fn rejected<F: Json>(
            &self,
            schema: usize,
            value: &F::Node<'_>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> EvaluationNode {
            let site = &self.built.schemas[schema];
            let errors = vec![rejection::<F>(site, value, location)];
            self.node(site, errors, None, ChildList::default(), position)
        }

        /// Moves the items `run` holds onto `children`, as one run of the schema at `schema`, which
        /// has leaf units only and annotates nothing, over the array at `parent`.
        #[inline]
        fn end_run(
            &mut self,
            children: &mut ChildList,
            run: &mut PendingRun,
            units: &mut Option<RunUnits>,
            schema: usize,
            parent: InstancePosition,
        ) {
            if run.is_empty() {
                return;
            }
            let units = *units.get_or_insert_with(|| self.run_units(schema));
            children.push_run(&mut self.arena, parent, run, units);
        }

        /// The units each valid item of the leaf schema at `schema` reports, below the current
        /// `$ref`s.
        #[inline]
        fn run_units(&mut self, schema: usize) -> RunUnits {
            let site = &self.built.schemas[schema];
            let latest = if self.rebases.is_empty() {
                self.arena.latest_run_units(schema, &site.keyword)
            } else {
                self.arena
                    .latest_run_units(schema, &self.rebased_path(site))
            };
            latest.unwrap_or_else(|| self.add_run_units(schema))
        }

        /// Records the units [`Self::run_units`] finds no latest ones for.
        #[cold]
        #[inline(never)]
        fn add_run_units(&mut self, schema: usize) -> RunUnits {
            let (first, count) = self.built.schema_units[schema];
            let shared: Vec<RunUnit> = std::iter::once(&self.built.schemas[schema])
                .chain(&self.built.units[first..first + count])
                .map(|site| RunUnit {
                    evaluation_path: self.evaluation_path(site),
                    schema_location: Arc::clone(&site.schema),
                })
                .collect();
            self.arena.add_run_units(schema, shared)
        }

        /// `units` with the node of the static reference unit at `unit` added, which evaluates its
        /// target with `evaluate`.
        #[inline]
        #[must_use]
        pub fn reference<'a, F: Json>(
            &mut self,
            mut units: ChildList,
            unit: usize,
            guard: Option<fn(&F::Node<'a>, bool) -> bool>,
            evaluate: fn(&mut Self, F::Node<'a>, &LazyLocation, InstancePosition) -> EvaluationNode,
            instance: F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> ChildList {
            let reference = self.built.units[unit].reference;
            let pushed = if self.built.keeps_scope {
                self.pushed(reference.leaves, reference.caller)
            } else {
                None
            };
            let node = self
                .reference_node::<F>(unit, guard, pushed, evaluate, instance, location, position);
            units.push(&mut self.arena, node);
            units
        }

        /// The node of the schema at `schema`, whose only unit is a static reference, which
        /// evaluates its target with `evaluate`.
        #[inline]
        pub fn reference_schema<'a, F: Json>(
            &mut self,
            schema: usize,
            guard: Option<fn(&F::Node<'a>, bool) -> bool>,
            evaluate: fn(&mut Self, F::Node<'a>, &LazyLocation, InstancePosition) -> EvaluationNode,
            instance: F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> EvaluationNode {
            let errors = if self.reporting {
                self.errors_at::<F>(&self.built.schemas[schema], &instance, location)
            } else {
                Vec::new()
            };
            let (unit, _) = self.built.schema_units[schema];
            let units = self.reference::<F>(
                ChildList::default(),
                unit,
                guard,
                evaluate,
                instance,
                location,
                position,
            );
            self.schema(schema, errors, units, position)
        }

        /// `units` with the node of the schema-form `items` or `additionalItems` unit at `unit`
        /// added, applying the schema at `schema`, whose only unit is a static reference, to each
        /// item of the array `instance` from `skip` on.
        #[must_use]
        pub fn reference_items<'a, F: Json>(
            &mut self,
            units: ChildList,
            unit: usize,
            schema: usize,
            skip: usize,
            guard: Option<fn(&F::Node<'a>, bool) -> bool>,
            evaluate: fn(&mut Self, F::Node<'a>, &LazyLocation, InstancePosition) -> EvaluationNode,
            instance: &F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> ChildList {
            let Some(array) = instance.as_array() else {
                return self.unit(
                    units,
                    unit,
                    Vec::new(),
                    None,
                    ChildList::default(),
                    position,
                );
            };
            let len = array.len();
            let mut children = ChildList::default();
            for (index, item) in array.elements().enumerate().skip(skip) {
                let path = location.push(index);
                let below = self.position_below(position, &path);
                let child = self.reference_schema::<F>(schema, guard, evaluate, item, &path, below);
                children.push(&mut self.arena, child);
            }
            let annotations = Some(Annotations::boolean(len > skip));
            self.unit(units, unit, Vec::new(), annotations, children, position)
        }

        /// The node of the reference unit at `unit` entering its static target, `pushed` joining
        /// the dynamic scope. Code the generator emitted inside the target's own code enters it
        /// under `guard`.
        #[inline]
        pub fn reference_node<'a, F: Json>(
            &mut self,
            unit: usize,
            guard: Option<fn(&F::Node<'a>, bool) -> bool>,
            pushed: Option<usize>,
            evaluate: fn(&mut Self, F::Node<'a>, &LazyLocation, InstancePosition) -> EvaluationNode,
            instance: F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> EvaluationNode {
            let Reference {
                target, function, ..
            } = self.built.units[unit].reference;
            self.enter_reference(unit, target, pushed);
            let guarded = guard.filter(|_| self.compiling(function));
            let node = if guarded.is_some_and(|guard| guard(&instance, true)) {
                self.reentered(target, position)
            } else {
                let outer = self.function.replace(function);
                let node = match guarded {
                    Some(guard) => {
                        let node = evaluate(self, instance.clone(), location, position);
                        guard(&instance, false);
                        node
                    }
                    None => evaluate(self, instance, location, position),
                };
                self.function = outer;
                node
            };
            self.leave_reference(pushed.is_some());
            node
        }

        /// The node of the reference unit at `unit` landing on `landed` through the dynamic scope,
        /// `pushed` joining it.
        pub fn landed<'a, F: Json>(
            &mut self,
            unit: usize,
            landed: usize,
            pushed: Option<usize>,
            evaluate: fn(&mut Self, F::Node<'a>, &LazyLocation, InstancePosition) -> EvaluationNode,
            instance: F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> EvaluationNode {
            self.enter_reference(unit, landed, pushed);
            // In-place applicators pass the same location on, so its address identifies it.
            let entry = (landed, std::ptr::from_ref(location).addr());
            let node = if self.active.contains(&entry) {
                self.reentered(landed, position)
            } else {
                self.active.push(entry);
                let outer = self.function.take();
                let node = evaluate(self, instance, location, position);
                self.function = outer;
                self.active.pop();
                node
            };
            self.leave_reference(pushed.is_some());
            node
        }

        /// `units` with the node of the schema-form `items` or `additionalItems` unit at `unit`
        /// added, applying the schema at `schema`, which joins item runs, to each item of the array
        /// `instance` from `skip` on, through `leaf_item`.
        #[must_use]
        pub fn leaf_items<'a, F: Json>(
            &mut self,
            units: ChildList,
            unit: usize,
            schema: usize,
            skip: usize,
            leaf_item: fn(
                &mut Self,
                F::Node<'a>,
                &LazyLocation,
                InstancePosition,
            ) -> Option<EvaluationNode>,
            instance: &F::Node<'a>,
            location: &LazyLocation,
            position: InstancePosition,
        ) -> ChildList {
            let Some(array) = instance.as_array() else {
                return self.unit(
                    units,
                    unit,
                    Vec::new(),
                    None,
                    ChildList::default(),
                    position,
                );
            };
            let len = array.len();
            let mut children = ChildList::default();
            if self.reporting || self.checks {
                let mut run = PendingRun::default();
                let mut run_units = None;
                for (index, item) in array.elements().enumerate().skip(skip) {
                    let path = location.push(index);
                    match leaf_item(self, item, &path, position) {
                        Some(child) => {
                            self.end_run(&mut children, &mut run, &mut run_units, schema, position);
                            children.push(&mut self.arena, child);
                        }
                        None => run.add(index),
                    }
                }
                self.end_run(&mut children, &mut run, &mut run_units, schema, position);
            } else if len > skip {
                // Without errors to report, every item joins one run.
                let mut run = PendingRun {
                    start: skip,
                    count: u32::try_from(len - skip).expect("evaluation exceeded u32 run items"),
                };
                let run_units = self.run_units(schema);
                children.push_run(&mut self.arena, position, &mut run, run_units);
            }
            let annotations = Some(Annotations::boolean(len > skip));
            self.unit(units, unit, Vec::new(), annotations, children, position)
        }

        /// Enters the schema at `target` through the reference unit at `unit`: its units report
        /// below the reference, and `pushed` joins the dynamic scope.
        #[inline]
        fn enter_reference(&mut self, unit: usize, target: usize, pushed: Option<usize>) {
            let site = &self.built.units[unit];
            let rebase = Rebase {
                strip: self.built.schemas[target].keyword.as_str().len(),
                prefix: self.evaluation_path(site),
                site: site.keyword.as_ptr(),
                id: Cell::new(0),
            };
            self.rebases.push(rebase);
            self.scope.extend(pushed);
        }

        /// Leaves the reference entered last, which `pushed` a resource to the dynamic scope.
        #[inline]
        fn leave_reference(&mut self, pushed: bool) {
            if pushed {
                self.scope.pop();
            }
            self.rebases.pop();
        }

        /// Descends into a resource embedded in the one at `enclosing`, which joins the dynamic
        /// scope below it unless a reference staying in it put it there already. Whether it
        /// joined goes to [`Self::leave_resource`].
        #[inline]
        #[must_use]
        pub fn enter_resource(&mut self, enclosing: usize) -> bool {
            let joins = self.scope.last() != Some(&enclosing);
            if joins {
                self.scope.push(enclosing);
            }
            joins
        }

        /// Leaves the resource [`Self::enter_resource`] descended into.
        #[inline]
        pub fn leave_resource(&mut self, joined: bool) {
            if joined {
                self.scope.pop();
            }
        }

        /// Whether the evaluation is in code the generator emitted inside the code at `function`,
        /// so that code guards its references to it.
        fn compiling(&self, function: usize) -> bool {
            let mut current = self.function;
            while let Some(index) = current {
                if index == function {
                    return true;
                }
                current = self.built.functions[index].parent;
            }
            false
        }

        /// The node of the schema at `target` reached again on an instance it is being evaluated
        /// against: the runtime's guard reports it valid, with nothing below it.
        fn reentered(&self, target: usize, position: InstancePosition) -> EvaluationNode {
            self.node(
                &self.built.schemas[target],
                Vec::new(),
                None,
                ChildList::default(),
                position,
            )
        }

        /// The resource a static reference in the resource `caller` adds to the dynamic scope: the
        /// caller's, when the scope is empty or the reference `leaves` it.
        #[inline]
        fn pushed(&self, leaves: bool, caller: usize) -> Option<usize> {
            (self.scope.is_empty() || leaves).then_some(caller)
        }

        /// The schema the reference unit at `unit` lands on, resolving through the dynamic scope
        /// by `anchor` or under `$recursiveRef` when `recursive`, and the resource following it
        /// adds to the dynamic scope.
        #[must_use]
        pub fn resolve(&self, unit: usize, anchor: Option<&str>, recursive: bool) -> Landing {
            let Reference {
                target,
                leaves,
                caller,
                ..
            } = self.built.units[unit].reference;
            let resources = &self.built.resources;
            if recursive {
                // The outermost resource in the scope whose root has `$recursiveAnchor: true`.
                let landed = match self
                    .scope
                    .iter()
                    .find(|&&resource| resources[resource].recursive)
                {
                    Some(&resource) => (
                        resources[resource]
                            .root
                            .expect("a recursive resource's root"),
                        resource,
                    ),
                    None => (target, caller),
                };
                let pushed = (self.scope.is_empty() || landed.1 != caller).then_some(caller);
                return Landing {
                    schema: landed.0,
                    pushed,
                };
            }
            let Some(name) = anchor else {
                return Landing {
                    schema: target,
                    pushed: self.pushed(leaves, caller),
                };
            };
            // The outermost resource with that anchor in the scope or the caller's. Landing in
            // another resource keeps the caller's in the scope.
            let found = self
                .scope
                .iter()
                .chain(std::iter::once(&caller))
                .find_map(|&resource| {
                    resources[resource]
                        .anchors
                        .iter()
                        .find(|(anchor, _)| anchor == name)
                        .map(|&(_, schema)| (schema, resource))
                });
            match found {
                Some((schema, resource)) => Landing {
                    schema,
                    pushed: self.pushed(resource != caller, caller),
                },
                None => Landing {
                    schema: target,
                    pushed: self.pushed(leaves, caller),
                },
            }
        }

        /// Sets the targets entered through the dynamic scope aside, for a name evaluated at its
        /// object's location as a different instance.
        #[must_use]
        pub fn take_active(&mut self) -> ActiveTargets {
            ActiveTargets(std::mem::take(&mut self.active))
        }

        /// Restores the targets [`Self::take_active`] set aside.
        pub fn restore_active(&mut self, active: ActiveTargets) {
            self.active = active.0;
        }

        /// Puts the evaluation's dynamic scope on the generated stacks, for generated code running
        /// in `resource`.
        pub fn enter_scope(&self, resource: usize) {
            if let Some(stacks) = self.stacks {
                for &entered in &self.scope {
                    (stacks.enter)(entered);
                }
                (stacks.enter)(resource);
            }
        }

        /// Takes off what [`Self::enter_scope`] put on the generated stacks.
        pub fn leave_scope(&self, resource: usize) {
            if let Some(stacks) = self.stacks {
                (stacks.leave)(resource);
                for &entered in self.scope.iter().rev() {
                    (stacks.leave)(entered);
                }
            }
        }

        /// The errors run's errors of an `unevaluated*` subschema, rebased below the current
        /// `$ref`s, for the evaluation nested under that subschema.
        #[must_use]
        pub fn nested_errors<'n, 'm>(
            &self,
            errors: &'n [ValidationError<'m>],
        ) -> NestedErrors<'n, 'm> {
            let mut flat = Vec::with_capacity(errors.len());
            flatten(errors, &mut flat);
            let paths = flat
                .iter()
                .map(|error| self.rebased_str(error.evaluation_path().as_str()))
                .collect();
            NestedErrors {
                errors: flat,
                paths,
            }
        }

        /// An evaluation continuing this one, with its units reporting `errors`.
        #[must_use]
        pub fn nest<'n, 'm>(&mut self, errors: &'n NestedErrors<'_, 'm>) -> Evaluator<'n, 'm> {
            let paths: Vec<&str> = errors.paths.iter().map(AsRef::as_ref).collect();
            let (reported, by_visit) = index(self.built, &errors.errors, &paths);
            Evaluator {
                built: self.built,
                checks: self.checks,
                reporting: self.reporting,
                stacks: self.stacks,
                errors: errors.errors.clone(),
                reported,
                by_visit,
                arena: std::mem::take(&mut self.arena),
                rebases: self.rebases.clone(),
                function: self.function,
                active: self.active.clone(),
                scope: self.scope.clone(),
                rebased_paths: RefCell::new(self.rebased_paths.take()),
                rebased_calls: self.rebased_calls.clone(),
                rebase_ids: RefCell::new(self.rebase_ids.take()),
                instance_location: Cell::default(),
            }
        }

        /// Continues this evaluation after `nested`.
        pub fn unnest(&mut self, nested: Evaluator<'_, '_>) {
            self.arena = nested.arena;
            self.rebased_paths = nested.rebased_paths;
            self.rebased_calls = nested.rebased_calls;
            self.rebase_ids = nested.rebase_ids;
        }

        #[inline]
        fn evaluation_path(&self, site: &UnitLocation) -> Location {
            if self.rebases.is_empty() {
                site.keyword.clone()
            } else {
                self.rebased_path(site)
            }
        }

        // Out of line, so the clone above needs no stack frame.
        #[inline(never)]
        fn rebased_path(&self, site: &UnitLocation) -> Location {
            let rebase = self.rebases.last().expect("a rebase below a `$ref`");
            let build = || {
                rebase
                    .prefix
                    .join_raw_suffix(&site.keyword.as_str()[rebase.strip..])
            };
            let calls = self.rebased_calls.get();
            if calls < EVALUATION_PATH_CACHE_THRESHOLD {
                self.rebased_calls.set(calls + 1);
                return build();
            }
            let rebase = match rebase.id.get() {
                0 => self.rebase_id(self.rebases.len() - 1),
                id => id,
            };
            let key = RebasedPath {
                rebase,
                keyword: site.keyword.as_ptr(),
            };
            self.rebased_paths
                .borrow_mut()
                .entry(key)
                .or_insert_with(build)
                .clone()
        }

        /// The identity of `rebases[index]`, from the rebase it sits in and its `$ref` unit.
        fn rebase_id(&self, index: usize) -> usize {
            let rebase = &self.rebases[index];
            if rebase.id.get() != 0 {
                return rebase.id.get();
            }
            let outer = index
                .checked_sub(1)
                .map_or(0, |outer| self.rebase_id(outer));
            let mut ids = self.rebase_ids.borrow_mut();
            let next = ids.len() + 1;
            let id = *ids
                .entry(RebaseIdentity {
                    outer,
                    site: rebase.site,
                })
                .or_insert(next);
            rebase.id.set(id);
            id
        }

        fn rebased_str<'k>(&self, path: &'k str) -> Cow<'k, str> {
            match self.rebases.last() {
                None => Cow::Borrowed(path),
                Some(rebase) => Cow::Owned(format!(
                    "{}{}",
                    rebase.prefix.as_str(),
                    &path[rebase.strip..]
                )),
            }
        }

        /// The errors reported at `site` or one of its aliases for the instance at `location`, in
        /// the order they were reported.
        #[inline]
        fn errors_at<F: Json>(
            &self,
            site: &UnitLocation,
            instance: &F::Node<'_>,
            location: &LazyLocation,
        ) -> Vec<ErrorDescription> {
            if self.reported.get(site.site).copied().unwrap_or(false) {
                self.reported_errors_at::<F>(site, instance, location)
            } else {
                Vec::new()
            }
        }

        fn reported_errors_at<F: Json>(
            &self,
            site: &UnitLocation,
            instance: &F::Node<'_>,
            location: &LazyLocation,
        ) -> Vec<ErrorDescription> {
            // Built on the first path and reused for the aliases.
            let mut instance_location = None;
            let mut string = None;
            // A visit's errors all come from one of the unit's schema locations.
            for path in std::iter::once(&site.keyword).chain(&site.aliases) {
                let instance_location = instance_location.get_or_insert_with(|| {
                    let mut buffer = self.instance_location.take();
                    buffer.clear();
                    write_location(location, &mut buffer);
                    buffer
                });
                let string = string.get_or_insert_with(|| instance.as_string());
                let evaluation_path = self.rebased_str(path.as_str());
                let visit = Visit {
                    evaluation_path: &evaluation_path,
                    instance_location: instance_location.as_str(),
                    string: string.as_deref(),
                };
                let errors: Vec<ErrorDescription> = match &self.by_visit {
                    Visits::Scan(visits) => visits
                        .iter()
                        .zip(&self.errors)
                        .filter(|(reported, _)| **reported == visit)
                        .map(|(_, error)| ErrorDescription::from_validation_error(error))
                        .collect(),
                    Visits::Keyed(by_visit) => by_visit
                        .get(&visit)
                        .map(|found| {
                            found
                                .as_slice()
                                .iter()
                                .map(|&index| {
                                    ErrorDescription::from_validation_error(self.errors[index])
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                if !errors.is_empty() {
                    self.instance_location
                        .set(std::mem::take(instance_location));
                    return errors;
                }
            }
            if let Some(buffer) = instance_location {
                self.instance_location.set(buffer);
            }
            Vec::new()
        }

        // With `#[inline]` alone it stays a call per node in the generated loops.
        #[allow(clippy::inline_always)]
        #[inline(always)]
        fn node(
            &self,
            site: &UnitLocation,
            errors: Vec<ErrorDescription>,
            annotations: Option<Annotations>,
            children: ChildList,
            position: InstancePosition,
        ) -> EvaluationNode {
            let valid = errors.is_empty() && children.all_valid();
            // A failing unit's annotations are dropped.
            let (annotations, dropped_annotations) = if valid {
                (annotations, None)
            } else {
                (None, annotations)
            };
            EvaluationNode {
                keyword_location: self.evaluation_path(site),
                absolute_keyword_location: site.absolute.clone(),
                schema_location: Arc::clone(&site.schema),
                instance_location: position,
                valid,
                annotations,
                dropped_annotations,
                errors,
                first_child: children.first,
                next_sibling: NO_NODE,
            }
        }
    }

    /// The errors an `unevaluated*` subschema's units report, with their evaluation paths.
    pub struct NestedErrors<'n, 'm> {
        errors: Vec<&'n ValidationError<'m>>,
        paths: Vec<Cow<'n, str>>,
    }

    /// Where a reference through the dynamic scope lands.
    pub struct Landing {
        /// The schema's index.
        pub schema: usize,
        /// The resource following the reference adds to the dynamic scope.
        pub pushed: Option<usize>,
    }

    /// The names of the declared properties an object has, as a `properties` annotation.
    #[must_use]
    pub fn names(names: Vec<&'static str>) -> Annotations {
        Annotations::names(names.into_boxed_slice())
    }

    /// An annotation of `values`.
    #[must_use]
    pub fn values(values: Vec<Value>) -> Annotations {
        Annotations::new(Value::Array(values))
    }

    /// The positions of the items `contains` matched, as its annotation.
    #[must_use]
    pub fn indices(indices: Vec<usize>) -> Annotations {
        Annotations::new(Value::from(indices))
    }

    /// The error of an array with `count` items matching `contains`, if that count fails its
    /// `minContains` and `maxContains`.
    #[must_use]
    pub fn contains_errors<F: Json>(
        count: usize,
        min: Option<u64>,
        max: Option<u64>,
        instance: &F::Node<'_>,
    ) -> Vec<ErrorDescription> {
        ContainsBounds { min, max }
            .error::<F>(count, instance)
            .into_iter()
            .collect()
    }

    /// The annotation `value`.
    #[must_use]
    pub fn boolean(value: bool) -> Annotations {
        Annotations::boolean(value)
    }

    /// The annotation of an array-form `items` or `prefixItems` whose `children` are the items of
    /// an array of `len` with a subschema: `true` when every item had one, otherwise the largest
    /// position.
    #[must_use]
    pub fn tuple(children: ChildList, len: usize) -> Annotations {
        let applied = children.len();
        Annotations::new(if applied == len {
            Value::Bool(true)
        } else {
            Value::from(applied - 1)
        })
    }

    /// Whether `properties` checks the declared names of an object of `members` in declared order:
    /// the runtime looks a few declared names up on a wide object.
    #[inline]
    #[must_use]
    pub fn in_declared_order<F: Json>(members: usize, declared: usize) -> bool {
        declared < HASHMAP_THRESHOLD && !scan_members::<F>(members, declared)
    }

    /// Whether the branch at `branch` of a discriminated `anyOf` or `oneOf` may accept the
    /// instance, given the `candidates` its discriminator value leaves.
    #[inline]
    #[must_use]
    pub fn candidate(candidates: Option<&[usize]>, branch: usize) -> bool {
        candidates.is_none_or(|candidates| candidates.binary_search(&branch).is_ok())
    }

    /// Whether several branches accept the instance.
    #[inline]
    #[must_use]
    pub fn several(accepts: &[bool]) -> bool {
        accepts.iter().filter(|accepts| **accepts).nth(1).is_some()
    }

    /// Orders the `(declared index, value)` matches of `properties` by declared index.
    pub fn sort_declared<T>(matched: &mut [(usize, T)]) {
        matched.sort_unstable_by_key(|(index, _)| *index);
    }

    /// The error of a `oneOf` with several branches accepting `instance`.
    #[must_use]
    pub fn one_of_multiple_valid<F: Json>(instance: &F::Node<'_>) -> Vec<ErrorDescription> {
        vec![ErrorDescription::from_validation_error(
            &ValidationError::one_of_multiple_valid(
                Location::new(),
                Location::new(),
                Location::new(),
                instance.lazy_value(),
                Vec::new(),
            ),
        )]
    }

    /// The errors of a leaf unit for `instance`, from its keyword's errors code `check`.
    pub fn leaf_errors<N>(
        check: fn(N, &LazyLocation, &mut Descriptions) -> bool,
        instance: N,
    ) -> Vec<ErrorDescription> {
        let mut errors = Descriptions::default();
        let _ = check(instance, &LazyLocation::new(), &mut errors);
        errors.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use test_case::test_case;

    fn loc() -> Location {
        Location::new()
    }

    fn annotation(value: serde_json::Value) -> Annotations {
        Annotations::new(value)
    }

    impl ErrorDescription {
        fn from_string(s: &str) -> Self {
            ErrorDescription {
                keyword: "error".into(),
                message: s.to_string(),
            }
        }
    }

    fn leaf_with_annotation(schema: &str, ann: serde_json::Value) -> EvaluationNode {
        EvaluationNode::valid(
            loc(),
            None,
            schema.to_string(),
            InstancePosition::ROOT,
            Some(annotation(ann)),
            ChildList::default(),
        )
    }

    fn leaf_with_error(schema: &str, msg: &str) -> EvaluationNode {
        EvaluationNode::invalid(
            loc(),
            None,
            schema.to_string(),
            InstancePosition::ROOT,
            None,
            vec![ErrorDescription::from_string(msg)],
            ChildList::default(),
        )
    }

    #[test]
    fn iter_annotations_visits_all_nodes() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_annotation("/child", json!({"k": "v"}));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"root": true}))),
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let entries: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].schema_location, "/root");
        assert_eq!(entries[1].schema_location, "/child");
    }

    #[test]
    fn iter_errors_visits_all_nodes() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_error("/child", "boom");
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            vec![ErrorDescription::from_string("root error")],
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let entries: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].error.to_string(), "root error");
        assert_eq!(entries[1].error.to_string(), "boom");
    }

    #[test]
    fn flag_output_valid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        assert!(flag.valid);
    }

    #[test]
    fn flag_output_invalid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            vec![ErrorDescription::from_string("error")],
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        assert!(!flag.valid);
    }

    #[test]
    fn flag_output_serialization() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        let serialized = serde_json::to_value(flag).expect("serialization succeeds");
        assert_eq!(serialized, json!({"valid": true}));
    }

    #[test]
    fn list_output_serialization_valid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "#".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = evaluation.list();
        let serialized = serde_json::to_value(list).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "#",
                        "instanceLocation": ""
                    }
                ]
            })
        );
    }

    #[test]
    fn list_output_serialization_with_children() {
        let mut arena = EvaluationArena::default();
        let child1 = leaf_with_annotation("/child1", json!({"key": "value"}));
        let child2 = leaf_with_error("/child2", "child error");
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"root": true}))),
            ChildList::from_nodes(&mut arena, vec![child1, child2]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = evaluation.list();
        let serialized = serde_json::to_value(list).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/root",
                        "instanceLocation": "",
                        "annotations": {"root": true}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/child1",
                        "instanceLocation": "",
                        "annotations": {"key": "value"}
                    },
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/child2",
                        "instanceLocation": "",
                        "errors": {"error": "child error"}
                    }
                ]
            })
        );
    }

    #[test]
    fn hierarchical_output_serialization() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_annotation("/child", json!({"nested": "data"}));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"root": "annotation"}))),
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let hierarchical = evaluation.hierarchical();
        let serialized = serde_json::to_value(hierarchical).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "evaluationPath": "",
                "schemaLocation": "/root",
                "instanceLocation": "",
                "annotations": {"root": "annotation"},
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/child",
                        "instanceLocation": "",
                        "annotations": {"nested": "data"}
                    }
                ]
            })
        );
    }

    #[test]
    fn outputs_include_errors_and_dropped_annotations() {
        let mut arena = EvaluationArena::default();
        let second_item =
            arena.position_below(InstancePosition::ROOT, &JsonPointerSegment::Index(1));
        let invalid_child = EvaluationNode::invalid(
            loc(),
            None,
            "/items/type".to_string(),
            second_item,
            None,
            vec![ErrorDescription::from_string("child error")],
            ChildList::default(),
        );
        let prefix_child = leaf_with_annotation("/prefix", json!(0));
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"dropped": true}))),
            vec![ErrorDescription::from_string("root failure")],
            ChildList::from_nodes(&mut arena, vec![invalid_child, prefix_child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = serde_json::to_value(evaluation.list()).expect("serialization succeeds");
        assert_eq!(
            list,
            json!({
                "valid": false,
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/root",
                        "instanceLocation": "",
                        "droppedAnnotations": {"dropped": true},
                        "errors": {"error": "root failure"}
                    },
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/items/type",
                        "instanceLocation": "/1",
                        "errors": {"error": "child error"}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/prefix",
                        "instanceLocation": "",
                        "annotations": 0
                    }
                ]
            })
        );
        let hierarchical =
            serde_json::to_value(evaluation.hierarchical()).expect("serialization succeeds");
        assert_eq!(
            hierarchical,
            json!({
                "valid": false,
                "evaluationPath": "",
                "schemaLocation": "/root",
                "instanceLocation": "",
                "droppedAnnotations": {"dropped": true},
                "errors": {"error": "root failure"},
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/items/type",
                        "instanceLocation": "/1",
                        "errors": {"error": "child error"}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/prefix",
                        "instanceLocation": "",
                        "annotations": 0
                    }
                ]
            })
        );
    }

    #[test]
    fn empty_evaluation_tree() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);

        assert_eq!(evaluation.iter_annotations().count(), 0);
        assert_eq!(evaluation.iter_errors().count(), 0);

        let flag = evaluation.flag();
        assert!(flag.valid);
    }

    #[test]
    fn deep_nesting() {
        let mut arena = EvaluationArena::default();
        // Create a deeply nested tree: root -> level1 -> level2 -> level3
        let level3 = leaf_with_annotation("/level3", json!({"level": 3}));
        let level2 = EvaluationNode::valid(
            loc(),
            None,
            "/level2".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"level": 2}))),
            ChildList::from_nodes(&mut arena, vec![level3]),
        );
        let level1 = EvaluationNode::valid(
            loc(),
            None,
            "/level1".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"level": 1}))),
            ChildList::from_nodes(&mut arena, vec![level2]),
        );
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"level": 0}))),
            ChildList::from_nodes(&mut arena, vec![level1]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 4);

        // Check depth-first order
        assert_eq!(annotations[0].schema_location, "/root");
        assert_eq!(annotations[1].schema_location, "/level1");
        assert_eq!(annotations[2].schema_location, "/level2");
        assert_eq!(annotations[3].schema_location, "/level3");
    }

    #[test]
    fn wide_tree() {
        let mut arena = EvaluationArena::default();
        let children = ChildList::from_nodes(
            &mut arena,
            (0..10).map(|i| leaf_with_annotation(&format!("/child{i}"), json!({"index": i}))),
        );

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"root": true}))),
            children,
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 11); // root + 10 children
    }

    #[test]
    fn multiple_errors_per_node() {
        let arena = EvaluationArena::default();
        let errors = vec![
            ErrorDescription::from_string("error 1"),
            ErrorDescription::from_string("error 2"),
            ErrorDescription::from_string("error 3"),
        ];
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            errors,
            ChildList::default(),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let error_entries: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(error_entries.len(), 3);
        assert_eq!(error_entries[0].error.to_string(), "error 1");
        assert_eq!(error_entries[1].error.to_string(), "error 2");
        assert_eq!(error_entries[2].error.to_string(), "error 3");
    }

    #[test]
    fn mixed_valid_and_invalid_nodes() {
        let mut arena = EvaluationArena::default();
        let valid_child = leaf_with_annotation("/valid", json!({"ok": true}));
        let invalid_child = leaf_with_error("/invalid", "failed");

        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"attempted": true}))),
            vec![ErrorDescription::from_string("root failed")],
            ChildList::from_nodes(&mut arena, vec![valid_child, invalid_child]),
        );

        let evaluation = Evaluation::with_root(arena, root);

        // The failing root drops the annotations of its valid child too
        assert_eq!(evaluation.iter_annotations().count(), 0);

        // Should have 2 errors (root + invalid child)
        let errors: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn annotations_iterator_skips_nodes_without_annotations() {
        let mut arena = EvaluationArena::default();
        let no_annotation = EvaluationNode::valid(
            loc(),
            None,
            "/no_ann".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::default(),
        );
        let with_annotation = leaf_with_annotation("/with_ann", json!({"present": true}));

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::from_nodes(&mut arena, vec![no_annotation, with_annotation]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].schema_location, "/with_ann");
    }

    #[test]
    fn errors_iterator_skips_nodes_without_errors() {
        let mut arena = EvaluationArena::default();
        let no_error = EvaluationNode::valid(
            loc(),
            None,
            "/no_error".to_string(),
            InstancePosition::ROOT,
            Some(annotation(json!({"ok": true}))),
            ChildList::default(),
        );
        let with_error = leaf_with_error("/with_error", "failed");

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            None,
            ChildList::from_nodes(&mut arena, vec![no_error, with_error]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let errors: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].schema_location, "/with_error");
    }

    #[test]
    fn error_entries_serialization_empty() {
        let entries = ErrorEntriesSerializer(&[]);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert!(serialized.is_object());
        assert_eq!(serialized.as_object().unwrap().len(), 0);
    }

    #[test]
    fn error_entries_serialization_single() {
        let errors = vec![ErrorDescription::from_string("test error")];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert!(serialized.is_object());
        assert_eq!(serialized.as_object().unwrap().len(), 1);
        assert!(serialized.get("error").is_some());
    }

    #[test]
    fn error_entries_serialization_multiple() {
        let errors = vec![
            ErrorDescription::new("alpha", "error 1".to_string()),
            ErrorDescription::new("beta", "error 2".to_string()),
            ErrorDescription::new("gamma", "error 3".to_string()),
        ];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert_eq!(serialized.as_object().unwrap().len(), 3);
        assert!(serialized.get("alpha").is_some());
        assert!(serialized.get("beta").is_some());
        assert!(serialized.get("gamma").is_some());
    }

    #[test]
    fn error_entries_serialization_preserves_duplicates() {
        let errors = vec![
            ErrorDescription::new("required", "\"foo\" is required".to_string()),
            ErrorDescription::new("required", "\"bar\" is required".to_string()),
        ];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        let value = serialized
            .get("required")
            .expect("required keyword present")
            .as_array()
            .expect("multiple errors serialized as array");
        assert_eq!(value.len(), 2);
        assert_eq!(value[0], "\"foo\" is required");
        assert_eq!(value[1], "\"bar\" is required");
    }

    #[test]
    fn list_output_preserves_multiple_errors_per_keyword() {
        let arena = EvaluationArena::default();
        let errors = vec![
            ErrorDescription::new("required", "\"foo\" is required".to_string()),
            ErrorDescription::new("required", "\"bar\" is required".to_string()),
        ];
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/required".to_string(),
            InstancePosition::ROOT,
            None,
            errors,
            ChildList::default(),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let list = serde_json::to_value(evaluation.list()).expect("serialization succeeds");
        let root_unit = list
            .get("details")
            .and_then(|value| value.as_array())
            .and_then(|details| details.first())
            .expect("list output contains root unit");
        let errors = root_unit
            .get("errors")
            .and_then(|errors| errors.get("required"))
            .and_then(|value| value.as_array())
            .expect("errors serialized as array");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "\"foo\" is required");
        assert_eq!(errors[1], "\"bar\" is required");
    }

    #[test]
    fn format_keyword_location_without_absolute() {
        let location = Location::new().join("properties").join("name");
        let formatted = format_keyword_location(&location, None);
        assert_eq!(formatted.as_ref(), "/properties/name");
    }

    #[test]
    fn format_keyword_location_keeps_encoded_fragment() {
        let location = Location::new().join("properties").join("s p");
        let uri = Arc::new(
            Uri::parse("http://example.com/schema.json#/properties/s%20p")
                .unwrap()
                .to_owned(),
        );
        let formatted = format_keyword_location(&location, Some(&uri));
        assert_eq!(
            formatted.as_ref(),
            "http://example.com/schema.json#/properties/s%20p"
        );
    }

    #[test]
    fn dropped_annotations_on_invalid_node() {
        let annotations = Some(annotation(json!({"dropped": true})));
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            annotations.clone(),
            vec![ErrorDescription::from_string("failed")],
            ChildList::default(),
        );

        assert!(!root.valid);
        assert!(root.annotations.is_none());
        assert!(root.dropped_annotations.is_some());
        assert_eq!(
            root.dropped_annotations.as_ref().unwrap(),
            annotations.as_ref().unwrap()
        );
    }

    #[test]
    fn valid_node_has_no_dropped_annotations() {
        let annotations = Some(annotation(json!({"kept": true})));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            InstancePosition::ROOT,
            annotations.clone(),
            ChildList::default(),
        );

        assert!(root.valid);
        assert!(root.annotations.is_some());
        assert!(root.dropped_annotations.is_none());
        assert_eq!(
            root.annotations.as_ref().unwrap(),
            annotations.as_ref().unwrap()
        );
    }

    #[test]
    fn absolute_keyword_location_populated_with_id() {
        use serde_json::json;

        // Schema with $id should populate absoluteKeywordLocation
        let schema = json!({
            "$id": "https://example.com/schema",
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });

        let validator = crate::validator_for(&schema).expect("schema compiles");
        let evaluation = validator.evaluate(&json!({"name": "test"}));

        // Verify that absoluteKeywordLocation is populated for nodes
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert!(!annotations.is_empty());

        // At least one annotation should have an absolute keyword location
        let with_absolute = annotations
            .iter()
            .filter(|a| a.absolute_keyword_location.is_some())
            .count();

        assert!(with_absolute > 0);

        // Verify the absolute locations start with the schema's $id
        for annotation in annotations
            .iter()
            .filter(|a| a.absolute_keyword_location.is_some())
        {
            let uri_str = annotation.absolute_keyword_location.unwrap().as_str();
            assert!(uri_str.starts_with("https://example.com/schema"));
        }
    }

    #[test]
    fn annotations_value_returns_reference() {
        let expected = json!({"key": "value"});
        let annotations = Annotations::new(expected.clone());

        assert_eq!(annotations.value(), &expected);
    }

    #[test]
    fn annotations_into_inner_consumes_and_returns_value() {
        let expected = json!({"key": "value", "nested": {"array": [1, 2, 3]}});
        let annotations = Annotations::new(expected.clone());

        let inner = annotations.into_inner();
        assert_eq!(inner, expected);
    }

    #[test]
    fn error_description_into_inner_consumes_and_returns_message() {
        let expected_message = "test error message";
        let error = ErrorDescription::from_string(expected_message);

        let message = error.into_inner();
        assert_eq!(message, expected_message);
    }

    #[test_case(json!(42), true)]
    #[test_case(json!("not a number"), false)]
    #[allow(clippy::needless_pass_by_value)]
    fn test_evaluation_is_valid(instance: Value, expected: bool) {
        let validator = crate::validator_for(&json!({"type": "number"})).expect("valid schema");
        assert_eq!(validator.evaluate(&instance).is_valid(), expected);
    }

    // A failing schema produces no annotations, including those of its subschemas
    #[test_case(
        json!({"properties": {"a": {"title": "x"}}, "required": ["b"]}),
        json!({"a": 1}),
        vec![];
        "invalid root"
    )]
    #[test_case(
        json!({"properties": {"obj": {"properties": {"a": {"title": "x"}}, "required": ["b"]}}}),
        json!({"obj": {"a": 1}}),
        vec![];
        "invalid nested object"
    )]
    #[test_case(
        json!({"properties": {"a": {"title": "x"}}, "required": ["a"]}),
        json!({"a": 1}),
        vec![
            ("/properties", "", json!(["a"])),
            ("/properties/a", "/a", json!({"title": "x"})),
        ];
        "valid root"
    )]
    #[test_case(
        json!({"anyOf": [{"title": "t0", "required": ["b"]}, {"title": "t1"}]}),
        json!({"a": 1}),
        vec![("/anyOf/1", "", json!({"title": "t1"}))];
        "valid anyOf branch"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn test_iter_annotations(schema: Value, instance: Value, expected: Vec<(&str, &str, Value)>) {
        let validator = crate::validator_for(&schema).expect("valid schema");
        let evaluation = validator.evaluate(&instance);
        let annotations: Vec<_> = evaluation
            .iter_annotations()
            .map(|entry| {
                (
                    entry.schema_location,
                    entry.instance_location.as_str(),
                    entry.annotations.value().clone(),
                )
            })
            .collect();
        assert_eq!(annotations, expected);
    }

    #[test]
    fn test_error_entry_display() {
        let schema = json!({
            "type": "object",
            "properties": {"age": {"type": "number"}},
            "required": ["name"]
        });
        let validator = crate::validator_for(&schema).expect("valid schema");
        let evaluation = validator.evaluate(&json!({"age": "oops"}));
        let rendered: Vec<String> = evaluation.iter_errors().map(|e| e.to_string()).collect();
        assert_eq!(
            rendered,
            vec![
                "\"name\" is a required property".to_string(),
                "/age: \"oops\" is not of type \"number\"".to_string(),
            ]
        );
    }

    fn node(evaluation_path: &str, schema_location: &str) -> Value {
        json!({
            "valid": true,
            "evaluationPath": evaluation_path,
            "schemaLocation": schema_location,
            "instanceLocation": ""
        })
    }

    fn with_details(mut node: Value, details: Vec<Value>) -> Value {
        node["details"] = Value::Array(details);
        node
    }

    fn outputs(schema: &Value) -> (Value, Value, usize) {
        let validator = crate::validator_for(schema).expect("valid schema");
        let evaluation = validator.evaluate(&json!("x"));
        (
            serde_json::to_value(evaluation.list()).expect("serialization succeeds"),
            serde_json::to_value(evaluation.hierarchical()).expect("serialization succeeds"),
            evaluation.iter_annotations().count(),
        )
    }

    // Drafts 4-7 ignore every `$ref` sibling, annotations included
    #[test_case("http://json-schema.org/draft-04/schema#"; "draft 4")]
    #[test_case("http://json-schema.org/draft-06/schema#"; "draft 6")]
    #[test_case("http://json-schema.org/draft-07/schema#"; "draft 7")]
    fn ref_siblings_produce_no_annotations(dialect: &str) {
        let schema = json!({
            "$schema": dialect,
            "definitions": {"a": {"type": "string"}},
            "$ref": "#/definitions/a",
            "type": "integer",
            "title": "T",
            "x-unknown": 1
        });
        let root = node("", "");
        let reference = node("/$ref", "/definitions/a");
        let target_type = node("/$ref/type", "/definitions/a/type");
        assert_eq!(
            outputs(&schema),
            (
                json!({"valid": true, "details": [root, reference, target_type]}),
                with_details(root, vec![with_details(reference, vec![target_type])]),
                0
            )
        );
    }

    #[test]
    fn draft7_unknown_keywords_without_ref_are_annotations() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "definitions": {"a": {"type": "string"}},
            "type": "string",
            "title": "T",
            "x-unknown": 1
        });
        let mut root = node("", "");
        root["annotations"] = json!({"title": "T", "x-unknown": 1});
        let target_type = node("/type", "/type");
        assert_eq!(
            outputs(&schema),
            (
                json!({"valid": true, "details": [root, target_type]}),
                with_details(root, vec![target_type]),
                1
            )
        );
    }

    // Draft 4 and 6 do not define the content keywords
    #[test_case("http://json-schema.org/draft-04/schema#"; "draft 4")]
    #[test_case("http://json-schema.org/draft-06/schema#"; "draft 6")]
    fn undefined_content_keywords_are_annotations(dialect: &str) {
        let schema = json!({
            "$schema": dialect,
            "contentEncoding": "base64",
            "contentMediaType": "application/json"
        });
        let mut root = node("", "");
        root["annotations"] =
            json!({"contentEncoding": "base64", "contentMediaType": "application/json"});
        assert_eq!(
            outputs(&schema),
            (json!({"valid": true, "details": [root]}), root, 1)
        );
    }

    #[test]
    fn draft2020_ref_siblings_apply() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"a": {"type": "string"}},
            "$ref": "#/$defs/a",
            "maxLength": 5,
            "title": "T",
            "x-unknown": 1
        });
        let mut root = node("", "");
        root["annotations"] = json!({"title": "T", "x-unknown": 1});
        let max_length = node("/maxLength", "/maxLength");
        let reference = node("/$ref", "/$defs/a");
        let target_type = node("/$ref/type", "/$defs/a/type");
        assert_eq!(
            outputs(&schema),
            (
                json!({"valid": true, "details": [root, max_length, reference, target_type]}),
                with_details(
                    root,
                    vec![max_length, with_details(reference, vec![target_type])]
                ),
                1
            )
        );
    }

    // One line per unit: `[valid, evaluationPath, schemaLocation, instanceLocation]`, then any other
    // members, indented by depth in the hierarchical output.
    fn unit_line(unit: &Value, depth: usize) -> String {
        let mut members = unit.as_object().expect("a unit is an object").clone();
        let mut line = vec![
            members.remove("valid").expect("every unit has `valid`"),
            members
                .remove("evaluationPath")
                .expect("every unit has `evaluationPath`"),
            members
                .remove("schemaLocation")
                .expect("every unit has `schemaLocation`"),
            members
                .remove("instanceLocation")
                .expect("every unit has `instanceLocation`"),
        ];
        if !members.is_empty() {
            line.push(Value::Object(members));
        }
        format!("{}{}", "  ".repeat(depth), Value::Array(line))
    }

    fn hierarchical_lines(unit: &Value, depth: usize, out: &mut Vec<String>) {
        let mut unit = unit.clone();
        let details = unit
            .as_object_mut()
            .expect("a unit is an object")
            .remove("details");
        out.push(unit_line(&unit, depth));
        for child in details
            .iter()
            .flat_map(|details| details.as_array().expect("`details` is an array"))
        {
            hierarchical_lines(child, depth + 1, out);
        }
    }

    // The hierarchical output, read through `OutputUnit` accessors.
    fn walk(unit: OutputUnit<'_>) -> Value {
        let mut members = serde_json::Map::new();
        members.insert("valid".into(), Value::Bool(unit.valid()));
        members.insert(
            "evaluationPath".into(),
            json!(unit.evaluation_path().as_str()),
        );
        members.insert("schemaLocation".into(), json!(unit.schema_location()));
        members.insert(
            "instanceLocation".into(),
            json!(unit.instance_location().as_str()),
        );
        if let Some(annotations) = unit.annotations() {
            members.insert("annotations".into(), annotations.value().clone());
        }
        if let Some(annotations) = unit.dropped_annotations() {
            members.insert("droppedAnnotations".into(), annotations.value().clone());
        }
        if !unit.errors().is_empty() {
            let mut errors = serde_json::Map::new();
            for error in unit.errors() {
                match errors.get_mut(error.keyword()) {
                    None => {
                        errors.insert(error.keyword().into(), json!(error.message()));
                    }
                    Some(Value::Array(messages)) => messages.push(json!(error.message())),
                    Some(first) => {
                        let message = first.take();
                        *first = json!([message, error.message()]);
                    }
                }
            }
            members.insert("errors".into(), Value::Object(errors));
        }
        let details: Vec<Value> = unit.children().map(walk).collect();
        if !details.is_empty() {
            members.insert("details".into(), Value::Array(details));
        }
        Value::Object(members)
    }

    #[test_case(
        json!({"items":{"type":"number"}}),
        json!([1,2.5,3]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/type","/items/type","/0"]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"    [true,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
        ],
        &[];
        "valid leaf items"
    )]
    #[test_case(
        json!({"items":{"type":"number"}}),
        json!([1,"x",3,4]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/type","/items/type","/0"]"#,
            r#"    [false,"/items","/items","/1"]"#,
            r#"      [false,"/items/type","/items/type","/1",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"    [true,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
            r#"    [true,"/items","/items","/3"]"#,
            r#"      [true,"/items/type","/items/type","/3"]"#,
        ],
        &[],
        &[
            r#"["/items/type",null,"/1","\"x\" is not of type \"number\""]"#,
        ];
        "failing item splits the run"
    )]
    #[test_case(
        json!({"items":{"type":"number"}}),
        json!(["x",1,2,"y"]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [false,"/items","/items","/0"]"#,
            r#"      [false,"/items/type","/items/type","/0",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"    [true,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
            r#"    [false,"/items","/items","/3"]"#,
            r#"      [false,"/items/type","/items/type","/3",{"errors":{"type":"\"y\" is not of type \"number\""}}]"#,
        ],
        &[],
        &[
            r#"["/items/type",null,"/0","\"x\" is not of type \"number\""]"#,
            r#"["/items/type",null,"/3","\"y\" is not of type \"number\""]"#,
        ];
        "failing first and last items"
    )]
    #[test_case(
        json!({"items":{"type":"number"}}),
        json!([]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":false}]"#,
        ],
        &[
            r#"["/items",null,"",false]"#,
        ],
        &[];
        "empty array"
    )]
    #[test_case(
        json!({"items":{"type":"integer"},"prefixItems":[{"type":"string"}]}),
        json!(["a",1,2]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"    [true,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
            r#"  [true,"/prefixItems","/prefixItems","",{"annotations":0}]"#,
            r#"    [true,"/prefixItems/0","/prefixItems/0","/0"]"#,
            r#"      [true,"/prefixItems/0/type","/prefixItems/0/type","/0"]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
            r#"["/prefixItems",null,"",0]"#,
        ],
        &[];
        "prefix items then rest"
    )]
    #[test_case(
        json!({"items":{"type":"integer"},"prefixItems":[{"type":"string"}]}),
        json!(["a",1,"b",3]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"    [false,"/items","/items","/2"]"#,
            r#"      [false,"/items/type","/items/type","/2",{"errors":{"type":"\"b\" is not of type \"integer\""}}]"#,
            r#"    [true,"/items","/items","/3"]"#,
            r#"      [true,"/items/type","/items/type","/3"]"#,
            r#"  [true,"/prefixItems","/prefixItems","",{"annotations":0}]"#,
            r#"    [true,"/prefixItems/0","/prefixItems/0","/0"]"#,
            r#"      [true,"/prefixItems/0/type","/prefixItems/0/type","/0"]"#,
        ],
        &[],
        &[
            r#"["/items/type",null,"/2","\"b\" is not of type \"integer\""]"#,
        ];
        "prefix items then failing rest"
    )]
    #[test_case(
        json!({"$schema":"http://json-schema.org/draft-07/schema#","additionalItems":{"type":"string"},"items":[{"type":"integer"}]}),
        json!([1,"a","b",2,"c"]),
        &[
            r#"[false,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":0}]"#,
            r#"    [true,"/items/0","/items/0","/0"]"#,
            r#"      [true,"/items/0/type","/items/0/type","/0"]"#,
            r#"  [false,"/additionalItems","/additionalItems","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/additionalItems","/additionalItems","/1"]"#,
            r#"      [true,"/additionalItems/type","/additionalItems/type","/1"]"#,
            r#"    [true,"/additionalItems","/additionalItems","/2"]"#,
            r#"      [true,"/additionalItems/type","/additionalItems/type","/2"]"#,
            r#"    [false,"/additionalItems","/additionalItems","/3"]"#,
            r#"      [false,"/additionalItems/type","/additionalItems/type","/3",{"errors":{"type":"2 is not of type \"string\""}}]"#,
            r#"    [true,"/additionalItems","/additionalItems","/4"]"#,
            r#"      [true,"/additionalItems/type","/additionalItems/type","/4"]"#,
        ],
        &[],
        &[
            r#"["/additionalItems/type",null,"/3","2 is not of type \"string\""]"#,
        ];
        "additional items"
    )]
    #[test_case(
        json!({"items":{"items":{"type":"number"},"type":"array"}}),
        json!([[1,2],[],[3,"x",4]]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/type","/items/type","/0"]"#,
            r#"      [true,"/items/items","/items/items","/0",{"annotations":true}]"#,
            r#"        [true,"/items/items","/items/items","/0/0"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/0/0"]"#,
            r#"        [true,"/items/items","/items/items","/0/1"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/0/1"]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"      [true,"/items/items","/items/items","/1",{"annotations":false}]"#,
            r#"    [false,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
            r#"      [false,"/items/items","/items/items","/2",{"droppedAnnotations":true}]"#,
            r#"        [true,"/items/items","/items/items","/2/0"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/2/0"]"#,
            r#"        [false,"/items/items","/items/items","/2/1"]"#,
            r#"          [false,"/items/items/type","/items/items/type","/2/1",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"        [true,"/items/items","/items/items","/2/2"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/2/2"]"#,
        ],
        &[],
        &[
            r#"["/items/items/type",null,"/2/1","\"x\" is not of type \"number\""]"#,
        ];
        "nested arrays"
    )]
    #[test_case(
        json!({"items":{"items":{"type":"number"},"minItems":2,"type":"array"}}),
        json!([[1,2],[3,4,5],[6,"x"]]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/type","/items/type","/0"]"#,
            r#"      [true,"/items/minItems","/items/minItems","/0"]"#,
            r#"      [true,"/items/items","/items/items","/0",{"annotations":true}]"#,
            r#"        [true,"/items/items","/items/items","/0/0"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/0/0"]"#,
            r#"        [true,"/items/items","/items/items","/0/1"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/0/1"]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
            r#"      [true,"/items/minItems","/items/minItems","/1"]"#,
            r#"      [true,"/items/items","/items/items","/1",{"annotations":true}]"#,
            r#"        [true,"/items/items","/items/items","/1/0"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/1/0"]"#,
            r#"        [true,"/items/items","/items/items","/1/1"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/1/1"]"#,
            r#"        [true,"/items/items","/items/items","/1/2"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/1/2"]"#,
            r#"    [false,"/items","/items","/2"]"#,
            r#"      [true,"/items/type","/items/type","/2"]"#,
            r#"      [true,"/items/minItems","/items/minItems","/2"]"#,
            r#"      [false,"/items/items","/items/items","/2",{"droppedAnnotations":true}]"#,
            r#"        [true,"/items/items","/items/items","/2/0"]"#,
            r#"          [true,"/items/items/type","/items/items/type","/2/0"]"#,
            r#"        [false,"/items/items","/items/items","/2/1"]"#,
            r#"          [false,"/items/items/type","/items/items/type","/2/1",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
        ],
        &[],
        &[
            r#"["/items/items/type",null,"/2/1","\"x\" is not of type \"number\""]"#,
        ];
        "nested array shapes"
    )]
    #[test_case(
        json!({"items":{"maximum":5,"minimum":1}}),
        json!([1,2,7,3]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/minimum","/items/minimum","/0"]"#,
            r#"      [true,"/items/maximum","/items/maximum","/0"]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/minimum","/items/minimum","/1"]"#,
            r#"      [true,"/items/maximum","/items/maximum","/1"]"#,
            r#"    [false,"/items","/items","/2"]"#,
            r#"      [true,"/items/minimum","/items/minimum","/2"]"#,
            r#"      [false,"/items/maximum","/items/maximum","/2",{"errors":{"maximum":"7 is greater than the maximum of 5"}}]"#,
            r#"    [true,"/items","/items","/3"]"#,
            r#"      [true,"/items/minimum","/items/minimum","/3"]"#,
            r#"      [true,"/items/maximum","/items/maximum","/3"]"#,
        ],
        &[],
        &[
            r#"["/items/maximum",null,"/2","7 is greater than the maximum of 5"]"#,
        ];
        "leaf with several keywords"
    )]
    #[test_case(
        json!({"items":{}}),
        json!([1,"a"]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"    [true,"/items","/items","/1"]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
        ],
        &[];
        "empty subschema"
    )]
    #[test_case(
        json!({"items":true}),
        json!([1,"a"]),
        &[
            r#"[true,"","",""]"#,
        ],
        &[],
        &[];
        "true subschema"
    )]
    #[test_case(
        json!({"items":{"title":"t","type":"string"}}),
        json!(["a","b"]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/0",{"annotations":{"title":"t"}}]"#,
            r#"      [true,"/items/type","/items/type","/0"]"#,
            r#"    [true,"/items","/items","/1",{"annotations":{"title":"t"}}]"#,
            r#"      [true,"/items/type","/items/type","/1"]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
            r#"["/items",null,"/0",{"title":"t"}]"#,
            r#"["/items",null,"/1",{"title":"t"}]"#,
        ],
        &[];
        "subschema annotations"
    )]
    #[test_case(
        json!({"items":{"format":"email"}}),
        json!(["a@b.c","x"]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/format","/items/format","/0",{"annotations":"email"}]"#,
            r#"    [true,"/items","/items","/1"]"#,
            r#"      [true,"/items/format","/items/format","/1",{"annotations":"email"}]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
            r#"["/items/format",null,"/0","email"]"#,
            r#"["/items/format",null,"/1","email"]"#,
        ],
        &[];
        "format annotations"
    )]
    #[test_case(
        json!({"items":{"minimum":1,"x-unknown":1}}),
        json!([1,2]),
        &[
            r#"[true,"","",""]"#,
            r#"  [true,"/items","/items","",{"annotations":true}]"#,
            r#"    [true,"/items","/items","/0",{"annotations":{"x-unknown":1}}]"#,
            r#"      [true,"/items/minimum","/items/minimum","/0"]"#,
            r#"    [true,"/items","/items","/1",{"annotations":{"x-unknown":1}}]"#,
            r#"      [true,"/items/minimum","/items/minimum","/1"]"#,
        ],
        &[
            r#"["/items",null,"",true]"#,
            r#"["/items",null,"/0",{"x-unknown":1}]"#,
            r#"["/items",null,"/1",{"x-unknown":1}]"#,
        ],
        &[];
        "unknown keyword annotations"
    )]
    #[test_case(
        json!({"$defs":{"n":{"type":"number"}},"items":{"$ref":"#/$defs/n"}}),
        json!([1,"x",2]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/items","/items","",{"droppedAnnotations":true}]"#,
            r#"    [true,"/items","/items","/0"]"#,
            r#"      [true,"/items/$ref","/$defs/n","/0"]"#,
            r#"        [true,"/items/$ref/type","/$defs/n/type","/0"]"#,
            r#"    [false,"/items","/items","/1"]"#,
            r#"      [false,"/items/$ref","/$defs/n","/1"]"#,
            r#"        [false,"/items/$ref/type","/$defs/n/type","/1",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"    [true,"/items","/items","/2"]"#,
            r#"      [true,"/items/$ref","/$defs/n","/2"]"#,
            r#"        [true,"/items/$ref/type","/$defs/n/type","/2"]"#,
        ],
        &[],
        &[
            r#"["/$defs/n/type",null,"/1","\"x\" is not of type \"number\""]"#,
        ];
        "reference to a leaf"
    )]
    #[test_case(
        json!({"$defs":{"list":{"items":{"type":"number"}}},"$ref":"#/$defs/list"}),
        json!([1,2,"x",3]),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/$ref","/$defs/list",""]"#,
            r#"    [false,"/$ref/items","/$defs/list/items","",{"droppedAnnotations":true}]"#,
            r#"      [true,"/$ref/items","/$defs/list/items","/0"]"#,
            r#"        [true,"/$ref/items/type","/$defs/list/items/type","/0"]"#,
            r#"      [true,"/$ref/items","/$defs/list/items","/1"]"#,
            r#"        [true,"/$ref/items/type","/$defs/list/items/type","/1"]"#,
            r#"      [false,"/$ref/items","/$defs/list/items","/2"]"#,
            r#"        [false,"/$ref/items/type","/$defs/list/items/type","/2",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"      [true,"/$ref/items","/$defs/list/items","/3"]"#,
            r#"        [true,"/$ref/items/type","/$defs/list/items/type","/3"]"#,
        ],
        &[],
        &[
            r#"["/$defs/list/items/type",null,"/2","\"x\" is not of type \"number\""]"#,
        ];
        "leaf items below a reference"
    )]
    #[test_case(
        json!({"$defs":{"list":{"items":{"type":"number"}}},"properties":{"a":{"$ref":"#/$defs/list"},"b":{"$ref":"#/$defs/list"}}}),
        json!({"a":[1,2],"b":[3,"x",4]}),
        &[
            r#"[false,"","",""]"#,
            r#"  [false,"/properties","/properties","",{"droppedAnnotations":["a","b"]}]"#,
            r#"    [true,"/properties/a","/properties/a","/a"]"#,
            r#"      [true,"/properties/a/$ref","/$defs/list","/a"]"#,
            r#"        [true,"/properties/a/$ref/items","/$defs/list/items","/a",{"annotations":true}]"#,
            r#"          [true,"/properties/a/$ref/items","/$defs/list/items","/a/0"]"#,
            r#"            [true,"/properties/a/$ref/items/type","/$defs/list/items/type","/a/0"]"#,
            r#"          [true,"/properties/a/$ref/items","/$defs/list/items","/a/1"]"#,
            r#"            [true,"/properties/a/$ref/items/type","/$defs/list/items/type","/a/1"]"#,
            r#"    [false,"/properties/b","/properties/b","/b"]"#,
            r#"      [false,"/properties/b/$ref","/$defs/list","/b"]"#,
            r#"        [false,"/properties/b/$ref/items","/$defs/list/items","/b",{"droppedAnnotations":true}]"#,
            r#"          [true,"/properties/b/$ref/items","/$defs/list/items","/b/0"]"#,
            r#"            [true,"/properties/b/$ref/items/type","/$defs/list/items/type","/b/0"]"#,
            r#"          [false,"/properties/b/$ref/items","/$defs/list/items","/b/1"]"#,
            r#"            [false,"/properties/b/$ref/items/type","/$defs/list/items/type","/b/1",{"errors":{"type":"\"x\" is not of type \"number\""}}]"#,
            r#"          [true,"/properties/b/$ref/items","/$defs/list/items","/b/2"]"#,
            r#"            [true,"/properties/b/$ref/items/type","/$defs/list/items/type","/b/2"]"#,
        ],
        &[],
        &[
            r#"["/$defs/list/items/type",null,"/b/1","\"x\" is not of type \"number\""]"#,
        ];
        "one subschema below two references"
    )]
    #[test_case(
        json!({"$id":"https://example.com/root.json","items":{"type":"number"}}),
        json!([1,2]),
        &[
            r#"[true,"","https://example.com/root.json#",""]"#,
            r#"  [true,"/items","https://example.com/root.json#/items","",{"annotations":true}]"#,
            r#"    [true,"/items","https://example.com/root.json#/items","/0"]"#,
            r#"      [true,"/items/type","https://example.com/root.json#/items/type","/0"]"#,
            r#"    [true,"/items","https://example.com/root.json#/items","/1"]"#,
            r#"      [true,"/items/type","https://example.com/root.json#/items/type","/1"]"#,
        ],
        &[
            r#"["https://example.com/root.json#/items","https://example.com/root.json#/items","",true]"#,
        ],
        &[];
        "absolute locations"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn array_item_outputs(
        schema: Value,
        instance: Value,
        units: &[&str],
        annotations: &[&str],
        errors: &[&str],
    ) {
        let validator = crate::validator_for(&schema).expect("valid schema");
        let evaluation = validator.evaluate(&instance);
        let hierarchical =
            serde_json::to_value(evaluation.hierarchical()).expect("serialization succeeds");
        let mut lines = Vec::new();
        hierarchical_lines(&hierarchical, 0, &mut lines);
        assert_eq!(lines, units);
        let list = serde_json::to_value(evaluation.list()).expect("serialization succeeds");
        let list_lines: Vec<String> = list["details"]
            .as_array()
            .expect("`details` is an array")
            .iter()
            .map(|unit| unit_line(unit, 0))
            .collect();
        let flat: Vec<&str> = units.iter().map(|line| line.trim_start()).collect();
        assert_eq!(list_lines, flat);
        assert_eq!(list["valid"], hierarchical["valid"]);
        assert_eq!(Value::Bool(evaluation.flag().valid), hierarchical["valid"]);
        assert_eq!(walk(evaluation.root()), hierarchical);
        let annotation_lines: Vec<String> = evaluation
            .iter_annotations()
            .map(|entry| {
                json!([
                    entry.schema_location,
                    entry.absolute_keyword_location.map(Uri::as_str),
                    entry.instance_location.as_str(),
                    entry.annotations.value()
                ])
                .to_string()
            })
            .collect();
        assert_eq!(annotation_lines, annotations);
        let error_lines: Vec<String> = evaluation
            .iter_errors()
            .map(|entry| {
                json!([
                    entry.schema_location,
                    entry.absolute_keyword_location.map(Uri::as_str),
                    entry.instance_location.as_str(),
                    entry.error.to_string()
                ])
                .to_string()
            })
            .collect();
        assert_eq!(error_lines, errors);
    }
}

#[cfg(test)]
mod public_api {
    use super::{Evaluation, HierarchicalOutput, ListOutput};

    // These are returned from the public API, so dropping a derive breaks downstream `{:?}`.
    #[test]
    fn output_types_are_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<Evaluation>();
        assert_debug::<ListOutput<'_>>();
        assert_debug::<HierarchicalOutput<'_>>();
    }
}
