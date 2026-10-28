use douyin_history_archive::{
    desktop_lifecycle::{
        abort_exit_before_marker, continue_desktop_startup_from_ready,
        recover_desktop_startup_once, request_app_exit, BackgroundTaskRegistry,
        ControlledExitContext, ExitRequestDecisionV1, ExitRequestGate, startup_window_decision,
        DesktopStartupContext, DesktopStartupState, ExitCompletionV1, StartupStageV1,
        StartupWindowDecisionInputV1, StartupWindowDecisionV1,
    },
    sync::{OperationAdmission, OperationAdmissionMode},
};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

fn push(log: &Arc<Mutex<Vec<&'static str>>>, value: &'static str) {
    log.lock().expect("log").push(value);
}

#[test]
fn startup_runs_the_fixed_global_order_and_continues_once() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut context = DesktopStartupContext::new();
    for (stage, label) in [
        (StartupStageV1::UninstallRecovery, "uninstall"),
        (StartupStageV1::Bootstrap, "bootstrap"),
        (StartupStageV1::Schema, "schema"),
        (StartupStageV1::SchemaCleanup, "schema-cleanup"),
        (StartupStageV1::NoRunningAssertion, "no-running"),
    ] {
        let log = Arc::clone(&log);
        context.set_stage(stage, move || {
            push(&log, label);
            Ok(())
        });
    }
    let recovery_log = Arc::clone(&log);
    context.set_sync_recovery(move || {
        push(&recovery_log, "sync-recovery");
        Ok(Some(douyin_history_archive::startup_recovery::StartupContinuationPermitV1::new()))
    });

    let permit = recover_desktop_startup_once(&mut context)
        .expect("startup gate")
        .expect("ready permit");
    assert_eq!(context.state(), DesktopStartupState::Ready);

    let continuation_log = Arc::clone(&log);
    continue_desktop_startup_from_ready(permit, move |stage| {
        push(
            &continuation_log,
            match stage {
                StartupStageV1::DeviceAndOnboarding => "device",
                StartupStageV1::TrayAndWindow => "desktop",
                StartupStageV1::Worker => "worker",
                other => panic!("unexpected continuation stage: {other:?}"),
            },
        );
        Ok(())
    })
    .expect("continuation");

    assert_eq!(
        *log.lock().expect("log"),
        vec![
            "uninstall",
            "bootstrap",
            "schema",
            "schema-cleanup",
            "sync-recovery",
            "no-running",
            "device",
            "desktop",
            "worker",
        ]
    );
}

#[test]
fn startup_failure_stops_before_later_stages_and_creates_no_permit() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut context = DesktopStartupContext::new();
    for (stage, label) in [
        (StartupStageV1::UninstallRecovery, "uninstall"),
        (StartupStageV1::Bootstrap, "bootstrap"),
        (StartupStageV1::Schema, "schema"),
        (StartupStageV1::SchemaCleanup, "schema-cleanup"),
        (StartupStageV1::NoRunningAssertion, "no-running"),
    ] {
        let log = Arc::clone(&log);
        context.set_stage(stage, move || {
            push(&log, label);
            if stage == StartupStageV1::SchemaCleanup {
                Err("schema_cleanup_failed".to_string())
            } else {
                Ok(())
            }
        });
    }
    context.set_sync_recovery(|| {
        Ok(Some(douyin_history_archive::startup_recovery::StartupContinuationPermitV1::new()))
    });

    assert_eq!(
        recover_desktop_startup_once(&mut context).expect_err("must fail"),
        "schema_cleanup_failed"
    );
    assert_eq!(context.state(), DesktopStartupState::Blocked);
    assert_eq!(
        *log.lock().expect("log"),
        vec!["uninstall", "bootstrap", "schema", "schema-cleanup"]
    );
}

#[test]
fn continuation_is_idempotent_even_when_resolution_is_repeated() {
    let mut context = DesktopStartupContext::new();
    for stage in StartupStageV1::PRE_READY {
        if stage != StartupStageV1::SyncRecovery {
            context.set_stage(stage, || Ok(()));
        }
    }
    context.set_sync_recovery(|| {
        Ok(Some(douyin_history_archive::startup_recovery::StartupContinuationPermitV1::new()))
    });
    let permit = recover_desktop_startup_once(&mut context)
        .expect("startup")
        .expect("permit");
    let replay = permit.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let first_calls = Arc::clone(&calls);
    continue_desktop_startup_from_ready(permit, move |stage| {
        first_calls.lock().expect("calls").push(stage);
        Ok(())
    })
    .expect("first continuation");

    continue_desktop_startup_from_ready(replay, |_| panic!("stages must not replay"))
        .expect("idempotent replay");
    assert_eq!(calls.lock().expect("calls").len(), 3);
}

#[test]
fn failed_continuation_retries_only_the_failed_and_remaining_stages() {
    let mut context = DesktopStartupContext::new();
    for stage in StartupStageV1::PRE_READY {
        if stage != StartupStageV1::SyncRecovery {
            context.set_stage(stage, || Ok(()));
        }
    }
    context.set_sync_recovery(|| {
        Ok(Some(douyin_history_archive::startup_recovery::StartupContinuationPermitV1::new()))
    });
    let permit = recover_desktop_startup_once(&mut context)
        .expect("startup")
        .expect("permit");
    let replay = permit.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let first_calls = Arc::clone(&calls);
    assert_eq!(
        continue_desktop_startup_from_ready(permit, move |stage| {
            first_calls.lock().expect("calls").push(stage);
            if stage == StartupStageV1::TrayAndWindow {
                Err("tray_failed".to_string())
            } else {
                Ok(())
            }
        })
        .expect_err("first continuation fails"),
        "tray_failed"
    );

    let retry_calls = Arc::clone(&calls);
    continue_desktop_startup_from_ready(replay, move |stage| {
        retry_calls.lock().expect("calls").push(stage);
        Ok(())
    })
    .expect("retry remaining stages");

    assert_eq!(
        *calls.lock().expect("calls"),
        vec![
            StartupStageV1::DeviceAndOnboarding,
            StartupStageV1::TrayAndWindow,
            StartupStageV1::TrayAndWindow,
            StartupStageV1::Worker,
        ]
    );
}

#[test]
fn blocked_sync_recovery_does_not_run_the_no_running_assertion() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut context = DesktopStartupContext::new();
    for stage in [
        StartupStageV1::UninstallRecovery,
        StartupStageV1::Bootstrap,
        StartupStageV1::Schema,
        StartupStageV1::SchemaCleanup,
    ] {
        context.set_stage(stage, || Ok(()));
    }
    let recovery_log = Arc::clone(&log);
    context.set_sync_recovery(move || {
        push(&recovery_log, "blocked");
        Ok(None)
    });
    let assertion_log = Arc::clone(&log);
    context.set_stage(StartupStageV1::NoRunningAssertion, move || {
        push(&assertion_log, "no-running");
        Ok(())
    });

    assert!(recover_desktop_startup_once(&mut context)
        .expect("blocked result")
        .is_none());
    assert_eq!(context.state(), DesktopStartupState::Blocked);
    assert_eq!(*log.lock().expect("log"), vec!["blocked"]);
}

#[test]
fn startup_window_decision_never_hides_without_completed_onboarding_and_tray() {
    let visible_cases = [
        StartupWindowDecisionInputV1 {
            recovery_blocked: true,
            autostart: true,
            silent_autostart: true,
            onboarding_completed: true,
            tray_available: true,
        },
        StartupWindowDecisionInputV1 {
            recovery_blocked: false,
            autostart: false,
            silent_autostart: true,
            onboarding_completed: true,
            tray_available: true,
        },
        StartupWindowDecisionInputV1 {
            recovery_blocked: false,
            autostart: true,
            silent_autostart: true,
            onboarding_completed: false,
            tray_available: true,
        },
        StartupWindowDecisionInputV1 {
            recovery_blocked: false,
            autostart: true,
            silent_autostart: true,
            onboarding_completed: true,
            tray_available: false,
        },
    ];
    assert_eq!(
        startup_window_decision(visible_cases[0]),
        StartupWindowDecisionV1::ShowRecovery
    );
    for input in &visible_cases[1..] {
        assert_eq!(
            startup_window_decision(*input),
            StartupWindowDecisionV1::ShowMain
        );
    }
    assert_eq!(
        startup_window_decision(StartupWindowDecisionInputV1 {
            recovery_blocked: false,
            autostart: true,
            silent_autostart: true,
            onboarding_completed: true,
            tray_available: true,
        }),
        StartupWindowDecisionV1::KeepHidden
    );
}

#[test]
fn exit_failure_before_marker_aborts_without_stopping_or_cancelling() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission.clone());
    {
        let events = Arc::clone(&events);
        context.before_owned_wait = Box::new(move || {
            push(&events, "before-owned-wait");
            Ok(())
        });
    }
    let owned = admission
        .begin_device_settings_update()
        .expect("owned operation");
    context.owned_operation_timeout = Duration::from_millis(10);
    for (slot, label) in [
        (&mut context.write_marker, "marker"),
        (&mut context.stop_worker, "stop-worker"),
        (&mut context.cancel_running, "cancel-running"),
        (&mut context.sweep_queued, "sweep-queued"),
        (&mut context.remove_marker, "remove-marker"),
    ] {
        let events = Arc::clone(&events);
        *slot = Box::new(move || {
            push(&events, label);
            Ok(())
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect_err("exit must abort"),
        "operation_timeout"
    );
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::ExitPending
    );
    assert_eq!(
        *events.lock().expect("events"),
        Vec::<&'static str>::new()
    );
    drop(owned);
    assert_eq!(
        admission.snapshot().expect("reopened snapshot").mode,
        OperationAdmissionMode::Open
    );
}

#[test]
fn stable_root_is_read_only_after_owned_operations_finish() {
    let admission = OperationAdmission::new();
    let owned = admission
        .begin_device_settings_update()
        .expect("owned operation");
    let cancellation = owned.cancellation_token();
    let finishing = thread::spawn(move || {
        while !cancellation.is_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        thread::sleep(Duration::from_millis(50));
        drop(owned);
    });

    let mut context = ControlledExitContext::new(admission.clone());
    context.owned_operation_timeout = Duration::from_secs(1);
    let stable_admission = admission.clone();
    context.before_owned_wait = Box::new(move || {
        if stable_admission
            .snapshot()?
            .owned_operation_count
            != 1
        {
            return Err("stable_root_read_before_owned_operations_complete".to_string());
        }
        Ok(())
    });

    assert_eq!(
        request_app_exit(&mut context).expect("controlled exit"),
        ExitCompletionV1::Clean
    );
    finishing.join().expect("owned finalizer");
}

#[test]
fn exit_marker_is_the_linearization_point_and_cleanup_is_ordered() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission.clone());
    {
        let events = Arc::clone(&events);
        context.before_owned_wait = Box::new(move || {
            push(&events, "before-owned-wait");
            Ok(())
        });
    }
    for (slot, label) in [
        (&mut context.write_marker, "marker"),
        (&mut context.stop_worker, "stop-worker"),
        (&mut context.cancel_running, "cancel-running"),
        (&mut context.sweep_queued, "sweep-queued"),
        (&mut context.assert_idle, "assert-idle"),
        (&mut context.cleanup_task_files, "cleanup-files"),
        (&mut context.remove_marker, "remove-marker"),
    ] {
        let events = Arc::clone(&events);
        *slot = Box::new(move || {
            push(&events, label);
            Ok(())
        });
    }
    {
        let events = Arc::clone(&events);
        context.wait_runtime = Box::new(move |timeout| {
            assert!(timeout <= Duration::from_secs(10));
            push(&events, "wait-runtime");
            Ok(true)
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect("controlled exit"),
        ExitCompletionV1::Clean
    );
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::Exiting
    );
    assert_eq!(
        *events.lock().expect("events"),
        vec![
            "before-owned-wait",
            "marker",
            "stop-worker",
            "cancel-running",
            "wait-runtime",
            "sweep-queued",
            "assert-idle",
            "cleanup-files",
            "remove-marker",
        ]
    );
}

#[test]
fn marker_write_failure_aborts_before_any_destructive_action() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission.clone());
    {
        let events = Arc::clone(&events);
        context.write_marker = Box::new(move || {
            push(&events, "marker-failed");
            Err("shutdown_marker_write_failed".to_string())
        });
    }
    {
        let events = Arc::clone(&events);
        context.stop_worker = Box::new(move || {
            push(&events, "stop-worker");
            Ok(())
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect_err("marker failure"),
        "shutdown_marker_write_failed"
    );
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::Open
    );
    assert_eq!(*events.lock().expect("events"), vec!["marker-failed"]);
}

#[test]
fn hard_timeout_preserves_marker_and_skips_the_final_queue_sweep() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission.clone());
    for (slot, label) in [
        (&mut context.write_marker, "marker"),
        (&mut context.stop_worker, "stop-worker"),
        (&mut context.cancel_running, "cancel-running"),
        (&mut context.sweep_queued, "sweep-queued"),
        (&mut context.remove_marker, "remove-marker"),
        (&mut context.best_effort_cleanup, "best-effort"),
    ] {
        let events = Arc::clone(&events);
        *slot = Box::new(move || {
            push(&events, label);
            Ok(())
        });
    }
    {
        let events = Arc::clone(&events);
        context.wait_runtime = Box::new(move |_| {
            push(&events, "wait-runtime");
            Ok(false)
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect("hard timeout result"),
        ExitCompletionV1::HardTimeout
    );
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::Exiting
    );
    assert_eq!(
        *events.lock().expect("events"),
        vec![
            "marker",
            "stop-worker",
            "cancel-running",
            "wait-runtime",
            "wait-runtime",
            "best-effort",
        ]
    );
}

#[test]
fn runtime_wait_forces_process_cleanup_after_grace_then_uses_remaining_deadline() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let waits = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission);
    context.process_cleanup_grace = Duration::from_millis(20);
    context.total_timeout = Duration::from_millis(100);
    {
        let waits = Arc::clone(&waits);
        let events = Arc::clone(&events);
        context.wait_runtime = Box::new(move |timeout| {
            waits.lock().expect("waits").push(timeout);
            let mut events = events.lock().expect("events");
            events.push("wait-runtime");
            let first = events.iter().filter(|value| **value == "wait-runtime").count() == 1;
            drop(events);
            if first {
                thread::sleep(timeout);
                Ok(false)
            } else {
                Ok(true)
            }
        });
    }
    {
        let events = Arc::clone(&events);
        context.force_process_cleanup = Box::new(move || {
            push(&events, "force-processes");
            Ok(())
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect("controlled exit"),
        ExitCompletionV1::Clean
    );
    let waits = waits.lock().expect("wait durations");
    assert_eq!(waits.len(), 2);
    assert!(waits[0] <= Duration::from_millis(20));
    assert!(waits[1] <= Duration::from_millis(80));
    assert_eq!(
        *events.lock().expect("events"),
        vec!["wait-runtime", "force-processes", "wait-runtime"]
    );
}

#[test]
fn post_marker_failure_preserves_marker_and_uses_hard_timeout_cleanup() {
    let admission = OperationAdmission::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut context = ControlledExitContext::new(admission.clone());
    {
        let events = Arc::clone(&events);
        context.write_marker = Box::new(move || {
            push(&events, "marker");
            Ok(())
        });
    }
    {
        let events = Arc::clone(&events);
        context.stop_worker = Box::new(move || {
            push(&events, "stop-worker-failed");
            Err("worker_quiesce_failed".to_string())
        });
    }
    {
        let events = Arc::clone(&events);
        context.best_effort_cleanup = Box::new(move || {
            push(&events, "best-effort");
            Ok(())
        });
    }

    assert_eq!(
        request_app_exit(&mut context).expect("hard timeout result"),
        ExitCompletionV1::HardTimeout
    );
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::Exiting
    );
    assert_eq!(
        *events.lock().expect("events"),
        vec!["marker", "stop-worker-failed", "best-effort"]
    );
}

#[test]
fn explicit_abort_before_marker_restores_open_idempotently() {
    let admission = OperationAdmission::new();
    let exit = admission.begin_exit().expect("exit pending");
    abort_exit_before_marker(exit, "bootstrap_unavailable").expect("abort");
    assert_eq!(
        admission.snapshot().expect("snapshot").mode,
        OperationAdmissionMode::Open
    );
}

#[test]
fn background_task_registry_waits_for_registered_monitor_completion() {
    let registry = BackgroundTaskRegistry::default();
    let task = registry.register().expect("register monitor");
    assert_eq!(registry.active_count().expect("active count"), 1);

    let completing = thread::spawn(move || {
        thread::sleep(Duration::from_millis(25));
        drop(task);
    });

    assert!(registry
        .wait_until_idle(Duration::from_secs(1))
        .expect("wait for monitor"));
    completing.join().expect("monitor completion");
    assert_eq!(registry.active_count().expect("idle count"), 0);
}

#[test]
fn closing_background_task_registry_rejects_new_tasks_but_waits_for_existing_tasks() {
    let registry = BackgroundTaskRegistry::default();
    let task = registry.register().expect("register existing task");

    registry.close().expect("close registrations");
    assert_eq!(
        registry.register().err().as_deref(),
        Some("background_task_registry_closed")
    );
    drop(task);
    assert!(registry
        .wait_until_idle(Duration::from_secs(1))
        .expect("wait existing task"));
}

#[test]
fn exit_request_gate_prevents_first_request_and_allows_only_after_cleanup() {
    let gate = ExitRequestGate::default();

    assert_eq!(
        gate.on_exit_requested(true).expect("first request"),
        ExitRequestDecisionV1::PreventAndStart
    );
    assert_eq!(
        gate.on_exit_requested(true).expect("duplicate request"),
        ExitRequestDecisionV1::Prevent
    );

    gate.mark_cleanup_complete().expect("cleanup complete");
    assert_eq!(
        gate.on_exit_requested(true).expect("final request"),
        ExitRequestDecisionV1::Allow
    );
}

#[test]
fn exit_before_normal_runtime_is_ready_does_not_start_controlled_cleanup() {
    let gate = ExitRequestGate::default();
    assert_eq!(
        gate.on_exit_requested(false).expect("early exit"),
        ExitRequestDecisionV1::Allow
    );
    assert_eq!(
        gate.on_exit_requested(true).expect("later normal exit"),
        ExitRequestDecisionV1::PreventAndStart
    );
}
