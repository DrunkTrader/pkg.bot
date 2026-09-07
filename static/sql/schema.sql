-- name: pragma
-- Concurrency (minimal write concern)
PRAGMA journal_mode       = WAL;
PRAGMA busy_timeout       = 10000;
PRAGMA wal_autocheckpoint = 0;          -- Disable auto-checkpoint; do it manually during maintenance
PRAGMA cache_size         = -256000;    -- 256MB cache (or more if available)
PRAGMA temp_store         = MEMORY;
PRAGMA mmap_size          = 1073741824; -- 1GB mmap - keep entire DB in memory if possible
PRAGMA foreign_keys       = ON;
PRAGMA query_only         = OFF;
PRAGMA analysis_limit     = 1000;


-- name: schema
CREATE TABLE IF NOT EXISTS repos (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    slug              TEXT    NOT NULL,           -- 'nixpkgs-unstable', 'arch-extra', 'aur'
    name              TEXT    NOT NULL,           -- 'Nixpkgs (unstable)'
    manager           TEXT    NOT NULL,           -- 'nix' | 'pacman' | 'aur' | 'apt' | 'apk' | ...
    distro            TEXT,                       -- 'nixos', 'arch', 'debian' (NULL for distro-agnostic)
    branch            TEXT,                       -- 'unstable', '25.05', 'extra', 'trixie/main'

    homepage_url      TEXT,                       -- repo homepage
    source_url        TEXT,                       -- repo data dump url
    revision          TEXT,                       -- revision/last change indicator for the repo (file hash, timestamp etc)
    pkg_url_template  TEXT,

    score             REAL    NOT NULL DEFAULT 0, -- for ranking in cross-repo search
    package_count     INTEGER NOT NULL DEFAULT 0,

    created_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    synced_at         TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    UNIQUE (slug)
) STRICT;

-- packages
CREATE TABLE IF NOT EXISTS packages (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id           INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    slug              TEXT    NOT NULL,           -- unique string slug that the repo uses. eg: nix (something.name), pacman (pkg), aur (name) etc.
    name              TEXT    NOT NULL,
    name_norm         TEXT    NOT NULL,           -- normalized name for cross-repo search. eg: 'foo-bar' -> 'foobar', 'FooBar' -> 'foobar'
    excerpt           TEXT,
    description       TEXT,

    -- Source/base package that several binary packages are built from.
    -- eg: pacman -> pkgbase, aur -> PackageBase, deb -> Source. NULL for nix.
    pkg_base          TEXT,

    version           TEXT,                       -- eg: 5.0.1, 125.0, 1.2.1-1, unstable-2024-01-05 etc.
    version_norm      TEXT,                       -- 0 padded semver form for lexicographic search, eg: 2.1.1 = 00002.00001.00001

    homepage_url      TEXT,                       -- upstream project URL
    repo_url          TEXT,
    source_url        TEXT,

    licenses          TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(licenses)),  -- eg: ["GPL-3.0-or-later"]
    is_foss           INTEGER CHECK (is_foss IN (0, 1)),
    platforms         TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(platforms)), -- eg: ["x86_64-linux","aarch64-darwin"]

    "groups"          TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid("groups")),
    keywords          TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(keywords)),

    -- Deduplicated tokens from name, slug, excerpt, description, groups, keywords.
    tokens            TEXT,

    status            TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'broken', 'outdated', 'deleted')),
    download_bytes    INTEGER,
    installed_bytes   INTEGER,
    score             REAL    NOT NULL DEFAULT 0,

    meta              TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(meta)),
    hash              TEXT,                       -- use this to skip re-indexing if the hash matches the previous one

    created_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    built_at          TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    UNIQUE (repo_id, slug)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_packages_name      ON packages (name);
CREATE INDEX IF NOT EXISTS idx_packages_repo_name ON packages (repo_id, name);
CREATE INDEX IF NOT EXISTS idx_packages_name_norm ON packages (name_norm);
CREATE INDEX IF NOT EXISTS idx_packages_pkg_base  ON packages (repo_id, pkg_base) WHERE pkg_base IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_packages_hash      ON packages (repo_id, hash);

-- maintainers
CREATE TABLE IF NOT EXISTS maintainers (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id           INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    slug              TEXT    NOT NULL,           -- normalized handle
    handle            TEXT    NOT NULL,           -- as displayed upstream. eg: 'UserName' or @username
    name              TEXT,                       -- display name. eg: 'User Name'
    email             TEXT,
    url               TEXT,

    package_count     INTEGER NOT NULL DEFAULT 0, -- recomputed after every sync
    created_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    UNIQUE (repo_id, slug)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_maintainers_slug ON maintainers (slug) WHERE slug IS NOT NULL;


CREATE TABLE IF NOT EXISTS package_maintainers (
    package_id        INTEGER NOT NULL REFERENCES packages(id)    ON DELETE CASCADE,
    maintainer_id     INTEGER NOT NULL REFERENCES maintainers(id) ON DELETE CASCADE,

    PRIMARY KEY (package_id, maintainer_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_pkg_maintainers ON package_maintainers (maintainer_id, package_id);

-- licenses and keywords flattened out of their JSON array columns. The
-- (repo_id, value, name, package_id) key lets a filtered listing seek straight
-- to its page and stay in name order, which a JSON array column cannot do.
CREATE TABLE IF NOT EXISTS package_licenses (
    repo_id           INTEGER NOT NULL,
    license           TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    package_id        INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,

    PRIMARY KEY (repo_id, license, name, package_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_package_licenses_pkg ON package_licenses (package_id);

CREATE TABLE IF NOT EXISTS package_keywords (
    repo_id           INTEGER NOT NULL,
    keyword           TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    package_id        INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,

    PRIMARY KEY (repo_id, keyword, name, package_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_package_keywords_pkg ON package_keywords (package_id);

-- FTS.
-- packages_fts
CREATE VIRTUAL TABLE IF NOT EXISTS packages_fts USING fts5 (
    -- SQLite's built in rowid is used as the primary key to reference packages.id
    tokens,

    -- This is sqlite's `content` keyword. Setting this to ''
    -- avoids text content being duplicated in the fts table.
    -- contentless_delete allows DELETE/UPDATE on a contentless table (SQLite >= 3.43).
    content='',
    contentless_delete=1
);

-- Keep the fts table in sync with rows in packages.
CREATE TRIGGER IF NOT EXISTS trg_packages_after_insert AFTER INSERT ON packages
BEGIN
    INSERT INTO packages_fts (rowid, tokens) VALUES (NEW.id, NEW.tokens);
END;

CREATE TRIGGER IF NOT EXISTS trg_packages_after_delete AFTER DELETE ON packages
BEGIN
    DELETE FROM packages_fts WHERE rowid = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_packages_after_update
AFTER UPDATE OF tokens ON packages
BEGIN
    DELETE FROM packages_fts WHERE rowid = OLD.id;

    INSERT INTO packages_fts (rowid, tokens) VALUES (NEW.id, NEW.tokens);
END;

-- Keep the flattened license/keyword tables in sync with rows in packages.
-- Deletes are handled by ON DELETE CASCADE.
CREATE TRIGGER IF NOT EXISTS trg_packages_facets_after_insert AFTER INSERT ON packages
BEGIN
    INSERT OR IGNORE INTO package_licenses (repo_id, license, name, package_id)
        SELECT NEW.repo_id, l.value, NEW.name, NEW.id FROM JSON_EACH(NEW.licenses) l;

    INSERT OR IGNORE INTO package_keywords (repo_id, keyword, name, package_id)
        SELECT NEW.repo_id, k.value, NEW.name, NEW.id FROM JSON_EACH(NEW.keywords) k;
END;

CREATE TRIGGER IF NOT EXISTS trg_packages_facets_after_update
AFTER UPDATE OF repo_id, name, licenses, keywords ON packages
BEGIN
    DELETE FROM package_licenses WHERE package_id = OLD.id;
    DELETE FROM package_keywords WHERE package_id = OLD.id;

    INSERT OR IGNORE INTO package_licenses (repo_id, license, name, package_id)
        SELECT NEW.repo_id, l.value, NEW.name, NEW.id FROM JSON_EACH(NEW.licenses) l;

    INSERT OR IGNORE INTO package_keywords (repo_id, keyword, name, package_id)
        SELECT NEW.repo_id, k.value, NEW.name, NEW.id FROM JSON_EACH(NEW.keywords) k;
END;

-- maintainers_fts
CREATE VIRTUAL TABLE IF NOT EXISTS maintainers_fts USING fts5 (
    -- SQLite's built in rowid is used as the primary key to reference maintainers.id
    handle,
    name,
    email,
    content='',
    contentless_delete=1
);

-- Keep the fts table in sync with rows in maintainers.
CREATE TRIGGER IF NOT EXISTS trg_maintainers_after_insert AFTER INSERT ON maintainers
BEGIN
    INSERT INTO maintainers_fts (rowid, handle, name, email)
    VALUES (NEW.id, NEW.handle, NEW.name, NEW.email);
END;

CREATE TRIGGER IF NOT EXISTS trg_maintainers_after_delete AFTER DELETE ON maintainers
BEGIN
    DELETE FROM maintainers_fts WHERE rowid = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_maintainers_after_update
AFTER UPDATE OF handle, name, email ON maintainers
BEGIN
    DELETE FROM maintainers_fts WHERE rowid = OLD.id;

    INSERT INTO maintainers_fts (rowid, handle, name, email)
    VALUES (NEW.id, NEW.handle, NEW.name, NEW.email);
END;
