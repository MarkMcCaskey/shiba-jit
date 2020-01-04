use crate::ir::*;
use crate::reg_alloc;
use std::collections::*;

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

// does not handle register spilling right now
// TODO: handle register spilling
fn compute_register_map(bbm: &BasicBlockManager) -> BTreeMap<RegisterIndex, MachineRegister> {
    let mut available_registers = VecDeque::new();
    available_registers.push_back(MachineRegister::Rdx);
    available_registers.push_back(MachineRegister::Rbx);
    available_registers.push_back(MachineRegister::R8);
    available_registers.push_back(MachineRegister::R9);
    available_registers.push_back(MachineRegister::R10);
    available_registers.push_back(MachineRegister::R11);
    available_registers.push_back(MachineRegister::R12);
    available_registers.push_back(MachineRegister::R13);
    available_registers.push_back(MachineRegister::R14);
    available_registers.push_back(MachineRegister::R15);
    let current_mapping: BTreeMap<RegisterIndex, MachineRegister> = BTreeMap::new();
    let mut out: BTreeMap<RegisterIndex, MachineRegister> = BTreeMap::new();
    let gd = reg_alloc::compute_graph(bbm);
    let gq = reg_alloc::GraphQuery::new(gd, bbm);
    let mut seen = BTreeSet::new();
    build_register_map_inner(
        bbm,
        &gq,
        bbm.start,
        &mut out,
        current_mapping,
        available_registers,
        &mut seen,
    );

    out
}

fn build_register_map_inner(
    bbm: &BasicBlockManager,
    gq: &reg_alloc::GraphQuery,
    cur_idx: BasicBlockIndex,
    reg_map: &mut BTreeMap<RegisterIndex, MachineRegister>,
    mut current_map: BTreeMap<RegisterIndex, MachineRegister>,
    mut available_registers: VecDeque<MachineRegister>,
    seen: &mut BTreeSet<BasicBlockIndex>,
) {
    if seen.contains(&cur_idx) {
        return;
    } else {
        seen.insert(cur_idx);
    }

    // =====================================================
    // free registers that are not used on this path
    // TODO: optimize [this can probably avoid the clone AND also only be done
    // in cases where the parent has multiple paths]
    let cm_copy = current_map.clone();
    for (k, _) in cm_copy {
        if !gq.is_live_in(k, cur_idx) {
            let machine_reg = current_map.remove(&k).unwrap();
            available_registers.push_back(machine_reg);
        }
    }

    // TODO: generate liveness info from inside basic blocks too to reduce register pressure
    // this should cause basic tests to fail in the short-term so should be implemented
    // very soon
    for declared_reg in bbm.get(cur_idx).unwrap().iter_defined_registers() {
        let machine_reg = available_registers
            .pop_front()
            .expect("Ran out of machine registers! Need to implement register spilling");
        let existing_reg = current_map.insert(*declared_reg, machine_reg);
        assert!(existing_reg.is_none());
        let existing_reg = reg_map.insert(*declared_reg, machine_reg);
        assert!(existing_reg.is_none());
    }

    // =====================================================
    // free registers that are not used on any path after
    // TODO: optimize
    let cm_copy = current_map.clone();
    for (k, _) in cm_copy {
        if !gq.is_live_out(k, cur_idx) {
            let machine_reg = current_map.remove(&k).unwrap();
            available_registers.push_back(machine_reg);
        }
    }
    for exit in bbm.get(cur_idx).unwrap().iter_exits() {
        build_register_map_inner(
            bbm,
            gq,
            *exit,
            reg_map,
            current_map.clone(),
            available_registers.clone(),
            seen,
        );
    }
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
                    ; xor Rd(dest as u8), Rd(dest as u8)
                    ; mov Rd(dest as u8), BYTE val
            );
        }
        PrimitiveValue::U16 | PrimitiveValue::I16 => {
            let val = imm as u16 as i32;
            // 32bit
            dynasm!(ops
                    ; xor Rd(dest as u8), Rd(dest as u8)
                    ; mov Ra(dest as u8), WORD val
            );
        }
        PrimitiveValue::U32 | PrimitiveValue::I32 => {
            let val = imm as i32;
            // 32bit
            dynasm!(ops
                    ; xor Rd(dest as u8), Rd(dest as u8)
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
        constant_map.insert(ConstantIndex::new(i as _), dyn_lab);
    }
    constant_map
}

pub fn generate_code(ctx: &Context) -> Result<(ExecutableBuffer, AssemblyOffset), CodeGenError> {
    let mut ops = Assembler::new().unwrap();

    dynasm!(ops
            ; .arch x64
    );

    let start_offset;

    // =================================================================
    // set up the constants

    let constant_map = set_up_constants(ctx, &mut ops);

    // =================================================================
    // generate some machine code
    start_offset = ops.offset();

    let register_map = compute_register_map(&ctx.basic_blocks);
    dynasm!(ops
            ; push rbp
            ; mov rbp, rsp
            ; sub rsp, 0x8
            ; push rbx
            ; push rdi
            ; push rsi
    );

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
                    );
                }
                IR::JumpIfEqual {
                    src_register,
                    true_bb_idx,
                    false_bb_idx,
                } => {
                    // TODO: evaluate IR in the context of this instruction: seems suboptimal
                    let true_ent = bb_map
                        .entry(true_bb_idx)
                        .or_insert_with(|| ops.new_dynamic_label())
                        .clone();
                    let false_ent = bb_map
                        .entry(false_bb_idx)
                        .or_insert_with(|| ops.new_dynamic_label());
                    match src_register {
                        Value::Register(r1) => {
                            let mr1 = register_map[&r1];
                            dynasm!(ops
                                    ; cmp Ra(mr1 as u8), DWORD 0
                                    ; je => true_ent
                                    ; jmp => *false_ent
                            )
                        }
                        _ => unimplemented!("Conditional jumps on immediate values"),
                    }
                }
                IR::Add {
                    dest_register,
                    src1,
                    src2,
                } => {
                    let mdest = register_map[&dest_register];
                    match (src1, src2) {
                        (Value::Register(r1), Value::Register(r2)) => {
                            let mr1 = register_map[&r1];
                            let mr2 = register_map[&r2];
                            dynasm!(ops
                                     ; mov Ra(mdest as u8), Ra(mr1 as u8)
                                     ; add Ra(mdest as u8), Ra(mr2 as u8)
                            );
                        }
                        (Value::Register(r1), Value::Immediate { _type, value })
                        | (Value::Immediate { _type, value }, Value::Register(r1)) => {
                            let mr1 = register_map[&r1];
                            emit_mov_imm(&mut ops, mdest, value, _type);
                            dynasm!(ops
                                   ; add Ra(mdest as u8), Ra(mr1 as u8)
                            );
                        }
                        (
                            Value::Immediate { _type, value: v1 },
                            Value::Immediate { value: v2, .. },
                        ) => {
                            emit_mov_imm(&mut ops, mdest, v1 + v2, _type);
                        }
                    }
                }
                IR::Subtract {
                    dest_register,
                    src1,
                    src2,
                } => {
                    let mdest = register_map[&dest_register];
                    match (src1, src2) {
                        (Value::Register(r1), Value::Register(r2)) => {
                            let mr1 = register_map[&r1];
                            let mr2 = register_map[&r2];
                            dynasm!(ops
                                     ; mov Ra(mdest as u8), Ra(mr1 as u8)
                                     ; sub Ra(mdest as u8), Ra(mr2 as u8)
                            );
                        }
                        (Value::Register(_), Value::Immediate { .. }) => {
                            // emit_mov_imm is insufficient hee
                            todo!("Implement this by updating the core abstraction");
                            /*let mr1 = register_map[&r1];
                            dynasm!(ops
                                    ; mov Ra(mdest as u8), Ra(mr1 as u8));
                            emit_mov_imm(&mut ops, mdest, value, _type);
                            dynasm!(ops
                                   ; sub Ra(mdest as u8), Ra(mr1 as u8)
                            );*/
                        }
                        (Value::Immediate { _type, value }, Value::Register(r2)) => {
                            let mr2 = register_map[&r2];
                            emit_mov_imm(&mut ops, mdest, value, _type);
                            dynasm!(ops
                                   ; sub Ra(mdest as u8), Ra(mr2 as u8)
                            );
                        }
                        (
                            Value::Immediate { _type, value: v1 },
                            Value::Immediate { value: v2, .. },
                        ) => {
                            emit_mov_imm(&mut ops, mdest, v1 - v2, _type);
                        }
                    }
                }
                IR::Alloca {
                    dest_register,
                    _type,
                    ..
                } => {
                    let mdest = register_map[&dest_register];
                    match _type {
                        PrimitiveValue::I32 | PrimitiveValue::U32 => {
                            dynasm!(ops
                                    ; lea Ra(mdest as u8), [rbp - 4]
                            );
                        }
                        _ => {
                            unimplemented!("should probably rewrite allocas and not implement this")
                        }
                    }
                }
                IR::Load {
                    dest_register,
                    src_register,
                } => {
                    let mdest = register_map[&dest_register];
                    match src_register {
                        Value::Register(src) => {
                            let msrc = register_map[&src];
                            dynasm!(ops
                                    ; mov Rd(mdest as u8), [Ra(msrc as u8)]
                            );
                        }
                        Value::Immediate { .. } => {
                            todo!("deref raw pointers");
                            // lazy hack, assert pointer type; should be done in validation
                            /*assert!(_type == PrimitiveValue::U64);
                            dynasm!(ops
                                    ; mov Ra(mdest as u8), (QWORD value))*/
                        }
                    }
                }
                IR::Store {
                    dest_register,
                    src_register,
                } => match (dest_register, src_register) {
                    (Value::Register(dest), Value::Register(src)) => {
                        let mdest = register_map[&dest];
                        let msrc = register_map[&src];

                        dynasm!(ops
                                ; mov [Ra(mdest as u8)], Ra(msrc as u8)
                        );
                    }
                    (Value::Register(dest), Value::Immediate { _type, value }) => {
                        let mdest = register_map[&dest];

                        match _type {
                            PrimitiveValue::U32 => {
                                dynasm!(ops
                                        ; mov eax, DWORD value as i32
                                        ; mov [Ra(mdest as u8)], eax
                                );
                            }
                            _ => unimplemented!("storing anything but a u32"),
                        }
                    }
                    _ => unimplemented!("Store for constant destinations"),
                },
                IR::Return => {
                    dynasm!(ops
                            ; pop rsi
                            ; pop rdi
                            ; pop rbx
                            ; add rsp, 0x8
                            ; mov rsp, rbp
                            ; pop rbp
                            ; ret
                    );
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
        .map(|r| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open("out")
                .unwrap();
            f.write_all(&r[start_offset.0..]).unwrap();
            (r, start_offset)
        })
}
