-- Hiding a label from the overview is not the same as releasing or revoking it.
--
-- Release describes a use that ended; revoke makes the sticker stop opening. This column
-- is only the overview's opinion: the label is still minted, still scannable, and still
-- recoverable. A delete and a restore each append an event, so a sticker that has been
-- hidden and shown several times can say so on its one card.
ALTER TABLE codes ADD COLUMN deleted_at timestamptz;

CREATE INDEX codes_deleted_idx ON codes (deleted_at);
