"use client";

import * as React from "react";
import { useLocale, useTranslations } from "next-intl";
import {
  Maximize,
  Minimize,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import { buildPlayerDictionary, playerLang } from "@/lib/media/player-i18n";
import {
  DEFAULT_FRAME_SECONDS,
  frameSecondsFromDecoded,
  frameSecondsFromRow,
  frameStepTarget,
  sitsInFrame,
} from "@/lib/media/frame-step";

const SEEK_OFFSET = "5";
const RATES = "0.5 0.75 1 1.25 1.5 2";

export interface MediaPlayerProps {
  /** Always our own /raw URL, which answers Range requests with 206. */
  src: string;
  /** The type playableVideoType()/playableAudioType() approved - never guessed here. */
  type: string;
  variant?: "video" | "audio";
  /** Fallback text for a browser with no <video>/<audio> at all. */
  unsupported: string;
  /**
   * Where the scrub-bar preview track lives, as a path on the page's OWN
   * origin, or null when this file cannot have one. See the <track> below.
   */
  timelineTrack?: string | null;
  /**
   * Still frame to show before playback, again as a same-origin path, or null.
   * Always a resize request (`?w=`), never the file itself.
   */
  poster?: string | null;
  /** `uploads.frames`, when the row carries one. Used to derive the frame rate. */
  frames?: number | null;
}

/**
 * The viewer's player: media-chrome (MIT, Mux) painted with our own tokens.
 *
 * Why a component library rather than the browser's default controls: the
 * native bar is the one part of a shared link that ignores all twelve skins,
 * it hides playback speed behind a right-click in Chrome and drops it entirely
 * in Firefox, and it has no keyboard story until the element itself is focused.
 * Why media-chrome rather than a player with a look of its own (Plyr, Vidstack,
 * video.js): its controls are unstyled primitives driven by CSS variables, so
 * the skin decides how they look - see the `.ml-player` block in globals.css,
 * which is the entire visual definition and contains no colour of its own.
 *
 * Three things about the shape of this file are load-bearing:
 *
 *  - `type` is passed in. The caller has already asked lib/media/mime.ts what
 *    the browser can decode (a .mov is advertised as video/mp4 and plays; a
 *    .mkv never gets here at all), and the raw route sends the same string as
 *    its Content-Type. The player must not second-guess that with canPlayType.
 *  - the library is loaded with a dynamic import, in an effect. It is roughly
 *    35 kB gzip and this page is public, so it must not ride along on an image
 *    or a paste; this way the chunk is fetched only once a player is mounted.
 *  - until the custom elements upgrade, the <video> keeps its native `controls`
 *    and the control bar stays hidden (`:not(:defined)` in globals.css). A
 *    visitor whose JS is slow, blocked or broken gets a working player rather
 *    than an inert rectangle, and there is no moment with two sets of controls.
 *
 * VOLUME IS REMEMBERED AND THE PLAYBACK RATE IS NOT, and neither is done here.
 * media-chrome's own state layer writes `media-chrome-pref-volume` and
 * `media-chrome-pref-muted` to localStorage on every change and restores them
 * when a media element is attached (media-store/state-mediator.js), each read
 * and write inside its own try/catch, so a private window or blocked site data
 * simply starts at full volume. Nothing persists the playback rate - that state
 * has no pref branch at all - which is exactly the split asked for. A second
 * layer of our own would fight it: restoring after the store had already
 * restored means two writers for one number, and the visible symptom of losing
 * that race is a player that un-mutes itself. The one thing worth stating is
 * what is NOT set here: `novolumepref`/`nomutedpref` would switch it off.
 */
export function MediaPlayer({
  src,
  type,
  variant = "video",
  unsupported,
  timelineTrack = null,
  poster = null,
  frames = null,
}: MediaPlayerProps) {
  const t = useTranslations("viewer.player");
  const lang = playerLang(useLocale());
  const [upgraded, setUpgraded] = React.useState(false);
  /**
   * The upgrade could not complete, so the native controls are the interface.
   *
   * This is not defensive decoration. `Promise.all` rejects on the FIRST failed
   * import, but every module that already resolved has run and registered its
   * element - so a chunk that goes missing mid-load (an old build swept away by
   * a deploy while the tab was open is the ordinary way this happens) leaves
   * `media-controller` DEFINED, which un-hides our bar, while `upgraded` stays
   * false, which keeps the video's native controls. That is the two-players
   * report: ours on top, an inert native bar under it, and no way to dismiss it.
   * Failing to one interface or the other is the whole point.
   */
  const [failed, setFailed] = React.useState(false);
  /** Focus is somewhere inside the player, so the bar must not fade out. */
  const [keyboardHeld, setKeyboardHeld] = React.useState(false);
  /**
   * The poster URL did not answer with an image, so the attribute has to go.
   *
   * Measured in Chrome, and the reason this state exists at all: a `<video>`
   * whose poster fails to load does NOT fall back to painting the first decoded
   * frame. It paints NOTHING - an empty rectangle where the picture should be,
   * for as long as the visitor does not press play - and there is no event on
   * the element to notice it by. The endpoint refuses honestly in several
   * ordinary cases (a file past the ffmpeg input ceiling, a probe that finds no
   * video stream, and 503 when the transform queue is full), so this is not a
   * hypothetical. Taking the attribute back off restores exactly the behaviour
   * this player had before there was a poster at all. Holds the URL that failed
   * rather than a flag, so a new poster needs no reset.
   */
  const [failedPoster, setFailedPoster] = React.useState<string | null>(null);
  const isAudio = variant === "audio";

  // Built once and read through a ref: the effect below must not re-run (and
  // re-import) because a parent re-rendered and handed us a new `t` identity.
  const dictionary = React.useRef<Record<string, string> | null>(null);
  if (dictionary.current == null) {
    dictionary.current = buildPlayerDictionary((id, values) => t(id, values));
  }

  React.useEffect(() => {
    let alive = true;

    void (async () => {
      // Language first, elements second, and that order is load-bearing: the
      // error dialog resolves its four titles and messages at module evaluation
      // time, so importing it before the dictionary is registered pins it to
      // English for the life of the page.
      const i18n = await import("media-chrome/dist/utils/i18n.js");
      // The library's type wants all ~70 of its strings; a partial dictionary is
      // deliberate and safe, because an unmapped key resolves to the key itself
      // and every key IS its English text.
      type Dictionary = Parameters<typeof i18n.addTranslation>[1];
      i18n.addTranslation(lang, (dictionary.current ?? {}) as Dictionary);
      i18n.setLanguage(lang);

      // Imported one element at a time rather than through the barrel: the
      // barrel also pulls in captions, cast, AirPlay and the menu system, none
      // of which this app can populate, for about 6 kB gzip.
      await Promise.all([
        import("media-chrome/dist/media-controller.js"),
        import("media-chrome/dist/media-control-bar.js"),
        import("media-chrome/dist/media-play-button.js"),
        import("media-chrome/dist/media-seek-backward-button.js"),
        import("media-chrome/dist/media-seek-forward-button.js"),
        import("media-chrome/dist/media-time-display.js"),
        // Brings media-preview-thumbnail.js with it, which is what draws the
        // frame from the track below - there is no separate element to import.
        import("media-chrome/dist/media-time-range.js"),
        import("media-chrome/dist/media-mute-button.js"),
        import("media-chrome/dist/media-volume-range.js"),
        import("media-chrome/dist/media-playback-rate-button.js"),
        import("media-chrome/dist/media-fullscreen-button.js"),
        import("media-chrome/dist/media-loading-indicator.js"),
        import("media-chrome/dist/media-error-dialog.js"),
      ]);

      if (alive) setUpgraded(true);
    })().catch(() => {
      // The half-upgraded case above, actually reported. Without this the
      // rejection was an unhandled promise and `failed` could never become
      // true, so everything the state is documented to protect against - our
      // bar un-hidden over a native bar that still works - was unreachable.
      if (alive) setFailed(true);
    });

    return () => {
      alive = false;
    };
  }, [lang]);

  /**
   * Ask for the poster once, off to the side, purely to find out whether it is
   * there. The attribute below is set at the same time, so a poster that works
   * paints with no flicker and no second request - the browser's image cache
   * serves both from one fetch - and a poster that does not is withdrawn a
   * round trip later, before anybody has finished reading the page.
   */
  React.useEffect(() => {
    if (!poster || isAudio) return;
    const probe = new Image();
    // The URL rather than a flag, so a different poster is simply a different
    // value and there is nothing to reset - resetting a flag would mean writing
    // state from the body of an effect, which is a cascading render and a lint
    // error, and both for no reason.
    const onError = () => setFailedPoster(poster);
    probe.addEventListener("error", onError);
    probe.src = poster;
    return () => probe.removeEventListener("error", onError);
  }, [poster, isAudio]);

  // -------------------------------------------------------------------------
  // Frame stepping
  // -------------------------------------------------------------------------
  const videoRef = React.useRef<HTMLVideoElement | null>(null);
  /** How long one frame is. The policy behind every value is lib/media/frame-step.ts. */
  const frameSecondsRef = React.useRef<number>(DEFAULT_FRAME_SECONDS);
  /** Set once the file's own rate has been measured; it then beats every guess. */
  const measuredRef = React.useRef<number | null>(null);
  /** Media time of the frame currently on screen, straight from the compositor. */
  const frameStartRef = React.useRef<number | null>(null);
  /** Start of the stretch of playback the rate is being measured over. */
  const markRef = React.useRef<{ time: number; frames: number } | null>(null);

  /**
   * Learn the frame rate from the file, and keep track of the frame on screen.
   *
   * Two different jobs, deliberately on two different APIs, because each one is
   * only trustworthy for its own:
   *
   *  - the RATE comes from `getVideoPlaybackQuality().totalVideoFrames` over a
   *    stretch of uninterrupted playback. That counter follows the DECODER. The
   *    obvious alternative - the gap between consecutive
   *    `requestVideoFrameCallback` calls - follows the COMPOSITOR, which
   *    presents a steady subset of the frames whenever it cannot keep up and
   *    reports nothing to say so. See lib/media/frame-step.ts for what that
   *    measured.
   *  - the FRAME ON SCREEN comes from `requestVideoFrameCallback`, which is the
   *    only API that gives a presented frame's own position on the media
   *    timeline. That is what lets a step aim at the middle of the neighbouring
   *    frame rather than at a boundary. It re-registers itself forever rather
   *    than only while playing, because a seek presents a frame too; while the
   *    video is paused nothing is presented and nothing runs. Where it is
   *    missing, or silent, `sitsInFrame` simply never agrees and every step is
   *    a plain frame from wherever the element is.
   */
  React.useEffect(() => {
    const video = videoRef.current;
    if (!video || isAudio) return;

    const seed = () => {
      // Measurement always beats arithmetic on a column somebody else filled.
      if (measuredRef.current != null) return;
      const fromRow = frameSecondsFromRow(frames, video.duration);
      if (fromRow != null) frameSecondsRef.current = fromRow;
    };
    seed();
    video.addEventListener("loadedmetadata", seed);

    const onTimeUpdate = () => {
      const quality = video.getVideoPlaybackQuality?.();
      if (!quality) return;
      const now = { time: video.currentTime, frames: quality.totalVideoFrames };
      const mark = markRef.current;
      if (!mark || now.time < mark.time) {
        markRef.current = now;
        return;
      }
      const measured = frameSecondsFromDecoded(now.time - mark.time, now.frames - mark.frames);
      // Not yet a long enough stretch: keep the mark where it is and let the
      // window grow. Re-marking on every timeupdate would cap every window at
      // the quarter second Chrome fires them on, which is too few frames to
      // divide by.
      if (measured == null) return;
      measuredRef.current = measured;
      frameSecondsRef.current = measured;
      markRef.current = now;
    };
    // A window that spans a seek is thrown away rather than corrected: seeking
    // decodes from the preceding keyframe, so those frames would make the file
    // look faster than it is. Our own steps seek too, which is why this is
    // wired to the event rather than to a flag we would have to remember to set.
    const onSeeking = () => {
      markRef.current = null;
    };
    video.addEventListener("timeupdate", onTimeUpdate);
    video.addEventListener("seeking", onSeeking);

    const supported = typeof video.requestVideoFrameCallback === "function";
    let handle = 0;
    const onFrame: VideoFrameRequestCallback = (_now, metadata) => {
      frameStartRef.current = metadata.mediaTime;
      handle = video.requestVideoFrameCallback(onFrame);
    };
    if (supported) handle = video.requestVideoFrameCallback(onFrame);

    return () => {
      video.removeEventListener("loadedmetadata", seed);
      video.removeEventListener("timeupdate", onTimeUpdate);
      video.removeEventListener("seeking", onSeeking);
      if (supported && handle) video.cancelVideoFrameCallback(handle);
    };
  }, [frames, isAudio]);

  const stepFrame = React.useCallback((direction: 1 | -1) => {
    const video = videoRef.current;
    if (!video) return;
    // Stepping is a thing you do to a still picture. Pausing first is also what
    // makes the key forgiving during playback, instead of seeking under a
    // moving picture and looking like it did nothing.
    if (!video.paused) video.pause();

    const frameSeconds = frameSecondsRef.current;
    const now = video.currentTime;
    const start = frameStartRef.current;
    const snapped = sitsInFrame(now, start, frameSeconds);

    video.currentTime = frameStepTarget({
      from: snapped ? start : now,
      frameSeconds,
      direction,
      snapped,
      duration: video.duration,
    });
  }, []);

  /**
   * `,` and `.` step a frame. Chosen because they are the two keys every other
   * player uses for this, and because they are free here: media-chrome's hotkey
   * table is `ArrowLeft/Right/Up/Down Enter Space f m k c l j > < p`, where the
   * angle brackets are SHIFTED comma and period (playback rate). Requiring the
   * unshifted key is therefore not decoration - it is what keeps frame stepping
   * and speed control off each other.
   *
   * The listener sits on the controller rather than on window, like the focus
   * handlers below: the copy-link buttons a few pixels lower keep their own
   * keys, and a page with the player on it does not lose punctuation.
   */
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (isAudio || event.defaultPrevented) return;
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    if (event.key !== "," && event.key !== ".") return;
    // Events from inside a control's shadow root are retargeted to the control
    // itself, so this can only ever be one of our own elements - but a text
    // field slotted in later must keep its punctuation, and saying so here
    // makes that a property of the handler rather than of where it is attached.
    if (isTextEntry(event.target)) return;

    event.preventDefault();
    stepFrame(event.key === "." ? 1 : -1);
  };

  const media = isAudio ? (
    <audio slot="media" preload="metadata" controls={!upgraded || failed} className="ml-player-media">
      <source src={src} type={type} />
      {unsupported}
    </audio>
  ) : (
    <video
      ref={videoRef}
      slot="media"
      preload="metadata"
      playsInline
      /**
       * The still frame, from the same /raw endpoint the tiles use.
       *
       * It used to say here that a video must never have a poster, because the
       * thumbnail endpoint only rasterised images and for a video that URL
       * streamed the whole file back. That is no longer true: /raw?w= answers a
       * video with a real frame (lib/media/poster.ts seeks instead of decoding,
       * and the result is cached), and refuses with 415 rather than ever
       * falling through to the bytes. The caller only ever passes a resize URL.
       *
       * Nothing waits on it: measured against a poster deliberately held back
       * for six seconds, metadata was ready at once and `play()` resolved in
       * 3 ms with the clip running, while the image was still in flight. It
       * also clears itself - the poster frame is painted only while the
       * element's show-poster flag is set, and playing clears that flag, so a
       * poster cannot survive over a playing video.
       *
       * A poster that FAILS is the case the browser handles badly, and
       * `failedPoster` above is the answer to it.
       */
      poster={poster && failedPoster !== poster ? poster : undefined}
      controls={!upgraded || failed}
      className="ml-player-media"
    >
      <source src={src} type={type} />
      {/*
        The scrub-bar preview, which media-chrome draws itself given a metadata
        track it can find. Three details are the whole of the contract:

          - `label="thumbnails"` is a literal selector, not a name for people:
            the store looks up `track[label="thumbnails"]` of kind metadata.
          - `default` is required. Without it the browser leaves the track
            `disabled`, never fetches the file and never parses a cue, so the
            bar would silently behave as if there were no preview at all.
          - the src is a path on the PAGE's origin, not on the owner's preferred
            hostname. A text track is fetched under CORS rules while an <img
            crossorigin> inside the preview element fetches the sheet, and this
            deployment answers on three hostnames with no CORS headers between
            them. Same-origin also means the unlock cookie of a protected file
            travels with both requests. Cue text is a bare relative name, so the
            sheet resolves next to the track wherever that turns out to be.

        Absent (the file is too short, too large, or has no video stream) the
        bar keeps working exactly as before: the preview picture is hidden until
        a cue produces an image, so there is no empty box and no broken tile.
      */}
      {timelineTrack ? (
        <track kind="metadata" label="thumbnails" default src={timelineTrack} />
      ) : null}
      {unsupported}
    </video>
  );

  return (
    <media-controller
      className={isAudio ? "ml-player ml-player-audio" : "ml-player"}
      audio={isAudio ? "" : undefined}
      lang={lang}
      // Reachable by Tab, so the hotkeys below work for someone who never
      // touches the picture. The listener lives on this element, not on window:
      // the copy-link buttons a few pixels lower keep their own Space and
      // Enter, and the Formats dropdown keeps its arrow keys.
      tabIndex={0}
      role="region"
      aria-label={t(isAudio ? "aria.audioPlayer" : "aria.videoPlayer")}
      onKeyDown={onKeyDown}
      keyboardforwardseekoffset={SEEK_OFFSET}
      keyboardbackwardseekoffset={SEEK_OFFSET}
      // The library's default volume step is 0.025, which is 40 presses from
      // full to silent. A tenth per press is what every other player does.
      keyboardupvolumestep="0.1"
      keyboarddownvolumestep="0.1"
      // Widths at which the bar drops its optional controls. Measured on the
      // controller, not the viewport, so a phone and a narrow window agree.
      breakpoints="sm:420 md:660"
      // Auto-hide is right for a mouse and wrong for a keyboard: the controller
      // marks itself userinactive on a timer regardless of where focus is, so a
      // few seconds after tabbing onto the play button the whole bar faded to
      // opacity 0 while that button was still the active element and still
      // clickable. Someone navigating by keyboard simply lost the interface.
      // The class below re-shows the bar whenever focus is inside it.
      onFocus={() => setKeyboardHeld(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
          setKeyboardHeld(false);
        }
      }}
      data-keyboard-held={keyboardHeld ? "" : undefined}
      // Hides our chrome even once the elements are defined. `:not(:defined)`
      // alone cannot cover the half-upgraded case above, because the controller
      // may well be defined at that point.
      data-player-failed={failed ? "" : undefined}
    >
      {media}

      {!isAudio ? (
        <media-loading-indicator slot="centered-chrome" noautohide=""></media-loading-indicator>
      ) : null}
      <media-error-dialog slot="dialog"></media-error-dialog>

      <media-control-bar>
        <media-play-button>
          <span slot="play">
            <Play aria-hidden />
          </span>
          <span slot="pause">
            <Pause aria-hidden />
          </span>
        </media-play-button>

        <media-seek-backward-button seekoffset={SEEK_OFFSET} className="ml-player-optional">
          <span slot="icon">
            <RotateCcw aria-hidden />
          </span>
        </media-seek-backward-button>
        <media-seek-forward-button seekoffset={SEEK_OFFSET} className="ml-player-optional">
          <span slot="icon">
            <RotateCw aria-hidden />
          </span>
        </media-seek-forward-button>

        <media-time-display showduration=""></media-time-display>
        <media-time-range></media-time-range>

        <media-mute-button>
          <span slot="off">
            <VolumeX aria-hidden />
          </span>
          <span slot="low">
            <Volume1 aria-hidden />
          </span>
          <span slot="medium">
            <Volume1 aria-hidden />
          </span>
          <span slot="high">
            <Volume2 aria-hidden />
          </span>
        </media-mute-button>
        <media-volume-range className="ml-player-optional"></media-volume-range>

        <media-playback-rate-button rates={RATES}></media-playback-rate-button>

        {!isAudio ? (
          <>
            <media-fullscreen-button>
              <span slot="enter">
                <Maximize aria-hidden />
              </span>
              <span slot="exit">
                <Minimize aria-hidden />
              </span>
            </media-fullscreen-button>
          </>
        ) : null}
      </media-control-bar>
    </media-controller>
  );
}

/** Somewhere a keystroke means a character rather than a command. */
function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}
