// Port of upstream app/[userCode]/[mediaCode]/page.tsx (the public file page).
// @upstream app/[userCode]/[mediaCode]/page.tsx
//
// Server (Rust), as upstream, in this order: getMedia(userCode, mediaCode)
// (missing -> the tagless-action redirect /{code}/raw... -> /{tag}/{code}/raw...
// with the query string rebuilt, else notFound); viewerRelation(ownerId);
// unpublished and not privileged -> notFound; a direct-media bot (User-Agent)
// on a published, unprotected, uncapped image/video/audio -> redirect to the
// raw URL (?w=1200&fit=inside for images over 8 MiB); a password-protected file
// without a valid unlock cookie -> the "gate" view; an end-to-end encrypted
// upload -> the "encrypted" view (no view counted); otherwise the owner's
// copyRaw, consumeView() (not for privileged viewers or preview bots; a refused
// budget -> notFound), the inline text, the render ticket, signInUrl, the
// timeline/poster paths and the playback diagnosis -> the "media" view.
// generateMetadata is the Open Graph / Twitter / oEmbed card (lib/media/embed.ts),
// computed into the payload `meta`.
//
// The three views render exactly the upstream JSX. A fourth, "vault", is not
// upstream: the owner keeps their files encrypted and this browser has not
// entered the file password (docs/ENCRYPTION.md); nothing about the file is
// in the payload but its id.
import { config } from "@/lib/config";
import type { BackgroundStyle } from "@/lib/background";
import { MediaViewer, type MediaViewerProps } from "@/components/viewer/MediaViewer";
import { EncryptedViewer } from "@/components/viewer/EncryptedViewer";
import { PasswordGate } from "@/components/viewer/PasswordGate";
import { ThemeApplier } from "@/components/theme-applier";
import { VaultGate } from "../../../encryption/VaultGate";

interface OwnerLook {
  /** found.ownerTheme (getMedia: owner.theme ?? "modern") */
  ownerTheme: string;
  /** found.ownerBackground (getMedia: backgroundStyle of the owner's wallpaper) */
  ownerBackground: BackgroundStyle | null;
}

export type MediaPageProps = OwnerLook &
  (
    | {
        view: "gate";
        /** media.id */
        mediaId: string;
        /** media.passwordVersion != null && media.passwordVersion === getOwnerPasswordVersion(ownerId) */
        onDefaultPassword: boolean;
      }
    | {
        view: "encrypted";
        /** rawUrl(ownerUserCode, media.code, media.ext, "") - relative on purpose */
        rawPath: string;
        /** media.size */
        cipherSize: number;
        /** media.createdAt.toISOString() */
        createdAt: string;
        views: number;
        downloads: number;
      }
    | {
        view: "media";
        /** Every prop upstream passes to <MediaViewer> (createdAt as a Date). */
        viewer: MediaViewerProps;
      }
    | {
        /** Not upstream: the owner's file password (docs/ENCRYPTION.md). */
        view: "vault";
        /** media.id */
        mediaId: string;
      }
  );

export default function MediaPage({ props }: { props: MediaPageProps }) {
  const { ownerTheme, ownerBackground } = props;

  if (props.view === "gate") {
    return (
      <CenteredShell>
        <ThemeApplier theme={ownerTheme} background={ownerBackground} />
        <PasswordGate
          mediaId={props.mediaId}
          appName={config.appName}
          onDefaultPassword={props.onDefaultPassword}
        />
      </CenteredShell>
    );
  }

  if (props.view === "vault") {
    return (
      <CenteredShell>
        <ThemeApplier theme={ownerTheme} background={ownerBackground} />
        <VaultGate target={{ mediaId: props.mediaId }} kind="file" appName={config.appName} />
      </CenteredShell>
    );
  }

  if (props.view === "encrypted") {
    return (
      <Shell>
        <ThemeApplier theme={ownerTheme} background={ownerBackground} />
        <EncryptedViewer
          // Relative on purpose. The deployment answers on several hostnames and
          // the viewer fetches with the page's own origin; an absolute URL built
          // from the owner's preferred domain would be a cross-origin request
          // with no CORS to allow it.
          rawPath={props.rawPath}
          cipherSize={props.cipherSize}
          createdAt={props.createdAt}
          views={props.views}
          downloads={props.downloads}
          appName={config.appName}
        />
      </Shell>
    );
  }

  return (
    <Shell>
      <ThemeApplier theme={ownerTheme} background={ownerBackground} />
      <MediaViewer {...props.viewer} />
    </Shell>
  );
}

/**
 * Full-viewport stage for the viewer.
 *
 * The decorative layers that used to live here — a primary-tinted radial glow
 * bleeding in from the top edge and a masked grid texture — are gone: they read
 * as a stray light leak above every file, whatever its type. A plain background
 * lets the media be the only thing on the page. The height is fixed rather than
 * a minimum so the stage inside resolves to a real height (percentages need a
 * definite one) and the page itself never scrolls under the floating bar.
 */
function Shell({ children }: { children: React.ReactNode }) {
  return (
    <main className="page-surface flex h-dvh w-full flex-col overflow-hidden">{children}</main>
  );
}

/** Centered shell for the interstitials (password gate), which size to content. */
function CenteredShell({ children }: { children: React.ReactNode }) {
  return (
    <main className="page-surface flex min-h-dvh w-full items-center justify-center px-4 py-10 sm:py-16">
      {children}
    </main>
  );
}
