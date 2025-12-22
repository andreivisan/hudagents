use super::ids::{DeviceId, UserId};
use super::message::AgentMessage;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(15);
pub const SESSION_MAX_MESSAGES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SessionKey {
    pub device_id: DeviceId,
    pub user_id: UserId,
}

pub struct SessionState {
    pub last_activity: Instant,
    pub messages: VecDeque<AgentMessage>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            messages: VecDeque::new(),
        }
    }
}
