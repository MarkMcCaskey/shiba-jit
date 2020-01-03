use shiba_jit::{codegen::x86_64::*, ir::*, *};

fn main() {
    let mut ctx = Context::new();
    let hello_world_const = ctx.add_constant(b"Hello, world\n");
    let end_const = ctx.add_constant(b"Goodbye, world\n");
    let prog_start = ctx.new_basic_block();
    let loop_inner = ctx.new_basic_block();
    let loop_outer = ctx.new_basic_block();
    let loop_exit = ctx.new_basic_block();
    let prog_start_bb = ctx.build_basic_block(prog_start);
    let counter = prog_start_bb.alloca(PrimitiveValue::U32, 4);
    prog_start_bb.store(counter, Value::u32(0));
    // look at other APIs; seems like we shouldn't need a jump
    prog_start_bb.jump(loop_inner);

    let inner_bb = ctx.build_basic_block(loop_inner);
    inner_bb.push_instruction(IR::PrintConstant {
        constant_ref: hello_world_const,
    });
    let loaded_counter = inner_bb.load(counter);
    let add_result = inner_bb.add(loaded_counter, Value::u32(1));
    inner_bb.store(counter, add_result);
    let sub_result = inner_bb.subtract(Value::u32(4), add_result);
    inner_bb.finish();

    let loop_exit_bb = ctx.build_basic_block(loop_exit);
    loop_exit_bb
        .add_parent(loop_outer)
        .push_instruction(IR::PrintConstant {
            constant_ref: end_const,
        });
    loop_exit_bb.ret();
    loop_exit_bb.finish();

    ctx.build_basic_block(loop_outer)
        .add_parent(loop_inner)
        .jump_if_equal(sub_result, loop_inner, loop_exit);

    ctx.finalize();
    println!("IR finished!");

    println!("Compiling...");
    let (executable_buffer, offset) = generate_code(&ctx).unwrap();
    println!("Compilation finished!");
    let hello_fn: extern "C" fn() = unsafe { std::mem::transmute(executable_buffer.ptr(offset)) };

    hello_fn();
}
