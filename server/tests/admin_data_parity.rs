//! The `lib/data/admin.ts` read models and probes that upstream's admin
//! ACTIONS call (`loadOrphans`, `loadDeletionImpact`, `recalculateEveryQuota`,
//! `runStorageScan`; the actions themselves belong to the library port), run
//! against a database `tests/parity/panel.py` seeded identically on both
//! servers, so that script can compare them with the reference's own actions.
//!
//! Runs only when asked (panel.py does it):
//!
//! ```text
//! MODLIGHT_PARITY_DATABASE_URL=postgresql://...   the seeded Rust parity database
//! MODLIGHT_PARITY_STORAGE=/abs/storage/root       the tree to scan
//! MODLIGHT_PARITY_USERS=id1,id2,...               accounts for userDeletionImpact
//! cargo test --test admin_data_parity -- --ignored --nocapture
//! ```
//!
//! and prints one line, `PARITY-JSON <json>`, in the order the script calls
//! the reference: orphans, deletion impacts, quota recalculation (which
//! writes), storage scan.

use std::collections::HashMap;

use modlight_server::config::Config;
use modlight_server::data::admin::{
    ORPHAN_PAGE_SIZE, list_orphans, orphan_summary, recalculate_all_quotas, scan_storage_orphans, user_deletion_impact,
};
use modlight_server::db::types::JsDate;
use serde_json::json;

#[tokio::test]
#[ignore = "driven by tests/parity/panel.py"]
async fn admin_data_for_parity() {
    let Ok(url) = std::env::var("MODLIGHT_PARITY_DATABASE_URL") else {
        eprintln!("MODLIGHT_PARITY_DATABASE_URL is not set: nothing to do");
        return;
    };
    let storage = std::env::var("MODLIGHT_PARITY_STORAGE").expect("MODLIGHT_PARITY_STORAGE");
    let users: Vec<String> = std::env::var("MODLIGHT_PARITY_USERS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let env: HashMap<String, String> = [
        ("STORAGE_DRIVER", "local"),
        ("STORAGE_LOCAL_PATH", storage.as_str()),
        ("DATABASE_URL", url.as_str()),
        ("APP_URL", "http://localhost:8436"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let config = Config::from_env_map(&env).expect("config");
    let db = modlight_server::db::connect(&url, 2).await.expect("connect to the parity database");

    let summary = orphan_summary(&db).await.expect("orphanSummary");
    let items = list_orphans(&db, ORPHAN_PAGE_SIZE).await.expect("listOrphans");
    let mut impacts = Vec::new();
    for id in &users {
        impacts.push(serde_json::to_value(user_deletion_impact(&db, id).await.expect("userDeletionImpact")).unwrap());
    }
    let recalc = recalculate_all_quotas(&db).await.expect("recalculateAllQuotas");
    let scan = scan_storage_orphans(&db, &config, JsDate::now().millis()).await.expect("scanStorageOrphans");

    let out = json!({
        "orphans": { "summary": summary, "items": items },
        "impacts": impacts,
        "recalc": recalc,
        "scan": scan,
    });
    println!("PARITY-JSON {}", serde_json::to_string(&out).unwrap());
}
