// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one product-load receipt whose result is already claim 18.
//!
//! The pack runs the named program and reads the named root. This module
//! copies that receipt into the world catalog. It does not rewrite a checker
//! result and it does not derive a seal.

use std::collections::BTreeMap;
use std::path::Path;

use sim_conformance_packs::packs::product::observe_product_load;
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes the product load for `subject` and assembles that receipt.
///
/// `program` and `root` are the pack's command and constellation root. A
/// refused observation is not supplied. The release input is empty because
/// this result has no tail.
///
/// # Errors
///
/// Returns the pack refusal when the load fails, a parse refusal when the
/// result is not a completion claim, and a catalog refusal when assembly
/// fails.
pub fn admit_product_load(
    subject: &str,
    program: &Path,
    root: &Path,
) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_product_load(subject, program, root).map_err(|failure| OwnerReceiptRefusal {
            code: failure.code.to_owned(),
            detail: failure.detail,
        })?;
    let claim = claim_number(&observed.result)?;
    let mut revocations = BTreeMap::new();
    revocations.insert(observed.checker.clone(), observed.revocation);
    supply_owner_claim(SuppliedOwnerClaim {
        subject: observed.subject,
        claim,
        admitted: observed.admitted,
        checker: observed.checker,
        catalog: observed.catalog,
        scope: observed.scope,
        revocations,
    });
    final_catalog("").map_err(|error| OwnerReceiptRefusal {
        code: "catalog-refused".to_owned(),
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_product_load;

    const RECEIPT: &str = "#(prove/load-receipt-v1 (provider \"platform/site/model\") (loader \"loader/static-v1\") (artifact \"lib/prove-command\"))";
    const ROADMAP: &str = "#(roadmap/local-result-v1 (outcome \"ok\") (execution \"local/no-model\") (conduct \"conduct/local\") (model-pick \"model/local\") (proof-catalog \"proof/local\") (runner-generation \"generation/local\") (journal-acknowledged true) (detail \"execution not started\"))";
    const FRAME: &str = "without placing its behavior in the frame.";
    const WORLD_TASK: &str =
        "Project supplied semantic facts and explain their exact affected closure";
    const PROVE_TASK: &str = "Run a cold proof and read its frontier";
    const DELIVER_TASK: &str = "Read a delivery record through the installed world command";
    const ROADMAP_TASK: &str = "Read the local roadmap status record";
    const WORLD_ID: &str = "route/run-world-command";
    const PROVE_ID: &str = "route/run-prove-command";
    const DELIVER_ID: &str = "route/run-deliver-command";
    const ROADMAP_ID: &str = "route/run-roadmap-command";
    const WORLD_SPECIMEN: &str =
        "recipe/sim-runtime/crates/sim-lib-world/01-projection/world-explain";
    const PROVE_SPECIMEN: &str = "spec-test/sim-run/crates/sim-run/tests/prove_gate";
    const DELIVER_SPECIMEN: &str = "spec-test/sim-run/crates/sim-run/tests/deliver_gate";
    const ROADMAP_SPECIMEN: &str = "spec-test/sim-run/crates/sim-run/tests/roadmap";

    #[test]
    fn a_claim_label_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let error = admit_product_load("completion/claim-18", Path::new("sim"), Path::new("."))
            .expect_err("refused");
        assert_eq!(error.code, "bad-record");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "18"),
            "a claim label must not admit claim 18"
        );
        assert_eq!(
            field(&before, "subject").as_deref(),
            field(&after, "subject").as_deref()
        );
    }

    #[test]
    fn the_phase_union_load_is_admitted_as_claim_18() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "18"),
            "claim 18 is not proved before the product-load receipt"
        );
        let made = passing_program();
        let after = admit_product_load(&subject, &made.program, &made.root).expect("admitted");
        assert_eq!(claims(&after), vec!["18".to_owned()]);
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
    }

    struct Made {
        program: PathBuf,
        root: PathBuf,
    }

    fn passing_program() -> Made {
        let dir =
            std::env::temp_dir().join(format!("sim-product-load-admit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("git init")
                .success()
        );
        let root = dir.join("tree");
        write_tree(&root);
        let program = write_program(&dir);
        Made { program, root }
    }

    fn write_tree(root: &Path) {
        let world = root.join("sim-runtime/crates/sim-lib-world");
        let run = root.join("sim-run/crates/sim-run");
        std::fs::create_dir_all(world.join("recipes/01-projection/world-explain")).unwrap();
        std::fs::create_dir_all(run.join("tests")).unwrap();
        std::fs::write(
            world.join("recipes/01-projection/world-explain/recipe.toml"),
            "name = \"world-explain\"\n",
        )
        .unwrap();
        std::fs::write(run.join("tests/prove_gate.rs"), "// prove\n").unwrap();
        std::fs::write(run.join("tests/deliver_gate.rs"), "// deliver\n").unwrap();
        std::fs::write(run.join("tests/roadmap.rs"), "// roadmap\n").unwrap();
        let text = "In one line: Load the product through the bootloader.\n";
        std::fs::write(world.join("BROCHURE.md"), text).unwrap();
        std::fs::write(run.join("BROCHURE.md"), text).unwrap();
    }

    fn write_program(dir: &Path) -> PathBuf {
        let path = dir.join("sim");
        std::fs::write(&path, format!("#!/bin/sh\nset -e\n{}", script())).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn script() -> String {
        let world_route = shell_quote(&route(
            WORLD_ID,
            WORLD_SPECIMEN,
            &format!("The world row loads the product {FRAME}"),
        ));
        let prove_route = shell_quote(&route(
            PROVE_ID,
            PROVE_SPECIMEN,
            &format!("The prove row loads the product {FRAME}"),
        ));
        let deliver_route = shell_quote(&route(
            DELIVER_ID,
            DELIVER_SPECIMEN,
            &format!("The deliver row loads the product {FRAME}"),
        ));
        let roadmap_route = shell_quote(&route(
            ROADMAP_ID,
            ROADMAP_SPECIMEN,
            &format!("The roadmap row loads the product {FRAME}"),
        ));
        format!(
            r#"cmd="$1"
sub="$2"
receipt='{RECEIPT}'
roadmap='{ROADMAP}'
case "$cmd:$sub" in
world:why)
  printf '%s\n' '#(world/explanation-v1 (path ["owner/sim-private/disclosure"]))'
  ;;
world:deliver)
  printf '%s\n' '#(world/deliver-v1 (realization "model") (effects 0) (subprocesses 0) (resident true) (recovered true))'
  ;;
prove:plan)
  printf '%s\n' '#(prove/plan-v1 (denominator 29) (excluded ["leaf/native-worker-adapter"]))'
  printf '%s\n' "$receipt"
  ;;
roadmap:status)
  printf '%s\n' "$roadmap"
  ;;
index:route)
  case "$3" in
  '{WORLD_TASK}')
    printf '%s\n' '{world_route}'
    ;;
  '{PROVE_TASK}')
    printf '%s\n' '{prove_route}'
    ;;
  '{DELIVER_TASK}')
    printf '%s\n' '{deliver_route}'
    ;;
  '{ROADMAP_TASK}')
    printf '%s\n' '{roadmap_route}'
    ;;
  esac
  ;;
esac
"#
        )
    }

    fn route(id: &str, specimen: &str, why: &str) -> String {
        format!(
            "{id}\tTitle\tuser,code,framework\n  feature\tfeature/sim-run/bootloader\tBootloader\n    The standard bootloader selects command libraries by verb.\n  feature\tfeature/sim-run/product\tProduct\n    {why}\n  specimen\t{specimen}\tSpecimen\n    The specimen is the checked file."
        )
    }

    fn shell_quote(text: &str) -> String {
        text.replace('\'', "'\\''")
    }

    fn field(datum: &Datum, name: &str) -> Option<String> {
        let Datum::Node { fields, .. } = datum else {
            return None;
        };
        fields.iter().find_map(|(symbol, value)| {
            (symbol.as_qualified_str() == name).then(|| match value {
                Datum::String(text) => text.clone(),
                _ => String::new(),
            })
        })
    }

    fn claims(datum: &Datum) -> Vec<String> {
        let Datum::Node { fields, .. } = datum else {
            return Vec::new();
        };
        let Some(value) = fields
            .iter()
            .find(|(symbol, _)| symbol.as_qualified_str() == "admissions")
            .map(|(_, value)| value)
        else {
            return Vec::new();
        };
        let Datum::Vector(items) = value else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| match item {
                Datum::String(text) => text.clone(),
                _ => String::new(),
            })
            .collect()
    }
}
