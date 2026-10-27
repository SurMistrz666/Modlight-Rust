CREATE TABLE "releases" (
	"id" text PRIMARY KEY NOT NULL,
	"platform" text NOT NULL,
	"arch" text NOT NULL,
	"version" text NOT NULL,
	"build" text DEFAULT '' NOT NULL,
	"built" timestamp with time zone NOT NULL,
	"published" timestamp with time zone DEFAULT now() NOT NULL,
	"size" bigint NOT NULL,
	"sha256" text NOT NULL,
	"filename" text NOT NULL,
	"uploader" text NOT NULL,
	"note" text DEFAULT '' NOT NULL,
	CONSTRAINT "releases_platform_check" CHECK ("releases"."platform" in ('macos', 'windows')),
	CONSTRAINT "releases_arch_check" CHECK ("releases"."arch" in ('arm64', 'x64'))
);
--> statement-breakpoint
CREATE INDEX "releases_platform_arch_published_idx" ON "releases" USING btree ("platform","arch","published");