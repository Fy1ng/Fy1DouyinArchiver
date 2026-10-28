use super::{cdp::CdpPage, deadline::Deadline};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

const SAVE_ACTION_OBSCURED_PREFIX: &str = "login_action_unavailable:obscured";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginAction {
    OneClickLogin,
    SaveLogin,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ActionTargetResponse {
    Found {
        action: LoginAction,
        x: f64,
        y: f64,
        token: String,
    },
    NotFound {
        action: LoginAction,
        reason: String,
        count: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct ActionTarget {
    x: f64,
    y: f64,
    token: String,
}

pub async fn click_login_action_once(page: &CdpPage, action: LoginAction, deadline: Deadline) -> Result<bool, String> {
    let Some(first) = find_target(page, action, deadline).await? else {
        return Ok(false);
    };
    let Some(second) = find_target(page, action, deadline).await? else {
        return Err("login_action_changed".to_string());
    };
    if first != second {
        return Err("login_action_changed".to_string());
    }
    page.dispatch_left_click_with_deadline(second.x, second.y, deadline).await?;
    Ok(true)
}

pub async fn process_optional_save_login(page: &CdpPage, wait: Duration, deadline: Deadline) -> Result<bool, String> {
    let modal_deadline = deadline.capped(wait)?;
    loop {
        match click_login_action_once(page, LoginAction::SaveLogin, modal_deadline).await {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) if error == "collector_deadline_exceeded" && deadline.remaining().is_ok() => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
        if modal_deadline.remaining().is_err() {
            return Ok(false);
        }
        match modal_deadline.run(async { page.wait_for_timeout(100).await; Ok(()) }).await {
            Ok(()) => {}
            Err(error) if error == "collector_deadline_exceeded" && deadline.remaining().is_ok() => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
    }
    loop {
        match find_target(page, LoginAction::SaveLogin, modal_deadline).await {
            Ok(None) => return Ok(true),
            Ok(Some(_)) => {}
            Err(error) if error.starts_with(SAVE_ACTION_OBSCURED_PREFIX) => {}
            Err(error)
                if error == "collector_deadline_exceeded" && deadline.remaining().is_ok() =>
            {
                return Err("save_login_modal_persisted".to_string())
            }
            Err(error) => return Err(error),
        }
        if modal_deadline.remaining().is_err() {
            return Err("save_login_modal_persisted".to_string());
        }
        match modal_deadline.run(async { page.wait_for_timeout(100).await; Ok(()) }).await {
            Ok(()) => {}
            Err(error)
                if error == "collector_deadline_exceeded" && deadline.remaining().is_ok() =>
            {
                return Err("save_login_modal_persisted".to_string())
            }
            Err(error) => return Err(error),
        }
    }
}

async fn find_target(page: &CdpPage, action: LoginAction, deadline: Deadline) -> Result<Option<ActionTarget>, String> {
    let response: ActionTargetResponse = page
        .call_function_with_deadline(
            include_str!("../../collector-scripts/login-action-target.js"),
            json!({"action": action}),
            deadline,
        )
        .await?;
    match response {
        ActionTargetResponse::Found {
            action: observed,
            x,
            y,
            token,
        } if observed == action && x.is_finite() && y.is_finite() && !token.is_empty() => {
            Ok(Some(ActionTarget { x, y, token }))
        }
        ActionTargetResponse::Found { .. } => Err("invalid_login_action_target".to_string()),
        ActionTargetResponse::NotFound {
            action: observed,
            reason,
            count,
        } if observed == action && reason == "no_candidate" && count == 0 => Ok(None),
        ActionTargetResponse::NotFound { reason, .. } => {
            Err(format!("login_action_unavailable:{reason}"))
        }
    }
}
