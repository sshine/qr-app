-- One-time cut-over to code ids with check characters.
--
-- Every id minted before this migration was uniform randomness. There is no way to
-- derive a checksum for it after the fact, and the owner chose to reprint rather than
-- keep a legacy exception alive. Removing the codes cascades to their bindings, photos,
-- link rows and forward targets; the events that referred to them are removed too,
-- because they are the history of stickers that no longer exist.
--
-- Sheets go after the codes they minted (codes.sheet_id is ON DELETE RESTRICT), taking
-- their PDF keys with them. The blob rows those PDFs point at are left for the collector;
-- the files themselves are not deleted here.
--
-- This migration is destructive by design and runs once. A fresh database applies it
-- before serving anything, so it has nothing to delete.
DELETE FROM events;
DELETE FROM codes;
DELETE FROM sheets;
