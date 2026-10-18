ALTER TABLE "user" ADD COLUMN "skip_tag_in_url" boolean DEFAULT false;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "allow_root_short_links" boolean DEFAULT false;--> statement-breakpoint
ALTER TABLE "user" ADD COLUMN "default_folder_id" uuid;--> statement-breakpoint
-- Hand-written: `user.default_folder_id` is declared in the schema without a
-- Drizzle `.references()` because folders live in app-schema, which already
-- imports auth-schema — a back-reference would close an import cycle. The
-- constraint itself still belongs in the database.
--
-- ON DELETE SET NULL: deleting a folder must never delete the user, it just
-- means new uploads go back to having no default.
ALTER TABLE "user"
  ADD CONSTRAINT "user_default_folder_id_folders_id_fk"
  FOREIGN KEY ("default_folder_id") REFERENCES "public"."folders"("id")
  ON DELETE SET NULL ON UPDATE NO ACTION;
