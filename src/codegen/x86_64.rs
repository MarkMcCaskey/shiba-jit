use crate::ir::*;
use std::collections::*;
use std::iter;
use std::mem;

use dynasmrt::x64::Assembler;
use dynasmrt::{mmap::ExecutableBuffer, AssemblyOffset, DynamicLabel, DynasmApi, DynasmLabelApi};

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

pub fn set_up_constants(
    ctx: &Context,
    ops: &mut Assembler,
) -> BTreeMap<ConstantIndex, DynamicLabel> {
    let mut constant_map: BTreeMap<ConstantIndex, DynamicLabel> = BTreeMap::new();
    for (i, constant) in ctx.constants.iter().enumerate() {
        // TODO: investigate dynamic vs global labels
        let dyn_lab = ops.new_dynamic_label();
        dynasm!(ops
                ; => dyn_lab
                ; .bytes constant.as_slice()
        );
        constant_map.insert(ConstantIndex::new(i), dyn_lab);
    }
    constant_map
}

pub fn generate_code(ctx: &Context) -> Result<(ExecutableBuffer, AssemblyOffset), CodeGenError> {
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

    // =================================================================
    // set up the constants

    let constant_map = set_up_constants(ctx, &mut ops);

    // =================================================================
    // generate some machine code
    start_offset = ops.offset();

    dbg!(&ctx);
    // TODO: investigate the different types of labels
    let mut bb_map: BTreeMap<BasicBlockIndex, DynamicLabel> = BTreeMap::new();
    for (i, basic_block) in ctx.iterate_basic_blocks() {
        let ent = bb_map.entry(i).or_insert_with(|| ops.new_dynamic_label());
        dynasm!(ops
                ; => *ent);
        for inst in basic_block.iterate_instructions() {
            match *inst {
                IR::PrintConstant { ref constant_ref } => {
                    let const_loc = constant_map[constant_ref];
                    let len = ctx.get_constant(*constant_ref).unwrap().len();
                    dynasm!(ops
                                ; push rax
                                ; push rcx
                                ; push rdx
                                ; push rsi
                                ; push rdi
                                ; push r8
                                ; push r9
                                ; push r10
                                ; push r11
                                ; lea rdi, [=>const_loc]
                                ; xor esi, esi
                                ; mov si, BYTE len as _
                                ; mov rax, QWORD guest_print as _
                                ; call rax
                                ; pop r11
                                ; pop r10
                                ; pop r9
                                ; pop r8
                                ; pop rdi
                                ; pop rsi
                                ; pop rdx
                                ; pop rcx
                                ; pop rax
                    );
                }
                IR::Jump { bb_idx } => {
                    let j_ent = bb_map
                        .entry(bb_idx)
                        .or_insert_with(|| ops.new_dynamic_label());
                    dynasm!(ops
                        ; jmp => *j_ent
                        ; ret )
                }
                _ => unimplemented!("not yet"),
            }
        }
    }

    /*

    // =================================================================
    // generate some machine code

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
            // Caller saved registers:
            //  RAX, RCX, RDX, RSI, RDI, R8, R9, R10, R11
            IR::Print { ref value } => {
                dynasm!(ops
                        ; push rax
                        ; push rcx
                        ; push rdx
                        ; push rsi
                        ; push rdi
                        ; push r8
                        ; push r9
                        ; push r10
                        ; push r11
                        ; lea rdi, [->hello]
                        ; xor esi, esi
                        ; mov si, BYTE value.len() as _
                        ; mov rax, QWORD guest_print as _
                        ; call rax
                        ; pop r11
                        ; pop r10
                        ; pop r9
                        ; pop r8
                        ; pop rdi
                        ; pop rsi
                        ; pop rdx
                        ; pop rcx
                        ; pop rax
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
        */

    ops.finalize()
        .map_err(|_| CodeGenError {
            location: 0,
            reason: CodeGenErrorReason::CodeGenFailure,
        })
        .map(|r| (r, start_offset))
}
