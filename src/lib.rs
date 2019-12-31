#![feature(proc_macro_hygiene)]

#[macro_use]
extern crate dynasm;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate smallvec;

pub mod codegen;
pub mod ir;
pub mod reg_alloc;
