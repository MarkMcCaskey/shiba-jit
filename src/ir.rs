#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PrimitiveValue {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
}

#[derive(Debug)]
pub struct Register {
    _type: PrimitiveValue,
}

#[derive(Debug)]
pub enum IR {
    Immediate {
        dest_register: usize,
        _type: PrimitiveValue,
        value: usize,
        alignment: u8,
    },
    Add {
        dest_register: usize,
        src_register1: usize,
        src_register2: usize,
    },
    Subtract {
        dest_register: usize,
        src_register1: usize,
        src_register2: usize,
    },
    Multiply {
        dest_register: usize,
        src_register1: usize,
        src_register2: usize,
    },
    Divide {
        dest_register: usize,
        src_register1: usize,
        src_register2: usize,
    },
    /// Src is a pointer that's  dereffed
    Load {
        dest_register: usize,
        src_register: usize,
    },
    /// Dest is a pointer that's dereffed
    Store {
        dest_register: usize,
        src_register: usize,
    },
    Label {
        label_idx: usize,
    },
    JumpIfEqual {
        src_register: usize,
        label_idx: usize,
    },
    JumpIfNotEqual {
        src_register: usize,
        label_idx: usize,
    },
}
