use super::*;

#[test]
fn xy_source_has_stable_automation_ids_and_independent_values() {
    assert_eq!(u32::from(P::XySourceX), 366);
    assert_eq!(u32::from(P::XySourceY), 367);

    let params = KurvParams::default();
    params.xy_source_x.set_value(0.125);
    params.xy_source_y.set_value(0.875);
    assert_eq!(params.xy_source_x.value().to_bits(), 0.125_f32.to_bits());
    assert_eq!(params.xy_source_y.value().to_bits(), 0.875_f32.to_bits());
}

#[test]
fn xy_route_identity_sidecars_round_trip_in_custom_state() {
    let params = KurvParams::default();
    params.xy_source_x_route_mask.store(1_u64 << 2);
    params.xy_source_y_route_mask.store(1_u64 << 41);

    let persisted = params.serialize_persist();
    let reopened = KurvParams::default();
    reopened.load_persist(&persisted);

    assert_eq!(reopened.xy_source_x_route_mask.load(), 1_u64 << 2);
    assert_eq!(reopened.xy_source_y_route_mask.load(), 1_u64 << 41);
}
