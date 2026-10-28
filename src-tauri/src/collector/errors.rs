#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorErrorDescription {
    pub code: String,
    pub message: String,
}

pub fn describe_collector_error(error: &str) -> CollectorErrorDescription {
    let translated = match error {
        "system_browser_not_found" => Some("未找到可用的系统 Edge 或 Chrome 浏览器。"),
        "login_validation_required" => Some("请先完成浏览器登录验证。"),
        "browser_lock_invalid" => Some("浏览器进程锁无效或已失效。"),
        "browser_desktop_create_failed" => Some("无法创建隐藏浏览器桌面。"),
        "cdp_start_timeout" => Some("等待浏览器 CDP 服务启动超时。"),
        "cdp_connection_failed" => Some("无法连接到浏览器 CDP 服务。"),
        "cdp_target_closed" => Some("浏览器页面已关闭。"),
        "cdp_evaluate_failed" => Some("浏览器页面脚本执行失败。"),
        "navigation_failed" => Some("浏览器页面导航失败。"),
        "account_identity_unavailable" => Some("无法从已登录的账号页面读取可靠身份。"),
        "account_identity_ambiguous" => Some("页面中检测到互相冲突的登录账号身份。"),
        "identity_result_write_failed" => Some("无法保存账号身份验证结果。"),
        "collector_cancelled" => Some("同步已取消。"),
        "session_expired" => Some("抖音登录状态已失效。"),
        "archive_page_not_ready" => Some("归档页面尚未准备完成。"),
        "favorite_tab_not_found" => Some("未能进入默认收藏内容页。"),
        "favorite_folder_tab_not_found" => Some("未能进入抖音收藏夹目录。"),
        "favorite_navigation_not_visible" => Some("收藏页面入口当前不可见。"),
        "remote_folder_list_not_found" => Some("未能读取抖音收藏夹目录。"),
        "remote_folder_not_found" => Some("未找到请求的抖音收藏夹。"),
        "remote_folder_not_selected" => Some("未能切换到指定的远端收藏夹。"),
        "ambiguous_archive_list" => Some("页面中检测到多个可能的归档列表。"),
        "archive_scroll_target_invalid" => Some("未能定位归档列表的滚动区域。"),
        "ambiguous_remote_folder_react_list" => Some("页面中检测到冲突的收藏夹目录数据。"),
        "archive_network_api_error" => Some("抖音收藏夹目录接口返回了错误。"),
        "archive_network_response_shape_changed" => Some("抖音收藏夹目录数据格式发生变化。"),
        "archive_network_response_too_large" => Some("抖音收藏夹目录响应过大。"),
        "archive_snapshot_prefix_changed" => Some("归档列表顺序在采集过程中发生变化。"),
        "final_stability_failed" => Some("归档列表在最终复核时仍不稳定。"),
        _ => None,
    };
    if let Some(message) = translated {
        return CollectorErrorDescription {
            code: error.to_string(),
            message: message.to_string(),
        };
    }
    if error.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || "_:-".contains(character)
    }) {
        return CollectorErrorDescription {
            code: error.to_string(),
            message: error.to_string(),
        };
    }
    CollectorErrorDescription {
        code: "collector_failed".to_string(),
        message: "采集器运行失败。".to_string(),
    }
}
