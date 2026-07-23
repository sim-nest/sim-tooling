use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VaultGranularity {
    Compact,
    Full,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultGraph {
    pub(crate) nodes: Vec<VaultNode>,
    pub(crate) relations: Vec<VaultRelation>,
    pub(crate) reverse_relations: Vec<VaultRelation>,
    pub(crate) coverage: VaultCoverage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VaultNode {
    Subject(VaultSubject),
    Anchor(VaultAnchor),
    Surface(VaultSurface),
    Specimen(VaultSpecimen),
    Draft(VaultFeatureDraft),
    Feature(VaultFeature),
    Route(VaultRoute),
}

impl VaultNode {
    pub(crate) fn endpoint(&self) -> VaultEndpoint {
        match self {
            Self::Subject(node) => VaultEndpoint::new(VaultNodeKind::Subject, &node.id),
            Self::Anchor(node) => VaultEndpoint::new(VaultNodeKind::Anchor, &node.id),
            Self::Surface(node) => VaultEndpoint::new(VaultNodeKind::Surface, &node.id),
            Self::Specimen(node) => VaultEndpoint::new(VaultNodeKind::Specimen, &node.id),
            Self::Draft(node) => VaultEndpoint::new(VaultNodeKind::Draft, &node.id),
            Self::Feature(node) => VaultEndpoint::new(VaultNodeKind::Feature, &node.id),
            Self::Route(node) => VaultEndpoint::new(VaultNodeKind::Route, &node.id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultSubject {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultAnchor {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultSurface {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultSpecimen {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) language: Option<String>,
    pub(crate) runnable: bool,
    pub(crate) checked: bool,
    pub(crate) checked_by: Option<String>,
    pub(crate) doc_anchor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultFeatureDraft {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) claims_anchors: Vec<String>,
    pub(crate) claims_surfaces: Vec<String>,
    pub(crate) claims_specimens: Vec<String>,
    pub(crate) grammar_contracts: Vec<VaultGrammarContract>,
    pub(crate) doc_anchor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultFeature {
    pub(crate) id: String,
    pub(crate) key: String,
    pub(crate) subject: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) anchors: Vec<String>,
    pub(crate) surfaces: Vec<String>,
    pub(crate) specimens: Vec<String>,
    pub(crate) grammar_contracts: Vec<VaultGrammarContract>,
    pub(crate) doc_anchor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct VaultGrammarContract {
    pub(crate) id: String,
    pub(crate) decoder: Option<String>,
    pub(crate) encoder: Option<String>,
    pub(crate) surface: Option<String>,
    pub(crate) round_trip: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultRoute {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) audiences: Vec<String>,
    pub(crate) steps: Vec<VaultRouteStep>,
    pub(crate) doc_anchor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultRouteStep {
    pub(crate) order: usize,
    pub(crate) target: VaultEndpoint,
    pub(crate) why: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum VaultNodeKind {
    Anchor,
    Draft,
    Feature,
    Route,
    Specimen,
    Subject,
    Surface,
}

impl VaultNodeKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Draft => "draft",
            Self::Feature => "feature",
            Self::Route => "route",
            Self::Specimen => "specimen",
            Self::Subject => "subject",
            Self::Surface => "surface",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct VaultEndpoint {
    pub(crate) kind: VaultNodeKind,
    pub(crate) id: String,
}

impl VaultEndpoint {
    pub(crate) fn new(kind: VaultNodeKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    pub(crate) fn summary(&self) -> String {
        format!("{}:{}", self.kind.label(), self.id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct VaultRelation {
    pub(crate) from: VaultEndpoint,
    pub(crate) rel: String,
    pub(crate) to: VaultEndpoint,
    pub(crate) order: Option<usize>,
}

impl VaultRelation {
    pub(crate) fn new(
        from: VaultEndpoint,
        rel: impl Into<String>,
        to: VaultEndpoint,
        order: Option<usize>,
    ) -> Self {
        Self {
            from,
            rel: rel.into(),
            to,
            order,
        }
    }

    pub(crate) fn reversed(&self) -> Self {
        Self {
            from: self.to.clone(),
            rel: self.rel.clone(),
            to: self.from.clone(),
            order: self.order,
        }
    }

    pub(crate) fn summary(&self) -> String {
        match self.order {
            Some(order) => format!(
                "{} -{}[{order}]-> {}",
                self.from.summary(),
                self.rel,
                self.to.summary()
            ),
            None => format!(
                "{} -{}-> {}",
                self.from.summary(),
                self.rel,
                self.to.summary()
            ),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct VaultCoverage {
    pub(crate) rows: BTreeSet<VaultEndpoint>,
    pub(crate) compact: BTreeSet<VaultEndpoint>,
    pub(crate) full: BTreeSet<VaultEndpoint>,
}

impl VaultCoverage {
    pub(crate) fn from_nodes(nodes: &[VaultNode]) -> Self {
        let rows = nodes
            .iter()
            .map(VaultNode::endpoint)
            .collect::<BTreeSet<_>>();
        Self {
            compact: rows.clone(),
            full: rows.clone(),
            rows,
        }
    }

    pub(crate) fn unrepresented_rows(&self) -> usize {
        self.unrepresented_for(VaultGranularity::Compact).len()
    }

    pub(crate) fn unrepresented_for(&self, granularity: VaultGranularity) -> Vec<VaultEndpoint> {
        let represented = match granularity {
            VaultGranularity::Compact => &self.compact,
            VaultGranularity::Full => &self.full,
        };
        self.rows
            .difference(represented)
            .cloned()
            .collect::<Vec<_>>()
    }
}
