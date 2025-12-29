use super::message::AgentMessage;
use crate::graph::NodeId;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub enum Control {
    RetryNode(NodeId),
    SkipNode(NodeId),
    Continue,
}

pub struct AgentContext {
    pub capacity: usize,
    pub que: VecDeque<AgentMessage>,
}

impl AgentContext {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            que: VecDeque::new(),
        }
    }

    pub fn push(&mut self, msg: AgentMessage) {
        if self.que.len() == self.capacity {
            self.que.pop_front();
        }
        self.que.push_back(msg);
    }

    pub fn iter(&self) -> impl Iterator<Item = &AgentMessage> {
        self.que.iter()
    }
}
