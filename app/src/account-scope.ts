import type { AccountScope, PortableAccountScope } from "./types";

interface AccountScopeIdentity {
  id: number;
  accountKey?: string;
}

export function resolvePortableAccountScope(
  scope: PortableAccountScope,
  accounts: AccountScopeIdentity[],
): AccountScope {
  if (scope.kind === "all") return { kind: "all" };
  if (scope.accountKeys.length === 0 || new Set(scope.accountKeys).size !== scope.accountKeys.length) {
    throw new Error("account_scope_invalid");
  }

  const idsByKey = new Map(
    accounts
      .filter((account): account is AccountScopeIdentity & { accountKey: string } =>
        Boolean(account.accountKey),
      )
      .map((account) => [account.accountKey, account.id]),
  );
  const accountIds = scope.accountKeys.map((key) => idsByKey.get(key));
  if (accountIds.some((id) => id == null)) throw new Error("account_scope_invalid");
  return {
    kind: "selected",
    accountIds: (accountIds as number[]).sort((left, right) => left - right),
  };
}
