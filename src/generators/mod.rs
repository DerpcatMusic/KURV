//! Editable generator stacks and their bounded audio-thread representation.

mod stack;
mod state;

pub use stack::{
    CompiledStack, Group, GroupId, Instruction, MAX_OSCILLATORS, Module, ModuleId, ModuleKind,
    OscillatorSlot, Patch, StackError,
};
pub use state::GeneratorStackState;
