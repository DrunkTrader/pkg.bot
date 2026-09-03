PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE repos (
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
CREATE TABLE packages (
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

    status            INTEGER NOT NULL DEFAULT 0, -- bitflags representing broken|deprecated|outdated|deleted etc
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

CREATE INDEX idx_packages_name      ON packages (name);
CREATE INDEX idx_packages_repo_name ON packages (repo_id, name);
CREATE INDEX idx_packages_name_norm ON packages (name_norm);
CREATE INDEX idx_packages_pkg_base  ON packages (repo_id, pkg_base) WHERE pkg_base IS NOT NULL;
CREATE INDEX idx_packages_hash      ON packages (repo_id, hash);

-- maintainers
CREATE TABLE maintainers (
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

CREATE INDEX idx_maintainers_slug ON maintainers (slug) WHERE slug IS NOT NULL;


CREATE TABLE package_maintainers (
    package_id        INTEGER NOT NULL REFERENCES packages(id)    ON DELETE CASCADE,
    maintainer_id     INTEGER NOT NULL REFERENCES maintainers(id) ON DELETE CASCADE,

    PRIMARY KEY (package_id, maintainer_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX idx_pkg_maintainers ON package_maintainers (maintainer_id, package_id);

-- FTS.

CREATE VIRTUAL TABLE packages_fts USING fts5 (
    -- SQLite's built in rowid is used as the primary key to reference packages.id
    name,
    name_norm,
    slug,
    pkg_base,
    excerpt,
    description,
    "groups",
    keywords,

    -- This is sqlite's `content` keyword. Setting this to ''
    -- avoids text content being duplicated in the fts table.
    -- contentless_delete allows DELETE/UPDATE on a contentless table (SQLite >= 3.43).
    content='',
    contentless_delete=1
);

-- Keep the fts table in sync with rows in packages.
-- Plain text fields are indexed as-is. `groups` and `keywords`, which are JSON
-- arrays in the format ["a", "b", ...], are flattened into a space separated
-- series of strings. Eg: groups => a b c
CREATE TRIGGER trg_packages_after_insert AFTER INSERT ON packages
BEGIN
    INSERT INTO packages_fts (rowid, name, name_norm, slug, pkg_base, excerpt, description, "groups", keywords)
    VALUES (
        NEW.id,
        NEW.name,
        NEW.name_norm,
        NEW.slug,
        NEW.pkg_base,
        NEW.excerpt,
        NEW.description,
        (SELECT GROUP_CONCAT(value, ' ') FROM JSON_EACH(NEW."groups")),
        (SELECT GROUP_CONCAT(value, ' ') FROM JSON_EACH(NEW.keywords))
    );
END;

CREATE TRIGGER trg_packages_after_delete AFTER DELETE ON packages
BEGIN
    DELETE FROM packages_fts WHERE rowid = OLD.id;
END;

CREATE TRIGGER trg_packages_after_update
AFTER UPDATE OF name, name_norm, slug, pkg_base, excerpt, description, "groups", keywords ON packages
BEGIN
    DELETE FROM packages_fts WHERE rowid = OLD.id;

    INSERT INTO packages_fts (rowid, name, name_norm, slug, pkg_base, excerpt, description, "groups", keywords)
    VALUES (
        NEW.id,
        NEW.name,
        NEW.name_norm,
        NEW.slug,
        NEW.pkg_base,
        NEW.excerpt,
        NEW.description,
        (SELECT GROUP_CONCAT(value, ' ') FROM JSON_EACH(NEW."groups")),
        (SELECT GROUP_CONCAT(value, ' ') FROM JSON_EACH(NEW.keywords))
    );
END;

CREATE VIRTUAL TABLE maintainers_fts USING fts5 (
    -- SQLite's built in rowid is used as the primary key to reference maintainers.id
    handle,
    name,
    email,
    content='',
    contentless_delete=1
);

-- Keep the fts table in sync with rows in maintainers.
CREATE TRIGGER trg_maintainers_after_insert AFTER INSERT ON maintainers
BEGIN
    INSERT INTO maintainers_fts (rowid, handle, name, email)
    VALUES (NEW.id, NEW.handle, NEW.name, NEW.email);
END;

CREATE TRIGGER trg_maintainers_after_delete AFTER DELETE ON maintainers
BEGIN
    DELETE FROM maintainers_fts WHERE rowid = OLD.id;
END;

CREATE TRIGGER trg_maintainers_after_update
AFTER UPDATE OF handle, name, email ON maintainers
BEGIN
    DELETE FROM maintainers_fts WHERE rowid = OLD.id;

    INSERT INTO maintainers_fts (rowid, handle, name, email)
    VALUES (NEW.id, NEW.handle, NEW.name, NEW.email);
END;
