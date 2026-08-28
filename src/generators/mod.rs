//! Editable generator stacks and their bounded audio-thread representation.

mod stack;
mod state;

pub use crate::filters::{FilterConfig, FilterMode};
pub use stack::{
    CompiledStack, FilterSlot, Group, GroupId, GroupOutput, Instruction, MAX_FILTERS,
    MAX_GENERATOR_MODULES, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, Module, ModuleId, ModuleKind,
    OscillatorSlot, Patch, StackError,
};
pub(crate) use state::{GeneratorHistoryStamp, GeneratorStackSnapshot};
pub use state::{
    GeneratorRtGroup, GeneratorRtModule, GeneratorRtSnapshot, GeneratorStackState,
    GeneratorModMode, OscillatorConfig, OscillatorEngineKind,
};
