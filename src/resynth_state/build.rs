use super::*;

pub(super) enum ResynthBuildKind {
    Analyzed {
        model: Arc<ResynthAnalysisModel>,
        selected: ResynthAlgorithm,
        controls: ResynthControls,
    },
    Rebuild {
        file_name: String,
        bytes: Vec<u8>,
        selected: ResynthAlgorithm,
        controls: ResynthControls,
        root_override_hz: Option<f32>,
    },
}

pub(super) struct ResynthBuildJob {
    revision: u64,
    kind: ResynthBuildKind,
}

#[derive(Clone)]
pub(super) struct ResynthDesiredSpec {
    pub(super) revision: u64,
    pub(super) file_name: String,
    pub(super) bytes: Arc<[u8]>,
    pub(super) selected: ResynthAlgorithm,
    pub(super) controls: ResynthControls,
    pub(super) root_override_hz: Option<f32>,
    pub(super) detected_root_hz: Option<f32>,
    pub(super) sample_rate: u32,
    pub(super) channels: u16,
    pub(super) frames: u32,
    pub(super) pitch_confidence: f32,
    pub(super) visuals: Arc<crate::oscillators::ResynthVisualModel>,
}

pub(super) enum ResynthSerializableSnapshot {
    Committed(Option<ResynthSlotDocument>),
    Desired(ResynthDesiredSpec),
}

pub(super) struct CompletedResynthBuild {
    pub(super) revision: u64,
    pub(super) selected: ResynthAlgorithm,
    pub(super) controls: ResynthControls,
    pub(super) model: Arc<ResynthAnalysisModel>,
    pub(super) artifact: Arc<ResynthRtArtifact>,
    pub(super) artifact_visuals: Arc<AlgorithmVisualCache>,
}

pub(super) enum PendingResynthCommit {
    Artifact(CompletedResynthBuild),
    Clear { revision: u64 },
}

impl PendingResynthCommit {
    fn revision(&self) -> u64 {
        match self {
            Self::Artifact(completed) => completed.revision,
            Self::Clear { revision } => *revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingCommitResult {
    Empty,
    Stale,
    Backpressured,
    Committed(u64),
}

impl ResynthSlotState {
    /// Allocate a non-zero document revision. Every caller holds `document`'s
    /// write side, which serializes revision allocation with aggregate restore.
    pub(super) fn next_desired_revision(&self) -> Option<u64> {
        let previous = self
            .desired_revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |revision| {
                revision.checked_add(1)
            })
            .ok()?;
        previous.checked_add(1)
    }

    pub(super) fn desired_spec_for_document(document: &ResynthSlotDocument) -> ResynthDesiredSpec {
        ResynthDesiredSpec {
            revision: document.revision,
            file_name: document.model.source.file_name.clone(),
            bytes: Arc::from(document.model.source.original_bytes.clone()),
            selected: document.selected,
            controls: document.controls,
            root_override_hz: document.model.root_override_hz,
            detected_root_hz: document.model.source.estimated_root_hz,
            sample_rate: document.model.source.sample_rate,
            channels: document.model.source.channels,
            frames: document.model.source.frames,
            pitch_confidence: document.model.source.pitch_confidence,
            visuals: Arc::clone(&document.model.visuals),
        }
    }

    pub fn replace(
        &self,
        model: ResynthAnalysisModel,
        selected: ResynthAlgorithm,
        controls: ResynthControls,
    ) -> Result<u64, crate::oscillators::ResynthImportError> {
        let source_name_bytes = model.source.file_name.len();
        if source_name_bytes > crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES {
            return Err(crate::oscillators::ResynthImportError::SourceNameTooLong {
                bytes: source_name_bytes,
                limit: crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES,
            });
        }
        let _work = acquire_resynth_analysis_work();
        let artifact = Arc::new(compile_rt_artifact_with_cancel(
            &model,
            selected,
            controls,
            || false,
        )?);
        let artifact_visuals = analyze_sounding_artifact_visuals(&artifact);
        let mut stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.rt.can_store() {
            return Err(crate::oscillators::ResynthImportError::PublicationBusy);
        }
        let revision = self
            .next_desired_revision()
            .ok_or(crate::oscillators::ResynthImportError::PublicationBusy)?;
        self.build_status.set_progress(70);
        let generation = self
            .rt
            .store(revision, Some(Arc::clone(&artifact)))
            .unwrap_or_else(|| unreachable!("RESYNTH store violated its document-gated preflight"));
        self.store_live_controls(controls);
        *stored = Some(ResynthSlotDocument {
            revision,
            selected,
            controls: controls.sanitized(),
            model: Arc::new(model),
            artifact: Arc::clone(&artifact),
            artifact_visuals,
            artifact_generation: generation,
        });
        *self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            stored.as_ref().map(Self::desired_spec_for_document);
        // Pointer publication is not audio acceptance. The callback advances
        // `sounding_revision` only after this node becomes `plan.to`.
        self.build_status.set_progress(99);
        Ok(generation)
    }

    pub fn select_algorithm(
        &self,
        selected: ResynthAlgorithm,
    ) -> Result<Option<u64>, crate::oscillators::ResynthImportError> {
        let (model, controls) = {
            let stored = self
                .document
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(document) = stored.as_ref() else {
                return Ok(None);
            };
            (document.model.clone(), document.controls)
        };
        let _work = acquire_resynth_analysis_work();
        let artifact = Arc::new(compile_rt_artifact_with_cancel(
            &model,
            selected,
            controls,
            || false,
        )?);
        let artifact_visuals = analyze_sounding_artifact_visuals(&artifact);
        let mut stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(document) = stored.as_mut() else {
            return Ok(None);
        };
        if !self.rt.can_store() {
            return Err(crate::oscillators::ResynthImportError::PublicationBusy);
        }
        let revision = self
            .next_desired_revision()
            .ok_or(crate::oscillators::ResynthImportError::PublicationBusy)?;
        self.build_status.set_progress(70);
        let generation = self
            .rt
            .store(revision, Some(Arc::clone(&artifact)))
            .unwrap_or_else(|| unreachable!("RESYNTH store violated its document-gated preflight"));
        document.revision = revision;
        document.selected = selected;
        document.artifact = artifact;
        document.artifact_visuals = artifact_visuals;
        document.artifact_generation = generation;
        *self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Self::desired_spec_for_document(document));
        self.build_status.set_progress(99);
        Ok(Some(generation))
    }

    pub fn clear(&self) {
        self.reset_source_audition();
        let stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let mut desired = self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let has_intent = stored.is_some()
            || desired.is_some()
            || self.desired_revision.load(Ordering::Acquire)
                != self.sounding_revision.load(Ordering::Acquire);
        if !has_intent {
            return;
        }
        let Some(revision) = self.next_desired_revision() else {
            return;
        };
        *self
            .pending_build
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        *desired = None;
        *self
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(PendingResynthCommit::Clear { revision });
        self.build_status.set_progress(99);
        drop(desired);
        drop(stored);

        match self.try_commit_pending() {
            PendingCommitResult::Backpressured => self.schedule_pending_retry(),
            PendingCommitResult::Empty
            | PendingCommitResult::Stale
            | PendingCommitResult::Committed(_) => {}
        }
    }

    pub fn rebuild_controls(
        &self,
        controls: ResynthControls,
    ) -> Result<Option<u64>, crate::oscillators::ResynthImportError> {
        let (file_name, bytes, selected, root_override_hz) = {
            let stored = self
                .document
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(document) = stored.as_ref() else {
                return Ok(None);
            };
            (
                document.model.source.file_name.clone(),
                document.model.source.original_bytes.clone(),
                document.selected,
                document.model.root_override_hz,
            )
        };
        let model = crate::oscillators::analyze_wav_with_root_override(
            file_name,
            bytes,
            controls,
            root_override_hz,
        )?;
        self.replace(model, selected, controls).map(Some)
    }

    /// Queue a document-side rebuild. The currently committed artifact remains
    /// sounding until this revision completes; stale completions are discarded.
    pub(super) fn committed_desired_spec(&self) -> Option<ResynthDesiredSpec> {
        let stored = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        stored.as_ref().map(Self::desired_spec_for_document)
    }

    fn queue_desired_update(
        self: &Arc<Self>,
        reuse_analysis: bool,
        update: impl FnOnce(&mut ResynthDesiredSpec),
    ) -> Option<u64> {
        if !detached_resynth_work_is_safe() {
            return None;
        }
        // All desired-spec mutation and revision allocation happens under the
        // document gate. Aggregate restore takes every such gate, so its
        // accepted-revision baseline cannot miss an intent that becomes visible
        // later through `desired_spec`.
        let publication_guard = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut desired = self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut spec = desired.clone().or_else(|| {
            publication_guard
                .as_ref()
                .map(Self::desired_spec_for_document)
        })?;
        let reusable_model = reuse_analysis
            .then(|| publication_guard.as_ref())
            .flatten()
            .map(|document| Arc::clone(&document.model));
        update(&mut spec);
        let revision = self.next_desired_revision()?;
        spec.revision = revision;
        *desired = Some(spec.clone());
        self.build_status.set_progress(1);
        *self
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        drop(desired);
        drop(publication_guard);
        let job = ResynthBuildJob {
            revision,
            kind: reusable_model.map_or_else(
                || ResynthBuildKind::Rebuild {
                    file_name: spec.file_name,
                    bytes: spec.bytes.to_vec(),
                    selected: spec.selected,
                    controls: spec.controls,
                    root_override_hz: spec.root_override_hz,
                },
                |model| ResynthBuildKind::Analyzed {
                    model,
                    selected: spec.selected,
                    controls: spec.controls,
                },
            ),
        };
        self.enqueue_build(job).then_some(revision)
    }

    pub(super) fn request_import(
        self: &Arc<Self>,
        model: ResynthAnalysisModel,
        selected: ResynthAlgorithm,
        controls: ResynthControls,
    ) -> Option<u64> {
        if model.source.file_name.len() > crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES
            || !detached_resynth_work_is_safe()
        {
            return None;
        }
        let controls = controls.sanitized();
        let publication_guard = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let revision = self.next_desired_revision()?;
        *self
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        *self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ResynthDesiredSpec {
            revision,
            file_name: model.source.file_name.clone(),
            bytes: Arc::from(model.source.original_bytes.clone()),
            selected,
            controls,
            root_override_hz: model.root_override_hz,
            detected_root_hz: model.source.estimated_root_hz,
            sample_rate: model.source.sample_rate,
            channels: model.source.channels,
            frames: model.source.frames,
            pitch_confidence: model.source.pitch_confidence,
            visuals: Arc::clone(&model.visuals),
        });
        self.build_status.set_progress(60);
        drop(publication_guard);
        let job = ResynthBuildJob {
            revision,
            kind: ResynthBuildKind::Analyzed {
                model: Arc::new(model),
                selected,
                controls,
            },
        };
        self.enqueue_build(job).then_some(revision)
    }

    pub fn request_rebuild(self: &Arc<Self>, controls: ResynthControls) -> Option<u64> {
        let revision = self.queue_desired_update(true, |spec| spec.controls = controls.sanitized());
        *self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        revision
    }

    pub fn request_analysis_rebuild(self: &Arc<Self>) -> Option<u64> {
        let revision = self.queue_desired_update(false, |_| {});
        *self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        revision
    }

    pub fn request_root_override(
        self: &Arc<Self>,
        root_override_hz: Option<f32>,
    ) -> Result<Option<u64>, crate::oscillators::ResynthImportError> {
        if root_override_hz
            .is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root))
        {
            return Err(crate::oscillators::ResynthImportError::NoStablePitch);
        }
        let current = {
            self.desired_spec
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
        .or_else(|| self.committed_desired_spec());
        let Some(current) = current else {
            return Ok(None);
        };
        if root_override_hz.is_none()
            && current.detected_root_hz.is_none()
            && matches!(
                current.selected,
                ResynthAlgorithm::Sample | ResynthAlgorithm::Rich
            )
        {
            return Err(crate::oscillators::ResynthImportError::NoStablePitch);
        }
        let revision = self.queue_desired_update(false, |spec| {
            spec.root_override_hz = root_override_hz;
        });
        *self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(revision)
    }

    /// Queue an Algorithm change without invalidating the last valid artifact.
    pub fn request_algorithm(self: &Arc<Self>, selected: ResynthAlgorithm) -> Option<u64> {
        let desired = {
            self.desired_spec
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
        .or_else(|| self.committed_desired_spec())?;
        if matches!(selected, ResynthAlgorithm::Sample | ResynthAlgorithm::Rich)
            && desired.root_override_hz.is_none()
            && desired.detected_root_hz.is_none()
        {
            return None;
        }
        if let Some(revision) = self.activate_cached_algorithm(selected) {
            return Some(revision);
        }
        self.queue_desired_update(true, |spec| spec.selected = selected)
    }

    fn activate_cached_algorithm(&self, selected: ResynthAlgorithm) -> Option<u64> {
        let mut stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let document = stored.as_mut()?;
        let mut cache = self
            .algorithm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = cache.take()?;
        if cached.selected != selected
            || cached.source_digest != document.model.visuals.source_digest()
            || !self.rt.can_store()
        {
            *cache = Some(cached);
            return None;
        }
        let Some(revision) = self.next_desired_revision() else {
            *cache = Some(cached);
            return None;
        };
        let generation = self
            .rt
            .store(revision, Some(Arc::clone(&cached.artifact)))
            .unwrap_or_else(|| unreachable!("cached RESYNTH activation violated its preflight"));
        *cache = Some(CachedResynthAlgorithm {
            selected: document.selected,
            source_digest: document.model.visuals.source_digest(),
            artifact: Arc::clone(&document.artifact),
            artifact_visuals: Arc::clone(&document.artifact_visuals),
        });
        document.revision = revision;
        document.selected = selected;
        document.artifact = cached.artifact;
        document.artifact_visuals = cached.artifact_visuals;
        document.artifact_generation = generation;
        *self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Self::desired_spec_for_document(document));
        self.build_status.set_progress(99);
        Some(revision)
    }

    fn enqueue_build(self: &Arc<Self>, job: ResynthBuildJob) -> bool {
        *self
            .pending_build
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(job);
        if self
            .worker_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return true;
        }
        let slot = Arc::clone(self);
        if std::thread::Builder::new()
            .name("kurv-resynth-build".into())
            .spawn(move || slot.run_build_worker())
            .is_err()
        {
            self.worker_running.store(false, Ordering::Release);
            self.build_status.fail();
            return false;
        }
        true
    }

    fn run_build_worker(self: Arc<Self>) {
        loop {
            let job = self
                .pending_build
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            let Some(job) = job else {
                self.worker_running.store(false, Ordering::Release);
                let pending = self
                    .pending_build
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_some();
                if pending
                    && self
                        .worker_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    continue;
                }
                return;
            };
            self.run_build_job(job);
        }
    }

    fn run_build_job(self: &Arc<Self>, job: ResynthBuildJob) {
        let revision = job.revision;
        // Acquire the process-wide analysis lease before decoding or allocating
        // worker scratch. Waiting jobs keep only their already-admitted bounded
        // source bytes and immediately stale-drop once admitted.
        let _work = acquire_resynth_analysis_work();
        if self.desired_revision.load(Ordering::Acquire) != revision {
            return;
        }
        let should_cancel = || self.desired_revision.load(Ordering::Acquire) != revision;
        let result = match job.kind {
            ResynthBuildKind::Analyzed {
                model,
                selected,
                controls,
            } => {
                self.build_status.set_progress(70);
                compile_rt_artifact_with_cancel(&model, selected, controls, &should_cancel)
                    .map(|artifact| (model, artifact, selected, Some(controls.sanitized())))
            }
            ResynthBuildKind::Rebuild {
                file_name,
                bytes,
                selected,
                controls,
                root_override_hz,
            } => {
                self.build_status.set_progress(15);
                analyze_wav_with_root_override_and_visuals_with_cancel(
                    file_name,
                    bytes,
                    controls,
                    root_override_hz,
                    None,
                    &should_cancel,
                )
                .and_then(|model| {
                    self.build_status.set_progress(70);
                    let artifact = compile_rt_artifact_with_cancel(
                        &model,
                        selected,
                        controls,
                        &should_cancel,
                    )?;
                    Ok((
                        Arc::new(model),
                        artifact,
                        selected,
                        Some(controls.sanitized()),
                    ))
                })
            }
        };
        if self.desired_revision.load(Ordering::Acquire) != revision {
            return;
        }
        let Ok((model, artifact, selected, controls)) = result else {
            self.build_status.fail();
            return;
        };
        self.build_status.set_progress(85);
        let artifact = Arc::new(artifact);
        let Some(artifact_visuals) =
            analyze_sounding_artifact_visuals_with_cancel(&artifact, &should_cancel)
        else {
            return;
        };
        let mut stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.desired_revision.load(Ordering::Acquire) != revision {
            return;
        }
        let controls = controls.or_else(|| stored.as_ref().map(|document| document.controls));
        let Some(controls) = controls else {
            return;
        };
        let Some(generation) = self.rt.store(revision, Some(Arc::clone(&artifact))) else {
            // Keep the document writer through the final identity check and
            // pending insertion. A newer clear/import therefore cannot install
            // its intent and then be overwritten by this older completion.
            let mut pending = self
                .pending_commit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.desired_revision.load(Ordering::Acquire) != revision {
                return;
            }
            *pending = Some(PendingResynthCommit::Artifact(CompletedResynthBuild {
                revision,
                selected,
                controls,
                model,
                artifact,
                artifact_visuals,
            }));
            self.build_status.set_progress(99);
            drop(pending);
            drop(stored);
            self.schedule_pending_retry();
            return;
        };
        self.store_live_controls(controls);
        if let Some(previous) = stored.as_ref().filter(|previous| {
            previous.selected != selected
                && previous.model.visuals.source_digest() == model.visuals.source_digest()
        }) {
            *self
                .algorithm_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(CachedResynthAlgorithm {
                selected: previous.selected,
                source_digest: previous.model.visuals.source_digest(),
                artifact: Arc::clone(&previous.artifact),
                artifact_visuals: Arc::clone(&previous.artifact_visuals),
            });
        }
        *stored = Some(ResynthSlotDocument {
            revision,
            selected,
            controls,
            model,
            artifact,
            artifact_visuals,
            artifact_generation: generation,
        });
        self.build_status.set_progress(99);
    }

    pub(super) fn schedule_pending_retry(&self) {
        if !detached_resynth_work_is_safe()
            || self
                .pending_retry_running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let Some(weak) = self.retry_weak.get().cloned() else {
            self.pending_retry_running.store(false, Ordering::Release);
            return;
        };
        let spawn = std::thread::Builder::new()
            .name("kurv-resynth-publish".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    let Some(slot) = weak.upgrade() else {
                        break;
                    };
                    let _ = slot.try_commit_pending();
                    let pending = slot
                        .pending_commit
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_some();
                    if !pending {
                        slot.pending_retry_running.store(false, Ordering::Release);
                        // Close the race with a new pending commit arriving
                        // immediately before the flag was cleared.
                        if slot
                            .pending_commit
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .is_some()
                        {
                            slot.schedule_pending_retry();
                        }
                        break;
                    }
                }
            });
        if spawn.is_err() {
            // The intent remains in `pending_commit`, so an off-thread caller
            // can retry later rather than losing the requested clear/build.
            self.pending_retry_running.store(false, Ordering::Release);
        }
    }

    pub(super) fn try_commit_pending(&self) -> PendingCommitResult {
        // `document` is the off-thread intent/commit gate. Every production
        // desired-revision writer holds a document guard, so validation and
        // pointer publication are one linearized transaction. There is no
        // meaningful stale rollback after `rt.store`: rejection must happen
        // before publication identity advances.
        let mut stored = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut pending = self
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(commit) = pending.as_ref() else {
            return PendingCommitResult::Empty;
        };
        let revision = commit.revision();
        if self.desired_revision.load(Ordering::Acquire) != revision {
            *pending = None;
            return PendingCommitResult::Stale;
        }
        let generation = match commit {
            PendingResynthCommit::Artifact(completed) => self
                .rt
                .store(revision, Some(Arc::clone(&completed.artifact))),
            PendingResynthCommit::Clear { .. } => self.rt.clear(revision),
        };
        let Some(generation) = generation else {
            return PendingCommitResult::Backpressured;
        };
        let Some(commit) = pending.take() else {
            return PendingCommitResult::Empty;
        };
        match commit {
            PendingResynthCommit::Artifact(completed) => {
                self.store_live_controls(completed.controls);
                if let Some(previous) = stored.as_ref().filter(|previous| {
                    previous.selected != completed.selected
                        && previous.model.visuals.source_digest()
                            == completed.model.visuals.source_digest()
                }) {
                    *self
                        .algorithm_cache
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(CachedResynthAlgorithm {
                            selected: previous.selected,
                            source_digest: previous.model.visuals.source_digest(),
                            artifact: Arc::clone(&previous.artifact),
                            artifact_visuals: Arc::clone(&previous.artifact_visuals),
                        });
                }
                *stored = Some(ResynthSlotDocument {
                    revision,
                    selected: completed.selected,
                    controls: completed.controls,
                    model: completed.model,
                    artifact: completed.artifact,
                    artifact_visuals: completed.artifact_visuals,
                    artifact_generation: generation,
                });
            }
            PendingResynthCommit::Clear { .. } => {
                self.live_sequence.store(0, Ordering::Release);
                *stored = None;
            }
        }
        self.build_status.set_progress(99);
        PendingCommitResult::Committed(generation)
    }
}
