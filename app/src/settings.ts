import type {
  AppSettingsView,
  DetailFields,
  LegacyFrontendSettings,
  LikeCountFormat,
  SyncMode,
} from "./types.js";

export const LEGACY_SETTINGS_KEY = "douyin-history-settings";

export const LIKE_COUNT_FORMAT_OPTIONS: ReadonlyArray<{
  value: LikeCountFormat;
  label: string;
}> = [
  { value: "exact", label: "准确数字" },
  { value: "w", label: "W" },
  { value: "wan", label: "万" },
  { value: "k", label: "K" },
  { value: "qian", label: "千" },
];

const likeCountFormats = new Set<LikeCountFormat>(
  LIKE_COUNT_FORMAT_OPTIONS.map((option) => option.value),
);

export function normalizeLikeCountFormat(value: unknown): LikeCountFormat {
  return typeof value === "string" && likeCountFormats.has(value as LikeCountFormat)
    ? (value as LikeCountFormat)
    : "wan";
}

export function normalizeSyncDays(value: unknown): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.max(1, Math.min(365, Math.floor(number))) : 7;
}

export function normalizeScrollSpeed(value: unknown): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.max(120, Math.floor(number)) : 900;
}

export function normalizeLegacyFrontendSettings(raw: string): LegacyFrontendSettings | null {
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const fields = parsed.detailFields;
    const detailFields: DetailFields = {
      author: readBooleanField(fields, "author", true),
      watchDate: readBooleanField(fields, "watchDate", true),
      likeCount: readBooleanField(fields, "likeCount", true),
      watchProgress: readBooleanField(fields, "watchProgress", true),
    };
    return {
      syncMode: parsed.syncMode === "bottom" ? "bottom" : "days",
      syncDays: normalizeSyncDays(parsed.syncDays),
      scrollSpeed: normalizeScrollSpeed(parsed.scrollSpeed),
      hideBrowser: parsed.hideBrowser === true,
      likeCountFormat: normalizeLikeCountFormat(parsed.likeCountFormat),
      detailFields,
    };
  } catch {
    return null;
  }
}

function readBooleanField(
  value: unknown,
  field: keyof DetailFields,
  fallback: boolean,
): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fallback;
  const candidate = (value as Partial<Record<keyof DetailFields, unknown>>)[field];
  return typeof candidate === "boolean" ? candidate : fallback;
}

export class SettingsSaveQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue<T>(task: () => Promise<T>): Promise<T> {
    const result = this.tail.then(task, task);
    this.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

export class SettingsRequestGeneration {
  private value = 0;

  begin(): number {
    this.value += 1;
    return this.value;
  }

  invalidate(): void {
    this.value += 1;
  }

  isCurrent(generation: number): boolean {
    return generation === this.value;
  }
}

interface LegacySettingsLoadOptions {
  accountId: number | null;
  storage: Storage;
  getAppSettings: (accountId: number | null) => Promise<AppSettingsView>;
  importLegacySettings: (
    accountId: number | null,
    settings: LegacyFrontendSettings,
  ) => Promise<number>;
}

export async function loadSettingsWithLegacyImport({
  accountId,
  storage,
  getAppSettings,
  importLegacySettings,
}: LegacySettingsLoadOptions): Promise<AppSettingsView> {
  const initial = await getAppSettings(accountId);
  const raw = storage.getItem(LEGACY_SETTINGS_KEY);
  if (!initial.legacyImportRequired) {
    if (raw !== null) storage.removeItem(LEGACY_SETTINGS_KEY);
    return initial;
  }
  if (raw === null) return initial;
  const legacy = normalizeLegacyFrontendSettings(raw);
  if (!legacy) return initial;

  try {
    await importLegacySettings(accountId, legacy);
  } catch (error) {
    const confirmed = await getAppSettings(accountId).catch(() => null);
    if (confirmed && !confirmed.legacyImportRequired) {
      storage.removeItem(LEGACY_SETTINGS_KEY);
      return confirmed;
    }
    throw error;
  }

  const confirmed = await getAppSettings(accountId);
  if (confirmed.legacyImportRequired) {
    throw new Error("legacy_settings_import_not_confirmed");
  }
  storage.removeItem(LEGACY_SETTINGS_KEY);
  return confirmed;
}

const DATA_ROOT_ERROR_MESSAGES: Record<string, string> = {
  data_root_not_absolute: "请选择绝对路径。",
  data_root_not_unicode: "该路径包含当前版本不支持的字符。",
  data_root_is_drive_root: "不能将数据库文件夹设置在磁盘根目录。",
  data_root_is_remote: "暂不支持网络位置或映射网络磁盘。",
  data_root_overlaps_current: "新目录不能与当前数据目录互为父子目录。",
  data_root_not_empty: "目标文件夹必须为空，且不能包含未知文件。",
  data_root_reparse_point: "目标路径包含符号链接或联接点，无法安全使用。",
  data_root_permission_denied: "当前用户没有使用该目录所需的读写权限。",
  data_root_wal_probe_failed: "该目录无法可靠创建 SQLite WAL 文件。",
  data_root_migration_in_progress: "数据目录正在迁移，请稍候。",
  data_root_checkpoint_busy: "数据库仍在使用中，请稍后重试迁移。",
  data_root_verification_failed: "迁移副本校验失败，仍在使用原目录。",
  data_root_not_retained: "该旧数据目录已不存在或不再受应用管理。",
};

export function dataRootErrorMessage(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  return DATA_ROOT_ERROR_MESSAGES[code] ?? `数据目录操作失败：${code}`;
}
