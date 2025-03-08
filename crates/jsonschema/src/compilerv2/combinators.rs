use serde_json::{Map, Value};

use super::SchemaCompiler;

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    if let Some(Value::Array(value)) = obj.get("anyOf") {
        compile_any_of(compiler, value);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Array(value)) = obj.get("allOf") {
        compile_all_of(compiler, value);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Array(value)) = obj.get("oneOf") {
        compile_one_of(compiler, value);
        jumps.push(compiler.emit_jump_if_invalid());
    }
}

fn compile_any_of(compiler: &mut SchemaCompiler, schemas: &[Value]) {
    assert!(!schemas.is_empty());
    // `anyOf` short-circuits on the first valid subschema, hence emit JUMP_IF_TRUE after each
    // subschema but the last one
    match schemas {
        [schema] => {
            compiler.compile_schema(schema);
        }
        [schemas @ .., last] => {
            let mut targets = Vec::with_capacity(schemas.len());
            for schema in schemas {
                compiler.compile_schema(schema);
                targets.push(compiler.emit_jump_if_valid());
            }
            compiler.compile_schema(last);
            for target in targets {
                compiler.patch_jump_if_valid(target);
            }
        }
        _ => unreachable!(),
    }
}

fn compile_all_of(compiler: &mut SchemaCompiler, schemas: &[Value]) {
    assert!(!schemas.is_empty());
    // `allOf` short-circuits on the first invalid subschema, hence emit JUMP_IF_FALSE after each
    // subschema but the last one
    match schemas {
        [schema] => {
            compiler.compile_schema(schema);
        }
        [schemas @ .., last] => {
            let mut targets = Vec::with_capacity(schemas.len());
            for schema in schemas {
                compiler.compile_schema(schema);
                targets.push(compiler.emit_jump_if_invalid());
            }
            compiler.compile_schema(last);
            for target in targets {
                compiler.patch_jump_if_invalid(target);
            }
        }
        _ => unreachable!(),
    }
}

fn compile_one_of(compiler: &mut SchemaCompiler, schemas: &[Value]) {
    assert!(!schemas.is_empty());
    // `oneOf` short-circuits on the second valid subschema
    compiler.emit_push_one_of();
    let mut targets = Vec::with_capacity(schemas.len());
    match schemas {
        [schema] => {
            compiler.compile_schema(schema);
            compiler.emit_set_one_valid();
        }
        [first, rest @ ..] => {
            compiler.compile_schema(first);
            compiler.emit_set_one_valid();
            for schema in rest {
                compiler.compile_schema(schema);
                targets.push(compiler.emit_jump_if_second_valid());
            }
        }
        [] => unreachable!(),
    };
    compiler.emit_pop_one_of();
    for target in targets {
        compiler.patch_jump_if_second_valid(target);
    }
}

/// A stack for managing "oneOf" state using bit‑packed inline storage in a u128.
///
/// Each oneOf level occupies 2 bits:
/// - 00: Unused (no level present)
/// - 01: Level pushed but no valid subschema seen yet ("empty")
/// - 10: Level has seen one valid subschema ("valid")
#[derive(Debug)]
pub(crate) enum OneOfStack {
    Inline(u128),
    Heap(Vec<u128>),
}

impl OneOfStack {
    /// Create a new, empty `OneOfStack` with inline storage.
    pub(crate) fn new() -> Self {
        OneOfStack::Inline(0)
    }

    /// Push a new `oneOf` level onto the stack.
    /// The new level is initialized to 0b01 ("empty").
    pub(crate) fn push(&mut self) {
        match self {
            OneOfStack::Inline(bits) => {
                let depth = bits.count_ones() as usize;
                if depth >= 64 {
                    todo!("Promote to Heap");
                }
                // Set the 2-bit field at (depth * 2) to 0b01.
                *bits |= 0b01 << (depth * 2);
            }
            OneOfStack::Heap(_) => todo!(),
        }
    }

    /// Mark the current (top) level as valid.
    /// Returns true if the update is successful (first valid encountered),
    /// or false if a second valid is detected (indicating a violation).
    pub(crate) fn mark_valid(&mut self) -> bool {
        match self {
            OneOfStack::Inline(bits) => {
                let depth = bits.count_ones() as usize;
                assert!(depth > 0, "Cannot mark valid: no oneOf level pushed");
                let index = depth - 1;
                let position = index * 2;
                let mask = 0b11 << position;
                let current = ((*bits) & mask) >> position;
                match current {
                    0b01 => {
                        // Update from 0b01 to 0b10.
                        *bits = (*bits & !mask) | (0b10 << position);
                        true
                    }
                    0b10 => {
                        // Second valid encountered: propagate failure immediately.
                        false
                    }
                    _ => unreachable!("Invalid state in oneOf level"),
                }
            }
            OneOfStack::Heap(_) => todo!(),
        }
    }

    /// Pop the current `oneOf` level from the stack.
    pub(crate) fn pop(&mut self) -> bool {
        match self {
            OneOfStack::Inline(bits) => {
                let depth = bits.count_ones() as usize;
                assert!(depth > 0, "Cannot pop: oneOf stack is empty");
                let index = depth - 1;
                let position = index * 2;
                let mask = 0b11 << position;
                let current = ((*bits) & mask) >> position;
                // Clear the 2 bits for this level.
                *bits &= !mask;

                match current {
                    0b01 => false,
                    0b10 => true,
                    _ => unreachable!("Invalid state in oneOf level"),
                }
            }
            OneOfStack::Heap(_) => {
                todo!("Heap variant not implemented yet");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl OneOfStack {
        fn depth(&self) -> usize {
            match self {
                OneOfStack::Inline(bits) => bits.count_ones() as usize,
                OneOfStack::Heap(_) => {
                    todo!("Heap variant not implemented yet")
                }
            }
        }
    }

    #[test]
    fn test_empty_stack() {
        let stack = OneOfStack::new();
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn test_push_and_depth() {
        let mut stack = OneOfStack::new();
        stack.push();
        assert_eq!(stack.depth(), 1);
        stack.push();
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn test_mark_valid_first_time() {
        let mut stack = OneOfStack::new();
        stack.push();
        let result = stack.mark_valid();
        assert_eq!(result, true);
        // Depth remains 1.
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_mark_valid_second_time() {
        let mut stack = OneOfStack::new();
        stack.push();
        assert_eq!(stack.mark_valid(), true);
        // A second valid should return false.
        assert_eq!(stack.mark_valid(), false);
        // Depth remains 1.
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_push_pop_depth() {
        let mut stack = OneOfStack::new();
        stack.push();
        stack.push();
        assert_eq!(stack.depth(), 2);
        stack.pop();
        assert_eq!(stack.depth(), 1);
        stack.pop();
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    #[should_panic(expected = "Cannot pop: oneOf stack is empty")]
    fn test_pop_empty() {
        let mut stack = OneOfStack::new();
        stack.pop();
    }

    #[test]
    #[should_panic(expected = "Cannot mark valid: no oneOf level pushed")]
    fn test_mark_valid_empty() {
        let mut stack = OneOfStack::new();
        stack.mark_valid();
    }
}
