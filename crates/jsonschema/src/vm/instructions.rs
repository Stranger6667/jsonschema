use num_cmp::NumCmp;
use serde_json::Value;

pub struct VirtualMachine {
    ip: usize,
    stack: Vec<*const Value>,
}

impl VirtualMachine {
    pub fn new() -> VirtualMachine {
        VirtualMachine {
            ip: 0,
            stack: Vec::with_capacity(8),
        }
    }

    pub fn execute(&mut self, instructions: &[Instruction], instance: &Value) -> bool {
        self.ip = 0;
        let mut reg1: *const Value = std::ptr::null();
        let mut reg2: *const Value = std::ptr::null();
        let mut reg3: *const Value = std::ptr::null();

        let key_pool = &["foo", "bar", "spam", "baz"][..];
        let mut sp = 0_u8;
        // Macro for accessing the current item
        macro_rules! current_item {
            () => {
                match sp {
                    0 => instance,
                    1 => unsafe { &*reg1 },
                    2 => unsafe { &*reg2 },
                    3 => unsafe { &*reg3 },
                    _ => unsafe { &**self.stack.last().expect("Stack should not be empty") },
                }
            };
        }

        // Macro for pushing to the stack or registers
        macro_rules! push_item {
            ($value:expr) => {
                match sp {
                    0 => {
                        sp += 1;
                        reg1 = $value;
                    }
                    1 => {
                        sp += 1;
                        reg2 = $value;
                    }
                    2 => {
                        sp += 1;
                        reg3 = $value;
                    }
                    _ => {
                        self.stack.push($value);
                    }
                }
            };
        }

        // Macro for popping from the stack or registers
        macro_rules! pop_item {
            () => {
                match sp {
                    1 => {
                        sp -= 1;
                        reg1 = std::ptr::null();
                    }
                    2 => {
                        sp -= 1;
                        reg2 = std::ptr::null();
                    }
                    3 => {
                        sp -= 1;
                        reg3 = std::ptr::null();
                    }
                    _ => {
                        self.stack.pop();
                    }
                }
            };
        }

        while self.ip < instructions.len() {
            match instructions[self.ip] {
                Instruction::Properties { start, end } => {
                    if let Value::Object(map) = current_item!() {
                        for key in &key_pool[start..end] {
                            if let Some(value) = map.get(*key) {
                                push_item!(value);
                            }
                        }
                    }
                }
                Instruction::MaximumU64 { limit } => {
                    // TODO: What to do if there is no key in the instance?
                    if let Some(item) = current_item!().as_number() {
                        let t = if let Some(item) = item.as_u64() {
                            item <= limit
                        } else if let Some(item) = item.as_i64() {
                            !NumCmp::num_gt(item, limit)
                        } else {
                            let item = item.as_f64().expect("Always valid");
                            !NumCmp::num_gt(item, limit)
                        };
                        if t != true {
                            return false;
                        }
                    }
                }
                Instruction::MinimumU64 { limit } => {
                    if let Some(item) = current_item!().as_number() {
                        let t = if let Some(item) = item.as_u64() {
                            item >= limit
                        } else if let Some(item) = item.as_i64() {
                            !NumCmp::num_lt(item, limit)
                        } else {
                            let item = item.as_f64().expect("Always valid");
                            !NumCmp::num_lt(item, limit)
                        };
                        if t != true {
                            return false;
                        }
                    }
                }
                Instruction::Pop => {
                    pop_item!()
                }
                _ => {}
            }
            self.ip += 1;
        }
        self.stack.clear();
        true
    }
}

struct ByteCode {
    instructions: Vec<Instruction>,
}

#[derive(Debug)]
pub enum Instruction {
    Pop,
    // NOTE: fuse some numeric stuff directly into `Properties` if there is enough size left.
    //       for 2 or 3 properties it could be realistic
    // Properties {
    //     items: [(usize, Maximum); 2]
    // }
    Properties { start: usize, end: usize },
    MaximumU64 { limit: u64 },
    MaximumI64 { limit: i64 },
    MaximumF64 { limit: f64 },
    MinimumU64 { limit: u64 },
    MinimumI64 { limit: i64 },
    MinimumF64 { limit: f64 },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_something() {
        println!("Size: {}", std::mem::size_of::<Instruction>());
        let instructions = vec![
            Instruction::Properties { start: 0, end: 2 }, // For "foo" and "bar"
            Instruction::MinimumU64 { limit: 3 },         // Validate "bar"
            Instruction::Pop,                             // Return to parent context
            Instruction::MaximumU64 { limit: 1 },         // Validate "foo"
            Instruction::Pop,                             // Return to parent context
        ];
        let mut vm = VirtualMachine::new();
        let instance = json!({"foo": 0, "bar": 4});
        assert!(vm.execute(&instructions, &instance));
        let instance = json!({"foo": 0, "bar": 2});
        assert!(!vm.execute(&instructions, &instance));
        let instance = json!({"foo": 2, "bar": 3});
        assert!(!vm.execute(&instructions, &instance));
        let instructions = vec![
            Instruction::MinimumU64 { limit: 1 },
            Instruction::MaximumU64 { limit: 3 },
        ];
        let mut vm = VirtualMachine::new();
        let instance = json!(2);
        assert!(vm.execute(&instructions, &instance));
        let instance = json!(0);
        assert!(!vm.execute(&instructions, &instance));
        let instance = json!(4);
        assert!(!vm.execute(&instructions, &instance));
    }
}
