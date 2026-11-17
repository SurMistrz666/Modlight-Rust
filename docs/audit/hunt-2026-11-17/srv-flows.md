# Bug hunt 2026-11-17 (round 2): srv-flows (server, whole flows)

Round 2 traces operations end to end instead of file by file. Worktree on main 6352d74
(round 1's fixes included).

## Flows traced

1. A file's life: ShareX upload (`upload/sharex.rs`) and tus (`upload/tus.rs`) into
   `upload/finalize.rs` (quota reservation, strip step, vault sealing, insert under the vault
   lock); serving gate (`media/serve.rs` lookup, view budget, cache headers); trash and restore
   (`actions/uploads.rs`, `routes/api/uploads.rs` DELETE, `media/actions.rs`); the trash sweep
   (`maintenance/trash.rs`); permanent deletes (deletion link `routes/media_delete.rs`, empty
   trash `uploads.rs`, admin `actions/admin_library.rs`) and their quota refunds; the size
   changes outside uploads (`media/convert_in_place.rs` replace and copy, `vault/pass.rs`).
2. Folders and albums: `folders.rs` (create, move, delete in both modes, visibility),
   `upload/folder_target.rs`, the default folder setter (`actions/account.rs`), the public
   album (`data/folders.rs` `get_public_folder`) and its ZIP (`routes/album_zip.rs`), the
   public profile gallery (`pages/public_profile.rs`).
3. Short links: `shortlinks.rs` (create, resolve, click counter), `actions/shortlink_unlock.rs`.
4. Accounts: account deletion with and without files (`actions/admin_library.rs`,
   `auth/admin.rs` `remove_user`, `auth/adapter.rs` `delete_user`, foreign keys in
   `migrations/`), admin edits of other accounts (`actions/admin.rs`), invites
   (`invites.rs`), API token capabilities on every `/api/*` route and the upload paths,
   quota recalculation (`data/admin.rs`, `actions/account.rs`).
5. Settings that change existing data: the default file password with "apply to existing"
   (`actions/account.rs`), file encryption on/off (the pass against deletes and conversions),
   strip metadata and the other per-account switches read by uploads in flight.

## 1. Files read

- [x] `server/src/quota.rs`
- [x] `server/src/upload/finalize.rs`
- [x] `server/src/upload/sharex.rs` (the handler flow, lines 180-330)
- [x] `server/src/upload/tus.rs` (auth, create, finish, target folder)
- [x] `server/src/upload/folder_target.rs`
- [x] `server/src/media/actions.rs`
- [x] `server/src/media/serve.rs` (lookup, viewer relation, view budget, cache headers)
- [x] `server/src/media/convert_in_place.rs` (commit replace, commit copy, charge_quota)
- [x] `server/src/maintenance/trash.rs`
- [x] `server/src/vault/pass.rs` (row selection, encrypt/decrypt, switch, retire)
- [x] `server/src/actions/uploads.rs`, `server/src/uploads.rs` (setters, trash totals, empty trash)
- [x] `server/src/routes/api/uploads.rs` (DELETE)
- [x] `server/src/routes/media_delete.rs`
- [x] `server/src/actions/admin_library.rs`
- [x] `server/src/actions/admin.rs` (user update planning and writing)
- [x] `server/src/auth/admin.rs` (`remove_user`), `server/src/auth/routes/update_user.rs` (self-delete)
- [x] `server/src/auth/adapter.rs` (`delete_user`)
- [x] `server/src/folders.rs` (delete, move, visibility, move uploads)
- [x] `server/src/data/folders.rs` (public folder)
- [x] `server/src/routes/album_zip.rs`
- [x] `server/src/pages/public_profile.rs` (gallery query)
- [x] `server/src/shortlinks.rs`, `server/src/actions/shortlink_unlock.rs`, `server/src/actions/unlock.rs`
- [x] `server/src/invites.rs`
- [x] `server/src/actions/account.rs` (default folder, default file password, quota recalculation)
- [x] `server/src/data/admin.rs` (`recalculate_all_quotas`)
- [x] `server/src/routes/api/duplicates.rs` (POST), `server/src/actions/duplicates.rs` (outline)
- [x] `server/src/actions/tokens.rs` (outline), `server/src/api_token.rs` (`authenticate_api_token`)
- [x] Token capabilities of every route (grep over `routes/`, `upload/`)
- [x] `server/migrations/*.sql` (foreign keys)

## 2. Findings

| Id | Severity | Where | What goes wrong | Fix | Test |
|---|---|---|---|---|---|
| FLOW-01 | medium (storage leak; a "deleted forever" file's bytes stay, in plaintext after a decrypting pass) | `server/src/media/actions.rs` `delete_media_hard`, `delete_media_bulk` | Both delete the object at the key the CALLER read, then the row. The file-vault pass (`vault/pass.rs` `switch_row`, while encryption is switched on or off) and an in-place conversion (`media/convert_in_place.rs`) point a row at a NEW object and retire the old one. A permanent delete that read the row before such a switch (deletion link, "Delete forever", empty trash, `DELETE /api/uploads` permanent, account deletion with files, orphan purge) deleted the old object and the row; the new object stayed in storage with no row pointing at it. Switching encryption off and deleting files while the pass runs leaves their decrypted copies behind. The trash sweep was already safe (it reads under the row lock). | The `DELETE` now returns the removed row's `storage_key` and `thumb_key`; when they differ from what the caller read, those objects are deleted too (unless another row still points at the key, checked with `vault::store::is_referenced`). A failure there is logged, never fails the delete. | `tests/media_actions_db.rs` `a_delete_that_raced_a_rewrite_leaves_no_object_behind` (single and bulk; "the new object was left" before the fix; also checks a key another row uses is kept and the refund) |
| FLOW-02 | low-medium (privacy: a file kept its location although "Remove metadata" was on when it was stored) | `server/src/upload/sharex.rs` (the handler, before `finalize_upload`) | ShareX (and the desktop app's multipart upload) took the account row from the token check at the START of the request and handed it to finalize after the whole body had arrived, which for a recording can be minutes. Finalize follows that row's switches, so anything changed meanwhile was missed: "Remove metadata" switched on (not upstream, SKELETON 9.47) let the file keep its GPS position and author; a default file password set with "apply to existing files" missed the file (stored without it, or with the old one); hidden uploads, the default folder. tus already re-reads the account at its finish; the vault state is re-read under a lock. | The account row is read again once the body has arrived (`auth::session::find_user`), as the tus finish does. Who may upload stays decided by the token check (a token revoked or an account banned mid-body still finishes, as upstream). | `tests/strip_db.rs` `a_sharex_upload_follows_a_switch_turned_on_while_it_was_sent` (the body is fed through a channel; the switch is turned on after the handler has read half of it; "PLANTED survived" before the fix) |

## 3. Not fixed

| Id | Severity | Where | What | Recommendation |
|---|---|---|---|---|
| FLOW-N1 (fixed in round 2b) | low | `server/src/actions/admin_library.rs` `delete_user_account` | "Delete the account and its files" deletes the files first and the account last, while the account's sessions and tokens still work. A ShareX or tus upload that finishes in between is not in any batch; `remove_user` then orphans it (owner set to NULL) and the action reports success with `orphaned` 0. Same order upstream. | Revoke the account's sessions and API tokens (or ban it) before the file loop, or run one more batch over `owner_id IS NULL` rows created after the loop started. |
| FLOW-N2 (fixed in round 2b) | low | `server/src/folders.rs` `delete_folder_for`, `move_folder_for`; `folders.parent_id` has no foreign key | "Delete with contents" collects the subtree before its transaction: a folder created inside it (or moved into it) meanwhile survives with a parent that no longer exists (the tree shows it at the root) and its files are not trashed. Two moves at once (A into B, B into A) both pass the cycle check, and both folders drop out of the tree. Same upstream. | Lock the owner's folder rows (`SELECT ... FOR UPDATE` on the owner's folders) for the duration of a delete or move, or add the missing `ON DELETE CASCADE` foreign key (a migration). |
| FLOW-N3 (fixed in round 2b) | low | `server/src/data/admin.rs` `recalculate_all_quotas`, `server/src/actions/account.rs` `recalculate_user_quota` | The recalculation sets `used_quota` to the sum of the rows. An upload in flight holds a reservation without a row yet, so a recalculation during it drops that reservation: when the row lands, the account is low by its size until the next recalculation. The write is also a plain `SET` after a separate read, so a refund or a reservation in between is lost. Admin-triggered, same upstream. | Compute and write in one statement (`UPDATE "user" SET used_quota = (SELECT coalesce(sum(size), 0) ...)`) and accept the in-flight drift, or document it next to the button. |
| FLOW-N4 (fixed in round 2b) | info | `server/src/invites.rs` `accept_invite` | Read, write the password, then revoke: two accepts of one invitation at the same moment both succeed (the last password wins). Both callers hold the link, so nothing is gained. | Delete the invitation row first (`DELETE ... RETURNING`) and only then write the password. |

## 4. Outside my area

Nothing. FLOW-01 changes `media/actions.rs` (srv-media in round 1) because the flow needed it;
its callers need no change.

## 5. Checks run at the end

- `cargo fmt` (server): clean.
- `cargo clippy --all-targets -- -D warnings` (server): no warnings.
- `cargo test` (server) with `MODLIGHT_TEST_DATABASE_URL` set (the local test Postgres): 39 test
  binaries, 1465 passed, 0 failed, 4 ignored.
- Both new tests were run against the code before the fix (the original `media/actions.rs`
  from HEAD swapped in; the ShareX test before `sharex.rs` was changed) and failed:
  "the new object was left" and "PLANTED survived".

## 6. Notes for merging

- No migrations, no snapshots, no web or app changes.
- Changed: `server/src/media/actions.rs` (the `DELETE ... RETURNING` of both permanent deletes,
  new `delete_moved_objects`), `server/src/upload/sharex.rs` (one re-read of the account before
  finalize), tests in `server/tests/media_actions_db.rs` and `server/tests/strip_db.rs`.
- FLOW-02 makes ShareX read the account's switches when the file has arrived instead of when the
  request started (as tus already did). One more indexed query per ShareX upload.

## Round 2b (2026-11-17): the open items fixed

The coordinator staged round 2 (FLOW-01, FLOW-02, this report); round 2b is the unstaged diff
plus the new `server/tests/flows_db.rs`. Asked for: FLOW-N1 to N4. No migration.

| Id | Severity | Where | What went wrong | Fix | Test |
|---|---|---|---|---|---|
| FLOW-N1 | low | `server/src/actions/admin_library.rs` `delete_user_account` (new `delete_files_of`, `remove_account`) | "Delete the account and its files" deleted the files while the account could still act, then called `removeUser`. An upload that finished in between was in no batch; dropping the account orphaned it (owner set to NULL) and the action answered `ok` with `orphaned: 0`. | The account's sessions and API tokens end first (so nothing new starts). After the files, the account is removed in ONE transaction that first locks the user row (`FOR UPDATE`): an insert still in progress holds that row through its foreign-key check, so the lock waits for it, and any later insert fails. If a file landed meanwhile, nothing is removed and the files round runs again (at most 5 times, then the action answers `files-removed`, or `forbidden` when nothing was deleted: the account is still there). The removal is what Better Auth's `removeUser` did (sessions, credential accounts, the user) plus the account's invitations, now in that transaction instead of through the endpoint; self-deletion stays refused. Rows storage refused still become orphans and are reported as `failed`/`orphaned`. "Keep" goes through the same removal without the file check. | `tests/library_actions_db.rs` `an_upload_landing_during_an_account_deletion_is_deleted_with_it` (an uncommitted upload row commits once the deletion waits on a lock; before: `deleted: 2`, the late file left ownerless; after: `deleted: 3`, row and object gone, tokens gone) |
| FLOW-N2 | low | `server/src/folders.rs` (`lock_tree`, `create_folder_for`, `move_folder_for`, `delete_folder_for`) | The tree was read, checked and written in separate statements outside any lock (`folders.parent_id` has no foreign key): "delete with contents" missed a subfolder created inside meanwhile (it survived with a deleted parent, its files live); two moves at once (A into B, B into A) both passed the cycle check and made a cycle. | Creating a subfolder, moving and deleting (both modes) now run in a transaction that first locks ALL the owner's folder rows (`FOR UPDATE`, in id order, so they queue without deadlocks) and then reads the tree again (a statement that waited for the lock still sees the rows of its own start). Cycle, depth, ownership and the subtree are decided on that locked tree; a folder gone meanwhile answers "not found". The plain delete re-parents the children to the parent as locked. No migration: the lock makes the missing foreign key harmless for these paths. | `tests/flows_db.rs` `a_folder_created_inside_a_subtree_being_deleted_goes_with_it` and `two_moves_at_once_cannot_make_a_cycle` (the concurrent change is a transaction holding the same row locks; before: the subfolder survived and the file was not trashed / the second move made the cycle) |
| FLOW-N3 | low | `server/src/upload/finalize.rs` (new `recount_used_quota`, in-flight ledger, `commit_reserved_row`), `server/src/data/admin.rs` `recalculate_all_quotas`, `server/src/actions/account.rs` `recalculate_user_quota`, `server/src/actions/pastes.rs`, `server/src/media/actions.rs` | The recalculations read every sum first and wrote each total later with a plain `SET`: a refund or a reservation committed in between was overwritten. They also counted the rows alone, dropping what uploads in progress had reserved (the account then went over its quota once those files landed). | (1) `recount_used_quota` locks the account row first, then sums the rows in the same transaction, then writes. (2) Every operation that changes rows and `used_quota` together does it in one transaction: the permanent deletes (`delete_media_bulk`, `delete_media_hard`) now refund inside the transaction of their `DELETE` (per batch, accounts in a fixed order); the trash sweep and conversions already did. (3) Reservations without a row are tracked in memory per account: `reserve_quota`/`release_quota` change the ledger while their `UPDATE` still holds the account row, and finalize and pastes commit their row through `commit_reserved_row`, which moves the bytes out of the ledger while the insert's foreign-key check holds the account row. The recount adds the ledger, so it sees each reservation either as reserved or as a row, never both, never neither. The ledger is per process (one server process per install, as the rate limiters already assume). | `tests/flows_db.rs` `a_refund_made_while_the_quota_is_recounted_is_kept` (before: the overwritten refund, 300 instead of 200) and `a_recount_keeps_what_uploads_in_progress_reserved` (before: the recount "corrected" 800 to 300; this test needs `reserve_quota`, which was crate-private, so it was checked against the old recalculation only) |
| FLOW-N4 | info | `server/src/invites.rs` `accept_invite`; `server/src/credentials.rs` (new `write_credential_hash`) | Read, write the password, revoke: two acceptances of one link at the same moment both succeeded, the last password won. | The password is hashed first, then one transaction CLAIMS the invitation (`DELETE ... WHERE identifier, value, not expired RETURNING`) and only then writes the password, verifies the address and drops the account's other invitations. The second acceptance waits for the first, finds nothing to claim and gets `Invalid`; a failure on the way leaves the invitation in place. | `tests/flows_db.rs` `an_invitation_is_accepted_once` (both succeeded before) |

### Round 2b: to know before merging

- No migration, no snapshots, no web or app changes.
- Larger changes: `actions/admin_library.rs` (account deletion restructured), `folders.rs` (tree lock),
  `upload/finalize.rs` (ledger and recount). Smaller: `media/actions.rs` (refund in the delete
  transaction), `data/admin.rs`, `actions/account.rs`, `actions/pastes.rs`, `invites.rs`,
  `credentials.rs`. New test file `server/tests/flows_db.rs`.
- `finalize::reserve_quota` and `release_quota` are now `pub` (the tests use them).
- Account deletion no longer calls the Better Auth `removeUser` endpoint; it does the same deletes
  itself under the lock. The admin's own session is no longer refreshed by that call (harmless).
- FLOW-N1 ends the account's API tokens at the start of a "delete" deletion; if the deletion then
  fails (`files-removed`), the account is left without sessions and tokens.
- Remaining limits: uploads that land in a folder (`move_uploads_to_folder_for`, finalize) do not
  take the tree lock; a file moved into a folder at the moment that folder is deleted with its
  contents can end up live at the root instead of in the trash (no orphan, no loss).

### Round 2b checks

- `cargo fmt` (server): clean. `cargo clippy --all-targets -- -D warnings`: no warnings.
- `cargo test` (server) with the test database: 40 test binaries, 1471 passed, 0 failed, 4 ignored.
- Each new test was run against the code before its fix (the staged version of the changed file
  swapped in) and failed as described in the table.
