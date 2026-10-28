CREATE TABLE accounts_v4 (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  uid TEXT,
  sec_uid TEXT,
  profile_key TEXT NOT NULL UNIQUE,
  nickname TEXT,
  avatar_url TEXT,
  status TEXT NOT NULL CHECK(status IN ('pending_login', 'active', 'login_expired', 'identity_conflict', 'archived')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_login_validated_at TEXT,
  CHECK(uid IS NULL OR length(trim(uid)) > 0),
  CHECK(sec_uid IS NULL OR length(trim(sec_uid)) > 0),
  CHECK(uid IS NOT NULL OR sec_uid IS NOT NULL OR status = 'pending_login')
);

INSERT INTO accounts_v4
  (id, uid, sec_uid, profile_key, nickname, avatar_url, status,
   created_at, updated_at, last_login_validated_at)
SELECT id, uid, sec_uid, profile_key, nickname, avatar_url, status,
       created_at, updated_at, last_login_validated_at
FROM accounts;

DROP TABLE accounts;
ALTER TABLE accounts_v4 RENAME TO accounts;

CREATE UNIQUE INDEX idx_accounts_uid ON accounts(uid) WHERE uid IS NOT NULL;
CREATE UNIQUE INDEX idx_accounts_sec_uid ON accounts(sec_uid) WHERE sec_uid IS NOT NULL;

PRAGMA user_version = 4;
