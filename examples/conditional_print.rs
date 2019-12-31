use shiba_jit::{codegen::x86_64::*, ir::*, *};

fn main() {
    let mut ctx = Context::new();
    let hello_world_const = ctx.add_constant(b"Hello, world\n");
    let loop_inner = ctx.new_basic_block();
    let loop_outer = ctx.new_basic_block();
    let inner_bb = ctx.build_basic_block(loop_inner);
    inner_bb.push_instruction(IR::PrintConstant {
        constant_ref: hello_world_const,
    });
    let add_result = inner_bb.add(Value::u32(2), Value::u32(2));
    inner_bb.subtract(add_result, Value::u32(4));
    inner_bb.finish();

    ctx.build_basic_block(loop_outer)
        .add_parent(loop_inner)
        .jump(loop_inner);
    ctx.finalize();
    println!("IR finished!");

    println!("Compiling...");
    let (executable_buffer, offset) = generate_code(&ctx).unwrap();
    println!("Compilation finished!");
    let hello_fn: extern "C" fn() = unsafe { std::mem::transmute(executable_buffer.ptr(offset)) };

    hello_fn();
}
