use super::{log, ServerState};
use crate::daemon::spawn;
use std::{sync::Arc, time::Duration};

pub(super) async fn watch(state: Arc<ServerState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if state.registry.active_count() != 0 {
            continue;
        }
        let Ok(Ok(true)) = tokio::task::spawn_blocking(spawn::macos_gui_available).await else {
            continue;
        };
        if let Some(recovery) = &state.recovery {
            if recovery.prepare_gui_handoff(&state).await {
                log("Aqua is available; handing the headless daemon over to launchd");
                state.shutdown.notify_one();
                return;
            }
        }
    }
}
