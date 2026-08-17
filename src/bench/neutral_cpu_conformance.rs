use super::{
    DecisionOutcome,
    neutral_specimen_fixture::{
        CpuOperation, assert_local_fingerprint, assert_report_round_trip, assert_retained_failure,
        assert_synthetic_host_refused, local_environment, matched_report,
    },
};

#[test]
fn cpu_bound_specimen_produces_a_matched_browsable_report() {
    let environment = local_environment();
    assert_local_fingerprint(&environment);

    let report = matched_report("neutral-cpu-bound");
    assert_eq!(report.comparison.outcome, DecisionOutcome::Pass);
    assert!(report.comparison.paired_effect.is_some());
    assert_report_round_trip(&report);
    assert_synthetic_host_refused(&report);
    assert_retained_failure(CpuOperation, "neutral-cpu-bound-failure");
}
