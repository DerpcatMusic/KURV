use std::{
    collections::VecDeque,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicPtr, AtomicU64, Ordering, fence},
    },
};

use crate::{
    generators::MAX_OSCILLATORS,
    oscillators::{ResynthAlgorithm, ResynthRtArtifact},
};

/// Exact identity of one immutable realtime publication.
///
/// `{ generation: 0, revision: 0 }` is the only absent identity. Every real
/// artifact, including generation-bearing silence, uses non-zero members.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResynthPublicationIdentity {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
}

impl ResynthPublicationIdentity {
    pub(crate) const NONE: Self = Self {
        generation: 0,
        revision: 0,
    };

    #[must_use]
    pub(crate) const fn is_present(self) -> bool {
        self.generation != 0 && self.revision != 0
    }
}

struct ResynthArtifactNode {
    generation: u64,
    revision: u64,
    artifact: Option<Arc<ResynthRtArtifact>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResynthArtifactView(*const ResynthArtifactNode);

impl ResynthArtifactView {
    pub(crate) const NONE: Self = Self(ptr::null());

    #[must_use]
    pub(crate) fn publication_identity(self) -> ResynthPublicationIdentity {
        if self.0.is_null() {
            ResynthPublicationIdentity::NONE
        } else {
            // SAFETY: the publisher retains current/retired nodes until an RT
            // acknowledgement excludes this generation from the live set.
            unsafe {
                ResynthPublicationIdentity {
                    generation: (*self.0).generation,
                    revision: (*self.0).revision,
                }
            }
        }
    }

    #[must_use]
    pub(crate) fn generation(self) -> u64 {
        self.publication_identity().generation
    }

    /// The Algorithm embodied by this immutable realtime publication.
    #[must_use]
    pub(crate) fn algorithm(self) -> Option<ResynthAlgorithm> {
        if self.0.is_null() {
            None
        } else {
            // SAFETY: the publisher retains current/retired nodes until the
            // audio-owned playback plan acknowledges that it is no longer live.
            unsafe {
                (*self.0)
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.algorithm)
            }
        }
    }

    /// A view onto a leaked node carrying a default artifact, so tests can
    /// build a plan that genuinely `requires_render()` without standing up the
    /// whole publisher. Leaking keeps the node address-stable for the process
    /// lifetime, which is exactly the invariant real publications maintain
    /// until acknowledgement.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn leaked_for_test(algorithm: ResynthAlgorithm) -> Self {
        let artifact = ResynthRtArtifact {
            algorithm,
            ..ResynthRtArtifact::default()
        };
        let node = Box::leak(Box::new(ResynthArtifactNode {
            generation: 1,
            revision: 1,
            artifact: Some(Arc::new(artifact)),
        }));
        Self(std::ptr::from_ref(node))
    }

    /// # Safety
    /// The returned borrow must not escape the render operation covered by the
    /// owning slot's acknowledgement protocol.
    pub(crate) unsafe fn artifact(self) -> Option<&'static ResynthRtArtifact> {
        if self.0.is_null() {
            None
        } else {
            // SAFETY: required from the caller and node is immutable.
            unsafe { (*self.0).artifact.as_deref() }
        }
    }
}

// SAFETY: views are immutable raw handles. Node lifetime is protected by the
// slot publication/acknowledgement protocol; helpers never receive a view.
unsafe impl Send for ResynthArtifactView {}
// SAFETY: the same immutable-node/lifetime contract applies to shared access.
unsafe impl Sync for ResynthArtifactView {}

/// One bounded, epoch-coherent set of publications for the audio callback.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResynthRtUpdate {
    pub(crate) changed_mask: u32,
    pub(crate) views: [ResynthArtifactView; MAX_OSCILLATORS],
}

pub(super) struct RetiredArtifact {
    node: Arc<ResynthArtifactNode>,
    retired_by: u64,
}

pub(super) struct ResynthArtifactOwners {
    current: Option<Arc<ResynthArtifactNode>>,
    pub(super) retired: VecDeque<RetiredArtifact>,
    pub(super) next_generation: u64,
}

/// Audio-owned playback facts published at the final callback point.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResynthRtPlanAck {
    pub(crate) live_generations: [u64; 2],
    pub(crate) accepted: ResynthPublicationIdentity,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ResynthRtAckSnapshot {
    seen_publication: u64,
    plan: ResynthRtPlanAck,
}

struct ResynthRtAck {
    sequence: AtomicU64,
    seen_publication: AtomicU64,
    live: [AtomicU64; 2],
    accepted_generation: AtomicU64,
    accepted_revision: AtomicU64,
}

impl ResynthRtAck {
    fn snapshot(&self) -> ResynthRtAckSnapshot {
        loop {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let seen_publication = self.seen_publication.load(Ordering::Relaxed);
            let live_generations = [
                self.live[0].load(Ordering::Relaxed),
                self.live[1].load(Ordering::Relaxed),
            ];
            let accepted = ResynthPublicationIdentity {
                generation: self.accepted_generation.load(Ordering::Relaxed),
                revision: self.accepted_revision.load(Ordering::Relaxed),
            };
            // Keep the payload reads before the closing validation load.
            fence(Ordering::Acquire);
            let after = self.sequence.load(Ordering::Relaxed);
            if before == after {
                return ResynthRtAckSnapshot {
                    seen_publication,
                    plan: ResynthRtPlanAck {
                        live_generations,
                        accepted,
                    },
                };
            }
        }
    }
}

pub(super) struct AtomicResynthArtifact {
    published: AtomicPtr<ResynthArtifactNode>,
    pub(super) owners: Mutex<ResynthArtifactOwners>,
    ack: ResynthRtAck,
}

impl AtomicResynthArtifact {
    pub(super) fn new() -> Self {
        Self {
            published: AtomicPtr::new(ptr::null_mut()),
            owners: Mutex::new(ResynthArtifactOwners {
                current: None,
                retired: VecDeque::with_capacity(2),
                next_generation: 0,
            }),
            ack: ResynthRtAck {
                sequence: AtomicU64::new(0),
                seen_publication: AtomicU64::new(0),
                live: [AtomicU64::new(0), AtomicU64::new(0)],
                accepted_generation: AtomicU64::new(0),
                accepted_revision: AtomicU64::new(0),
            },
        }
    }

    pub(super) fn store(
        &self,
        revision: u64,
        artifact: Option<Arc<ResynthRtArtifact>>,
    ) -> Option<u64> {
        if revision == 0 {
            return None;
        }
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::collect_locked(&mut owners, self.ack.snapshot());
        if owners.retired.len() >= 2 {
            return None;
        }
        let generation = owners.next_generation.checked_add(1)?;
        owners.next_generation = generation;
        let node = Arc::new(ResynthArtifactNode {
            generation,
            revision,
            artifact,
        });
        if let Some(previous) = owners.current.replace(Arc::clone(&node)) {
            owners.retired.push_back(RetiredArtifact {
                node: previous,
                retired_by: generation,
            });
        }
        self.published
            .store(Arc::as_ptr(&node).cast_mut(), Ordering::Release);
        Some(generation)
    }

    pub(super) fn clear(&self, revision: u64) -> Option<u64> {
        self.store(revision, None)
    }

    pub(super) fn can_store(&self) -> bool {
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::collect_locked(&mut owners, self.ack.snapshot());
        owners.retired.len() < 2 && owners.next_generation < u64::MAX
    }

    pub(super) fn try_view_after(&self, observed: u64) -> Option<ResynthArtifactView> {
        let pointer = self.published.load(Ordering::Acquire);
        if pointer.is_null() {
            return None;
        }
        // SAFETY: current ownership cannot be removed until an acknowledgement
        // observes this publication; this callback has not acknowledged yet.
        let generation = unsafe { (*pointer).generation };
        (generation != observed).then_some(ResynthArtifactView(pointer))
    }

    pub(super) fn published_generation(&self) -> u64 {
        let pointer = self.published.load(Ordering::Acquire);
        if pointer.is_null() {
            0
        } else {
            // SAFETY: the current owner remains retained.
            unsafe { (*pointer).generation }
        }
    }

    pub(super) fn acknowledge(&self, seen: u64, plan: ResynthRtPlanAck) {
        self.ack.sequence.fetch_add(1, Ordering::AcqRel);
        self.ack.seen_publication.store(seen, Ordering::Relaxed);
        self.ack.live[0].store(plan.live_generations[0], Ordering::Relaxed);
        self.ack.live[1].store(plan.live_generations[1], Ordering::Relaxed);
        self.ack
            .accepted_generation
            .store(plan.accepted.generation, Ordering::Relaxed);
        self.ack
            .accepted_revision
            .store(plan.accepted.revision, Ordering::Relaxed);
        self.ack.sequence.fetch_add(1, Ordering::Release);
    }

    pub(super) fn collect(&self) {
        let mut owners = self
            .owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::collect_locked(&mut owners, self.ack.snapshot());
    }

    fn collect_locked(owners: &mut ResynthArtifactOwners, ack: ResynthRtAckSnapshot) {
        owners.retired.retain(|retired| {
            ack.seen_publication < retired.retired_by
                || ack.plan.live_generations.contains(&retired.node.generation)
        });
    }
}
