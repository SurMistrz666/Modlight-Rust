//! Port of `lib/audit.ts`: write one entry to `audit_log`.
//!
//! Runs AFTER the change it describes, so a failure here must not fail the
//! caller: the change really has happened by then, and reporting the whole
//! operation as failed would be a lie that invites the operator to do it
//! twice. A missing log line is a problem for tomorrow.
//!
//! `action` lands in a varchar(64), so names stay short and dotted:
//! `admin.user.email_verified`, `mail.test_sent`, `admin.upload.deleted`.
//! `ip` is [`crate::password::client_ip`] of the request, as upstream.

use serde_json::Value;
use sqlx::PgPool;

pub async fn record_audit(db: &PgPool, actor_id: &str, action: &str, detail: Value, ip: &str) {
    record_audit_by(db, Some(actor_id), action, detail, ip).await;
}

/// Not upstream: [`record_audit`] for an actor that is no account (Downloads'
/// `RELEASE_PUBLISH_TOKEN`), whose entry carries a NULL `user_id`.
pub async fn record_audit_by(db: &PgPool, actor_id: Option<&str>, action: &str, detail: Value, ip: &str) {
    let result =
        sqlx::query("INSERT INTO \"audit_log\" (\"user_id\", \"action\", \"detail\", \"ip\") VALUES ($1, $2, $3, $4)")
            .bind(actor_id)
            .bind(action)
            .bind(detail)
            .bind(ip)
            .execute(db)
            .await;
    if let Err(err) = result {
        tracing::error!("[audit] could not record {action}: {err}");
    }
}
