ALTER TABLE "user" ADD COLUMN "username" text;--> statement-breakpoint
CREATE UNIQUE INDEX "user_username_idx" ON "user" USING btree ("username");--> statement-breakpoint
-- Backfill logins for accounts that predate this column. Those accounts carry
-- their login in `name`, and that is exactly what they will type into the
-- sign-in box.
--
-- Guarded twice: only where `name` is unique across the table, and only where
-- the value is not already claimed — the unique index must not blow up on an
-- install where two people happen to share a display name.
UPDATE "user" u
SET username = u.name
WHERE u.username IS NULL
  AND u.name IS NOT NULL
  AND u.name <> ''
  AND (SELECT count(*) FROM "user" x WHERE lower(x.name) = lower(u.name)) = 1
  AND NOT EXISTS (SELECT 1 FROM "user" y WHERE lower(y.username) = lower(u.name));
