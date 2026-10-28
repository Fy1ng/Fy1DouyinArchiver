#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceStage {
    Login,
    RecoverLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceDecision {
    Continue,
    Fail,
}

pub fn classify_login_resource(
    _stage: ResourceStage,
    resource_type: &str,
    _url: &str,
) -> ResourceDecision {
    if resource_type == "Media" {
        ResourceDecision::Fail
    } else {
        ResourceDecision::Continue
    }
}
