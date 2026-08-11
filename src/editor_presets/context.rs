use std::io;

use truce::params::Params;
use truce_core::editor::PluginContext;

use crate::{KurvParams, P};

use super::format::{Snapshot, validate_snapshot};
use super::invalid_data;

pub(super) fn init_snapshot() -> io::Result<Snapshot> {
    snapshot_params(&KurvParams::default(), Vec::new())
}

pub(super) fn capture(context: &PluginContext<KurvParams>) -> io::Result<Snapshot> {
    let custom = context.get_state();
    let persist = context.params().serialize_persist();
    let mut params = Vec::new();
    for info in context.params().param_infos() {
        if !is_preset_param(info.id) {
            continue;
        }
        let normalized = context
            .params()
            .get_normalized(info.id)
            .ok_or_else(|| invalid_data("parameter metadata has no value"))?;
        if !normalized.is_finite() || !(0.0..=1.0).contains(&normalized) {
            return Err(invalid_data("plugin returned an invalid parameter value"));
        }
        params.push((info.id, normalized));
    }
    validate_snapshot(&params, custom.len(), persist.len())?;
    Ok(Snapshot {
        params,
        custom,
        persist,
    })
}

pub(super) fn apply(snapshot: Snapshot, context: &PluginContext<KurvParams>) {
    let Snapshot {
        params,
        custom,
        persist,
    } = snapshot;
    for (id, normalized) in params {
        if is_preset_param(id) && context.params().get_normalized(id).is_some() {
            context.set_param(id, normalized);
        }
    }
    if persist.is_empty() {
        context.params().generator_stack.reset_legacy();
    } else {
        context.params().load_persist(&persist);
    }
    context.set_state(custom);
}

fn snapshot_params(params: &KurvParams, custom: Vec<u8>) -> io::Result<Snapshot> {
    let persist = params.serialize_persist();
    let mut values = Vec::new();
    for info in params.param_infos() {
        if !is_preset_param(info.id) {
            continue;
        }
        let normalized = params
            .get_normalized(info.id)
            .ok_or_else(|| invalid_data("parameter metadata has no value"))?;
        values.push((info.id, normalized));
    }
    validate_snapshot(&values, custom.len(), persist.len())?;
    Ok(Snapshot {
        params: values,
        custom,
        persist,
    })
}

fn is_preset_param(id: u32) -> bool {
    id != u32::from(P::PitchBend) && id != u32::from(P::SustainPedal)
}
