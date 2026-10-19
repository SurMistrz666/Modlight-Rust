/**
 * JSX types for the media-chrome custom elements.
 *
 * We use the bare custom elements instead of the `media-chrome/react` wrappers:
 * that package does not tree-shake (importing two components costs the same as
 * importing all of them) and it drags in a React adapter for markup React 19
 * can already render on its own. The price is this file - the attributes are
 * declared by hand, and only the ones components/viewer/MediaPlayer.tsx uses.
 *
 * Attributes are spelled all-lowercase because that is how HTML sees them; a
 * camelCase prop would be set as a DOM property the element never reads.
 * Boolean attributes are typed as `string` and passed as "": React renders a
 * value-less attribute for `""`, and every check in the library is hasAttribute.
 */
import type * as React from "react";

type MediaChromeElement = React.DetailedHTMLProps<
  React.HTMLAttributes<HTMLElement>,
  HTMLElement
>;

/** Shared by every control that can be wired to a controller and disabled. */
interface MediaChromeControl extends MediaChromeElement {
  mediacontroller?: string;
  disabled?: string;
  notooltip?: string;
  tooltipplacement?: "top" | "right" | "bottom" | "left" | "none";
}

declare module "react" {
  namespace JSX {
    interface IntrinsicElements {
      "media-controller": MediaChromeElement & {
        audio?: string;
        breakpoints?: string;
        gesturesdisabled?: string;
        hotkeys?: string;
        nohotkeys?: string;
        noautohide?: string;
        autohide?: string;
        keyboardforwardseekoffset?: string;
        keyboardbackwardseekoffset?: string;
        keyboardupvolumestep?: string;
        keyboarddownvolumestep?: string;
        novolumepref?: string;
        nomutedpref?: string;
      };
      "media-control-bar": MediaChromeElement & { mediacontroller?: string };
      "media-play-button": MediaChromeControl;
      "media-mute-button": MediaChromeControl;
      "media-fullscreen-button": MediaChromeControl;
      "media-pip-button": MediaChromeControl;
      "media-playback-rate-button": MediaChromeControl & { rates?: string };
      "media-seek-backward-button": MediaChromeControl & { seekoffset?: string };
      "media-seek-forward-button": MediaChromeControl & { seekoffset?: string };
      "media-time-display": MediaChromeControl & {
        showduration?: string;
        remaining?: string;
        notoggle?: string;
      };
      "media-time-range": MediaChromeControl;
      "media-volume-range": MediaChromeControl;
      "media-loading-indicator": MediaChromeElement & {
        mediacontroller?: string;
        loadingdelay?: string;
        noautohide?: string;
      };
      "media-error-dialog": MediaChromeElement & { mediacontroller?: string; open?: string };
    }
  }
}
