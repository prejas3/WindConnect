//! CCS/1 M:touchpad FFI | RULE:handle is pinned to one adopted connection and generation; closed/obsolete handles cannot replay.
use crate::{CoreConnection, CoreError};
use std::sync::Arc;
use tokio::sync::{watch, Mutex};
use wind_core::{
    input::InputSender,
    protocol::{Button, Capability, Gesture},
};

#[derive(uniffi::Object)]
pub struct TouchpadCapabilities {
    updates: Mutex<watch::Receiver<Vec<Capability>>>,
}
fn accepted(caps: &[Capability]) -> bool {
    caps.iter()
        .any(|c| c.name() == "input" && (1..=2).contains(&c.version()))
}
#[uniffi::export(async_runtime = "tokio")]
impl TouchpadCapabilities {
    pub async fn current(&self) -> bool {
        accepted(&self.updates.lock().await.borrow())
    }
    pub async fn next(&self) -> Result<bool, CoreError> {
        let mut rx = self.updates.lock().await;
        rx.changed().await.map_err(error)?;
        let value = accepted(&rx.borrow_and_update());
        Ok(value)
    }
}
#[derive(uniffi::Enum)]
pub enum TouchpadCommand {
    Pointer { dx: i64, dy: i64 },
    Scroll { dx: i64, dy: i64 },
    Left { down: bool },
    Right { down: bool },
    TaskView,
    ShowDesktop,
    PreviousApp,
    NextApp,
}
#[derive(uniffi::Object)]
pub struct TouchpadSession {
    enabled: u64,
    input: InputSender,
    owner: Arc<Mutex<u64>>,
    generation: u64,
    closed: std::sync::atomic::AtomicBool,
}
#[uniffi::export(async_runtime = "tokio")]
impl TouchpadSession {
    pub fn gestures(&self) -> bool {
        self.input.supports_gestures()
    }
    pub async fn send(&self, commands: Vec<TouchpadCommand>) -> Result<(), CoreError> {
        let owner = self.owner.lock().await;
        if *owner != self.generation
            || self.closed.load(std::sync::atomic::Ordering::SeqCst)
            || self
                .input
                .announcement()
                .is_none_or(|a| a.enabled != self.enabled)
        {
            return Err(error("touchpad closed"));
        }
        if commands.len() > 64 {
            return Err(error("touchpad batch too large"));
        }
        for command in commands {
            let result = match command {
                TouchpadCommand::Pointer { dx, dy } => {
                    self.input
                        .pointer(dx.clamp(-409600, 409600), dy.clamp(-409600, 409600))
                        .await
                }
                TouchpadCommand::Scroll { dx, dy } => {
                    self.input
                        .scroll(dx.clamp(-120000, 120000), dy.clamp(-120000, 120000))
                        .await
                }
                TouchpadCommand::Left { down } => self.input.button(Button::Left, down).await,
                TouchpadCommand::Right { down } => self.input.button(Button::Right, down).await,
                TouchpadCommand::TaskView => self.input.gesture(Gesture::TaskView).await,
                TouchpadCommand::ShowDesktop => self.input.gesture(Gesture::ShowDesktop).await,
                TouchpadCommand::PreviousApp => self.input.gesture(Gesture::PreviousApp).await,
                TouchpadCommand::NextApp => self.input.gesture(Gesture::NextApp).await,
            };
            if let Err(e) = result {
                self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = self.input.end_session().await;
                return Err(error(e));
            }
        }
        Ok(())
    }
    pub async fn finish_input(&self) {
        let owner = self.owner.lock().await;
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        if *owner == self.generation {
            let _ = self.input.end_session().await;
        }
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl CoreConnection {
    pub fn watch_touchpad(&self) -> Arc<TouchpadCapabilities> {
        Arc::new(TouchpadCapabilities {
            updates: Mutex::new(self.file_caps.peer_caps()),
        })
    }
    pub async fn open_touchpad(&self) -> Result<Arc<TouchpadSession>, CoreError> {
        let input = self.session.input().map_err(error)?;
        let mut owner = self.touchpad_owner.lock().await;
        input.end_session().await;
        *owner = owner.wrapping_add(1);
        Ok(Arc::new(TouchpadSession {
            enabled: input
                .announcement()
                .ok_or_else(|| error("touchpad withdrawn"))?
                .enabled,
            input,
            owner: self.touchpad_owner.clone(),
            generation: *owner,
            closed: std::sync::atomic::AtomicBool::new(false),
        }))
    }
}
fn error(e: impl std::fmt::Display) -> CoreError {
    CoreError::InvalidInput {
        reason: e.to_string(),
    }
}
