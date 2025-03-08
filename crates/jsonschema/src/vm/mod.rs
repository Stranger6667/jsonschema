use std::slice::Iter;

use smallvec::SmallVec;

use serde_json::{map::Values, Value};

use crate::compilerv2::{is_unique, Instruction, OneOfStack, SchemaCompiler};

pub struct SchemaEvaluationVM<'a> {
    values: SmallVec<[&'a Value; 8]>,
    arrays: SmallVec<[Iter<'a, Value>; 4]>,
    object_values: Vec<Values<'a>>,
}

impl Default for SchemaEvaluationVM<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SchemaEvaluationVM<'a> {
    pub fn new() -> Self {
        Self {
            values: SmallVec::new(),
            arrays: SmallVec::new(),
            object_values: Vec::new(),
        }
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.values.clear();
        self.object_values.clear();
        self.arrays.clear();
    }

    fn is_valid(&mut self, instructions: &[Instruction], instance: &'a Value) -> bool {
        self.reset();
        //
        //dbg!(instructions);
        //dbg!(instructions.len());
        //dbg!(std::mem::size_of::<Instruction>());

        let mut ip = 0;
        let mut top = instance;
        let mut top_array = [].iter();
        let mut last_result = true;
        let mut one_of_stack = OneOfStack::new();

        macro_rules! execute {
            ($variant:ident, $keyword:expr) => {{
                if let Value::$variant(value) = top {
                    last_result = $keyword.execute(value);
                }
            }};
            ($keyword:expr) => {{
                last_result = $keyword.execute(top);
            }};
        }

        //let mut cnt = 0;
        while ip < instructions.len() {
            //cnt += 1;
            //println!("{:?}", &instructions[ip]);
            //println!("Stack depth: {}", self.object_values.len());
            match &instructions[ip] {
                Instruction::TypeNull => last_result = matches!(top, Value::Null),
                Instruction::TypeBoolean => last_result = matches!(top, Value::Bool(_)),
                Instruction::TypeString => last_result = matches!(top, Value::String(_)),
                Instruction::TypeNumber => last_result = matches!(top, Value::Number(_)),
                Instruction::TypeInteger => {
                    last_result = if let Value::Number(n) = top {
                        n.is_i64() || n.is_u64()
                    } else {
                        false
                    }
                }
                Instruction::TypeArray => last_result = matches!(top, Value::Array(_)),
                Instruction::TypeObject => last_result = matches!(top, Value::Object(_)),
                Instruction::TypeSet(inner) => execute!(inner),
                Instruction::Enum(inner) => execute!(inner),
                Instruction::EnumSingle(inner) => execute!(inner),
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
                Instruction::MultipleOfInteger(inner) => execute!(Number, inner),
                Instruction::MultipleOfFloat(inner) => execute!(Number, inner),
                Instruction::MinProperties(inner) => execute!(Object, inner),
                Instruction::MaxProperties(inner) => execute!(Object, inner),
                Instruction::Required(inner) => execute!(Object, inner),
                Instruction::MinItems(inner) => execute!(Array, inner),
                Instruction::MaxItems(inner) => execute!(Array, inner),
                Instruction::UniqueItems => {
                    if let Value::Array(items) = top {
                        last_result = is_unique(items);
                    }
                }
                Instruction::PushProperty {
                    name,
                    skip_if_missing,
                    required,
                } => {
                    if let Value::Object(obj) = top {
                        if let Some(value) = obj.get(name.as_ref()) {
                            self.values.push(top);
                            top = value;
                        } else if *required {
                            last_result = false;
                            ip += skip_if_missing;
                        } else {
                            ip += skip_if_missing;
                        }
                    } else {
                        ip += skip_if_missing;
                    }
                }
                Instruction::PopValue => {
                    top = self.values.pop().expect("Value stack underflow");
                }
                Instruction::ArrayIter(offset) => {
                    if let Value::Array(values) = top {
                        let mut values = values.iter();
                        if let Some(first) = values.next() {
                            self.values.push(top);
                            self.arrays.push(top_array);
                            top = first;
                            top_array = values;
                        } else {
                            ip += offset;
                        }
                    } else {
                        ip += offset;
                    }
                }
                Instruction::ArrayIterNext(offset) => {
                    if let Some(next) = top_array.next() {
                        ip -= offset;
                        top = next;
                        continue;
                    } else {
                        top = self.values.pop().expect("Value stack underflow");
                        top_array = self.arrays.pop().unwrap();
                    }
                }
                Instruction::ObjectValuesIter(offset) => {
                    if let Value::Object(object) = top {
                        let mut values = object.values();
                        if let Some(first) = values.next() {
                            self.values.push(top);
                            top = first;
                            self.object_values.push(values);
                        } else {
                            ip += offset;
                        }
                    } else {
                        ip += offset;
                    }
                }
                Instruction::ObjectValuesIterNext(offset) => {
                    if let Some(next) = self.object_values.last_mut().unwrap().next() {
                        ip -= offset;
                        top = next;
                        continue;
                    } else {
                        top = self.values.pop().expect("Value stack underflow");
                        self.object_values.pop();
                    }
                }
                Instruction::True => {
                    last_result = true;
                }
                Instruction::False => {
                    last_result = false;
                }
                Instruction::JumpIfValid(offset) => {
                    if last_result {
                        ip += offset;
                    } else {
                        ip += 1;
                        continue;
                    }
                }
                Instruction::JumpIfInvalid(offset) => {
                    if last_result {
                        ip += 1;
                        continue;
                    } else {
                        ip += offset;
                    }
                }
                Instruction::PushOneOf => {
                    one_of_stack.push();
                }
                Instruction::SetOneValid => {
                    if last_result {
                        one_of_stack.mark_valid();
                    }
                }
                Instruction::JumpIfSecondValid(offset) => {
                    if last_result {
                        if !one_of_stack.mark_valid() {
                            one_of_stack.pop();
                            last_result = false;
                            ip += offset;
                        }
                    }
                }
                Instruction::PopOneOf => {
                    last_result = one_of_stack.pop();
                }
            }
            ip += 1;
        }
        //dbg!(cnt);
        last_result
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
        vm.reset();
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

    #[test]
    fn test_geojson() {
        let schema = serde_json::from_slice(benchmark::GEOJSON).expect("Invalid JSON");
        let instance = serde_json::from_slice(benchmark::CANADA).expect("Invalid JSON");
        let validator_v1 = crate::validator_for(&schema).unwrap();
        let validator_v2 = ValidatorV2::new(&schema);
        assert!(validator_v1.is_valid(&instance));
        assert!(validator_v2.is_valid(&instance));
    }
}
