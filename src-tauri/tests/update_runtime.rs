use douyin_history_archive::update_runtime::{
    UpdateConfigurationStateV1, UpdatePhaseV1, UpdateReleaseV1, UpdateStateMachineV1,
};

#[test]
fn update_configuration_is_fail_closed_and_requires_https() {
    let missing = UpdateConfigurationStateV1::from_values(None, None);
    assert_eq!(missing.require().unwrap_err(), "update_not_configured");

    let partial = UpdateConfigurationStateV1::from_values(
        Some("https://updates.example.com/{{target}}/{{arch}}/{{current_version}}"),
        None,
    );
    assert_eq!(partial.require().unwrap_err(), "update_not_configured");

    let insecure = UpdateConfigurationStateV1::from_values(
        Some("http://updates.example.com/latest.json"),
        Some("trusted-public-key"),
    );
    assert_eq!(insecure.require().unwrap_err(), "invalid_update_endpoint");

    let configured = UpdateConfigurationStateV1::from_values(
        Some("https://updates.example.com/{{target}}/{{arch}}/{{current_version}}"),
        Some("  trusted-public-key  "),
    );
    let value = configured.require().unwrap();
    assert_eq!(value.endpoint.scheme(), "https");
    assert_eq!(value.pubkey, "trusted-public-key");
}

#[test]
fn update_state_machine_exposes_only_valid_actions() {
    let configuration = UpdateConfigurationStateV1::from_values(
        Some("https://updates.example.com/latest.json"),
        Some("trusted-public-key"),
    );
    let mut state = UpdateStateMachineV1::new(&configuration, "0.1.0");

    let initial = state.snapshot();
    assert_eq!(initial.phase, UpdatePhaseV1::Idle);
    assert!(initial.can_check);
    assert!(!initial.can_download);
    assert!(!initial.can_install);

    state.begin_check().unwrap();
    assert_eq!(state.snapshot().phase, UpdatePhaseV1::Checking);
    assert_eq!(
        state.begin_check().unwrap_err(),
        "update_operation_in_progress"
    );

    state
        .complete_check(Some(UpdateReleaseV1 {
            version: "0.2.0".to_string(),
            notes: Some("stability update".to_string()),
            published_at: Some("2026-08-15T12:00:00Z".to_string()),
        }))
        .unwrap();
    let available = state.snapshot();
    assert_eq!(available.phase, UpdatePhaseV1::Available);
    assert_eq!(available.available_version.as_deref(), Some("0.2.0"));
    assert!(available.can_download);

    state.begin_download().unwrap();
    state.record_download_chunk(512, Some(1_024)).unwrap();
    state.record_download_chunk(512, Some(1_024)).unwrap();
    state.complete_download().unwrap();
    let downloaded = state.snapshot();
    assert_eq!(downloaded.phase, UpdatePhaseV1::Downloaded);
    assert_eq!(downloaded.downloaded_bytes, 1_024);
    assert_eq!(downloaded.total_bytes, Some(1_024));
    assert!(downloaded.can_install);
    assert!(!downloaded.can_download);

    state.begin_install().unwrap();
    assert_eq!(state.snapshot().phase, UpdatePhaseV1::Installing);
    state.restore_downloaded_after_install_failure("update_backup_failed");
    let restored = state.snapshot();
    assert_eq!(restored.phase, UpdatePhaseV1::Downloaded);
    assert_eq!(restored.last_error.as_deref(), Some("update_backup_failed"));
    assert!(restored.can_install);
}

#[test]
fn update_state_machine_handles_no_update_and_retryable_failures() {
    let configuration = UpdateConfigurationStateV1::from_values(
        Some("https://updates.example.com/latest.json"),
        Some("trusted-public-key"),
    );
    let mut state = UpdateStateMachineV1::new(&configuration, "0.1.0");

    state.begin_check().unwrap();
    state.complete_check(None).unwrap();
    let current = state.snapshot();
    assert_eq!(current.phase, UpdatePhaseV1::UpToDate);
    assert!(current.can_check);
    assert!(!current.can_download);

    state.begin_check().unwrap();
    state.fail("update_check_failed");
    let failed_check = state.snapshot();
    assert_eq!(failed_check.phase, UpdatePhaseV1::Failed);
    assert_eq!(
        failed_check.last_error.as_deref(),
        Some("update_check_failed")
    );
    assert!(failed_check.can_check);

    state.begin_check().unwrap();
    state
        .complete_check(Some(UpdateReleaseV1 {
            version: "0.2.0".to_string(),
            notes: None,
            published_at: None,
        }))
        .unwrap();
    state.begin_download().unwrap();
    state.fail("update_download_failed");
    let failed_download = state.snapshot();
    assert!(failed_download.can_check);
    assert!(failed_download.can_download);
    assert!(!failed_download.can_install);
}
