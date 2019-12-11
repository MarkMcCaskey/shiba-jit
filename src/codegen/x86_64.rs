use crate::ir::*;
use std::collections::*;

struct Register {
    _type: PrimitiveValue,
    value: RegisterValueLocation,
}

pub enum RegisterValueLocation {
    Constant(usize),
    DependsOn(Vec<usize>),
    Memory(usize),
}

#[derive(Debug)]
pub struct CodeGenError {
    /// Which IR instruction the error happened at
    location: usize,
    reason: CodeGenErrorReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineRegister {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum RegisterEvent {
    Acquire(usize),
    Release(usize),
}

#[derive(Debug)]
pub enum CodeGenErrorReason {
    RegisterValueTaken(usize),
    RegisterNotFound(usize),
    TypeMismatch(PrimitiveValue, PrimitiveValue),
}

pub fn generate_code(instruction_stream: &[IR]) -> Result<Vec<u8>, CodeGenError> {
    let mut out = vec![];
    let mut register_map: BTreeMap<usize, Register> = BTreeMap::new();
    // when the registers were first seen
    let mut register_first_seen: BTreeMap<usize, usize> = BTreeMap::new();
    // when the registers were last seen
    let mut register_last_seen: BTreeMap<usize, usize> = BTreeMap::new();
    // label_id -> idx in out
    let mut label_map: BTreeMap<usize, usize> = BTreeMap::new();

    let get_register = |register, location| {
        register_last_seen[&register] = location;
        register_map.get(&register).ok_or_else(|| CodeGenError {
            location,
            reason: CodeGenErrorReason::RegisterNotFound(register),
        })
    };

    let create_register = |register, _type, value, location| {
        let res = register_map.insert(register, Register { _type, value });
        if res.is_some() {
            return Err(CodeGenError {
                location,
                reason: CodeGenErrorReason::RegisterValueTaken(register),
            });
        }
        register_first_seen.insert(register, location);
        register_last_seen.insert(register, location);
        Ok(())
    };

    let assert_type = |type1, type2, location| {
        if type1 == type2 {
            Ok(())
        } else {
            return Err(CodeGenError {
                location,
                reason: CodeGenErrorReason::TypeMismatch(type1, type2),
            });
        }
    };

    for (location, instruction) in instruction_stream.iter().enumerate() {
        match *instruction {
            IR::Immediate {
                dest_register,
                _type,
                value,
                ..
            } => {
                let value = RegisterValueLocation::Constant(value);
                create_register(dest_register, _type, value, location)?;
            }
            IR::Add {
                dest_register,
                src_register1,
                src_register2,
            }
            | IR::Subtract {
                dest_register,
                src_register1,
                src_register2,
            }
            | IR::Multiply {
                dest_register,
                src_register1,
                src_register2,
            }
            | IR::Divide {
                dest_register,
                src_register1,
                src_register2,
            } => {
                let reg1 = get_register(src_register1, location)?;
                let reg2 = get_register(src_register2, location)?;
                assert_type(reg1._type, reg2._type, location)?;

                let value = RegisterValueLocation::DependsOn(vec![src_register1, src_register2]);
                create_register(dest_register, reg1._type, value, location)?;
            }
            IR::Load {
                dest_register,
                src_register,
            }
            | IR::Store {
                dest_register,
                src_register,
            } => {
                let src = get_register(src_register, location)?;

                let value = RegisterValueLocation::DependsOn(vec![src_register]);
                create_register(dest_register, src._type, value, location)?;
            }
            IR::Label { label_idx } => {
                // TODO error checking here
                let res = label_map.insert(label_idx, location);
                assert!(res.is_none());
            }
            IR::JumpIfEqual {
                src_register,
                label_idx,
            }
            | IR::JumpIfNotEqual {
                src_register,
                label_idx,
            } => {
                // TODO error checking here
                assert!(label_map.contains_key(&label_idx));
                get_register(src_register, location)?;
            }
        }
    }

    // ===================================================================
    // hack out some register allocation
    //
    // TODO: look up algorithms. something something 4 color theorem

    // mapping from location to register event
    // TODO: should use a set not a vec
    let mut register_events: BTreeMap<usize, HashSet<RegisterEvent>> = BTreeMap::new();

    for (register, location) in register_first_seen.iter() {
        if let RegisterValueLocation::Constant(_) = register_map[register].value {
            // constants don't need a register allocated
            continue;
        }
        let mut inserter = register_events.entry(*location).or_default();
        inserter.insert(RegisterEvent::Acquire(*register));
    }
    for (register, location) in register_last_seen.iter() {
        if let RegisterValueLocation::Constant(_) = register_map[register].value {
            // constants don't need a register allocated
            continue;
        }
        let mut inserter = register_events.entry(*location).or_default();
        inserter.insert(RegisterEvent::Release(*register));
    }
    let mut registers = VecDeque::new();
    // init register queue
    registers.push_back(MachineRegister::Rax);
    registers.push_back(MachineRegister::Rcx);
    registers.push_back(MachineRegister::Rdx);
    registers.push_back(MachineRegister::Rbx);
    registers.push_back(MachineRegister::Rsi);
    registers.push_back(MachineRegister::Rdi);
    registers.push_back(MachineRegister::R8);
    registers.push_back(MachineRegister::R9);
    registers.push_back(MachineRegister::R10);
    registers.push_back(MachineRegister::R11);
    registers.push_back(MachineRegister::R12);
    registers.push_back(MachineRegister::R13);
    registers.push_back(MachineRegister::R14);
    registers.push_back(MachineRegister::R15);

    let mut machine_register_map: BTreeMap<usize, MachineRegister> = BTreeMap::new();

    for (_, events) in register_events.iter() {
        for event in events.iter() {
            match event {
                RegisterEvent::Acquire(r) => {
                    let register = registers.pop_front().expect("OUT OF REGISTERS!");
                    machine_register_map.insert(*r, register);
                }
                RegisterEvent::Release(r) => {
                    let register = machine_register_map[r];
                    registers.push_front(register);
                }
            }
        }
    }

    // =================================================================
    // generate some machine code

    for (location, instruction) in instruction_stream.iter().enumerate() {
        match *instruction {
            IR::Immediate {
                dest_register,
                _type,
                value,
                ..
            } => {
                
            }
        }
    }

    out
}
