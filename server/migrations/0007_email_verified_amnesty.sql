-- Amnesty for accounts that existed before email confirmation could be required.
--
-- Confirming an address is a gate on NEW accounts. Applying it retroactively
-- punishes people for a change to the server's configuration: production holds
-- imported accounts and accounts created while no SMTP server
-- was configured, which between them never had a way to confirm anything. One
-- of them (9 files, active) would have been unable to sign in the moment mail
-- was switched on.
--
-- Scoped by time, not by "everyone": anything created from this migration
-- onwards goes through the normal flow. `email_verified` is NOT NULL DEFAULT
-- false, so the predicate is exact and the statement is idempotent.
UPDATE "user"
SET "email_verified" = true
WHERE "email_verified" = false
  AND "created_at" < now();
