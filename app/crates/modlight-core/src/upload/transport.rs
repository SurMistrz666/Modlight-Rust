//! The offline queue's network side over the real server: tus for the bytes
//! (`ModlightResumableUpload`) and `/api/me` for the room left (`HasRoom`).

use modlight_api::tus::{
    TransferOutcome as TusOutcome, TransferReason, TusClient, TusFile, TusSession,
};
use modlight_api::{Api, ApiError, Credential};
use tokio_util::sync::CancellationToken;

use crate::upload::item::QueueReason;
use crate::upload::queue::{
    BoxFuture, ProgressFn, QueueAccount, QueueTransport, QuotaAnswer, TransferItem,
    TransferOutcome, TransferResult,
};

pub struct ApiTransport {
    api: Api,
    tus: TusClient,
}

impl ApiTransport {
    pub fn new(api: Api) -> ApiTransport {
        ApiTransport {
            api,
            tus: TusClient::new(env!("CARGO_PKG_VERSION")).expect("the transfer client starts"),
        }
    }

    /// A transport whose tus client has its own durations (tests of stalls).
    pub fn with_tus(api: Api, tus: TusClient) -> ApiTransport {
        ApiTransport { api, tus }
    }
}

fn credential(account: &QueueAccount) -> Credential {
    Credential::new(&account.host, &account.token)
}

fn reason(reason: TransferReason) -> QueueReason {
    match reason {
        TransferReason::None => QueueReason::None,
        TransferReason::Unreachable => QueueReason::Unreachable,
        TransferReason::ServerError => QueueReason::ServerError,
        TransferReason::TokenRejected => QueueReason::TokenRejected,
        TransferReason::Refused => QueueReason::Refused,
        TransferReason::OutOfSpace => QueueReason::OutOfSpace,
        TransferReason::TooLarge => QueueReason::TooLarge,
    }
}

impl QueueTransport for ApiTransport {
    fn send<'a>(
        &'a self,
        account: &'a QueueAccount,
        item: &'a mut TransferItem,
        cancel: CancellationToken,
        mut progress: ProgressFn<'a>,
    ) -> BoxFuture<'a, TransferResult> {
        Box::pin(async move {
            let file = TusFile {
                path: item.source_path.clone(),
                name: item.file_name.clone(),
                length: item.length.max(0) as u64,
            };
            let mut session = TusSession {
                url: item.tus_url.clone(),
                offset: item.tus_offset.max(0) as u64,
                folder_id: item.folder_id.clone(),
            };
            let result = self
                .tus
                .send(
                    &credential(account),
                    &file,
                    &mut session,
                    |sent, total| progress(sent as i64, total as i64),
                    &cancel,
                )
                .await;
            item.tus_url = session.url;
            item.tus_offset = session.offset as i64;
            item.folder_id = session.folder_id;
            TransferResult {
                outcome: match result.outcome {
                    TusOutcome::Success => TransferOutcome::Success,
                    TusOutcome::Retry => TransferOutcome::Retry,
                    TusOutcome::Blocked => TransferOutcome::Blocked,
                    TusOutcome::Canceled => TransferOutcome::Canceled,
                },
                reason: reason(result.reason),
                detail: result.detail,
                copy_url: result.copy_url,
                view_url: result.view_url,
                raw_url: result.raw_url,
                retry_after: result.retry_after,
            }
        })
    }

    fn abandon(&self, account: QueueAccount, url: String) -> BoxFuture<'static, ()> {
        let tus = self.tus.clone();
        Box::pin(async move {
            match tus.abandon(&credential(&account), &url).await {
                Ok(status) => {
                    log::info!("Modlight queue: abandoned a resumable session, {status:?}.")
                }
                Err(error) => {
                    log::warn!("Modlight queue: a session could not be abandoned: {error}")
                }
            }
        })
    }

    fn quota(&self, account: QueueAccount) -> BoxFuture<'static, Result<QuotaAnswer, String>> {
        let api = self.api.clone();
        Box::pin(async move {
            match api.get_account(&credential(&account)).await {
                Ok(None) | Err(ApiError::Unauthorized) => Ok(QuotaAnswer::Refused),
                Ok(Some(info)) => Ok(QuotaAnswer::Remaining(match info.quota {
                    Some(quota) if quota.unlimited => None,
                    Some(quota) => quota.remaining,
                    None => None,
                })),
                Err(error) => Err(error.to_string()),
            }
        })
    }
}
