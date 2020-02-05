use shiba_jit::{codegen::x86_64::*, ir::*, *};

fn main() {
    // Context is how we'll build the CFG
    let mut ctx = Context::new();
    // Load constant strings and get a handle to them
    let hello_world_const = ctx.add_constant(b"Hello, world\n");
    let end_const = ctx.add_constant(b"Goodbye, world\n");
    // create all our basic blocks ahead of time (currently the first basic
    // block is implicitly the entry-point)
    let prog_start = ctx.new_basic_block();
    let loop_inner = ctx.new_basic_block();
    let loop_outer = ctx.new_basic_block();
    let loop_exit = ctx.new_basic_block();
    // start building the basic blocks
    // the first block allocates stack space and initializes it
    let prog_start_bb = ctx.build_basic_block(prog_start);
    let counter = prog_start_bb.alloca(PrimitiveValue::U32, 4);
    prog_start_bb.store(counter, Value::u32(0));
    // TODO: look at other APIs; seems like we shouldn't need a jump
    prog_start_bb.jump(loop_inner);

    // inside of the loop, print out the string, update the counter,
    // and evaluate the condition
    let inner_bb = ctx.build_basic_block(loop_inner);
    inner_bb.push_instruction(IR::PrintConstant {
        constant_ref: hello_world_const,
    });
    let loaded_counter = inner_bb.load(counter);
    let add_result = inner_bb.add(loaded_counter, Value::u32(1));
    inner_bb.store(counter, add_result);
    let sub_result = inner_bb.subtract(Value::u32(4), add_result);
    inner_bb.finish();

    // "outer" loop, conditonally jumps back to the inner loop
    ctx.build_basic_block(loop_outer)
        .add_parent(loop_inner)
        .jump_if_equal(sub_result, loop_exit, loop_inner);

    // handle the case of loop termination
    let loop_exit_bb = ctx.build_basic_block(loop_exit);
    loop_exit_bb
        .add_parent(loop_outer)
        .push_instruction(IR::PrintConstant {
            constant_ref: end_const,
        });
    loop_exit_bb.ret();
    loop_exit_bb.finish();

    // we've fully described the program CFG
    ctx.finalize();
    println!("IR finished!");

    println!("Compiling...");
    let (executable_buffer, offset) = generate_code(&ctx).unwrap();
    println!("Compilation finished!");
    let hello_fn: extern "C" fn() = unsafe { std::mem::transmute(executable_buffer.ptr(offset)) };

    im_going_to_break_here(hello_fn);
}

// useful for setting breakpoints to walk through the generated code
#[inline(never)]
#[no_mangle]
fn im_going_to_break_here(f: extern "C" fn()) {
    f()
}
