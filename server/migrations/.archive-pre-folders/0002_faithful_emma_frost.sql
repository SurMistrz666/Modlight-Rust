ALTER TABLE "short_links" ADD COLUMN "password_hash" text;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "theme" text DEFAULT 'modern';--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "locale" text;