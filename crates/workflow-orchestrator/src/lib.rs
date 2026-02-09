/******************************************************/
/**************** STRUCTS & ENUMS DEFS ****************/
/******************************************************/

struct WorkflowId;

struct NodeId(pub usize);

enum NodeKind {
    Tool,
    Agent,
    GroupChat,
    Custom,
}


