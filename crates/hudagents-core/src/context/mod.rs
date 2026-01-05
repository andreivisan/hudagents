pub mod blob;
pub mod ids;
pub mod message;

use super::graph::NodeId;
use ids::{RunId, UserId};
use message::AgentMessage;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub enum Control {
    RetryNode(NodeId),
    SkipNode(NodeId),
    Continue,
}

pub struct AgentContext {
    pub run_id: RunId,
    pub user_id: UserId,
    pub capacity: usize,
    pub msg_que: VecDeque<AgentMessage>,
}

impl AgentContext {
    pub fn new(run_id: RunId, user_id: UserId, capacity: usize) -> Self {
        Self {
            run_id,
            user_id,
            capacity,
            msg_que: VecDeque::new(),
        }
    }

    pub fn cap(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.msg_que.len()
    }

    pub fn is_empty(&self) -> bool {
        self.msg_que.is_empty()
    }

    pub fn push(&mut self, msg: AgentMessage) {
        if self.msg_que.len() == self.capacity {
            self.msg_que.pop_front();
        }
        self.msg_que.push_back(msg);
    }

    pub fn messages(&self) -> &VecDeque<AgentMessage> {
        &self.msg_que
    }

    pub fn iter(&self) -> impl Iterator<Item = &AgentMessage> {
        self.msg_que.iter()
    }

    pub fn last(&self) -> Option<&AgentMessage> {
        self.msg_que.back()
    }
}
