use super::TreeLimit;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct LimitsReport {
    pub(super) source: TreeLimit,
    pub(super) vendor: TreeLimit,
    pub(super) combined_input: TreeLimit,
}
