-- Where a sticker was put.
--
-- A shared catalog rather than a free-text field: a box usually lives somewhere that
-- already has a name, and choosing that name is faster than typing it again on a phone.
-- Coordinates are required because the device can supply them; an address without a
-- point on a map would not answer "which shelf" any better than the name does.
CREATE TABLE locations (
    id         uuid        PRIMARY KEY,
    name       text        NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 80),
    address    text        CHECK (address IS NULL OR length(address) <= 200),
    lat        double precision NOT NULL CHECK (lat >= -90 AND lat <= 90),
    lon        double precision NOT NULL CHECK (lon >= -180 AND lon <= 180),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX locations_name_idx ON locations (lower(name));

-- On the code, not the binding: the sticker is on the box, and emptying the box to use
-- it for something else does not move the shelf it sits on.
ALTER TABLE codes ADD COLUMN location_id uuid REFERENCES locations (id) ON DELETE SET NULL;
