use super::{
    DecisionOutcome,
    neutral_specimen_fixture::{
        AllocationOperation, assert_local_fingerprint, assert_report_round_trip,
        assert_retained_failure, dispersed_report, local_environment,
    },
};

#[test]
fn allocation_bound_specimen_retains_inconclusive_and_failure_evidence() {
    let environment = local_environment();
    assert_local_fingerprint(&environment);

    let report = dispersed_report("neutral-allocation-bound");
    assert_eq!(report.comparison.outcome, DecisionOutcome::Inconclusive);
    assert!(
        report
            .comparison
            .inconclusive_reasons
            .iter()
            .any(|reason| { reason.starts_with("maximum-relative-dispersion:") })
    );
    assert_report_round_trip(&report);
    assert_retained_failure(AllocationOperation, "neutral-allocation-bound-failure");
}
