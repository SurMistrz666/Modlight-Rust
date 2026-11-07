-- Removing metadata from files on upload (docs/SKELETON.md section 9, item 47).
--
-- Not upstream. `user.strip_metadata` is the account's switch, off unless the
-- owner turns it on, so an account that never touches it sees no change.
-- `uploads.metadata_kept` is NULL for every file except one uploaded with the
-- switch on whose metadata could not be removed: then it names why
-- (`damaged`, `format`, `protected`, `signed`, `too-large`, `busy`, `tool`)
-- and the file is stored as it came.
ALTER TABLE "user" ADD COLUMN "strip_metadata" boolean DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE "uploads" ADD COLUMN "metadata_kept" varchar(16);
