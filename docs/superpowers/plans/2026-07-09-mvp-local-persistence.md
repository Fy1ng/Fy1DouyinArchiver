# MVP Local Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the MVP data core that persists Douyin viewing history into SQLite with repeatable imports, deduplication, and queryable history.

**Architecture:** Keep the existing Playwright prototype as the collection engine for now. Add a local persistence layer that can import prototype JSON output into SQLite, then wire the prototype runner to write records into the database after each sync. The later Tauri app will call this same data layer instead of parsing JSON directly.

**Tech Stack:** TypeScript, Vitest, Playwright, SQLite via `better-sqlite3` for the Node-side MVP core.

## Global Constraints

- Stage two remains focused on viewing history only.
- Do not implement likes, favorites, favorite folders, tray, autostart, background scheduler, or analytics in this milestone.
- Do not invent exact `watched_at`; keep `page_group_label` and `collected_at` as separate time fields.
- Preserve the current prototype JSON output for debugging and verification.
- Repeated imports must not create duplicate videos or duplicate watch records for the same source identity.
- All data stays local under `data/`.

## Completion Status

Completed on 2026-07-09:

- SQLite schema/store implemented with repeat-sync dedupe.
- Prototype JSON import CLI implemented as `npm run import:prototype`.
- Sync job metadata implemented with `running`/`success`/`failed` status.
- Prototype runner supports `--db <file>` and still preserves JSON output.
- Verified with `npm test`, `npm run typecheck`, `npm run prototype:dry-run -- --db data/test-dry-run.sqlite`, repeated JSON imports, and a real `--scrolls 5 --db data/history.sqlite` smoke run.

---

## File Structure

- Create `src/storage/schema.ts`: SQLite schema constants and schema initializer.
- Create `src/storage/database.ts`: database open/close helper and typed database wrapper.
- Create `src/storage/history-store.ts`: insert/query functions for videos, watch records, and sync jobs.
- Create `src/storage/import-prototype-output.ts`: import prototype JSON records into SQLite.
- Create `scripts/import-prototype-output.ts`: CLI for importing an existing JSON output.
- Modify `scripts/douyin-history-poc.ts`: optionally write the current sync result into SQLite.
- Modify `package.json`: add import script and SQLite dependency.
- Create `tests/storage/history-store.test.ts`: persistence and dedupe tests.
- Create `tests/storage/import-prototype-output.test.ts`: JSON import tests.

---

### Task 1: SQLite Schema And Store

**Files:**
- Create: `src/storage/schema.ts`
- Create: `src/storage/database.ts`
- Create: `src/storage/history-store.ts`
- Test: `tests/storage/history-store.test.ts`
- Modify: `package.json`

**Interfaces:**
- Produces: `openHistoryDatabase(filePath: string): HistoryDatabase`
- Produces: `initializeHistorySchema(db: Database.Database): void`
- Produces: `upsertHistoryRecords(db: HistoryDatabase, records: HistoryRecord[], syncJobId?: number): ImportSummary`
- Produces: `listHistoryRecords(db: HistoryDatabase, input: ListHistoryInput): StoredHistoryRecord[]`

- [ ] **Step 1: Install SQLite dependency**

Run:

```powershell
npm install better-sqlite3
npm install -D @types/better-sqlite3
```

Expected: `package.json` and `package-lock.json` include `better-sqlite3` and `@types/better-sqlite3`.

- [ ] **Step 2: Write failing store test**

Create `tests/storage/history-store.test.ts`:

```ts
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { openHistoryDatabase } from "../../src/storage/database.js";
import {
  listHistoryRecords,
  upsertHistoryRecords,
} from "../../src/storage/history-store.js";
import type { HistoryRecord } from "../../src/history-records.js";

describe("history-store", () => {
  let tempDir: string | undefined;

  afterEach(() => {
    if (tempDir) {
      rmSync(tempDir, { recursive: true, force: true });
      tempDir = undefined;
    }
  });

  function createDb() {
    tempDir = mkdtempSync(path.join(tmpdir(), "douyin-history-db-"));
    return openHistoryDatabase(path.join(tempDir, "history.sqlite"));
  }

  it("deduplicates videos and watch records across repeated imports", () => {
    const db = createDb();
    const record: HistoryRecord = {
      aweme_id: "123",
      url: "https://www.douyin.com/video/123",
      title: "测试视频",
      author_name: "测试作者",
      cover_url: "https://example.test/cover.jpg",
      page_group_label: "昨天",
      collected_at: "2026-07-09T00:00:00.000Z",
      source: "dom",
      source_url: "https://www.douyin.com/user/self?showTab=record",
    };

    expect(upsertHistoryRecords(db, [record])).toEqual({
      insertedVideos: 1,
      insertedWatchRecords: 1,
      skippedRecords: 0,
    });
    expect(upsertHistoryRecords(db, [record])).toEqual({
      insertedVideos: 0,
      insertedWatchRecords: 0,
      skippedRecords: 0,
    });

    const rows = listHistoryRecords(db, { limit: 20, offset: 0 });
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      awemeId: "123",
      title: "测试视频",
      authorName: "测试作者",
      pageGroupLabel: "昨天",
    });

    db.close();
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run:

```powershell
npm test -- tests/storage/history-store.test.ts
```

Expected: FAIL because `src/storage/database.ts` and `src/storage/history-store.ts` do not exist.

- [ ] **Step 4: Implement schema**

Create `src/storage/schema.ts`:

```ts
import type Database from "better-sqlite3";

export function initializeHistorySchema(db: Database.Database): void {
  db.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS videos (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      aweme_id TEXT UNIQUE,
      canonical_url TEXT NOT NULL UNIQUE,
      title TEXT,
      author_name TEXT,
      cover_url TEXT,
      first_seen_at TEXT NOT NULL,
      last_seen_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS sync_jobs (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      started_at TEXT NOT NULL,
      finished_at TEXT,
      source_file TEXT,
      status TEXT NOT NULL,
      inserted_videos INTEGER NOT NULL DEFAULT 0,
      inserted_watch_records INTEGER NOT NULL DEFAULT 0,
      error_message TEXT
    );

    CREATE TABLE IF NOT EXISTS watch_records (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
      page_group_label TEXT,
      collected_at TEXT NOT NULL,
      source TEXT NOT NULL,
      source_url TEXT,
      sync_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
      created_at TEXT NOT NULL,
      UNIQUE(video_id, page_group_label, collected_at, source, COALESCE(source_url, ''))
    );

    CREATE INDEX IF NOT EXISTS idx_watch_records_collected_at
      ON watch_records(collected_at DESC);
    CREATE INDEX IF NOT EXISTS idx_videos_title
      ON videos(title);
    CREATE INDEX IF NOT EXISTS idx_videos_author
      ON videos(author_name);
  `);
}
```

- [ ] **Step 5: Implement database helper**

Create `src/storage/database.ts`:

```ts
import Database from "better-sqlite3";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { initializeHistorySchema } from "./schema.js";

export interface HistoryDatabase {
  raw: Database.Database;
  close(): void;
}

export function openHistoryDatabase(filePath: string): HistoryDatabase {
  mkdirSync(path.dirname(filePath), { recursive: true });
  const raw = new Database(filePath);
  initializeHistorySchema(raw);
  return {
    raw,
    close: () => raw.close(),
  };
}
```

- [ ] **Step 6: Implement history store**

Create `src/storage/history-store.ts`:

```ts
import type { HistoryRecord } from "../history-records.js";
import type { HistoryDatabase } from "./database.js";

export interface ImportSummary {
  insertedVideos: number;
  insertedWatchRecords: number;
  skippedRecords: number;
}

export interface ListHistoryInput {
  limit: number;
  offset: number;
  query?: string;
}

export interface StoredHistoryRecord {
  id: number;
  awemeId?: string;
  url: string;
  title?: string;
  authorName?: string;
  coverUrl?: string;
  pageGroupLabel?: string;
  collectedAt: string;
  source: string;
  sourceUrl?: string;
}

export function upsertHistoryRecords(
  db: HistoryDatabase,
  records: HistoryRecord[],
  syncJobId?: number,
): ImportSummary {
  const now = new Date().toISOString();
  const insertVideo = db.raw.prepare(`
    INSERT INTO videos (aweme_id, canonical_url, title, author_name, cover_url, first_seen_at, last_seen_at)
    VALUES (@awemeId, @url, @title, @authorName, @coverUrl, @now, @now)
    ON CONFLICT(canonical_url) DO UPDATE SET
      aweme_id = COALESCE(excluded.aweme_id, videos.aweme_id),
      title = COALESCE(excluded.title, videos.title),
      author_name = COALESCE(excluded.author_name, videos.author_name),
      cover_url = COALESCE(excluded.cover_url, videos.cover_url),
      last_seen_at = excluded.last_seen_at
  `);
  const findVideo = db.raw.prepare<{ url: string }, { id: number }>(
    "SELECT id FROM videos WHERE canonical_url = @url",
  );
  const insertWatch = db.raw.prepare(`
    INSERT OR IGNORE INTO watch_records
      (video_id, page_group_label, collected_at, source, source_url, sync_job_id, created_at)
    VALUES
      (@videoId, @pageGroupLabel, @collectedAt, @source, @sourceUrl, @syncJobId, @now)
  `);

  let insertedVideos = 0;
  let insertedWatchRecords = 0;
  let skippedRecords = 0;

  const transaction = db.raw.transaction(() => {
    for (const record of records) {
      if (!record.url) {
        skippedRecords += 1;
        continue;
      }

      const videoResult = insertVideo.run({
        awemeId: record.aweme_id,
        url: record.url,
        title: record.title,
        authorName: record.author_name,
        coverUrl: record.cover_url,
        now,
      });
      if (videoResult.changes > 0 && videoResult.lastInsertRowid) {
        insertedVideos += 1;
      }

      const video = findVideo.get({ url: record.url });
      if (!video) {
        skippedRecords += 1;
        continue;
      }

      const watchResult = insertWatch.run({
        videoId: video.id,
        pageGroupLabel: record.page_group_label,
        collectedAt: record.collected_at,
        source: record.source,
        sourceUrl: record.source_url,
        syncJobId,
        now,
      });
      insertedWatchRecords += watchResult.changes;
    }
  });

  transaction();
  return { insertedVideos, insertedWatchRecords, skippedRecords };
}

export function listHistoryRecords(
  db: HistoryDatabase,
  input: ListHistoryInput,
): StoredHistoryRecord[] {
  const limit = Math.max(1, Math.min(input.limit, 200));
  const offset = Math.max(0, input.offset);
  const query = input.query?.trim();
  const rows = db.raw
    .prepare(`
      SELECT
        wr.id,
        v.aweme_id AS awemeId,
        v.canonical_url AS url,
        v.title,
        v.author_name AS authorName,
        v.cover_url AS coverUrl,
        wr.page_group_label AS pageGroupLabel,
        wr.collected_at AS collectedAt,
        wr.source,
        wr.source_url AS sourceUrl
      FROM watch_records wr
      JOIN videos v ON v.id = wr.video_id
      WHERE
        @query IS NULL
        OR v.title LIKE @likeQuery
        OR v.author_name LIKE @likeQuery
      ORDER BY wr.collected_at DESC, wr.id DESC
      LIMIT @limit OFFSET @offset
    `)
    .all({
      query: query || null,
      likeQuery: query ? `%${query}%` : null,
      limit,
      offset,
    });

  return rows as StoredHistoryRecord[];
}
```

- [ ] **Step 7: Run tests**

Run:

```powershell
npm test -- tests/storage/history-store.test.ts
npm run typecheck
```

Expected: PASS.

---

### Task 2: Prototype JSON Import CLI

**Files:**
- Create: `src/storage/import-prototype-output.ts`
- Create: `scripts/import-prototype-output.ts`
- Test: `tests/storage/import-prototype-output.test.ts`
- Modify: `package.json`

**Interfaces:**
- Consumes: `openHistoryDatabase(filePath)`
- Consumes: `upsertHistoryRecords(db, records, syncJobId?)`
- Produces: `importPrototypeOutput(input: ImportPrototypeOutputInput): ImportPrototypeOutputResult`

- [ ] **Step 1: Write failing import test**

Create `tests/storage/import-prototype-output.test.ts`:

```ts
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { openHistoryDatabase } from "../../src/storage/database.js";
import { listHistoryRecords } from "../../src/storage/history-store.js";
import { importPrototypeOutput } from "../../src/storage/import-prototype-output.js";

describe("importPrototypeOutput", () => {
  let tempDir: string | undefined;

  afterEach(() => {
    if (tempDir) {
      rmSync(tempDir, { recursive: true, force: true });
      tempDir = undefined;
    }
  });

  it("imports records from prototype JSON into SQLite", async () => {
    tempDir = mkdtempSync(path.join(tmpdir(), "douyin-history-import-"));
    const jsonPath = path.join(tempDir, "prototype.json");
    const dbPath = path.join(tempDir, "history.sqlite");
    writeFileSync(
      jsonPath,
      JSON.stringify({
        generated_at: "2026-07-09T00:00:00.000Z",
        records: [
          {
            aweme_id: "123",
            url: "https://www.douyin.com/video/123",
            title: "测试视频",
            author_name: "测试作者",
            collected_at: "2026-07-09T00:00:00.000Z",
            source: "dom",
            page_group_label: "昨天",
          },
        ],
      }),
    );

    const result = await importPrototypeOutput({ dbPath, jsonPath });
    expect(result.insertedWatchRecords).toBe(1);

    const db = openHistoryDatabase(dbPath);
    expect(listHistoryRecords(db, { limit: 20, offset: 0 })).toHaveLength(1);
    db.close();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
npm test -- tests/storage/import-prototype-output.test.ts
```

Expected: FAIL because `importPrototypeOutput` does not exist.

- [ ] **Step 3: Implement importer**

Create `src/storage/import-prototype-output.ts`:

```ts
import { readFile } from "node:fs/promises";
import type { HistoryRecord } from "../history-records.js";
import { openHistoryDatabase } from "./database.js";
import { upsertHistoryRecords, type ImportSummary } from "./history-store.js";

export interface ImportPrototypeOutputInput {
  dbPath: string;
  jsonPath: string;
}

export interface ImportPrototypeOutputResult extends ImportSummary {
  sourceFile: string;
  totalInputRecords: number;
}

interface PrototypeOutputLike {
  records?: unknown;
}

export async function importPrototypeOutput(
  input: ImportPrototypeOutputInput,
): Promise<ImportPrototypeOutputResult> {
  const raw = await readFile(input.jsonPath, "utf8");
  const parsed = JSON.parse(raw) as PrototypeOutputLike;
  const records = Array.isArray(parsed.records)
    ? parsed.records.filter(isHistoryRecord)
    : [];

  const db = openHistoryDatabase(input.dbPath);
  try {
    const summary = upsertHistoryRecords(db, records);
    return {
      ...summary,
      sourceFile: input.jsonPath,
      totalInputRecords: records.length,
    };
  } finally {
    db.close();
  }
}

function isHistoryRecord(value: unknown): value is HistoryRecord {
  if (!value || typeof value !== "object") {
    return false;
  }

  const record = value as Partial<HistoryRecord>;
  return Boolean(record.url && record.collected_at && record.source);
}
```

- [ ] **Step 4: Implement CLI**

Create `scripts/import-prototype-output.ts`:

```ts
import process from "node:process";
import { importPrototypeOutput } from "../src/storage/import-prototype-output.js";

const DEFAULT_DB_PATH = "data/history.sqlite";

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const jsonPath = args[0];
  const dbPath = args[1] ?? DEFAULT_DB_PATH;

  if (!jsonPath) {
    throw new Error("用法：npm run import:prototype -- <prototype-json> [db-path]");
  }

  const result = await importPrototypeOutput({ jsonPath, dbPath });
  console.log(
    `导入完成：输入 ${result.totalInputRecords}，新增视频 ${result.insertedVideos}，新增观看记录 ${result.insertedWatchRecords}，跳过 ${result.skippedRecords}`,
  );
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
```

- [ ] **Step 5: Add npm script**

Modify `package.json`:

```json
"import:prototype": "tsx scripts/import-prototype-output.ts"
```

- [ ] **Step 6: Run tests and smoke import**

Run:

```powershell
npm test -- tests/storage/import-prototype-output.test.ts
npm run typecheck
npm run import:prototype -- data/prototype/load-stop-long-2.json data/history.sqlite
```

Expected: test/typecheck PASS, CLI prints import counts.

---

### Task 3: Sync Job Metadata

**Files:**
- Modify: `src/storage/history-store.ts`
- Test: `tests/storage/history-store.test.ts`

**Interfaces:**
- Produces: `startSyncJob(db: HistoryDatabase, input: StartSyncJobInput): number`
- Produces: `finishSyncJob(db: HistoryDatabase, input: FinishSyncJobInput): void`

- [ ] **Step 1: Write failing sync job test**

Append to `tests/storage/history-store.test.ts`:

```ts
import { finishSyncJob, startSyncJob } from "../../src/storage/history-store.js";

it("tracks sync job status and import counts", () => {
  const db = createDb();
  const id = startSyncJob(db, {
    startedAt: "2026-07-09T00:00:00.000Z",
    sourceFile: "data/prototype/sample.json",
  });

  finishSyncJob(db, {
    id,
    status: "success",
    insertedVideos: 1,
    insertedWatchRecords: 2,
    finishedAt: "2026-07-09T00:01:00.000Z",
  });

  const row = db.raw.prepare("SELECT * FROM sync_jobs WHERE id = ?").get(id) as {
    status: string;
    inserted_videos: number;
    inserted_watch_records: number;
  };

  expect(row.status).toBe("success");
  expect(row.inserted_videos).toBe(1);
  expect(row.inserted_watch_records).toBe(2);
  db.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
npm test -- tests/storage/history-store.test.ts
```

Expected: FAIL because sync job helpers do not exist.

- [ ] **Step 3: Implement sync job helpers**

Append to `src/storage/history-store.ts`:

```ts
export interface StartSyncJobInput {
  startedAt: string;
  sourceFile?: string;
}

export interface FinishSyncJobInput {
  id: number;
  status: "success" | "failed";
  insertedVideos: number;
  insertedWatchRecords: number;
  finishedAt: string;
  errorMessage?: string;
}

export function startSyncJob(db: HistoryDatabase, input: StartSyncJobInput): number {
  const result = db.raw
    .prepare(
      "INSERT INTO sync_jobs (started_at, source_file, status) VALUES (@startedAt, @sourceFile, 'running')",
    )
    .run({
      startedAt: input.startedAt,
      sourceFile: input.sourceFile,
    });

  return Number(result.lastInsertRowid);
}

export function finishSyncJob(db: HistoryDatabase, input: FinishSyncJobInput): void {
  db.raw
    .prepare(`
      UPDATE sync_jobs
      SET
        status = @status,
        finished_at = @finishedAt,
        inserted_videos = @insertedVideos,
        inserted_watch_records = @insertedWatchRecords,
        error_message = @errorMessage
      WHERE id = @id
    `)
    .run({
      id: input.id,
      status: input.status,
      finishedAt: input.finishedAt,
      insertedVideos: input.insertedVideos,
      insertedWatchRecords: input.insertedWatchRecords,
      errorMessage: input.errorMessage,
    });
}
```

- [ ] **Step 4: Run tests**

Run:

```powershell
npm test -- tests/storage/history-store.test.ts
npm run typecheck
```

Expected: PASS.

---

### Task 4: Prototype Runner Writes To SQLite

**Files:**
- Modify: `scripts/douyin-history-poc.ts`
- Test: `tests/prototype-output.test.ts` if output config changes

**Interfaces:**
- Consumes: `openHistoryDatabase(filePath)`
- Consumes: `startSyncJob`, `finishSyncJob`, `upsertHistoryRecords`
- Produces CLI option: `--db <path>`

- [ ] **Step 1: Add CLI option design**

The prototype keeps JSON output as the primary debug artifact. `--db data/history.sqlite` additionally imports deduped records into SQLite at the end of the run.

- [ ] **Step 2: Modify CLI options**

In `scripts/douyin-history-poc.ts`, add `dbPath?: string` to `CliOptions`, parse `--db <path>`, and document it in help text.

- [ ] **Step 3: Write database after JSON output**

After `writePrototypeOutput(...)` succeeds, if `options.dbPath` is set:

```ts
const db = openHistoryDatabase(options.dbPath);
const syncJobId = startSyncJob(db, {
  startedAt: generatedAt,
  sourceFile: options.outputPath,
});
try {
  const summary = upsertHistoryRecords(db, dedupeHistoryRecords(records), syncJobId);
  finishSyncJob(db, {
    id: syncJobId,
    status: "success",
    insertedVideos: summary.insertedVideos,
    insertedWatchRecords: summary.insertedWatchRecords,
    finishedAt: new Date().toISOString(),
  });
  console.log(
    `SQLite 写入完成：新增视频 ${summary.insertedVideos}，新增观看记录 ${summary.insertedWatchRecords}`,
  );
} catch (error) {
  finishSyncJob(db, {
    id: syncJobId,
    status: "failed",
    insertedVideos: 0,
    insertedWatchRecords: 0,
    finishedAt: new Date().toISOString(),
    errorMessage: error instanceof Error ? error.message : String(error),
  });
  throw error;
} finally {
  db.close();
}
```

- [ ] **Step 4: Run full verification**

Run:

```powershell
npm test
npm run typecheck
npm run prototype:dry-run -- --db data/test-dry-run.sqlite
npm run prototype -- --no-prompt --scrolls 5 --out data/prototype/mvp-db-smoke.json --db data/history.sqlite
```

Expected:
- tests PASS
- typecheck PASS
- dry-run writes JSON and SQLite
- real smoke prints `SQLite 写入完成`
- rerunning the same import does not duplicate watch records

---

## Self-Review

**Spec coverage:** This plan covers the MVP data core: SQLite schema, dedupe import, queryable history, sync job metadata, and optional prototype-to-database write path. It intentionally does not implement the Tauri shell, search UI, tray, autostart, analytics, likes, or favorites.

**Placeholder scan:** No `TBD`, generic TODO, or unspecified tests remain.

**Type consistency:** Store, importer, and runner tasks consistently use `HistoryDatabase`, `HistoryRecord`, `ImportSummary`, `openHistoryDatabase`, `upsertHistoryRecords`, `listHistoryRecords`, `startSyncJob`, and `finishSyncJob`.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-09-mvp-local-persistence.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.
