//! CCS/1 PURPOSE:shared bounded transfer snapshot for production and the observer/UI performance fixture.
use wind_core::{
    files::{id_hex, Direction, FailReason, TransferInfo, TransferState},
    protocol::ErrorCode,
};
const MAX_ROWS: usize = 50;

/// CCS/1 PERF:coalesce cross-file state bursts before serialization/IPC; one pending deadline, no idle tick.
pub async fn next_snapshot(
    updates: &mut tokio::sync::watch::Receiver<Vec<TransferInfo>>,
    last: &mut Option<tokio::time::Instant>,
) -> Option<Vec<TransferInfo>> {
    updates.changed().await.ok()?;
    if let Some(at) = last {
        tokio::time::sleep_until(*at + std::time::Duration::from_millis(100)).await;
    }
    *last = Some(tokio::time::Instant::now());
    Some(updates.borrow_and_update().clone())
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: String,
    pub id: String,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub direction: &'static str,
    pub rate: Option<u64>,
    pub can_retry: bool,
    pub can_open: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phone: Option<String>,
    pub rows: Vec<Row>,
    /// Over unfinished transfers.
    pub active: usize,
    pub active_size: u64,
    pub active_done: u64,
}

fn state_name(state: &TransferState) -> (&'static str, Option<&'static str>) {
    match state {
        TransferState::Queued => ("queued", None),
        TransferState::Running => ("running", None),
        TransferState::Waiting => ("waiting", Some("Czeka na połączenie")),
        TransferState::Done => ("done", None),
        TransferState::Cancelled { by_peer } => (
            "cancelled",
            Some(if *by_peer {
                "Anulowano na telefonie"
            } else {
                "Anulowano"
            }),
        ),
        TransferState::Failed(reason) => (
            "failed",
            Some(match reason {
                FailReason::NoSpace => "Brak miejsca u odbiorcy",
                FailReason::SourceChanged => "Plik zmienił się podczas wysyłania",
                FailReason::Refused(ErrorCode::CapabilityMissing) | FailReason::Withdrawn => {
                    "Odbiorca nie przyjmuje teraz plików"
                }
                FailReason::Refused(_) => "Odbiorca odrzucił plik",
                FailReason::Protocol(_) => "Błąd połączenia",
                FailReason::Io(_) => "Nie można odczytać pliku",
            }),
        ),
    }
}

/// CCS/1 IO:both transfer directions, newest first; bounded rows.
pub fn snapshot_of(phone: Option<String>, transfers: &[TransferInfo]) -> Snapshot {
    let all: Vec<&TransferInfo> = transfers.iter().collect();
    let active: Vec<&&TransferInfo> = all.iter().filter(|t| !t.state.is_finished()).collect();
    Snapshot {
        phone,
        active: active.len(),
        active_size: active.iter().map(|t| t.size).sum(),
        active_done: active.iter().map(|t| t.done).sum(),
        rows: all
            .iter()
            .rev()
            .take(MAX_ROWS)
            .map(|t| {
                let (state, reason) = state_name(&t.state);
                Row {
                    key: format!(
                        "{:?}:{}:{}",
                        t.direction,
                        t.peer
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>(),
                        id_hex(&t.id)
                    ),
                    id: id_hex(&t.id),
                    name: t.path.clone(),
                    size: t.size,
                    done: t.done,
                    state,
                    reason,
                    direction: if t.direction == Direction::Incoming {
                        "incoming"
                    } else {
                        "outgoing"
                    },
                    rate: None,
                    can_retry: t.direction == Direction::Outgoing
                        && matches!(
                            t.state,
                            TransferState::Failed(_) | TransferState::Cancelled { .. }
                        ),
                    can_open: t.source.is_some()
                        || (t.state == TransferState::Done && t.saved_as.is_some()),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn observer_coalesces_bursts_and_suspends_when_clean() {
        let (sender, mut updates) = tokio::sync::watch::channel(Vec::new());
        let mut last = None;
        sender.send_replace(Vec::new());
        next_snapshot(&mut updates, &mut last).await.unwrap();
        let started = tokio::time::Instant::now();
        let row = |done| TransferInfo {
            id: [0; 16],
            peer: [0; 32],
            direction: Direction::Outgoing,
            path: "file".into(),
            size: 10,
            done,
            state: TransferState::Running,
            source: None,
            saved_as: None,
        };
        sender.send_replace(vec![row(1)]);
        sender.send_replace(vec![row(2)]);
        sender.send_replace(vec![row(3)]);
        let snapshot = next_snapshot(&mut updates, &mut last).await.unwrap();
        assert_eq!(snapshot[0].done, 3);
        assert!(started.elapsed() >= std::time::Duration::from_millis(90));
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            next_snapshot(&mut updates, &mut last)
        )
        .await
        .is_err());
        drop(sender);
        assert!(next_snapshot(&mut updates, &mut last).await.is_none());
    }
}
