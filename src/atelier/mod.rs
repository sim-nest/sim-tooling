//! Atelier development-environment tooling.
//!
//! The Atelier commands describe SIM development as placed control-plane data:
//! editor, guard, agent, validation, docs, pin, index, and shell nodes are
//! assigned to SUP Site concepts and emitted as cacheable JSON.

mod capsule;
mod cassette;
mod cli;
mod eval;
mod guard;
mod guard_scan;
mod index;
mod index_doc;
mod index_graph_units;
mod index_manifest;
mod index_units;
mod io;
mod radar;
mod rust;
mod rust_metadata;
mod shell;
mod site;
mod tools;

pub use guard::{
    AtelierGuardOptions, AtelierGuardReport, GuidelineFinding, GuidelineRule, GuidelineSeverity,
    atelier_guard,
};
pub use site::{
    AtelierLayer, AtelierNode, AtelierNodeKind, AtelierSite, AtelierSiteOptions, AtelierSiteReport,
    atelier_site,
};
pub use tools::{
    AtelierToolAction, AtelierToolDescriptor, AtelierToolsOptions, AtelierToolsReport,
    atelier_tools,
};

pub(crate) use capsule::run as run_capsule;
pub(crate) use cassette::run as run_cassette;
pub(crate) use cli::run;
pub(crate) use guard::run as run_guard;
pub(crate) use index::run as run_index;
pub(crate) use radar::run as run_radar;
pub(crate) use shell::run as run_shell;
pub(crate) use tools::run as run_tools;

#[cfg(test)]
mod capsule_tests;
#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod radar_tests;
#[cfg(test)]
mod rust_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tools_tests;
