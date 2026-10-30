-- A sticker that stands in for another one.
--
-- A box can carry the same code on two sides. Scanning the second sticker has to open
-- the first one without asking anyone to type a secret again, so the link keeps the
-- target's capability secret in recoverable form. This is the one table in the schema
-- that does: every other secret exists only as a hash.
--
-- The row survives the link's release, like every other handler row, so recovering the
-- link recovers the delegation too. The target's own binding is what it points at, so a
-- released target, reclaimed as something else, is followed to whatever it is now.
CREATE TABLE code_links (
    binding_id    uuid        PRIMARY KEY REFERENCES bindings (id) ON DELETE CASCADE,
    target_code_id text       NOT NULL REFERENCES codes (id) ON DELETE CASCADE,
    target_secret text        NOT NULL
                              CHECK (target_secret ~ '^[0-9A-HJKMNP-TV-Z]{12,24}$'),
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- Answered when a scan reaches a link and the target has to be found. Not unique: two
-- stickers on two sides of the same box may both point at a third code.
CREATE INDEX code_links_target_idx ON code_links (target_code_id);
