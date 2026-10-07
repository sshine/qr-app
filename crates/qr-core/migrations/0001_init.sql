-- The anchor schema: codes, what they became, and the bytes behind them.
--
-- Handler tables live here too, prefixed by handler id. One migration directory rather
-- than one per crate, because sqlx takes a single directory and versions are globally
-- ordered; composing several migrators would need hand-maintained ordering across crates
-- for no benefit while there is one deployment.

-- Content-addressed, so identical uploads share storage and the collector has a
-- worklist. Created first because sheets.pdf_key refers to it.
CREATE TABLE blobs (
    key        text        PRIMARY KEY CHECK (key ~ '^[0-9a-f]{64}$'),
    byte_size  bigint      NOT NULL CHECK (byte_size >= 0),
    media_type text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- The key that signs the admin session cookie, generated on first start.
--
-- One row, so rotation is a DELETE and a restart. In Postgres rather than Vault because
-- the alternative costs four coupled edits across two files in the cluster repository
-- plus a `vault kv put` before the first deploy, and the service could not then go live
-- from a push alone. The cost is that a dump of this database carries the key.
CREATE TABLE app_secret (
    id          smallint    PRIMARY KEY CHECK (id = 1),
    signing_key bytea       NOT NULL CHECK (octet_length(signing_key) = 32),
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- One printing run.
--
-- handler_id stamps every code on the sheet with what it is destined to become, so a
-- photo-album sheet scans straight into an empty album; NULL leaves the chooser.
--
-- pdf_key keeps the rendered sheet so it can be reprinted. It is the only artefact
-- holding plaintext secrets, which is what pdf_expires_at is for; NULL means keep.
CREATE TABLE sheets (
    id             uuid        PRIMARY KEY,
    created_at     timestamptz NOT NULL DEFAULT now(),
    code_count     integer     NOT NULL CHECK (code_count > 0),
    base_url       text        NOT NULL,
    preset         text        NOT NULL,
    layout         jsonb       NOT NULL DEFAULT '{}'::jsonb,
    handler_id     text,
    handler_config jsonb       NOT NULL DEFAULT '{}'::jsonb,
    note           text,
    pdf_key        text        REFERENCES blobs (key) ON DELETE SET NULL,
    pdf_expires_at timestamptz
);

-- One sticker.
--
-- sheet_id is nullable: a code minted individually through the API belongs to no sheet.
-- The id alphabet is Crockford base32 with I, L, O and U left out, and the length is
-- configurable within this range, so the CHECK states both.
CREATE TABLE codes (
    id               text        PRIMARY KEY CHECK (id ~ '^[0-9A-HJKMNP-TV-Z]{6,24}$'),
    sheet_id         uuid        REFERENCES sheets (id) ON DELETE RESTRICT,
    secret_hash      bytea       NOT NULL CHECK (octet_length(secret_hash) = 32),
    intended_handler text,
    intended_config  jsonb       NOT NULL DEFAULT '{}'::jsonb,
    created_at       timestamptz NOT NULL DEFAULT now(),
    first_seen_at    timestamptz,
    revoked_at       timestamptz
);

CREATE INDEX codes_sheet_idx ON codes (sheet_id);

CREATE INDEX codes_intended_idx ON codes (intended_handler)
    WHERE intended_handler IS NOT NULL;

-- What a code became.
--
-- Released rather than deleted, so a box can be emptied and reused without losing what
-- was in it, and so handler rows can key on binding_id and survive the reuse.
CREATE TABLE bindings (
    id          uuid        PRIMARY KEY,
    code_id     text        NOT NULL REFERENCES codes (id) ON DELETE CASCADE,
    handler_id  text        NOT NULL CHECK (handler_id ~ '^[a-z0-9][a-z0-9-]{0,31}$'),
    config      jsonb       NOT NULL DEFAULT '{}'::jsonb,
    title       text        CHECK (title IS NULL OR length(title) <= 120),
    created_at  timestamptz NOT NULL DEFAULT now(),
    released_at timestamptz
);

-- A partial unique index rather than application logic, so two simultaneous claims
-- cannot both win.
CREATE UNIQUE INDEX bindings_one_active_per_code ON bindings (code_id)
    WHERE released_at IS NULL;

CREATE INDEX bindings_code_history_idx ON bindings (code_id, created_at DESC);

-- The photo-album handler's rows.
--
-- id is a UUIDv7 generated in Rust: monotonic, so ORDER BY id DESC is newest-first with
-- no second sort key and no tiebreak between two uploads in one millisecond.
CREATE TABLE photo_items (
    id           uuid        PRIMARY KEY,
    binding_id   uuid        NOT NULL REFERENCES bindings (id) ON DELETE CASCADE,
    original_key text        NOT NULL REFERENCES blobs (key) ON DELETE RESTRICT,
    view_key     text        NOT NULL REFERENCES blobs (key) ON DELETE RESTRICT,
    thumb_key    text        NOT NULL REFERENCES blobs (key) ON DELETE RESTRICT,
    media_type   text        NOT NULL,
    width        integer     NOT NULL CHECK (width > 0),
    height       integer     NOT NULL CHECK (height > 0),
    byte_size    bigint      NOT NULL CHECK (byte_size >= 0),
    taken_at     timestamptz,
    caption      text        CHECK (caption IS NULL OR length(caption) <= 280),
    created_at   timestamptz NOT NULL DEFAULT now(),
    deleted_at   timestamptz
);

-- The album query, answered from the index alone.
CREATE INDEX photo_items_album_idx ON photo_items (binding_id, id DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX photo_items_trash_idx ON photo_items (binding_id, deleted_at DESC)
    WHERE deleted_at IS NOT NULL;

-- The forwarding handler's rows.
--
-- Repointing supersedes rather than overwrites, so a poster whose destination moved can
-- be traced back.
CREATE TABLE forward_targets (
    id            uuid        PRIMARY KEY,
    binding_id    uuid        NOT NULL REFERENCES bindings (id) ON DELETE CASCADE,
    url           text        NOT NULL,
    status        smallint    NOT NULL DEFAULT 302
                              CHECK (status IN (301, 302, 307, 308)),
    note          text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    created_by    text,
    superseded_at timestamptz
);

CREATE UNIQUE INDEX forward_targets_one_live_per_binding ON forward_targets (binding_id)
    WHERE superseded_at IS NULL;

-- What happened.
--
-- Deliberately holds nothing identifying a person: the client address and user agent go
-- to the tracing log, which has a retention of its own, rather than to a table that
-- lives as long as the box does.
CREATE TABLE events (
    id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at         timestamptz NOT NULL DEFAULT now(),
    code_id    text        REFERENCES codes (id) ON DELETE SET NULL,
    binding_id uuid        REFERENCES bindings (id) ON DELETE SET NULL,
    kind       text        NOT NULL,
    detail     jsonb       NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX events_code_at_idx ON events (code_id, at DESC);

CREATE INDEX events_kind_at_idx ON events (kind, at DESC);
