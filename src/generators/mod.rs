//! Editable generator stacks and their bounded audio-thread representation.

mod stack;
mod state;

pub use stack::{
    CompiledStack, Group, GroupId, GroupOutput, Instruction, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS,
    Module, ModuleId, ModuleKind, OscillatorSlot, Patch, StackError,
};
pub(crate) use state::GeneratorStackSnapshot;
pub use state::{GeneratorStackState, OscillatorConfig};
