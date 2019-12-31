use smallvec::SmallVec;
use std::sync::{mpsc, Mutex};

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
pub enum Value {
    Register(RegisterIndex),
    Immediate { _type: PrimitiveValue, value: usize },
}

impl Value {
    pub fn u32(v: u32) -> Self {
        Value::Immediate {
            _type: PrimitiveValue::U32,
            value: v as _,
        }
    }
}

#[derive(Debug)]
pub enum IR {
    Alloca {
        dest_register: RegisterIndex,
        _type: PrimitiveValue,
        alignment: u8,
    },
    Add {
        dest_register: RegisterIndex,
        src1: Value,
        src2: Value,
    },
    Subtract {
        dest_register: RegisterIndex,
        src1: Value,
        src2: Value,
    },
    Multiply {
        dest_register: RegisterIndex,
        src1: Value,
        src2: Value,
    },
    Divide {
        dest_register: RegisterIndex,
        src1: Value,
        src2: Value,
    },
    /// Src is a pointer that's  dereffed
    Load {
        dest_register: Value,
        src_register: Value,
    },
    /// Dest is a pointer that's dereffed
    Store {
        dest_register: Value,
        src_register: Value,
    },
    JumpIfEqual {
        src_register: Value,
        true_bb_idx: BasicBlockIndex,
        false_bb_idx: BasicBlockIndex,
    },
    JumpIfNotEqual {
        src_register: Value,
        true_bb_idx: BasicBlockIndex,
        false_bb_idx: BasicBlockIndex,
    },
    Jump {
        bb_idx: BasicBlockIndex,
    },
    PrintConstant {
        constant_ref: ConstantIndex,
    },
}

/// Top level type to generate IR with
#[derive(Debug)]
pub struct Context {
    /// Global constants
    pub(crate) constants: Vec<Vec<u8>>,
    // TODO: add global variables here
    /// The basic block / CFG
    basic_blocks: BasicBlockManager,
}

impl Context {
    pub fn new() -> Context {
        Self {
            constants: vec![],
            basic_blocks: BasicBlockManager::new(),
        }
    }

    pub fn add_constant(&mut self, constant: &[u8]) -> ConstantIndex {
        self.constants.push(constant.to_vec());
        ConstantIndex(self.constants.len() - 1)
    }

    // TODO: revisit types
    pub fn get_constant(&self, ci: ConstantIndex) -> Option<&Vec<u8>> {
        self.constants.get(ci.0)
    }

    pub fn new_basic_block(&mut self) -> BasicBlockIndex {
        self.basic_blocks.new_basic_block()
    }

    pub fn build_basic_block(&mut self, bi: BasicBlockIndex) -> &mut BasicBlock {
        self.basic_blocks.get_mut(bi).unwrap()
    }

    pub fn finalize(&mut self) {
        self.basic_blocks.finalize();
        crate::reg_alloc::compute_graph(&self.basic_blocks);
    }

    pub(crate) fn iterate_basic_blocks(
        &self,
    ) -> impl Iterator<Item = (BasicBlockIndex, &BasicBlock)> {
        self.basic_blocks.iterate_basic_blocks()
    }
}

// TODO: maybe use an atomic here or think about data flow and avoid a global
lazy_static! {
    static ref LAST_REGISTER: Mutex<usize> = Mutex::new(0);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BasicBlockMessage {
    /// A Jump from the first index to the second occured.
    ///
    /// The manager will want to update the target's entry points to include the
    /// first.
    Jump(BasicBlockIndex, BasicBlockIndex),
}

/// Node in the control flow graph; core unit; straight line code
#[derive(Debug)]
pub struct BasicBlock {
    /// Pointers to basic blocks that may call into this one
    /// TODO: use fancier types here
    parents: SmallVec<[BasicBlockIndex; 2]>,
    /// Exits from this basic block
    /// TODO: use fancier types here
    exits: SmallVec<[BasicBlockIndex; 2]>,
    code: Vec<IR>,
    /// Its own index, used due to [`BasicBlockMessage`]
    self_idx: BasicBlockIndex,
    /// A bit of a hack to allow things like `jump` to exist on `BasicBlock`:
    /// we need to bidirectionally update both the src and target.
    ///
    /// NOTE: this is a bit hacky, I think it's justified at the time of writing
    /// because it will help keep the public API simple.  This should be reevaluated
    /// later though.
    manager_chan: mpsc::Sender<BasicBlockMessage>,
}

impl BasicBlock {
    pub fn add_parent(&mut self, parent: BasicBlockIndex) -> &mut Self {
        self.parents.push(parent);
        self
    }
    /// TODO: remove this and replace it with a method for each instruction to make a nicer API
    pub fn push_instruction(&mut self, inst: IR) -> &mut Self {
        self.code.push(inst);
        self
    }

    pub(crate) fn iter_parents(&self) -> impl Iterator<Item = &BasicBlockIndex> {
        self.parents.iter()
    }
    pub(crate) fn iter_exits(&self) -> impl Iterator<Item = &BasicBlockIndex> {
        self.exits.iter()
    }

    pub fn finish(&mut self) {}

    pub(crate) fn iterate_instructions(&self) -> impl Iterator<Item = &IR> {
        self.code.iter()
    }

    pub fn add(&mut self, v1: Value, v2: Value) -> Value {
        let n = {
            let mut lr = LAST_REGISTER.lock().unwrap();
            *lr += 1;
            *lr
        };
        let ri = RegisterIndex(n);
        self.code.push(IR::Add {
            dest_register: ri,
            src1: v1,
            src2: v2,
        });
        Value::Register(ri)
    }

    pub fn subtract(&mut self, v1: Value, v2: Value) -> Value {
        let n = {
            let mut lr = LAST_REGISTER.lock().unwrap();
            *lr += 1;
            *lr
        };
        let ri = RegisterIndex(n);
        self.code.push(IR::Subtract {
            dest_register: ri,
            src1: v1,
            src2: v2,
        });
        Value::Register(ri)
    }

    pub fn jump(&mut self, target: BasicBlockIndex) {
        self.exits.push(target);
        self.code.push(IR::Jump { bb_idx: target });
        self.manager_chan
            .send(BasicBlockMessage::Jump(self.self_idx, target))
            .unwrap();
    }
}

#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
#[repr(transparent)]
pub struct ConstantIndex(usize);

impl ConstantIndex {
    // TODO: probably remove this and create an iterator on them directly
    pub(crate) fn new(inner: usize) -> Self {
        Self(inner)
    }
}

#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
#[repr(transparent)]
pub struct BasicBlockIndex(usize);

#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegisterIndex(usize);

// TODO: get dominance tree (find blocks that are coupled (i.e. x dominates y if all paths to y include x))
// DFS on the tree
// def-use chain (list of uses of variables)
#[derive(Debug)]
pub struct BasicBlockManager {
    pub(crate) start: BasicBlockIndex,
    blocks: Vec<BasicBlock>,
    /// Messages from the [`BasicBlock`]s, used to apply changes without lots of
    /// mutable and cyclic pointers.
    message_recv: mpsc::Receiver<BasicBlockMessage>,
    /// only held on to for the `new_basic_block` method
    message_sender: mpsc::Sender<BasicBlockMessage>,
}

impl BasicBlockManager {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            start: BasicBlockIndex(0),
            blocks: vec![],
            message_recv: rx,
            message_sender: tx,
        }
    }

    fn process_messages(&mut self) {
        for message in self.message_recv.try_iter() {
            match message {
                BasicBlockMessage::Jump(src, target) => {
                    self.blocks[target.0].add_parent(src);
                }
            }
        }
    }

    #[allow(unreachable_code)]
    pub fn is_valid(&self) -> bool {
        todo!("Check start index exists");
        todo!("Check all basic blocks point to only valid basic blocks");
        todo!("Call into basic block validate method which checks invariants of basic blocks");
        todo!("Check connectivity of basic block");
    }

    pub fn new_basic_block(&mut self) -> BasicBlockIndex {
        self.process_messages();
        let idx = self.blocks.len();
        self.blocks.push(BasicBlock {
            parents: Default::default(),
            exits: Default::default(),
            code: Default::default(),
            self_idx: BasicBlockIndex(idx),
            manager_chan: self.message_sender.clone(),
        });

        BasicBlockIndex(self.blocks.len() - 1)
    }

    // TODO: probably don't expose this
    /// get the manager ready for further processing
    pub fn finalize(&mut self) {
        self.process_messages();
    }

    pub fn get_mut(&mut self, bi: BasicBlockIndex) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(bi.0)
    }

    pub(crate) fn iterate_basic_blocks(
        &self,
    ) -> impl Iterator<Item = (BasicBlockIndex, &BasicBlock)> {
        self.blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (BasicBlockIndex(i), b))
    }
}

/* Register usage detection on basic block:

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
*/

/* x86_64 register allocation, using above data:

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
*/
