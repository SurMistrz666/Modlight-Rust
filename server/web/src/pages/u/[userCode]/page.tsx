// Port of upstream app/u/[userCode]/page.tsx (public profile gallery).
// @upstream app/u/[userCode]/page.tsx
//
// Server (Rust), as upstream: loadGallery(userCode) (page-local function: the
// user by tag, only when publicProfile is on; up to ITEM_LIMIT = 200 public,
// live, unprotected, unexpired, unburnt file uploads, newest first) -> notFound
// when null. generateMetadata (title, description, canonical, Open Graph
// "profile", Twitter) is computed into the payload `meta`.
import { useTranslations } from "next-intl";
import { Images } from "lucide-react";
import { ThemeApplier } from "@/components/theme-applier";
import type { BackgroundStyle } from "@/lib/background";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { PublicGallery } from "@/components/public/PublicGallery";
import type { PublicTileItem } from "@/components/public/types";
import { config } from "@/lib/config";
import { VaultGate } from "../../../encryption/VaultGate";

const ITEM_LIMIT = 200;

export interface UserGalleryPageProps {
  /** The gallery owner: the fields the page reads */
  owner: { name: string; userCode: string; theme: string | null };
  /** The public uploads (at most 200) */
  items: PublicTileItem[];
  /** userBaseUrl(owner.preferredDomain) */
  ownerBase: string;
  /** avatarUrl(owner.userCode, owner.image) */
  ownerAvatar: string | null;
  /** backgroundStyle(owner.userCode, owner.backgroundKey, owner.backgroundDim) */
  background: BackgroundStyle | null;
  /** Not upstream: the owner keeps their files encrypted (docs/ENCRYPTION.md). */
  vault?: { locked: boolean; tag: string };
}

export default function UserGalleryPage({ props }: { props: UserGalleryPageProps }) {
  const t = useTranslations("public");
  const { owner, items, ownerBase, ownerAvatar } = props;
  const count = items.length;

  if (props.vault?.locked) {
    return (
      <main className="page-surface flex min-h-dvh w-full items-center justify-center px-4 py-10 sm:py-16">
        <ThemeApplier theme={owner.theme} background={props.background} />
        <VaultGate target={{ tag: props.vault.tag }} kind="files" appName={config.appName} />
      </main>
    );
  }

  return (
    <main className="page-surface text-foreground min-h-dvh w-full">
      <ThemeApplier
        theme={owner.theme}
        background={props.background}
      />

      {/* Ambient glow header backdrop */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-80"
        style={{
          backgroundImage:
            "radial-gradient(50rem 28rem at 50% -20%, color-mix(in oklch, var(--primary) 16%, transparent), transparent 70%)",
        }}
      />

      <div className="mx-auto w-full max-w-6xl px-4 py-10 sm:px-6 sm:py-16">
        {/* Header */}
        <header className="border-border flex flex-col gap-3 border-b pb-8">
          <div className="flex items-center gap-2">
            {/* This block IS the owner's identity, so their avatar belongs here
                - and the settings card promises exactly that ("anyone who can
                see your links or your galleries can see it"). Accounts without
                one keep the gallery tile they have always had. */}
            {ownerAvatar ? (
              <Avatar className="ring-primary/20 size-12 ring-1">
                <AvatarImage src={ownerAvatar} alt="" />
                <AvatarFallback className="bg-primary/10 text-primary">
                  <Images className="size-6" />
                </AvatarFallback>
              </Avatar>
            ) : (
              <span className="bg-primary/10 text-primary ring-primary/20 grid size-12 place-items-center rounded-2xl ring-1">
                <Images className="size-6" />
              </span>
            )}
            <div className="min-w-0">
              <h1 className="truncate text-2xl font-semibold tracking-tight sm:text-3xl">
                {owner.name}
              </h1>
              <p className="text-muted-foreground truncate text-sm">@{owner.userCode}</p>
            </div>
          </div>
          <p className="text-muted-foreground text-sm">
            {t("profile.uploadCount", { count })}
            {count >= ITEM_LIMIT ? ` ${t("profile.showingLatest")}` : ""}
          </p>
        </header>

        {/* Gallery */}
        {count === 0 ? (
          <EmptyState
            title={t("profile.emptyTitle")}
            body={t("profile.emptyBody", { name: owner.name })}
          />
        ) : (
          <div className="mt-8">
            <PublicGallery
              items={items}
              userCode={owner.userCode}
              base={ownerBase}
              label={t("gallery.profileLabel", { name: owner.name })}
            />
          </div>
        )}
      </div>
    </main>
  );
}

/** Empty state when the user has no public files. */
function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <div className="border-border bg-card/40 mt-8 flex flex-col items-center gap-3 rounded-2xl border border-dashed px-6 py-20 text-center">
      <span className="bg-muted text-muted-foreground grid size-14 place-items-center rounded-2xl">
        <Images className="size-7" />
      </span>
      <h2 className="text-lg font-medium">{title}</h2>
      <p className="text-muted-foreground max-w-sm text-sm">{body}</p>
    </div>
  );
}
