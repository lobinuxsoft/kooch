use super::*;

#[test]
fn the_project_id_parses() {
    assert!(uuid::Uuid::parse_str(PROJECT_ID).is_ok());
}

/// A build without the feature must not merely default to false — it
/// must have no way to reach true, or a project could ask for DLSS of a
/// binary that never linked it.
#[test]
#[cfg(not(feature = "dlss"))]
fn a_plain_build_reports_no_support() {
    let (_, support) = instance(InstanceDescriptor::new_without_display_handle());
    assert_eq!(support, DlssSupport::default());
    assert!(!support.super_resolution);
}
