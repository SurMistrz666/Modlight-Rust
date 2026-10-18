ALTER TABLE "uploads" DROP CONSTRAINT "uploads_owner_id_user_id_fk";
--> statement-breakpoint
ALTER TABLE "uploads" ALTER COLUMN "owner_id" DROP NOT NULL;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "naming_word_count" integer DEFAULT 2;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "naming_random_length" integer DEFAULT 6;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "preferred_domain" text;--> statement-breakpoint
ALTER TABLE "uploads" ADD CONSTRAINT "uploads_owner_id_user_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."user"("id") ON DELETE set null ON UPDATE no action;