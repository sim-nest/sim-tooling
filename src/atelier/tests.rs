use super::{
    AtelierLayer, AtelierNode, AtelierNodeKind, AtelierSite, AtelierSiteOptions, atelier_site,
};

#[test]
fn wrong_layer_is_rejected() {
    let err = AtelierNode::new(AtelierNodeKind::Editor, AtelierLayer::L4).unwrap_err();
    assert!(err.contains("editor"));
    assert!(err.contains("L4"));
}

#[test]
fn validator_uses_process_or_lan_site() {
    let node = AtelierNode::new(AtelierNodeKind::Validator, AtelierLayer::L4).unwrap();
    assert_eq!(node.site_kind(), "fabric");
    assert_eq!(node.address_kind(), "agent");
    assert!(node.is_process_or_lan());

    let lan_node = AtelierNode::new(AtelierNodeKind::Validator, AtelierLayer::L5).unwrap();
    assert_eq!(lan_node.address_kind(), "tcp");
    assert!(lan_node.is_process_or_lan());
}

#[test]
fn shell_uses_browser_site() {
    let node = AtelierNode::new(AtelierNodeKind::Shell, AtelierLayer::L6).unwrap();
    assert_eq!(node.site_kind(), "fabric");
    assert_eq!(node.address_kind(), "http");
    assert!(node.is_browser());
}

#[test]
fn default_graph_has_no_editable_meta_workspace_root() {
    let site = AtelierSite::default_for_roots(vec![
        "../repo-control".to_owned(),
        "../sim-tooling".to_owned(),
    ])
    .unwrap();
    assert_eq!(site.nodes().len(), 8);
    assert!(
        !site
            .editable_roots()
            .iter()
            .any(|root| root.contains(".meta-workspace"))
    );
}

#[test]
fn meta_workspace_root_is_rejected() {
    let err = AtelierSite::default_for_roots(vec![".meta-workspace/packages/xtask".to_owned()])
        .unwrap_err();
    assert!(err.contains(".meta-workspace"));
}

#[test]
fn cache_check_reports_stale_graph() {
    let dir = std::env::temp_dir().join(format!("sim-tooling-atelier-site-{}", std::process::id()));
    let cache = dir.join("site.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&cache, "{}\n").unwrap();

    let err = atelier_site(AtelierSiteOptions {
        cache_path: Some(cache.clone()),
        check: true,
        ..AtelierSiteOptions::default()
    })
    .unwrap_err();
    assert!(err.contains("stale"));

    let _ = std::fs::remove_file(cache);
    let _ = std::fs::remove_dir(dir);
}
