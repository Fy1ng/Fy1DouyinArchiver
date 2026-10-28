export function accountLoginButtonLabel(inProgress: boolean): string {
  return inProgress ? "等待登录…" : "添加账号";
}

export function accountRecoveryButtonLabel(inProgress: boolean): string {
  return inProgress ? "等待登录…" : "一键登录";
}

export function accountLoginErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  switch (message) {
    case "account_login_browser_closed":
      return "登录浏览器已关闭，未添加账号";
    case "account_login_cancelled":
      return "登录已取消，未添加账号";
    default:
      return message;
  }
}
