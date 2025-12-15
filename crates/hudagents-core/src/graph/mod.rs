use std::{
    collections::VecDeque,
    fmt::{self, Debug, Display},
    sync::Arc,
};

#[derive(Debug)]
pub enum HAGraphError {
    CycleDetected(String),
    InvalidGraph(String),
}

impl Display for HAGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAGraphError::CycleDetected(msg) => {
                write!(f, "cycle detected (not a DAG): {}", msg)
            }
            HAGraphError::InvalidGraph(msg) => write!(f, "invalid graph: {}", msg),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct NodeId(pub usize);

// TODO: Move this Agent into the agent module
pub trait Agent {
    fn id(&self) -> &str;
    // fn call(&self, AgentIntput) -> Result<AgentOutput, AgentError>;
    // fn describe(&self) -> String;
}

pub struct Node {
    pub name: String,
    pub worker: Arc<dyn Agent + Send + Sync>,
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub out: Vec<Vec<NodeId>>,
    pub layers: Vec<Vec<NodeId>>,
}

pub struct GraphBuilder {
    pub nodes: Vec<Node>,
    pub out: Vec<Vec<NodeId>>,
    pub indegree: Vec<usize>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            nodes: vec![],
            out: vec![],
            indegree: vec![],
        }
    }

    pub fn add_node(
        &mut self,
        name: impl Into<String>,
        worker: Arc<dyn Agent + Send + Sync>,
    ) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            name: name.into(),
            worker,
        });
        self.out.push(Vec::new());
        self.indegree.push(0);
        id
    }

    pub fn add_edge(&mut self, from: NodeId, to: NodeId) {
        self.out[from.0].push(to);
        self.indegree[to.0] += 1;
    }

    pub fn build(self) -> Result<Graph, HAGraphError> {
        let layers = kahn_layers(self.nodes.len(), &self.out, &self.indegree)?;
        Ok(Graph {
            nodes: self.nodes,
            out: self.out,
            layers,
        })
    }
}

fn kahn_layers(
    n: usize,
    out: &[Vec<NodeId>],
    indegree: &[usize],
) -> Result<Vec<Vec<NodeId>>, HAGraphError> {
    if out.len() != n || indegree.len() != n {
        return Err(HAGraphError::InvalidGraph("length mismatch".to_string()));
    }

    let mut indegree = indegree.to_vec();
    let mut q: VecDeque<usize> = VecDeque::new();
    for node_id in 0..n {
        if indegree[node_id] == 0 {
            q.push_back(node_id)
        }
    }
    let mut layers: Vec<Vec<NodeId>> = Vec::new();
    let mut seen = 0usize;
    while !q.is_empty() {
        let layer_size = q.len();
        let mut layer = Vec::with_capacity(layer_size);
        for _ in 0..layer_size {
            let node = q.pop_front().unwrap();
            seen += 1;
            layer.push(NodeId(node));
            for &next in &out[node] {
                let next_i = next.0;
                if next_i >= n {
                    return Err(HAGraphError::InvalidGraph(format!(
                        "edge points to missing node index {next_i}"
                    )));
                }
                indegree[next_i] -= 1;
                if indegree[next_i] == 0 {
                    q.push_back(next_i);
                }
            }
        }
        layers.push(layer);
    }

    if seen != n {
        return Err(HAGraphError::CycleDetected(format!(
            "processed {seen} of {n} nodes"
        )));
    }
    Ok(layers)
}
