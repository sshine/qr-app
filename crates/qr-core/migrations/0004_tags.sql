-- A shared tag catalog, attached to stickers rather than to uses.
--
-- The catalog exists for the same reason the location catalog does: the second box in
-- the loft should be tagged "loft" by picking it, not by typing a near-spelling of it on
-- a phone. Names are unique without regard to case so "Loft" and "loft" cannot both
-- become rows.
CREATE TABLE tags (
    id         uuid        PRIMARY KEY,
    name       text        NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 40),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX tags_name_key ON tags (lower(name));

-- On the code, not the binding: tags describe the sticker and the box it marks, and
-- releasing a code to reuse the box must not silently retag the shelf.
CREATE TABLE code_tags (
    code_id    text        NOT NULL REFERENCES codes (id) ON DELETE CASCADE,
    tag_id     uuid        NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (code_id, tag_id)
);

-- The overview asks "which codes carry this tag", so the reverse direction needs its own
-- index as much as the primary key's "which tags does this code carry".
CREATE INDEX code_tags_tag_idx ON code_tags (tag_id, code_id);
