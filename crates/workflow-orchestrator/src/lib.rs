use std::collections::HashMap;

/******************************************************/
/**************** STRUCTS & ENUMS DEFS ****************/
/******************************************************/

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Plan,
    Run,
    Done,
}

#[derive(Clone, Debug)]
pub enum FlowAtom {
    PhaseIs(Phase),
    HasOutput(NodeId),
    Flag(&'static str),
    VarLt { key: &'static str, n: i64 },
}

enum NodeKind {
    Tool,
    Agent,
    GroupChat,
    Custom,
}

#[derive(Clone, Debug)]
pub struct NodeId(pub usize);

pub struct NodeSpec<A> {
    pub id: NodeId,
    pub kind: NodeKind,
    pub config: NodeConfig,
    pub enabled_if: Cond<A>,
}

pub struct WorkflowId;

pub struct WorkflowCtx {
    pub outputs: HashMap<NodeId, String>,
    pub vars_i64l: HashMap<&'static str, i64>,
    pub flags: HashMap<&'static str, bool>,
}
