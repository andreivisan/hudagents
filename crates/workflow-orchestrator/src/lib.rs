use std::collections::{BTreeMap, hash_map::Entry, HashMap};
use actor_model::{ActorError, EchoAgentHandle, GroupManagerHandle};
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

pub enum NodeKind {
    Tool,
    Agent,
    GroupChat,
    Custom,
}

#[derive(Clone, Debug)]
pub enum InputRef {
    Initial,
    Node(NodeId),
    LastOutput,
}

#[derive(Clone, Debug)]
pub enum OutputRef {
    Node(NodeId),           
    Final,
}

#[derive(Clone, Debug)]
pub enum NodeConfig {
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

pub enum WorkflowStop {
  Done,
  NoProgress,
  HitMaxPasses,
  InvalidGraph,
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
pub struct AgentConfig {
    agent_id: String,
    input_from: InputRef,
    output_to: OutputRef,
}

#[derive(Clone, Debug)]
pub struct ToolConfig {
    tool_id: String,
    args: ToolArgs,
    input_from: InputRef,
    output_to: OutputRef,
}

#[derive(Clone, Debug)]
pub struct GroupChatConfig {
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

#[derive(Default)]
pub struct WorkflowCtx {
    pub outputs: HashMap<NodeId, String>,
    pub vars_i64: HashMap<&'static str, i64>,
    pub flags: HashMap<&'static str, bool>,
}

pub struct WorkflowRuntimeState {
    pub phase: Phase,
    pub last_output: Option<String>,
}

#[derive(Default)]
pub struct Registry {
    agents: HashMap<String, EchoAgentHandle>,
    tools: HashMap<String, ToolImpl>,
    managers: HashMap<String, GroupManagerHandle>,
}

pub struct RunLimits {
    pub max_passes: usize,
    pub max_nodex_per_pass: usize,
}

type ToolImpl = fn(input: String, args: &ToolArgs, ctx: &mut WorkflowCtx) -> Result<String, ActorError>;

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
    pub fn insert_agent(&mut self, agent_id: impl Into<String>, agent: EchoAgentHandle) -> Result<(), ActorError> {
        match self.agents.entry(agent_id.into()) {
            Entry::Vacant(slot) => {
                slot.insert(agent);
                Ok(())
            }
            Entry::Occupied(_) => Err(ActorError::ActorAlreadyPresent)
        }
    }
    
    pub fn upsert_agent(&mut self, agent_id: impl Into<String>, new_agent: EchoAgentHandle) -> Result<(), ActorError> {
        self.agents.insert(agent_id.into(), new_agent);
        Ok(())
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<EchoAgentHandle, ActorError> {
        self.agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| ActorError::InitError)
    }

    pub fn insert_tool(&mut self, tool_id: impl Into<String>, func: ToolImpl) -> Result<(), ActorError> {
        match self.tools.entry(tool_id.into()) {
            Entry::Vacant(slot) => {
                slot.insert(func);
                Ok(())
            }
            Entry::Occupied(_) => Err(ActorError::ActorAlreadyPresent)
        }
    }

    pub fn upsert_tool(&mut self, tool_id: impl Into<String>, func: ToolImpl) -> Result<(), ActorError> {
        self.tools.insert(tool_id.into(), func);
        Ok(())
    }

    pub fn get_tool(&self, tool_id: &str) -> Result<ToolImpl, ActorError> {
        self.tools
            .get(tool_id)
            .cloned()
            .ok_or_else(|| ActorError::InitError)
    }

    pub fn insert_manager(&mut self, manager_id: impl Into<String>, manager: GroupManagerHandle) -> Result<(), ActorError> {
        match self.managers.entry(manager_id.into()) {
            Entry::Vacant(slot) => {
                slot.insert(manager);
                Ok(())
            }
            Entry::Occupied(_) => Err(ActorError::ActorAlreadyPresent)
        }
    }
    
    pub fn upsert_manager(&mut self, manager_id: impl Into<String>, new_manager: GroupManagerHandle) -> Result<(), ActorError> {
        self.managers.insert(manager_id.into(), new_manager);
        Ok(())
    }

    pub fn get_manager(&self, manager_id: &str) -> Result<GroupManagerHandle, ActorError> { 
        self.managers
            .get(manager_id)
            .cloned()
            .ok_or_else(|| ActorError::InitError)
    }
}
