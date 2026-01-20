use crate::agent::Agent;
use std::{
    collections::VecDeque,
    fmt::{self, Debug, Display},
    sync::Arc,
};

#[derive(Debug)]
pub enum HAGraphError {
    CycleDetected(String),
    InvalidGraph(String),
    InvalidNodeId(String),
}

impl Display for HAGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAGraphError::CycleDetected(msg) => write!(f, "cycle detected (not a DAG): {}", msg),
            HAGraphError::InvalidGraph(msg) => write!(f, "invalid graph: {}", msg),
            HAGraphError::InvalidNodeId(msg) => write!(f, "invalid node id: {}", msg),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct NodeId(pub usize);

pub struct Node {
    pub name: String,
    pub worker: Arc<dyn Agent + Send + Sync>,
}

pub struct Edge {
    pub from: String,
    pub to: String,
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

    pub fn add_edge(&mut self, from: NodeId, to: NodeId) -> Result<(), HAGraphError> {
        let n = self.nodes.len();
        if from.0 >= n || to.0 >= n {
            return Err(HAGraphError::InvalidNodeId(format!(
                "Invalid edge: from={} to={} (node count={})",
                from.0, to.0, n
            )));
        }
        self.out[from.0].push(to);
        self.indegree[to.0] += 1;
        Ok(())
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
    let mut q: VecDeque<NodeId> = VecDeque::new();
    for node_id in 0..n {
        if indegree[node_id] == 0 {
            q.push_back(NodeId(node_id))
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
            layer.push(node);
            for &next in &out[node.0] {
                let next_i = next.0;
                if next_i >= n {
                    return Err(HAGraphError::InvalidGraph(format!(
                        "edge points to missing node index {next_i}"
                    )));
                }
                if indegree[next_i] == 0 {
                    return Err(HAGraphError::InvalidGraph(format!(
                        "indegree underflow at node {next_i}"
                    )));
                }
                indegree[next_i] -= 1;
                if indegree[next_i] == 0 {
                    q.push_back(NodeId(next_i));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentInput, AgentOutput, HAAgentError};

    struct TestAgent(&'static str);

    impl Agent for TestAgent {
        fn id(&self) -> &str {
            self.0
        }

        fn call(&self, _agent_input: AgentInput) -> Result<AgentOutput, HAAgentError> {
            Ok(AgentOutput::FinalAnswer(String::new()))
        }

        fn describe(&self) -> String {
            format!("TestAgent({})", self.0)
        }
    }

    fn agent(id: &'static str) -> Arc<dyn Agent + Send + Sync> {
        Arc::new(TestAgent(id))
    }

    #[test]
    fn layers_chain() {
        let mut b = GraphBuilder::new();
        let a = b.add_node("A", agent("a"));
        let c = b.add_node("C", agent("c"));
        let d = b.add_node("D", agent("d"));

        b.add_edge(a, c).unwrap();
        b.add_edge(c, d).unwrap();

        let g = b.build().unwrap();
        assert_eq!(g.layers, vec![vec![a], vec![c], vec![d]]);
    }

    #[test]
    fn layers_branch_and_merge() {
        let mut b = GraphBuilder::new();
        let a = b.add_node("A", agent("a"));
        let b1 = b.add_node("B", agent("b"));
        let c = b.add_node("C", agent("c"));
        let d = b.add_node("D", agent("d"));

        b.add_edge(a, b1).unwrap();
        b.add_edge(a, c).unwrap();
        b.add_edge(b1, d).unwrap();
        b.add_edge(c, d).unwrap();

        let g = b.build().unwrap();
        assert_eq!(g.layers, vec![vec![a], vec![b1, c], vec![d]]);
    }

    #[test]
    fn layers_two_independent_chains() {
        let mut b = GraphBuilder::new();
        let a = b.add_node("A", agent("a"));
        let b1 = b.add_node("B", agent("b"));
        let c = b.add_node("C", agent("c"));
        let d = b.add_node("D", agent("d"));

        b.add_edge(a, c).unwrap();
        b.add_edge(b1, d).unwrap();

        let g = b.build().unwrap();
        assert_eq!(g.layers, vec![vec![a, b1], vec![c, d]]);
    }

    #[test]
    fn build_detects_cycle() {
        let mut b = GraphBuilder::new();
        let a = b.add_node("A", agent("a"));
        let b1 = b.add_node("B", agent("b"));

        b.add_edge(a, b1).unwrap();
        b.add_edge(b1, a).unwrap();

        match b.build() {
            Err(err) => assert!(matches!(err, HAGraphError::CycleDetected(_))),
            Ok(_) => panic!("expected cycle error"),
        }
    }

    #[test]
    fn add_edge_rejects_invalid_node_id() {
        let mut b = GraphBuilder::new();
        let a = b.add_node("A", agent("a"));
        let bogus = NodeId(999);

        let err = b.add_edge(a, bogus).unwrap_err();
        assert!(matches!(err, HAGraphError::InvalidNodeId(_)));
    }
}
