-- name: set-timeouts
SET LOCAL statement_timeout = 0;
SET LOCAL idle_in_transaction_session_timeout = 0;


-- name: schema
-- Repology dump schema. This is used to create the tables prior and them
-- stream and selectively import .sql COPY for these specific tables into the DB.
-- The full DB is large and time consuming to load, and we only care about these tables.
CREATE SCHEMA repology;
CREATE EXTENSION IF NOT EXISTS libversion WITH SCHEMA public;

CREATE TYPE repology.repository_state AS ENUM (
    'new',
    'active',
    'legacy',
    'readded'
);

CREATE TABLE repology.repositories (
    id smallint NOT NULL,
    name text NOT NULL,
    state repology.repository_state NOT NULL,
    num_packages integer DEFAULT 0 NOT NULL,
    num_packages_newest integer DEFAULT 0 NOT NULL,
    num_packages_outdated integer DEFAULT 0 NOT NULL,
    num_packages_ignored integer DEFAULT 0 NOT NULL,
    num_packages_unique integer DEFAULT 0 NOT NULL,
    num_packages_devel integer DEFAULT 0 NOT NULL,
    num_packages_legacy integer DEFAULT 0 NOT NULL,
    num_packages_incorrect integer DEFAULT 0 NOT NULL,
    num_packages_untrusted integer DEFAULT 0 NOT NULL,
    num_packages_noscheme integer DEFAULT 0 NOT NULL,
    num_packages_rolling integer DEFAULT 0 NOT NULL,
    num_packages_vulnerable integer DEFAULT 0 NOT NULL,
    num_metapackages integer DEFAULT 0 NOT NULL,
    num_metapackages_unique integer DEFAULT 0 NOT NULL,
    num_metapackages_newest integer DEFAULT 0 NOT NULL,
    num_metapackages_outdated integer DEFAULT 0 NOT NULL,
    num_metapackages_comparable integer DEFAULT 0 NOT NULL,
    num_metapackages_problematic integer DEFAULT 0 NOT NULL,
    num_metapackages_vulnerable integer DEFAULT 0 NOT NULL,
    num_problems integer DEFAULT 0 NOT NULL,
    num_maintainers integer DEFAULT 0 NOT NULL,
    first_seen timestamp with time zone NOT NULL,
    last_seen timestamp with time zone NOT NULL,
    last_fetched timestamp with time zone,
    last_parsed timestamp with time zone,
    last_updated timestamp with time zone,
    used_package_fields text[],
    ruleset_hash text,
    metadata jsonb,
    sortname text NOT NULL,
    "desc" text NOT NULL,
    incomplete boolean DEFAULT false NOT NULL,
    used_package_link_types integer[]
);

CREATE TABLE repology.packages (
    id integer NOT NULL,
    repo text NOT NULL,
    family text NOT NULL,
    subrepo text,
    name text,
    srcname text,
    binname text,
    binnames text[],
    trackname text NOT NULL,
    visiblename text NOT NULL,
    projectname_seed text NOT NULL,
    origversion text NOT NULL,
    rawversion text NOT NULL,
    arch text,
    maintainers text[],
    category text,
    comment text,
    licenses text[],
    cpe_vendor text,
    cpe_product text,
    cpe_edition text,
    cpe_lang text,
    cpe_sw_edition text,
    cpe_target_sw text,
    cpe_target_hw text,
    cpe_other text,
    effname text NOT NULL,
    version text NOT NULL,
    versionclass smallint,
    flags integer NOT NULL,
    shadow boolean NOT NULL,
    links json
);

CREATE TABLE repology.maintainers (
    id integer NOT NULL,
    maintainer text NOT NULL,
    num_packages integer DEFAULT 0 NOT NULL,
    num_packages_newest integer DEFAULT 0 NOT NULL,
    num_packages_outdated integer DEFAULT 0 NOT NULL,
    num_packages_ignored integer DEFAULT 0 NOT NULL,
    num_packages_unique integer DEFAULT 0 NOT NULL,
    num_packages_devel integer DEFAULT 0 NOT NULL,
    num_packages_legacy integer DEFAULT 0 NOT NULL,
    num_packages_incorrect integer DEFAULT 0 NOT NULL,
    num_packages_untrusted integer DEFAULT 0 NOT NULL,
    num_packages_noscheme integer DEFAULT 0 NOT NULL,
    num_packages_rolling integer DEFAULT 0 NOT NULL,
    num_packages_vulnerable integer DEFAULT 0 NOT NULL,
    num_projects integer DEFAULT 0 NOT NULL,
    num_projects_newest integer DEFAULT 0 NOT NULL,
    num_projects_outdated integer DEFAULT 0 NOT NULL,
    num_projects_problematic integer DEFAULT 0 NOT NULL,
    num_projects_vulnerable integer DEFAULT 0 NOT NULL,
    counts_per_repo jsonb,
    num_projects_per_category jsonb,
    num_repos integer DEFAULT 0 NOT NULL,
    first_seen timestamp with time zone DEFAULT now() NOT NULL,
    orphaned_at timestamp with time zone
);

CREATE TABLE repology.links (
    id integer NOT NULL,
    url text NOT NULL,
    refcount integer DEFAULT 0 NOT NULL,
    first_extracted timestamp with time zone DEFAULT now() NOT NULL,
    orphaned_since timestamp with time zone,
    next_check timestamp with time zone DEFAULT now() NOT NULL,
    last_checked timestamp with time zone,
    ipv4_status_code smallint,
    ipv4_permanent_redirect_target text,
    ipv6_status_code smallint,
    ipv6_permanent_redirect_target text,
    priority boolean DEFAULT false NOT NULL,
    last_success timestamp with time zone,
    last_failure timestamp with time zone,
    failure_streak smallint
);

CREATE TABLE repology.repo_tracks (
    repository_id smallint NOT NULL,
    refcount smallint NOT NULL,
    start_ts timestamp with time zone DEFAULT now() NOT NULL,
    restart_ts timestamp with time zone,
    end_ts timestamp with time zone,
    trackname text NOT NULL
);

CREATE TABLE repology.repo_track_versions (
    repository_id smallint NOT NULL,
    refcount smallint NOT NULL,
    trackname text NOT NULL,
    version text NOT NULL,
    start_ts timestamp with time zone DEFAULT now() NOT NULL,
    end_ts timestamp with time zone,
    any_statuses integer DEFAULT 0 NOT NULL,
    any_flags integer DEFAULT 0 NOT NULL
);


-- name: get-columns
SELECT table_name::text,
       string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position)
FROM information_schema.columns
WHERE table_schema = 'repology'
GROUP BY table_name;


-- name: create-indexes
-- Only indexes used by the SQLite importer's lookups and joins.
ALTER TABLE repology.repositories ADD PRIMARY KEY (id);
CREATE UNIQUE INDEX ON repology.repositories (name);
CREATE INDEX ON repology.packages (repo, trackname);
CREATE INDEX ON repology.maintainers (maintainer) WHERE num_packages > 0;
ALTER TABLE repology.links ADD PRIMARY KEY (id);
ALTER TABLE repology.repo_tracks ADD PRIMARY KEY (repository_id, trackname);
ALTER TABLE repology.repo_track_versions ADD PRIMARY KEY (repository_id, trackname, version);

ANALYZE repology.repositories;
ANALYZE repology.packages;
ANALYZE repology.maintainers;
ANALYZE repology.links;
ANALYZE repology.repo_tracks;
ANALYZE repology.repo_track_versions;
