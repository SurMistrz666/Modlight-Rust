// Not upstream, hand-written (scripts/gen-actions.mjs only generates stubs for
// upstream actions): the two authorize actions with the desktop app's pickup
// `challenge` (server/docs/SKELETON.md section 9, "Desktop sign-in pickup").
//
// `approveDesktopClient` is the upstream action with one more optional field;
// `denyDesktopClient` exists only in the Rust server. Both re-validate every
// value from scratch, exactly like the upstream action does.
import { callAction } from "../runtime/rpc";
import type { ApproveDesktopResult, AuthorizeRefusal } from "./actions/authorize";

export interface DesktopClientRequest {
  port: string;
  state: string;
  name: string;
  /** base64url(SHA-256(verifier)), 43 characters; absent for older apps. */
  challenge?: string;
}

export type DenyDesktopResult = { ok: true } | { ok: false; reason: AuthorizeRefusal };

/** Mint the token; with a challenge the server also keeps it for the app's pickup. */
export function approveDesktopClient(input: DesktopClientRequest): Promise<ApproveDesktopResult> {
  return callAction("authorize", "approveDesktopClient", [input]);
}

/** Record the refusal for the app's pickup (nothing to record without a challenge). */
export function denyDesktopClient(input: DesktopClientRequest): Promise<DenyDesktopResult> {
  return callAction("authorize", "denyDesktopClient", [input]);
}
