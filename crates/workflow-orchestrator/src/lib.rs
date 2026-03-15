use std::collections::{BTreeMap, hash_map::Entry, HashMap};
use actor_model::{ActorError, EchoAgentHandle, GroupManagerHandle};
use fsm_dag::{AtomEval, Cond};

/******************************************************/
/**************** STRUCTS & ENUMS DEFS ****************/
/******************************************************/

pub enum WorkflowError {
    Actor(ActorError),
    InvalidGraph,
    NoProgress,
    HitMaxPasses,
}

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

#[derive(Debug, Eq, PartialEq)]
pub enum WorkflowStop {
  Done,
  NoProgress,
  HitMaxPasses,
  InvalidGraph,
}

#[derive(Debug, Eq, PartialEq)]
pub enum WorkflowOutcome {
    Done(String),
    Stopped(WorkflowStop),
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
    pub agent_id: String,
    pub input_from: InputRef,
    pub output_to: OutputRef,
}

#[derive(Clone, Debug)]
pub struct ToolConfig {
    pub tool_id: String,
    pub args: ToolArgs,
    pub input_from: InputRef,
    pub output_to: OutputRef,
}

#[derive(Clone, Debug)]
pub struct GroupChatConfig {
    pub manager_id: String,
    pub max_turns: usize,
    pub input_from: InputRef,
    pub output_to: OutputRef,
}

pub struct WorkflowId;

pub struct WorkflowSpec<A> {
    pub nodes: Vec<NodeSpec<A>>,
    pub edges: Vec<EdgeSpec<A>>,
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
    pub agents: HashMap<String, EchoAgentHandle>,
    pub tools: HashMap<String, ToolImpl>,
    pub managers: HashMap<String, GroupManagerHandle>,
}

pub struct RunLimits {
    pub max_passes: usize,
    pub max_nodes_per_pass: usize,
}

type ToolImpl = fn(input: String, args: &ToolArgs, ctx: &mut WorkflowCtx) -> Result<String, ActorError>;

/******************************************************/
/****************** Implementations *******************/
/******************************************************/

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_passes: 3,
            max_nodes_per_pass: 3,
        }
    }
}

impl Default for WorkflowCtx {
    fn default() -> Self {
        Self {
            outputs: HashMap::new(),
            vars_i64: HashMap::new(),
            flags: HashMap::new(),
        }
    }
}

impl Default for WorkflowRuntimeState {
    fn default() -> Self {
        Self {
            phase: Phase::Plan,
            last_output: None,
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            tools: HashMap::new(),
            managers: HashMap::new(),
        }
    }
}

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

/******************************************************/
/********************** Workers ***********************/
/******************************************************/

fn validate_workflow_spec<A>(spec: &WorkflowSpec<A>) -> Result<(), WorkflowStop> {
    let nodes_len = spec.nodes.len();
    for (idx, node) in spec.nodes.iter().enumerate() {
        if node.id.0 >= nodes_len { return Err(WorkflowStop::InvalidGraph); }
        if node.id.0 != idx { return Err(WorkflowStop::InvalidGraph); }
    }
    for edge in &spec.edges {
        if edge.from.0 >= nodes_len || edge.to.0 >= nodes_len {
            return Err(WorkflowStop::InvalidGraph);
        }
    }
    Ok(())
}

fn edges_usize<A>(edges: &[EdgeSpec<A>]) -> Vec<(usize, usize)> {
    edges
        .iter()
        .map(|edge| (edge.from.0, edge.to.0))
        .collect()
}

// plan runnable nodes - enabled[i] is the condition for node i
fn plan_runnable_nodes<A>(
    spec: &WorkflowSpec<A>, 
    limits: &RunLimits,
    runtime: &WorkflowRuntimeState,
    ctx: &WorkflowCtx,
    topo_order: &[usize]) -> Vec<usize> 
where
    A: Clone + AtomEval<WorkflowRuntimeState, WorkflowCtx>,
{
    let enabled = spec.nodes.iter()
        .map(|node| node.enabled_if.clone())
        .collect::<Vec<_>>();
    let mut runnable = fsm_dag::run(runtime, ctx, topo_order, &enabled);
    runnable.retain(|&idx| !ctx.outputs.contains_key(&spec.nodes[idx].id));
    runnable.truncate(limits.max_nodes_per_pass);
    runnable
}

fn execute_node<A>(
    node_spec: &NodeSpec<A>,
    registry: &Registry,
    runtime: &WorkflowRuntimeState,
    ctx: &WorkflowCtx
    input: &InputRef) {
    match &node_spec.config {
        NodeConfig::Agent(agent_conf) => {
            //call agent handle
            let agent_id = &agent_conf.agent_id;
            let input_from = &agent_conf.input_from;
            let output_to = &agent_conf.output_to;
        }
        NodeConfig::GroupChat(group_conf) => {
            //call group manager handle
        }
        NodeConfig::Tool(tool_conf) => {
            //call a tool
        }
    }
}

pub async fn run_workflow<A>(spec: &WorkflowSpec<A>, limits: &RunLimits) -> Result<WorkflowOutcome, WorkflowError>
where
    A: Clone + AtomEval<WorkflowRuntimeState, WorkflowCtx>,
{
    // 1. validate spec
    // 2. build topo order
    // 3. create runtime state
    // 4. repeat passes
    //-> loop passes
    //  -> plan runnable nodes
    //  -> execute them in order
    //  -> stop if final output appears
    // 5. return final output or stop reason mapped to error
    match validate_workflow_spec(spec) {
        Err(err) => {
            match err {
                WorkflowStop::InvalidGraph => return Err(WorkflowError::InvalidGraph),
                _ => return Err(WorkflowError::Actor(ActorError::InitError))
            }
        }
        Ok(()) => {
            let num_nodes = spec.nodes.len();
            let edges = edges_usize(&spec.edges);
            let topo_order = fsm_dag::kahn(num_nodes, &edges);
            if num_nodes > 0 && topo_order.is_empty() { return Err(WorkflowError::InvalidGraph) }
            let mut runtime = WorkflowRuntimeState::default();
            let mut ctx = WorkflowCtx::default();
            for _ in 0..limits.max_passes {
                runtime.phase = Phase::Plan;
                let runnable = plan_runnable_nodes(spec, limits, &runtime, &ctx, &topo_order);
                if runnable.is_empty() { return Err(WorkflowError::NoProgress); }
                runtime.phase = Phase::Run;
                for node_idx in runnable {
                    let node_spec = &spec.nodes[node_idx];
                }
            }
        }
    }
    Ok(WorkflowOutcome::Done("Done".to_string()))
}
