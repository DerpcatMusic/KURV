//! Editable generator stacks and their bounded audio-thread representation.

mod stack;

pub use stack::{
    CompiledStack, Group, GroupId, Instruction, MAX_OSCILLATORS, Module, ModuleId, ModuleKind,
    Patch, StackError,
};
