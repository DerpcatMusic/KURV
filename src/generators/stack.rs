//! Generator ordering lives here; generator DSP does not.
//!
//! [`Patch`] is flexible editor/state-side storage. [`CompiledStack`] is the
//! fixed-capacity instruction sequence intended for later audio-thread
//! publication. Compiling walks `Vec`s and must therefore happen off the audio
//! thread; executing the returned instruction slice requires no allocation or
//! locking.

/// Maximum number of oscillator modules in a patch.
pub const MAX_OSCILLATORS: usize = 32;

/// Stable storage slot for one oscillator's settings and DSP state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OscillatorSlot(u8);

impl OscillatorSlot {
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        let encoded = u8::try_from(index).ok()?;
        (index < MAX_OSCILLATORS).then_some(Self(encoded))
    }

    /// Returns the zero-based oscillator storage index.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    pub(crate) const fn encoded(self) -> u8 {
        self.0
    }
}

/// Stable identity of a generator group.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GroupId(u64);

impl GroupId {
    /// Returns the persistent numeric identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable identity of a generator module.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleId(u64);

impl ModuleId {
    /// Returns the persistent numeric identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The module kinds currently supported by a generator stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModuleKind {
    Oscillator(OscillatorSlot),
    Filter,
}

/// One identity-bearing module in a group stack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Module {
    id: ModuleId,
    kind: ModuleKind,
}

impl Module {
    #[must_use]
    pub const fn id(&self) -> ModuleId {
        self.id
    }

    #[must_use]
    pub const fn kind(&self) -> ModuleKind {
        self.kind
    }

    #[must_use]
    pub const fn oscillator_slot(&self) -> Option<OscillatorSlot> {
        match self.kind {
            ModuleKind::Oscillator(slot) => Some(slot),
            ModuleKind::Filter => None,
        }
    }
}

/// An ordered module stack with one implicit shared output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    id: GroupId,
    modules: Vec<Module>,
}

impl Group {
    #[must_use]
    pub const fn id(&self) -> GroupId {
        self.id
    }

    #[must_use]
    pub fn modules(&self) -> &[Module] {
        &self.modules
    }
}

/// An editable patch document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Patch {
    groups: Vec<Group>,
    next_group_id: u64,
    next_module_id: u64,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            groups: vec![Group {
                id: GroupId(1),
                modules: vec![Module {
                    id: ModuleId(1),
                    kind: ModuleKind::Oscillator(OscillatorSlot(0)),
                }],
            }],
            next_group_id: 2,
            next_module_id: 2,
        }
    }
}

impl Patch {
    /// Creates the default one-group, one-oscillator patch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub(crate) const fn next_group_id(&self) -> u64 {
        self.next_group_id
    }

    pub(crate) const fn next_module_id(&self) -> u64 {
        self.next_module_id
    }

    #[must_use]
    pub fn oscillator_count(&self) -> usize {
        self.groups
            .iter()
            .flat_map(|group| &group.modules)
            .filter(|module| matches!(module.kind, ModuleKind::Oscillator(_)))
            .count()
    }

    #[must_use]
    pub fn contains_oscillator_slot(&self, slot: OscillatorSlot) -> bool {
        self.groups.iter().any(|group| {
            group
                .modules
                .iter()
                .any(|module| module.kind == ModuleKind::Oscillator(slot))
        })
    }

    /// Validates the patch-wide limits.
    pub fn validate(&self) -> Result<(), StackError> {
        let count = self.oscillator_count();
        if count > MAX_OSCILLATORS {
            return Err(StackError::OscillatorLimit {
                count,
                max: MAX_OSCILLATORS,
            });
        }
        Ok(())
    }

    /// Inserts an empty group. Its output is implicit and cannot be removed.
    pub fn insert_group(&mut self, index: usize) -> Result<GroupId, StackError> {
        if index > self.groups.len() {
            return Err(StackError::IndexOutOfBounds {
                index,
                len: self.groups.len(),
            });
        }
        let id = GroupId(self.take_group_id()?);
        self.groups.insert(
            index,
            Group {
                id,
                modules: Vec::new(),
            },
        );
        Ok(id)
    }

    pub fn remove_group(&mut self, id: GroupId) -> Result<Group, StackError> {
        let index = self
            .group_position(id)
            .ok_or(StackError::GroupNotFound(id))?;
        if self.groups.len() == 1 {
            return Err(StackError::CannotRemoveLastGroup);
        }
        Ok(self.groups.remove(index))
    }

    /// Moves a group to its final index without changing its identity.
    pub fn move_group(&mut self, id: GroupId, index: usize) -> Result<(), StackError> {
        let old_index = self
            .group_position(id)
            .ok_or(StackError::GroupNotFound(id))?;
        if index >= self.groups.len() {
            return Err(StackError::IndexOutOfBounds {
                index,
                len: self.groups.len(),
            });
        }
        if old_index != index {
            let group = self.groups.remove(old_index);
            self.groups.insert(index, group);
        }
        Ok(())
    }

    pub fn insert_oscillator(
        &mut self,
        group_id: GroupId,
        index: usize,
    ) -> Result<ModuleId, StackError> {
        let slot = (0..MAX_OSCILLATORS)
            .filter_map(OscillatorSlot::from_index)
            .find(|slot| !self.contains_oscillator_slot(*slot))
            .ok_or(StackError::OscillatorLimit {
                count: MAX_OSCILLATORS + 1,
                max: MAX_OSCILLATORS,
            })?;
        self.insert_oscillator_with_slot(group_id, index, slot)
    }

    pub(crate) fn insert_oscillator_with_slot(
        &mut self,
        group_id: GroupId,
        index: usize,
        slot: OscillatorSlot,
    ) -> Result<ModuleId, StackError> {
        if self.contains_oscillator_slot(slot) {
            return Err(StackError::DuplicateOscillatorSlot(slot));
        }
        self.insert_module(group_id, index, ModuleKind::Oscillator(slot))
    }

    pub fn insert_filter(
        &mut self,
        group_id: GroupId,
        index: usize,
    ) -> Result<ModuleId, StackError> {
        self.insert_module(group_id, index, ModuleKind::Filter)
    }

    fn insert_module(
        &mut self,
        group_id: GroupId,
        index: usize,
        kind: ModuleKind,
    ) -> Result<ModuleId, StackError> {
        let group_index = self
            .group_position(group_id)
            .ok_or(StackError::GroupNotFound(group_id))?;
        if index > self.groups[group_index].modules.len() {
            return Err(StackError::IndexOutOfBounds {
                index,
                len: self.groups[group_index].modules.len(),
            });
        }
        let id = ModuleId(self.take_module_id()?);
        self.groups[group_index]
            .modules
            .insert(index, Module { id, kind });
        Ok(id)
    }

    pub fn remove_module(&mut self, id: ModuleId) -> Result<Module, StackError> {
        let (group_index, module_index) = self
            .module_position(id)
            .ok_or(StackError::ModuleNotFound(id))?;
        Ok(self.groups[group_index].modules.remove(module_index))
    }

    /// Moves a module within or between groups without changing its identity.
    pub fn move_module(
        &mut self,
        id: ModuleId,
        destination: GroupId,
        index: usize,
    ) -> Result<(), StackError> {
        let (source_group, source_index) = self
            .module_position(id)
            .ok_or(StackError::ModuleNotFound(id))?;
        let destination_group = self
            .group_position(destination)
            .ok_or(StackError::GroupNotFound(destination))?;
        let destination_len = self.groups[destination_group].modules.len();
        let valid = if source_group == destination_group {
            index < destination_len
        } else {
            index <= destination_len
        };
        if !valid {
            return Err(StackError::IndexOutOfBounds {
                index,
                len: destination_len,
            });
        }
        if source_group != destination_group || source_index != index {
            let module = self.groups[source_group].modules.remove(source_index);
            self.groups[destination_group].modules.insert(index, module);
        }
        Ok(())
    }

    /// Compiles downstream stack semantics into a caller-selected fixed bound.
    ///
    /// `BeginGroup` starts a fresh accumulator. Oscillators add to it, filters
    /// transform the whole accumulator, and `GroupOutput` publishes it without
    /// routing it into the next group.
    pub fn compile<const CAPACITY: usize>(&self) -> Result<CompiledStack<CAPACITY>, StackError> {
        self.validate()?;
        let required = self
            .groups
            .iter()
            .map(|group| group.modules.len() + 2)
            .sum();
        if required > CAPACITY {
            return Err(StackError::InstructionCapacity {
                required,
                capacity: CAPACITY,
            });
        }

        let mut instructions = [EMPTY_INSTRUCTION; CAPACITY];
        let mut len = 0;
        for group in &self.groups {
            instructions[len] = Instruction::BeginGroup { group_id: group.id };
            len += 1;
            for module in &group.modules {
                instructions[len] = match module.kind {
                    ModuleKind::Oscillator(slot) => Instruction::Oscillator {
                        group_id: group.id,
                        module_id: module.id,
                        slot,
                    },
                    ModuleKind::Filter => Instruction::Filter {
                        group_id: group.id,
                        module_id: module.id,
                    },
                };
                len += 1;
            }
            instructions[len] = Instruction::GroupOutput { group_id: group.id };
            len += 1;
        }
        Ok(CompiledStack { instructions, len })
    }

    fn group_position(&self, id: GroupId) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    fn module_position(&self, id: ModuleId) -> Option<(usize, usize)> {
        self.groups.iter().enumerate().find_map(|(group, stack)| {
            stack
                .modules
                .iter()
                .position(|module| module.id == id)
                .map(|module| (group, module))
        })
    }

    fn take_group_id(&mut self) -> Result<u64, StackError> {
        let id = self.next_group_id;
        self.next_group_id = id.checked_add(1).ok_or(StackError::IdExhausted)?;
        Ok(id)
    }

    fn take_module_id(&mut self) -> Result<u64, StackError> {
        let id = self.next_module_id;
        self.next_module_id = id.checked_add(1).ok_or(StackError::IdExhausted)?;
        Ok(id)
    }

    pub(crate) fn restore(
        groups: Vec<(u64, Vec<(u64, ModuleKind)>)>,
        next_group_id: u64,
        next_module_id: u64,
    ) -> Result<Self, StackError> {
        if groups.is_empty() {
            return Err(StackError::CannotRemoveLastGroup);
        }
        let mut group_ids = Vec::with_capacity(groups.len());
        let mut module_ids = Vec::new();
        let mut oscillator_slots = Vec::new();
        let mut restored_groups = Vec::with_capacity(groups.len());

        for (group_id, modules) in groups {
            if group_id == 0 || group_ids.contains(&group_id) {
                return Err(StackError::InvalidPersistentIdentity);
            }
            group_ids.push(group_id);
            let mut restored_modules = Vec::with_capacity(modules.len());
            for (module_id, kind) in modules {
                if module_id == 0 || module_ids.contains(&module_id) {
                    return Err(StackError::InvalidPersistentIdentity);
                }
                if let ModuleKind::Oscillator(slot) = kind {
                    if oscillator_slots.contains(&slot) {
                        return Err(StackError::DuplicateOscillatorSlot(slot));
                    }
                    oscillator_slots.push(slot);
                }
                module_ids.push(module_id);
                restored_modules.push(Module {
                    id: ModuleId(module_id),
                    kind,
                });
            }
            restored_groups.push(Group {
                id: GroupId(group_id),
                modules: restored_modules,
            });
        }

        let highest_group_id = group_ids.into_iter().max().unwrap_or(0);
        let highest_module_id = module_ids.into_iter().max().unwrap_or(0);
        let patch = Self {
            groups: restored_groups,
            next_group_id: next_group_id.max(highest_group_id.saturating_add(1)),
            next_module_id: next_module_id.max(highest_module_id.saturating_add(1)),
        };
        patch.validate()?;
        Ok(patch)
    }
}

/// One operation in the allocation-free execution sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Instruction {
    BeginGroup {
        group_id: GroupId,
    },
    Oscillator {
        group_id: GroupId,
        module_id: ModuleId,
        slot: OscillatorSlot,
    },
    Filter {
        group_id: GroupId,
        module_id: ModuleId,
    },
    GroupOutput {
        group_id: GroupId,
    },
}

const EMPTY_INSTRUCTION: Instruction = Instruction::BeginGroup {
    group_id: GroupId(0),
};

/// A fixed-capacity instruction snapshot suitable for RT-safe publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledStack<const CAPACITY: usize> {
    instructions: [Instruction; CAPACITY],
    len: usize,
}

impl<const CAPACITY: usize> CompiledStack<CAPACITY> {
    /// Returns only initialized instructions; the backing storage remains fixed.
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions[..self.len]
    }
}

/// A rejected patch edit or compilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackError {
    GroupNotFound(GroupId),
    ModuleNotFound(ModuleId),
    IndexOutOfBounds { index: usize, len: usize },
    OscillatorLimit { count: usize, max: usize },
    DuplicateOscillatorSlot(OscillatorSlot),
    InvalidPersistentIdentity,
    CannotRemoveLastGroup,
    IdExhausted,
    InstructionCapacity { required: usize, capacity: usize },
}
