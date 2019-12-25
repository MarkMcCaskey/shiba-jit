use crate::ir::*;
use std::collections::*;
use std::iter;
use std::mem;

use dynasmrt::x64::Assembler;
use dynasmrt::{mmap::ExecutableBuffer, AssemblyOffset, DynasmApi, DynasmLabelApi};

#[derive(Debug, Clone)]
struct Register {
    _type: PrimitiveValue,
    value: RegisterValueLocation,
}

#[derive(Debug, Clone)]
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
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

pub extern "C" fn guest_print(buffer: *const u8, len: u64) {
    dbg!("MADE IT");
    use std::io::Write;
    std::io::stdout()
        .write_all(unsafe { std::slice::from_raw_parts(buffer, len as usize) })
        .unwrap()
}

fn emit_mov_imm(ops: &mut Assembler, dest: MachineRegister, imm: usize, _type: PrimitiveValue) {
    match _type {
        PrimitiveValue::U8 | PrimitiveValue::I8 => {
            let val = imm as u8 as i32;
            // 32bit
            dynasm!(ops
                    ; mov Ra(dest as u8), BYTE val
            );
        }
        PrimitiveValue::U16 | PrimitiveValue::I16 => {
            let val = imm as u16 as i32;
            // 32bit
            dynasm!(ops
                    ; mov Ra(dest as u8), WORD val
            );
        }
        PrimitiveValue::U32 | PrimitiveValue::I32 => {
            let val = imm as i32;
            // 32bit
            dynasm!(ops
                    ; mov Ra(dest as u8), DWORD val
            );
        }
        PrimitiveValue::U64 | PrimitiveValue::I64 => {
            let val = imm as i64;
            // 64bit
            dynasm!(ops
                    ; mov Ra(dest as u8), QWORD val
            );
        }
    }
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
    CodeGenFailure,
}

#[derive(Debug, Default)]
pub struct CodeGenState {
    register_map: BTreeMap<usize, Register>,
    /// when the registers were first seen
    register_first_seen: BTreeMap<usize, usize>,
    /// when the registers were last seen
    register_last_seen: BTreeMap<usize, usize>,
    /// label_id -> idx in out
    label_map: BTreeMap<usize, usize>,
}

impl CodeGenState {
    fn create_register(
        &mut self,
        register: usize,
        _type: PrimitiveValue,
        value: RegisterValueLocation,
        location: usize,
    ) -> Result<(), CodeGenError> {
        let res = self
            .register_map
            .insert(register, Register { _type, value });
        if res.is_some() {
            return Err(CodeGenError {
                location,
                reason: CodeGenErrorReason::RegisterValueTaken(register),
            });
        }
        self.register_first_seen.insert(register, location);
        self.register_last_seen.insert(register, location);
        Ok(())
    }

    fn get_register(&mut self, register: usize, location: usize) -> Result<Register, CodeGenError> {
        *self.register_last_seen.get_mut(&register).unwrap() = location;
        self.register_map
            .get(&register)
            .cloned()
            .ok_or_else(|| CodeGenError {
                location,
                reason: CodeGenErrorReason::RegisterNotFound(register),
            })
    }
}

pub fn generate_code(
    instruction_stream: &[IR],
) -> Result<(ExecutableBuffer, AssemblyOffset), CodeGenError> {
    let mut ops = Assembler::new().unwrap();

    dynasm!(ops
            ; .arch x64
    );

    let mut start_offset = ops.offset();

    let mut cgs = CodeGenState::default();

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
                cgs.create_register(dest_register, _type, value, location)?;
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
                let reg1 = cgs.get_register(src_register1, location)?;
                let reg2 = cgs.get_register(src_register2, location)?;
                assert_type(reg1._type, reg2._type, location)?;

                let value = RegisterValueLocation::DependsOn(vec![src_register1, src_register2]);
                cgs.create_register(dest_register, reg1._type, value, location)?;
            }
            IR::Load {
                dest_register,
                src_register,
            }
            | IR::Store {
                dest_register,
                src_register,
            } => {
                let src = cgs.get_register(src_register, location)?;

                let value = RegisterValueLocation::DependsOn(vec![src_register]);
                cgs.create_register(dest_register, src._type, value, location)?;
            }
            IR::Label { label_idx } => {
                // TODO error checking here
                let res = cgs.label_map.insert(label_idx, location);
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
                assert!(cgs.label_map.contains_key(&label_idx));
                cgs.get_register(src_register, location)?;
            }
            IR::Print { ref value } => {
                dynasm!(ops
                        ; ->hello:
                        ; .bytes value.as_bytes()
                );
            }
            _ => (),
        }
    }

    // ===================================================================
    // hack out some register allocation
    //
    // TODO: look up algorithms. something something 4 color theorem

    // mapping from location to register event
    // TODO: should use a set not a vec
    let mut register_events: BTreeMap<usize, HashSet<RegisterEvent>> = BTreeMap::new();

    for (register, location) in cgs.register_first_seen.iter() {
        if let RegisterValueLocation::Constant(_) = cgs.register_map[register].value {
            // constants don't need a register allocated
            continue;
        }
        let mut inserter = register_events.entry(*location).or_default();
        inserter.insert(RegisterEvent::Acquire(*register));
    }
    for (register, location) in cgs.register_last_seen.iter() {
        if let RegisterValueLocation::Constant(_) = cgs.register_map[register].value {
            // constants don't need a register allocated
            continue;
        }
        let mut inserter = register_events.entry(*location).or_default();
        inserter.insert(RegisterEvent::Release(*register));
    }
    let mut registers = VecDeque::new();
    // init register queue
    registers.push_back(MachineRegister::Rdx);
    registers.push_back(MachineRegister::Rbx);
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

    start_offset = ops.offset();
    let mut label_map: BTreeMap<usize, _> = BTreeMap::new();
    for (location, instruction) in instruction_stream.iter().enumerate() {
        if let Some(v) = label_map.get(&location) {
            dynasm!(ops
                    ; =>*v);
        }
        match *instruction {
            IR::Immediate { .. } => {
                // do nothing here
            }
            IR::Add {
                dest_register,
                src_register1,
                src_register2,
            } => {
                let dest_reg = machine_register_map[&dest_register];
                let _type = cgs.register_map[&src_register1]._type;
                match (
                    &cgs.register_map[&src_register1].value,
                    &cgs.register_map[&src_register2].value,
                ) {
                    (RegisterValueLocation::Constant(c1), RegisterValueLocation::Constant(c2)) => {
                        // mov
                        // mov is 0x48 or 0x49 depending on regsiter
                        emit_mov_imm(&mut ops, dest_reg, c1 + c2, _type);
                    }
                    (RegisterValueLocation::Constant(c1), RegisterValueLocation::DependsOn(_)) => {
                        emit_mov_imm(&mut ops, dest_reg, *c1, _type);
                        dynasm!(ops
                                ; add Ra(dest_reg as u8), Ra(src_register2 as u8));
                    }
                    (RegisterValueLocation::DependsOn(_), RegisterValueLocation::Constant(c2)) => {
                        emit_mov_imm(&mut ops, dest_reg, *c2, _type);
                        dynasm!(ops
                                ; add Ra(dest_reg as u8), Ra(src_register1 as u8));
                    }
                    (RegisterValueLocation::DependsOn(_), RegisterValueLocation::DependsOn(_)) => {
                        dynasm!(ops
                                ; mov Ra(dest_reg as u8), Ra(src_register1 as u8)
                                ; add Ra(dest_reg as u8), Ra(src_register2 as u8));
                    }
                    _ => panic!("Move cases not yet implemented in codegen"),
                }
            }
            IR::Subtract {
                dest_register,
                src_register1,
                src_register2,
            } => {
                let dest_reg = machine_register_map[&dest_register];
                let _type = cgs.register_map[&src_register1]._type;
                match (
                    &cgs.register_map[&src_register1].value,
                    &cgs.register_map[&src_register2].value,
                ) {
                    (RegisterValueLocation::Constant(c1), RegisterValueLocation::Constant(c2)) => {
                        // mov
                        // mov is 0x48 or 0x49 depending on regsiter
                        emit_mov_imm(&mut ops, dest_reg, c1 - c2, _type);
                    }
                    (RegisterValueLocation::Constant(c1), RegisterValueLocation::DependsOn(_)) => {
                        emit_mov_imm(&mut ops, dest_reg, *c1, _type);
                        dynasm!(ops
                                ; sub Ra(dest_reg as u8), Ra(src_register2 as u8));
                    }
                    (RegisterValueLocation::DependsOn(_), RegisterValueLocation::Constant(c2)) => {
                        emit_mov_imm(&mut ops, dest_reg, *c2, _type);
                        dynasm!(ops
                                ; sub Ra(dest_reg as u8), Ra(src_register1 as u8));
                    }
                    (RegisterValueLocation::DependsOn(_), RegisterValueLocation::DependsOn(_)) => {
                        dynasm!(ops
                                ; mov Ra(dest_reg as u8), Ra(src_register1 as u8)
                                ; sub Ra(dest_reg as u8), Ra(src_register2 as u8));
                    }
                    _ => panic!("Move cases not yet implemented in codegen"),
                }
            }
            IR::JumpIfEqual {
                src_register,
                label_idx,
            } => {
                let jump_loc = label_map[&label_idx];

                dynasm!(ops
                        ; cmp Ra(src_register as u8), BYTE 0
                        ; jz =>jump_loc
                        ; ret );
            }
            IR::Print { ref value } => {
                dynasm!(ops
                        ; sub rsp, BYTE 0x28
                        ; push rax
                        ; push rdx
                        ; push rcx
                        ; push rbp
                        ; mov rbp, rsp
                        ; lea rcx, [->hello]
                        ; xor edx, edx
                        ; mov dl, BYTE value.len() as _
                        ; mov rax, QWORD guest_print as _
                        ; sub rsp, BYTE 0x28
                        ; call rax
                        ; add rsp, BYTE 0x28
                        ; mov rsp, rbp
                        ; pop rbp
                        ; pop rcx
                        ; pop rdx
                        ; pop rax
                        ; add rsp, BYTE 0x28
                );
            }
            IR::Label { label_idx } => {
                let jump_loc = ops.new_dynamic_label();
                label_map.insert(label_idx, jump_loc);
                dynasm!(ops
                        ; =>jump_loc
                );
            }

            _ => panic!("Instruction not yet implemented in codegen"),
        }
    }

    ops.finalize()
        .map_err(|_| CodeGenError {
            location: 0,
            reason: CodeGenErrorReason::CodeGenFailure,
        })
        .map(|r| (r, start_offset))
}
