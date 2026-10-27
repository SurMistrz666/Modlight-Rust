-- Per-account file encryption (docs/ENCRYPTION.md).
--
-- Not upstream. Number 0016 because 0015 belongs to another branch; both
-- journals must be merged before either reaches a database (drizzle applies
-- an entry only when its `when` is newer than the newest applied one).
--
-- `uploads.encryption` is NULL for a plaintext object, otherwise the header of
-- the stored ciphertext (format, chunk size, plaintext size, nonce prefix and
-- the file key sealed to the owner's public key). It changes only together with
-- `storage_key`, in one statement.
ALTER TABLE "uploads" ADD COLUMN "encryption" text;--> statement-breakpoint
CREATE TABLE "file_vaults" (
	"user_id" text PRIMARY KEY NOT NULL,
	"state" varchar(16) NOT NULL,
	"public_key" bytea NOT NULL,
	"public_key_mac" bytea NOT NULL,
	"wrapped_key" bytea NOT NULL,
	"kdf_salt" bytea NOT NULL,
	"kdf_memory_kib" integer NOT NULL,
	"kdf_iterations" integer NOT NULL,
	"kdf_parallelism" integer NOT NULL,
	"generation" integer DEFAULT 1 NOT NULL,
	"job_key" text,
	"job_error" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "file_vaults_state_check" CHECK ("file_vaults"."state" in ('encrypting', 'on', 'decrypting'))
);
--> statement-breakpoint
CREATE TABLE "vault_grants" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" text NOT NULL,
	"kind" varchar(16) NOT NULL,
	"generation" integer NOT NULL,
	"wrapped_key" bytea NOT NULL,
	"label" varchar(200),
	"ip" varchar(64),
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"last_used_at" timestamp with time zone,
	CONSTRAINT "vault_grants_kind_check" CHECK ("vault_grants"."kind" in ('owner', 'visitor', 'app'))
);
--> statement-breakpoint
CREATE TABLE "vault_cleanup" (
	"storage_key" text PRIMARY KEY NOT NULL,
	"user_id" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "file_vaults" ADD CONSTRAINT "file_vaults_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "vault_grants" ADD CONSTRAINT "vault_grants_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "vault_grants_user_idx" ON "vault_grants" USING btree ("user_id");
