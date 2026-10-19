/**
 * What a 501 `{"notImplemented": true, "route", "file"}` page payload renders:
 * the Rust server has no loader yet for that page or one of its layouts. The
 * gap is shown where the missing part would render, so it cannot be mistaken
 * for a bug in the page. This happens in every build: the Rust server always
 * serves the production bundle (web/dist), and a 501 only exists while the
 * port is incomplete. The box names upstream source files only (public), no
 * server state.
 */
import type { PageNotImplemented } from "../runtime/payload";

export function NotImplemented({ payload }: { payload: PageNotImplemented }) {
  return (
    <div
      role="alert"
      data-ml-not-implemented=""
      className="border-destructive/50 bg-destructive/5 text-foreground mx-auto my-10 w-full max-w-xl rounded-xl border border-dashed p-5 font-mono text-xs leading-relaxed"
    >
      <p className="text-destructive mb-2 text-sm font-semibold">501: not ported to the Rust server yet</p>
      <p>
        page: <span className="font-semibold">{payload.route}</span>
      </p>
      <p>
        missing loader: <span className="font-semibold">{payload.file}</span>
      </p>
    </div>
  );
}
