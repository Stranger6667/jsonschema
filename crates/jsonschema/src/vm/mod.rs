use std::slice::Iter;

use serde_json::{map::Values, Value};

use crate::compilerv2::{EvaluationScope, Instruction, SchemaCompiler};

pub struct SchemaEvaluationVM<'a> {
    values: Vec<&'a Value>,
    arrays: Vec<Iter<'a, Value>>,
    object_values: Vec<Values<'a>>,
    scopes: EvaluationScopes,
}

impl Default for SchemaEvaluationVM<'_> {
    fn default() -> Self {
        Self::new()
    }
}

struct EvaluationScopes {
    scopes: Vec<EvaluationScope>,
    is_valid_at_root: bool,
}

impl EvaluationScopes {
    fn new() -> Self {
        Self {
            scopes: Vec::new(),
            is_valid_at_root: true,
        }
    }

    #[inline(always)]
    fn clear(&mut self) {
        self.scopes.clear();
        self.is_valid_at_root = true;
    }

    fn update_top(&mut self, is_valid: bool) {
        match (is_valid, self.scopes.as_mut_slice()) {
            // Valid result transitions
            (true, [.., scope @ EvaluationScope::OrSearching]) => {
                *scope = EvaluationScope::OrSatisfied
            }
            (true, [.., scope @ EvaluationScope::XorEmpty]) => *scope = EvaluationScope::XorSingle,
            (true, [.., scope @ EvaluationScope::XorSingle]) => {
                *scope = EvaluationScope::XorMultiple
            }

            // Invalid result transitions
            (false, [.., scope @ EvaluationScope::AndValid]) => {
                *scope = EvaluationScope::AndInvalid
            }

            // Empty stack case - update root
            (false, []) => {
                self.is_valid_at_root = false; // AND with false = false
            }

            // All other cases don't change state
            _ => {}
        }
    }

    #[inline(always)]
    fn pop(&mut self) -> EvaluationScope {
        self.scopes.pop().expect("Scope stack underflow")
    }

    #[inline(always)]
    fn push(&mut self, scope: EvaluationScope) {
        self.scopes.push(scope);
    }

    #[inline(always)]
    fn is_valid(&self) -> bool {
        self.is_valid_at_root
    }
}

impl<'a> SchemaEvaluationVM<'a> {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            arrays: Vec::new(),
            object_values: Vec::new(),
            scopes: EvaluationScopes::new(),
        }
    }

    #[inline(always)]
    fn reset(&mut self, instance: &'a Value) {
        self.values.clear();
        self.values.push(instance);
        self.object_values.clear();
        self.arrays.clear();
        self.arrays.push([].iter());
        self.scopes.clear();
    }

    fn is_valid(&mut self, instructions: &[Instruction], instance: &'a Value) -> bool {
        self.reset(instance);

        dbg!(instructions);
        dbg!(instructions.len());
        dbg!(std::mem::size_of::<Instruction>());

        let mut ip = 0;
        let mut top = instance;

        let mut top_array = &mut self.arrays[0];

        macro_rules! execute {
            ($variant:ident, $keyword:expr) => {{
                if let Value::$variant(value) = top {
                    self.scopes.update_top($keyword.execute(value));
                }
            }};
            ($keyword:expr) => {{
                self.scopes.update_top($keyword.execute(top));
            }};
        }

        while ip < instructions.len() {
            match &instructions[ip] {
                Instruction::TypeNull => self.scopes.update_top(top.is_null()),
                Instruction::TypeBoolean => self.scopes.update_top(top.is_boolean()),
                Instruction::TypeString => self.scopes.update_top(top.is_string()),
                Instruction::TypeNumber => self.scopes.update_top(top.is_number()),
                Instruction::TypeInteger => self.scopes.update_top(if let Value::Number(n) = top {
                    n.is_i64() || n.is_u64()
                } else {
                    false
                }),
                Instruction::TypeArray => self.scopes.update_top(top.is_array()),
                Instruction::TypeObject => self.scopes.update_top(top.is_object()),
                Instruction::TypeSet(inner) => execute!(inner),
                Instruction::MaxLength(inner) => execute!(String, inner),
                Instruction::MinLength(inner) => execute!(String, inner),
                Instruction::MinMaxLength(inner) => execute!(String, inner),
                Instruction::MaximumU64(inner) => execute!(Number, inner),
                Instruction::MaximumI64(inner) => execute!(Number, inner),
                Instruction::MaximumF64(inner) => execute!(Number, inner),
                Instruction::MinimumU64(inner) => execute!(Number, inner),
                Instruction::MinimumI64(inner) => execute!(Number, inner),
                Instruction::MinimumF64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMaximumU64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMaximumI64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMaximumF64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMinimumU64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMinimumI64(inner) => execute!(Number, inner),
                Instruction::ExclusiveMinimumF64(inner) => execute!(Number, inner),
                Instruction::MinProperties(inner) => execute!(Object, inner),
                Instruction::MaxProperties(inner) => execute!(Object, inner),
                Instruction::Required(inner) => execute!(Object, inner),
                Instruction::MinItems(inner) => execute!(Array, inner),
                Instruction::MaxItems(inner) => execute!(Array, inner),
                Instruction::PushProperty {
                    name,
                    skip_if_missing,
                } => {
                    if let Value::Object(obj) = top {
                        if let Some(value) = obj.get(name.as_ref()) {
                            self.values.push(value);
                            top = value;
                        } else {
                            ip += skip_if_missing;
                        }
                    } else {
                        ip += skip_if_missing;
                    }
                }
                Instruction::PopValue => {
                    self.values.pop().expect("Value stack underflow");
                    top = self.values[self.values.len() - 1];
                }
                Instruction::JumpBackward(offset) => {
                    self.values.pop().expect("Value stack underflow");
                    top = self.values[self.values.len() - 1];
                    ip -= offset;
                }
                Instruction::ArrayIter(offset) => {
                    if let Value::Array(values) = top {
                        let mut values = values.iter();
                        if let Some(first) = values.next() {
                            top = first;
                            self.arrays.push(values);
                            self.values.push(first);
                            let last_idx = self.arrays.len() - 1;
                            top_array = &mut self.arrays[last_idx];
                            ip += 1;
                        } else {
                            ip += offset;
                        }
                    } else {
                        ip += offset;
                    }
                }
                Instruction::ArrayIterNext(offset) => {
                    if let Some(next) = top_array.next() {
                        self.values.push(next);
                        top = next;
                    } else {
                        self.arrays.pop();
                        let last_idx = self.arrays.len() - 1;
                        top_array = &mut self.arrays[last_idx];
                        ip += offset;
                    }
                }
                Instruction::ObjectValuesIter(offset) => {
                    if let Value::Object(object) = top {
                        let mut values = object.values();
                        if let Some(first) = values.next() {
                            top = first;
                            self.object_values.push(object.values());
                            self.values.push(first);
                            ip += 1;
                        } else {
                            ip += offset;
                        }
                    } else {
                        ip += offset;
                    }
                }
                Instruction::ObjectValuesIterNext(offset) => {
                    if let Some(next) = self.object_values.last_mut().unwrap().next() {
                        self.values.push(next);
                        top = next;
                    } else {
                        self.object_values.pop();
                        ip += offset;
                    }
                }
                Instruction::PushScope(scope) => {
                    self.scopes.push(*scope);
                }
                Instruction::PopScope => {
                    let scope = self.scopes.pop();
                    let result = matches!(
                        scope,
                        EvaluationScope::AndValid
                            | EvaluationScope::OrSatisfied
                            | EvaluationScope::XorSingle
                    );
                    self.scopes.update_top(result);
                }
                Instruction::True => {
                    self.scopes.update_top(true);
                }
                Instruction::False => {
                    self.scopes.update_top(false);
                }
            }
            ip += 1;
        }
        self.scopes.is_valid()
    }
}

pub struct ErrorIterator<'a, 'b> {
    instructions: &'a [Instruction],
    vm: SchemaEvaluationVM<'b>,
    ip: usize,
}

#[derive(Debug)]
pub struct ValidatorV2 {
    instructions: Vec<Instruction>,
}

impl ValidatorV2 {
    pub fn new(schema: &Value) -> Self {
        let instructions = SchemaCompiler::compile(schema);
        ValidatorV2 { instructions }
    }
    pub fn is_valid(&self, instance: &Value) -> bool {
        let mut vm = SchemaEvaluationVM::new();
        self.is_valid_with(instance, &mut vm)
    }
    pub fn is_valid_with<'a>(&self, instance: &'a Value, vm: &mut SchemaEvaluationVM<'a>) -> bool {
        vm.is_valid(&self.instructions, instance)
    }
    pub fn iter_errors<'a, 'b>(&'a self, instance: &'b Value) -> ErrorIterator<'a, 'b> {
        // TODO: Create a VM / clone stored one
        // TODO: Rename method `reset`
        let mut vm = SchemaEvaluationVM::new();
        vm.reset(instance);
        ErrorIterator {
            instructions: &self.instructions,
            vm,
            ip: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use test_case::test_case;

    fn schema_numeric() -> Value {
        json!({"minimum": 5, "maximum": 10})
    }

    fn schema_properties() -> Value {
        json!({
            "properties": {
                "age": { "properties": { "inner": { "minimum": 18, "maximum": 65 } } },
                "score": { "properties": { "inner": { "minimum": 0, "maximum": 100 } } }
            }
        })
    }

    fn schema_anyof_numeric() -> Value {
        json!({"anyOf": [ { "minimum": 10 }, { "maximum": 5 } ]})
    }

    fn schema_anyof_objects() -> Value {
        json!({
            "anyOf": [
                { "properties": { "age": { "minimum": 18 } } },
                { "properties": { "name": { "type": "string" } } }
            ]
        })
    }

    fn schema_nested_anyof() -> Value {
        json!({
            "properties": {
                "data": { "anyOf": [ { "minimum": 50 }, { "maximum": 10 } ] }
            }
        })
    }

    fn schema_string_length() -> Value {
        json!({"minLength": 3, "maxLength": 10})
    }

    fn schema_nested_string_length() -> Value {
        json!({
            "properties": {
                "username": { "minLength": 5, "maxLength": 20 },
                "password": { "minLength": 8, "maxLength": 30 }
            }
        })
    }

    fn schema_anyof_string_length() -> Value {
        json!({
            "anyOf": [
                { "minLength": 15 },
                { "maxLength": 5 }
            ]
        })
    }

    #[test_case(schema_numeric(), json!(7))]
    #[test_case(schema_properties(), json!({"age": {"inner": 30}, "score": {"inner": 85}}))]
    #[test_case(schema_properties(), json!({"age": {"inner": 25}}))]
    #[test_case(schema_properties(), json!({"age": {"inner": 40}, "score": {"inner": 75}, "name": "John"}))]
    #[test_case(schema_properties(), json!(42))]
    #[test_case(schema_properties(), json!({"age": {"inner": "thirty"}, "score": {"inner": true}}))]
    #[test_case(schema_anyof_numeric(), json!(12))]
    #[test_case(schema_anyof_numeric(), json!(3))]
    #[test_case(schema_anyof_objects(), json!({"age": 20}))]
    #[test_case(schema_anyof_objects(), json!({"name": "John"}))]
    #[test_case(schema_nested_anyof(), json!({"data": 55}))]
    #[test_case(schema_nested_anyof(), json!({"data": 5}))]
    #[test_case(schema_string_length(), json!("hello"))]
    #[test_case(schema_string_length(), json!("abc"))]
    #[test_case(schema_string_length(), json!("1234567890"))]
    #[test_case(schema_nested_string_length(), json!({"username": "johndoe", "password": "secure_password123"}))]
    #[test_case(schema_nested_string_length(), json!({"username": "james"}))]
    #[test_case(schema_nested_string_length(), json!(123))]
    #[test_case(schema_anyof_string_length(), json!("this is a long string"))]
    #[test_case(schema_anyof_string_length(), json!("hi"))]
    fn valid_cases(schema: Value, instance: Value) {
        let validator = ValidatorV2::new(&schema);
        assert!(validator.is_valid(&instance));
    }

    #[test_case(schema_numeric(), json!(3))]
    #[test_case(schema_properties(), json!({"age": {"inner": 17}, "score": {"inner": 50}}))]
    #[test_case(schema_properties(), json!({"age": {"inner": 30}, "score": {"inner": 101}}))]
    #[test_case(schema_anyof_numeric(), json!(7))]
    #[test_case(schema_anyof_objects(), json!({"age": 16, "name": 123}))]
    #[test_case(schema_nested_anyof(), json!({"data": 30}))]
    #[test_case(schema_string_length(), json!("ab"))]
    #[test_case(schema_string_length(), json!("this string is too long"))]
    #[test_case(schema_nested_string_length(), json!({"username": "joe", "password": "secure"}))] // Both too short
    #[test_case(schema_nested_string_length(), json!({"username": "validusername", "password": "pw"}))] // Password too short
    #[test_case(schema_anyof_string_length(), json!("medium length"))]
    fn invalid_cases(schema: Value, instance: Value) {
        let validator = ValidatorV2::new(&schema);
        assert!(!validator.is_valid(&instance));
    }

    #[test]
    fn test_citm() {
        let schema = serde_json::from_slice(benchmark::CITM_SCHEMA).expect("Invalid JSON");
        let instance = serde_json::from_slice(benchmark::CITM).expect("Invalid JSON");
        let validator_v1 = crate::validator_for(&schema).unwrap();
        let validator_v2 = ValidatorV2::new(&schema);
        assert!(validator_v1.is_valid(&instance));
        assert!(validator_v2.is_valid(&instance));
    }
}
