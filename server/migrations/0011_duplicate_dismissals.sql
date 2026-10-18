CREATE TABLE "duplicate_dismissals" (
	"owner_id" text NOT NULL,
	"checksum" varchar(64) NOT NULL,
	"copies" integer DEFAULT 0 NOT NULL,
	"bytes" bigint DEFAULT 0 NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "duplicate_dismissals_owner_id_checksum_pk" PRIMARY KEY("owner_id","checksum")
);
--> statement-breakpoint
ALTER TABLE "duplicate_dismissals" ADD CONSTRAINT "duplicate_dismissals_owner_id_user_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;