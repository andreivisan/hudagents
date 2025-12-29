#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RunId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct UserId(pub usize);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct DeviceId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AgentId(pub String);
