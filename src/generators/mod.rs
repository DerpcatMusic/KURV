//! Editable generator stacks and their bounded audio-thread representation.

mod stack;
mod state;

pub use crate::filters::{FilterConfig, FilterMode};
pub use stack::{
    AuxSlot, CompiledStack, FilterSlot, Group, GroupId, GroupOutput, Instruction, MAX_AUX_MODULES,
    MAX_FILTERS, MAX_GENERATOR_MODULES, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, Module, ModuleId,
    ModuleKind, OscillatorSlot, Patch, StackError,
};
pub use state::{
    AuxConfig, AuxSource, GeneratorModMode, GeneratorRtGroup, GeneratorRtModule,
    GeneratorRtSnapshot, GeneratorStackState, OscillatorConfig, OscillatorEngineKind,
    OscillatorTuningMode,
};
pub(crate) use state::{GeneratorHistoryStamp, GeneratorStackSnapshot};
