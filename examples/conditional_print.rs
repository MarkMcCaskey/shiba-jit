use shiba_jit::{codegen::x86_64::*, ir::*, *};

fn main() {
    let program = vec![
        IR::Label { label_idx: 0 },
        IR::Print {
            value: "Hello, world\n".to_string(),
        },
        IR::Immediate {
            dest_register: 0,
            _type: PrimitiveValue::U32,
            value: 2,
            alignment: 4,
        },
        IR::Immediate {
            dest_register: 1,
            _type: PrimitiveValue::U32,
            value: 2,
            alignment: 4,
        },
        IR::Add {
            dest_register: 2,
            src_register1: 0,
            src_register2: 1,
        },
        IR::Immediate {
            dest_register: 3,
            _type: PrimitiveValue::U32,
            value: 4,
            alignment: 4,
        },
        IR::Subtract {
            dest_register: 4,
            src_register1: 2,
            src_register2: 3,
        },
        IR::JumpIfEqual {
            src_register: 4,
            label_idx: 0,
        },
    ];

    let (executable_buffer, offset) = generate_code(&program).unwrap();
    let hello_fn: extern "C" fn() = unsafe { std::mem::transmute(executable_buffer.ptr(offset)) };

    unsafe { hello_fn() };
}
