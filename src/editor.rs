use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use truce::params::Params;
use truce_core::editor::{Editor, PluginContext, PluginContextReadF32, RawWindowHandle};
use truce_egui::EguiEditor;

use crate::{KurvParams, P};

mod detached_import;
mod factory_reset;

pub(crate) use detached_import::{
    DetachedJob, ImportSource, detached_work_is_safe, spawn_detached_job,
    spawn_detached_job_after_pin,
};
pub(crate) use factory_reset::reset_to_defaults;

const EDITOR_SIZE: (u32, u32) = (1120, 720);
const EDITOR_MIN_SIZE: (u32, u32) = (960, 480);
const EDITOR_MAX_SIZE: (u32, u32) = (2240, 1440);
const EDITOR_PHASE_IDLE: u8 = 0;
const EDITOR_PHASE_DRAW: u8 = 1;

#[derive(Default)]
struct EditorLifecycle {
    phase: AtomicU8,
    closing: std::sync::atomic::AtomicBool,
}

struct EditorDrawGuard<'a>(&'a AtomicU8);

impl Drop for EditorDrawGuard<'_> {
    fn drop(&mut self) {
        self.0.store(EDITOR_PHASE_IDLE, Ordering::Release);
    }
}

fn draw_with_phase(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    lifecycle: &EditorLifecycle,
) {
    if lifecycle.closing.load(Ordering::Acquire) {
        return;
    }
    lifecycle.phase.store(EDITOR_PHASE_DRAW, Ordering::Release);
    if lifecycle.closing.load(Ordering::Acquire) {
        lifecycle.phase.store(EDITOR_PHASE_IDLE, Ordering::Release);
        return;
    }
    let _guard = EditorDrawGuard(&lifecycle.phase);
    crate::editor_shell::draw(ui, state);
    if ui.input(|input| input.pointer.any_released()) {
        notify_persisted_state_changed(state);
    }
}

struct PersistedEditor {
    inner: EguiEditor<KurvParams>,
    params: Arc<KurvParams>,
    context: Option<PluginContext<KurvParams>>,
    lifecycle: Arc<EditorLifecycle>,
}

impl PersistedEditor {
    fn quiesce_draw(&self) {
        self.lifecycle.closing.store(true, Ordering::Release);
        // A detached X11 thread is safe only after KURV author code has left
        // its host bridge. Future frames are gated above; on Linux wait for a
        // frame already in progress without a timeout rather than allowing it
        // to resume into a destroyed plug-in instance. Other platforms never
        // use baseview's X11 detach fallback and may drive close on the same
        // UI thread, so they must not wait for their own draw callback.
        #[cfg(target_os = "linux")]
        while self.lifecycle.phase.load(Ordering::Acquire) == EDITOR_PHASE_DRAW {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn prepare_to_close(&mut self) {
        self.quiesce_draw();
        self.params.resynth_assets.reset_source_auditions();
        if let Some(context) = self.context.take() {
            finish_active_edits(&context);
            notify_persisted_state_changed(&context);
        }
    }
}

impl Editor for PersistedEditor {
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn open(&mut self, parent: RawWindowHandle, context: PluginContext) {
        self.lifecycle.closing.store(false, Ordering::Release);
        self.params.reconcile_legacy_generator_stack();
        self.params.resynth_assets.reset_source_auditions();
        self.context = Some(context.with_params(Arc::clone(&self.params)));
        self.inner.open(parent, context);
    }

    fn close(&mut self) {
        self.prepare_to_close();
        self.inner.close();
    }

    fn idle(&mut self) {
        self.inner.idle();
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if self.inner.size() == (width, height) {
            return true;
        }
        if !self.inner.set_size(width, height) {
            return false;
        }
        store_editor_window_size(&self.params, width, height);
        true
    }

    fn can_resize(&self) -> bool {
        self.inner.can_resize()
    }

    fn can_maximize(&self) -> bool {
        self.inner.can_maximize()
    }

    fn min_size(&self) -> (u32, u32) {
        Editor::min_size(&self.inner)
    }

    fn max_size(&self) -> (u32, u32) {
        Editor::max_size(&self.inner)
    }

    fn size_increment(&self) -> Option<(u32, u32)> {
        self.inner.size_increment()
    }

    fn aspect_ratio(&self) -> Option<(u32, u32)> {
        Editor::aspect_ratio(&self.inner)
    }

    fn prefers_pow2(&self) -> bool {
        Editor::prefers_pow2(&self.inner)
    }

    fn set_scale_factor(&mut self, factor: f64) {
        self.inner.set_scale_factor(factor);
    }

    fn set_uses_system_scale(&mut self, yes: bool) {
        self.inner.set_uses_system_scale(yes);
    }

    fn state_changed(&mut self) {
        self.params.reconcile_legacy_generator_stack();
        if let Some(context) = &self.context {
            finish_active_edits(context);
        }
        remember_persisted_state(&self.params);
        self.inner.state_changed();
    }

    fn screenshot(&mut self, params: Arc<dyn Params>) -> Option<(Vec<u8>, u32, u32)> {
        self.inner.screenshot(params)
    }
}

impl Drop for PersistedEditor {
    fn drop(&mut self) {
        // A host dropping the editor without `close` may already be tearing
        // down its automation interfaces. Quiesce author code and the X11
        // window, but do not call the bridge from this destructor.
        self.quiesce_draw();
        self.params.resynth_assets.reset_source_auditions();
        self.context.take();
        self.inner.close();
    }
}

pub(crate) fn begin_edit(state: &PluginContext<KurvParams>, id: P) {
    if state.params().editor_edits.begin(u32::from(id)) {
        state.begin_edit(id);
    }
}

pub(crate) fn end_edit(state: &PluginContext<KurvParams>, id: P) {
    if state.params().editor_edits.end(u32::from(id)) {
        state.end_edit(id);
    }
}

pub(crate) fn automate(state: &PluginContext<KurvParams>, id: P, normalized: f64) {
    begin_edit(state, id);
    state.set_param(id, normalized);
    end_edit(state, id);
}

fn finish_active_edits(state: &PluginContext<KurvParams>) -> usize {
    state.params().editor_edits.drain(|id| state.end_edit(id))
}

fn remember_persisted_state(params: &KurvParams) {
    *params
        .editor_persist_snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = params.serialize_persist();
}

pub(crate) fn notify_persisted_state_changed(state: &PluginContext<KurvParams>) -> bool {
    let current = state.params().serialize_persist();
    let changed = {
        let mut previous = state
            .params()
            .editor_persist_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *previous == current {
            false
        } else {
            *previous = current;
            true
        }
    };
    if !changed {
        return false;
    }
    let revision = state.get_param(P::StateRevision) < 0.5;
    automate(state, P::StateRevision, f64::from(revision));
    true
}

fn store_editor_window_size(params: &KurvParams, width: u32, height: u32) {
    params.editor_window_size.store(
        (u64::from(width) << 32) | u64::from(height),
        Ordering::Relaxed,
    );
}

fn editor_window_size(params: &KurvParams) -> (u32, u32) {
    let packed = params.editor_window_size.load(Ordering::Relaxed);
    if packed == 0 {
        return EDITOR_SIZE;
    }
    let width = u32::try_from(packed >> 32).unwrap_or(EDITOR_SIZE.0);
    let height = u32::try_from(packed & 0xFFFF_FFFF).unwrap_or(EDITOR_SIZE.1);
    (
        width.clamp(EDITOR_MIN_SIZE.0, EDITOR_MAX_SIZE.0),
        height.clamp(EDITOR_MIN_SIZE.1, EDITOR_MAX_SIZE.1),
    )
}

pub fn create(params: Arc<KurvParams>) -> Box<dyn Editor> {
    let size = editor_window_size(&params);
    let lifecycle = Arc::new(EditorLifecycle::default());
    let draw_lifecycle = Arc::clone(&lifecycle);
    let inner = EguiEditor::new(params.clone(), size, move |ui, state| {
        draw_with_phase(ui, state, &draw_lifecycle);
    })
    .with_font(ttf_inter::REGULAR)
    .with_fallback_font(
        "kurv-phosphor",
        egui_phosphor::Variant::Regular.font_bytes(),
    )
    .with_visuals(truce_egui::theme::dark())
    .resizable(true)
    .min_size(EDITOR_MIN_SIZE)
    .max_size(EDITOR_MAX_SIZE)
    .prefers_pow2(false);
    remember_persisted_state(&params);
    let editor = Box::new(PersistedEditor {
        inner,
        params,
        context: None,
        lifecycle,
    });
    editor
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use truce::params::Params;
    use truce_core::editor::{ClosureBridge, PluginContext};

    use super::{notify_persisted_state_changed, remember_persisted_state};
    use crate::editor_controls::{accumulate_drag, magnetic_shape_snap};
    use crate::{KurvParams, P};

    #[test]
    fn knob_drag_accumulates_every_pointer_frame() {
        let value = [-3.0_f32; 10].into_iter().fold(0.25, accumulate_drag);
        assert!((value - 0.45).abs() < 1.0e-6);
    }

    #[test]
    fn shape_knob_snaps_only_near_canonical_shapes() {
        assert!((magnetic_shape_snap(0.34) - (1.0 / 3.0)).abs() < 1.0e-6);
        assert!((magnetic_shape_snap(0.42) - 0.42).abs() < 1.0e-6);
    }

    #[test]
    fn edit_tracker_covers_every_host_parameter_id() {
        let params = KurvParams::default();
        let highest = params
            .param_infos()
            .iter()
            .map(|info| info.id)
            .max()
            .expect("KURV has parameters");
        assert!(
            usize::try_from(highest).expect("u32 parameter ID fits usize")
                < crate::params::EditorEditTracker::capacity(),
            "parameter ID {highest} exceeds editor gesture tracker capacity"
        );
    }

    #[test]
    fn edit_tracker_deduplicates_and_drains_active_gestures() {
        let tracker = crate::params::EditorEditTracker::default();
        assert!(tracker.begin(3));
        assert!(!tracker.begin(3));
        assert!(tracker.begin(130));
        assert!(tracker.end(3));
        assert!(!tracker.end(3));
        let mut drained = Vec::new();
        assert_eq!(tracker.drain(|id| drained.push(id)), 1);
        assert_eq!(drained, [130]);
        assert_eq!(tracker.drain(|_| {}), 0);
    }

    #[test]
    fn persist_changes_emit_hidden_host_revision_gestures() {
        let params = Arc::new(KurvParams::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let begin_events = Arc::clone(&events);
        let set_events = Arc::clone(&events);
        let end_events = Arc::clone(&events);
        let params_for_set = Arc::clone(&params);
        let params_for_get = Arc::clone(&params);
        let params_for_plain = Arc::clone(&params);
        let params_for_format = Arc::clone(&params);
        let state = PluginContext::new(
            Arc::new(ClosureBridge {
                begin_edit: Box::new(move |id| {
                    begin_events.lock().expect("events").push(("begin", id));
                }),
                set_param: Box::new(move |id, normalized| {
                    params_for_set.set_normalized(id, normalized);
                    set_events.lock().expect("events").push(("set", id));
                }),
                end_edit: Box::new(move |id| {
                    end_events.lock().expect("events").push(("end", id));
                }),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| {
                    params_for_get.get_normalized(id).unwrap_or_default()
                }),
                get_param_plain: Box::new(move |id| {
                    params_for_plain.get_plain(id).unwrap_or_default()
                }),
                format_param: Box::new(move |id| {
                    let plain = params_for_format.get_plain(id).unwrap_or_default();
                    params_for_format
                        .format_value(id, plain)
                        .unwrap_or_default()
                }),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| {}),
                transport: Box::new(|| None),
            }),
            Arc::clone(&params),
        );
        remember_persisted_state(&params);
        params
            .editor_state
            .lock()
            .expect("editor state")
            .persistent_modulation_cables = true;

        assert!(notify_persisted_state_changed(&state));
        assert!(!notify_persisted_state_changed(&state));
        assert_eq!(params.get_normalized(P::StateRevision.into()), Some(1.0));

        params.editor_state.lock().expect("editor state").theme_tint += 1;
        assert!(notify_persisted_state_changed(&state));
        assert_eq!(params.get_normalized(P::StateRevision.into()), Some(0.0));
        assert_eq!(
            *events.lock().expect("events"),
            [
                ("begin", u32::from(P::StateRevision)),
                ("set", u32::from(P::StateRevision)),
                ("end", u32::from(P::StateRevision)),
                ("begin", u32::from(P::StateRevision)),
                ("set", u32::from(P::StateRevision)),
                ("end", u32::from(P::StateRevision)),
            ]
        );
    }
}
