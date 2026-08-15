use sim_index_core::{DeclarationFact, IndexDoc, ProtocolRelation};

pub(super) fn merge_source_facts(
    merged: &mut IndexDoc,
    declarations: Vec<DeclarationFact>,
    relations: Vec<ProtocolRelation>,
) -> Result<(), String> {
    for fact in declarations {
        let identity = (&fact.anchor, fact.role, fact.module_path.as_str());
        if let Some(previous) = merged.declarations.iter().find(|candidate| {
            (
                &candidate.anchor,
                candidate.role,
                candidate.module_path.as_str(),
            ) == identity
        }) {
            if previous != &fact {
                return Err(format!(
                    "conflicting declaration copies for {} {} {}",
                    fact.anchor,
                    fact.role.as_str(),
                    fact.module_path
                ));
            }
        } else {
            merged.declarations.push(fact);
        }
    }
    for relation in relations {
        let identity = (
            &relation.anchor,
            relation.implementor.as_str(),
            relation.source_spelling.as_str(),
        );
        if let Some(previous) = merged.protocol_relations.iter().find(|candidate| {
            (
                &candidate.anchor,
                candidate.implementor.as_str(),
                candidate.source_spelling.as_str(),
            ) == identity
        }) {
            if previous != &relation {
                return Err(format!(
                    "conflicting protocol-relation copies for {} {} {}",
                    relation.anchor, relation.implementor, relation.source_spelling
                ));
            }
        } else {
            merged.protocol_relations.push(relation);
        }
    }
    Ok(())
}
