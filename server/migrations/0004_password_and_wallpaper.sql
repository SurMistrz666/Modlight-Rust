ALTER TABLE "uploads" ADD COLUMN "password_version" integer;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "default_password_hash" text;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "default_password_version" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "background_key" text;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "background_dim" integer DEFAULT 70 NOT NULL;