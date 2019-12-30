use shiba_jit::{codegen::x86_64::*, ir::*, *};

fn main() {
    let mut ctx = Context::new();
    let hello_world_const = ctx.add_constant(b"Hello, world\n");
    let loop_inner = ctx.new_basic_block();
    let loop_outer = ctx.new_basic_block();
    ctx.build_basic_block(loop_inner)
        .add_parent(loop_outer)
        .push_instruction(IR::PrintConstant {
            constant_ref: hello_world_const,
        })
        /*        .push_instruction(IR::Immediate {
            dest_register: 0,
            _type: PrimitiveValue::U32,
            value: 2,
            alignment: 4,
        })
        .push_instruction(IR::Immediate {
            dest_register: 1,
            _type: PrimitiveValue::U32,
            value: 2,
            alignment: 4,
        })
        .push_instruction(IR::Add {
            dest_register: 2,
            src_register1: 0,
            src_register2: 1,
        })
        .push_instruction(IR::Immediate {
            dest_register: 3,
            _type: PrimitiveValue::U32,
            value: 4,
            alignment: 4,
        })
        .push_instruction(IR::Subtract {
            dest_register: 4,
            src_register1: 2,
            src_register2: 3,
        })*/
        .finish();

    ctx.build_basic_block(loop_outer)
        .add_parent(loop_inner)
        .push_instruction(IR::Jump { bb_idx: loop_inner })
        .finish();

    let (executable_buffer, offset) = generate_code(&ctx).unwrap();
    let hello_fn: extern "C" fn() = unsafe { std::mem::transmute(executable_buffer.ptr(offset)) };

    hello_fn();
}
