// Port of upstream app/a/[userCode]/[slug]/page.tsx (a public folder).
// @upstream app/a/[userCode]/[slug]/page.tsx
//
// Server (Rust), as upstream: getPublicFolder(userCode, slug) (tag or alias,
// folder must be public; gated files excluded) -> notFound when null; the items
// cut down to PublicTileItem. generateMetadata (title, description, canonical,
// Open Graph "website", Twitter) is computed into the payload `meta`.
import { useTranslations } from "next-intl";
import { Images, Lock, FileDown } from "lucide-react";
import { ThemeApplier } from "@/components/theme-applier";
import type { BackgroundStyle } from "@/lib/background";
import { PublicGallery } from "@/components/public/PublicGallery";
import type { PublicTileItem } from "@/components/public/types";
import { config } from "@/lib/config";
import { VaultGate } from "../../../../encryption/VaultGate";

export interface AlbumPageProps {
  /** data.folder: the fields the page reads */
  album: { name: string; slug: string };
  /** data.owner: the fields the page reads */
  owner: { name: string | null; theme: string | null };
  /** owner.userCode ?? userCode */
  ownerCode: string;
  /** userBaseUrl(owner.preferredDomain) */
  ownerBase: string;
  /** backgroundStyle(owner.userCode, owner.backgroundKey, owner.backgroundDim) */
  background: BackgroundStyle | null;
  /** items mapped to PublicTileItem (id, code, ext, filename, mimeType, size, blurhash) */
  tiles: PublicTileItem[];
  /** Not upstream: the owner keeps their files encrypted (docs/ENCRYPTION.md). */
  vault?: { locked: boolean; tag: string };
}

export default function PublicFolderPage({ props }: { props: AlbumPageProps }) {
  const t = useTranslations("public");
  const { album, owner, ownerCode, ownerBase, tiles } = props;
  const ownerName = owner.name || t("album.anonymous");
  const count = tiles.length;

  if (props.vault?.locked) {
    return (
      <main className="page-surface flex min-h-dvh w-full items-center justify-center px-4 py-10 sm:py-16">
        <ThemeApplier theme={owner.theme} background={props.background} />
        <VaultGate target={{ tag: props.vault.tag }} kind="files" appName={config.appName} />
      </main>
    );
  }

  return (
    <main className="page-surface relative min-h-dvh w-full overflow-hidden">
      <ThemeApplier
        theme={owner.theme}
        background={props.background}
      />
      {/* Ambient violet glow + faint grid, matching the media viewer aesthetic. */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10"
        style={{
          backgroundImage:
            "radial-gradient(60rem 60rem at 50% -10%, color-mix(in oklch, var(--primary) 14%, transparent), transparent 60%)",
        }}
      />
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 opacity-[0.04]"
        style={{
          backgroundImage:
            "linear-gradient(to right, var(--foreground) 1px, transparent 1px), linear-gradient(to bottom, var(--foreground) 1px, transparent 1px)",
          backgroundSize: "56px 56px",
          maskImage: "radial-gradient(70% 50% at 50% 0%, #000, transparent)",
        }}
      />

      <div className="mx-auto w-full max-w-6xl px-4 py-12 sm:px-6 sm:py-16">
        {/* Header */}
        <header className="flex flex-col items-center gap-4 text-center">
          <div className="bg-primary/10 text-primary ring-primary/20 grid size-14 place-items-center rounded-2xl ring-1">
            <Images className="size-7" />
          </div>
          <div className="space-y-2">
            <h1 className="font-heading text-3xl font-semibold tracking-tight sm:text-4xl">
              {album.name}
            </h1>
            <p className="text-muted-foreground text-sm">
              {t("album.byOwner")} <span className="text-foreground font-medium">{ownerName}</span>
              <span aria-hidden className="mx-2 opacity-40">
                ·
              </span>
              {t("album.itemCount", { count })}
            </p>
          </div>

          {/* The ZIP link stays relative on purpose: the archive is streamed by
              whichever host the visitor is already on, so a large download never
              takes an extra cross-origin hop. */}
          {count > 0 ? (
            <a
              href={`/a/${ownerCode}/${album.slug}/zip`}
              className="bg-primary/10 text-primary ring-primary/20 hover:bg-primary/15 inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium ring-1 transition-colors"
            >
              <FileDown className="size-4" />
              {t("album.downloadAllZip")}
            </a>
          ) : null}
        </header>

        {/* Gallery */}
        {count === 0 ? (
          <div className="mt-14 flex flex-col items-center gap-3 text-center">
            <div className="bg-muted text-muted-foreground grid size-12 place-items-center rounded-2xl">
              <Lock className="size-6" />
            </div>
            <p className="text-muted-foreground text-sm">{t("album.empty")}</p>
          </div>
        ) : (
          <div className="mt-12">
            <PublicGallery
              items={tiles}
              userCode={ownerCode}
              base={ownerBase}
              label={t("gallery.folderLabel", { name: album.name })}
            />
          </div>
        )}
      </div>
    </main>
  );
}
