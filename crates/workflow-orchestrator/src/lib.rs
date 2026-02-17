use std::collections::{BTreeMap, HashMap};
use fsm_dag::{AtomEval, Cond};

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
enum InputRef {
    Initial,
    Node(NodeId),
    LastOutput,
}

#[derive(Clone, Debug)]
enum OutputRef {
    Node(NodeId),           
    Final,
}

#[derive(Clone, Debug)]
enum NodeConfig {
    Agent(AgentConfig),
    Tool(ToolConfig),
    GroupChat(GroupChatConfig),
}


#[derive(Clone, Debug)]
pub enum ToolArgValue {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    List(Vec<ToolArgValue>),
    Map(BTreeMap<String, ToolArgValue>),
}

pub type ToolArgs = BTreeMap<String, ToolArgValue>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NodeId(pub usize);

pub struct NodeSpec<A> {
    pub id: NodeId,
    pub kind: NodeKind,
    pub config: NodeConfig,
    pub enabled_if: Cond<A>,
}

pub struct EdgeSpec<A> {
    pub from: NodeId,
    pub to: NodeId,
    pub enabled_if: Option<Cond<A>>,
}

#[derive(Clone, Debug)]
struct AgentConfig {
    agent_id: String,
    input_from: InputRef,
    output_to: OutputRef,
}

#[derive(Clone, Debug)]
struct ToolConfig {
    tool_id: String,
    args: ToolArgs,
    input_from: InputRef,
    output_to: OutputRef,
}

#[derive(Clone, Debug)]
struct GroupChatConfig {
    manager_id: String,
    max_turns: usize,
    input_from: InputRef,
    output_to: OutputRef,
}

pub struct WorkflowId;

pub struct WorkflowSpec<A> {
    nodes: Vec<NodeSpec<A>>,
    edges: Vec<EdgeSpec<A>>,
}

pub struct WorkflowCtx {
    pub outputs: HashMap<NodeId, String>,
    pub vars_i64: HashMap<&'static str, i64>,
    pub flags: HashMap<&'static str, bool>,
}

pub struct WorkflowRuntimeState {
    pub phase: Phase,
    pub last_output: Option<String>,
}

pub struct Registry {
    pub agent_id: String,
    pub tool_id: String,
    pub manager_id: String,
}

pub struct EchoAgentHandle {
    pub agent_id: String,
}

/******************************************************/
/****************** Implementations *******************/
/******************************************************/

impl AtomEval<WorkflowRuntimeState, WorkflowCtx> for FlowAtom {
    fn eval(&self, state: &WorkflowRuntimeState, ctx: &WorkflowCtx) -> bool {
        match self {
            Self::PhaseIs(p) => state.phase == *p,
            Self::HasOutput(node) => ctx.outputs.contains_key(&node),
            Self::Flag(name) => ctx.flags.get(name).copied().unwrap_or(false),
            Self::VarLt { key, n } => ctx.vars_i64.get(key).copied().unwrap_or(0) < *n,
        }
    }
}

impl Registry {
    pub fn get_agent(&self) -> EchoAgentHandle {

    }
}
