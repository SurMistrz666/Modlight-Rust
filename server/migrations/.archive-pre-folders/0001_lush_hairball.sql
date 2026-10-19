CREATE TABLE "short_links" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"owner_id" text NOT NULL,
	"code" varchar(64) NOT NULL,
	"target" text NOT NULL,
	"clicks" integer DEFAULT 0 NOT NULL,
	"last_used_at" timestamp with time zone,
	"expires_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "uploads" ADD COLUMN "kind" varchar(16) DEFAULT 'file' NOT NULL;--> statement-breakpoint
ALTER TABLE "uploads" ADD COLUMN "language" varchar(32);--> statement-breakpoint
ALTER TABLE "short_links" ADD CONSTRAINT "short_links_owner_id_user_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "short_links_code_idx" ON "short_links" USING btree ("code");--> statement-breakpoint
CREATE INDEX "short_links_owner_idx" ON "short_links" USING btree ("owner_id");