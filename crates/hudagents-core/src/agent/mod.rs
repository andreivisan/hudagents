pub mod speech_to_text;

pub trait Agent {
    fn id(&self) -> &str;
    // fn call(&self, AgentIntput) -> Result<AgentOutput, AgentError>;
    // fn describe(&self) -> String;
}
