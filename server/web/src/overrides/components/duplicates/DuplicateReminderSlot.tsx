// Client port of upstream components/duplicates/DuplicateReminderSlot.tsx.
// @upstream components/duplicates/DuplicateReminderSlot.tsx
//
// Upstream this is the SERVER half of the reminder: it reads the snooze cookie,
// then (only when not snoozed) runs the one-row reminder query and renders
// <DuplicateReminder> when the account is over a threshold. All of that is
// server work, so the Rust server does it while building the (app) layout data
// (contract/layouts.ts `duplicateReminder`) - including the order that makes
// a snooze cost no query - and this port only renders the outcome.
//
// Deviation: the props are the decision (`state`) instead of `userId`.
import { DuplicateReminder } from "@/components/duplicates/DuplicateReminder";

export function DuplicateReminderSlot({
  state,
}: {
  state: { redundant: number; reclaimable: number } | null;
}) {
  if (!state) return null;
  return <DuplicateReminder redundant={state.redundant} reclaimable={state.reclaimable} />;
}
