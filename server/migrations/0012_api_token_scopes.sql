-- Scoped API tokens.
--
-- Hand-edited from the generated form, which was a single
--   ALTER TABLE "api_tokens" ADD COLUMN "scope" varchar(16) NOT NULL;
-- and cannot run at all against a table that already has rows: there are live
-- tokens in ShareX configs and in the desktop app on six real accounts, and
-- Postgres has nothing to put in the new column for them.
--
-- Existing rows are backfilled to 'full', which is the honest value rather than
-- the convenient one. It is not that 'full' is easiest to write; it is that
-- these tokens ALREADY do everything, today, and a migration must not change
-- what a credential does. Backfilling them to 'upload' would have looked tidier
-- and would have been a silent, un-announced revocation: the desktop app's
-- library would empty, the deletion links ShareX users have already handed out
-- would stop working, and every one of those failures would arrive as a bug
-- report about the panel rather than as a security decision somebody made.
-- Narrowing is a choice the account holder makes, in the UI, one token at a
-- time, and can never be undone in place (lib/api-token.ts: isNarrowing).
--
-- The DEFAULT exists only to fill those rows and is dropped immediately below,
-- so from here on an INSERT that forgets to name a scope is a constraint
-- violation instead of a quietly unlimited credential. That is the asymmetry the
-- whole change rests on: open on the old path, closed on the new one.
ALTER TABLE "api_tokens" ADD COLUMN "scope" varchar(16) DEFAULT 'full' NOT NULL;--> statement-breakpoint
ALTER TABLE "api_tokens" ALTER COLUMN "scope" DROP DEFAULT;--> statement-breakpoint
ALTER TABLE "api_tokens" ADD CONSTRAINT "api_tokens_scope_check" CHECK ("api_tokens"."scope" in ('upload', 'read', 'write', 'full'));
