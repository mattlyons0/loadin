use std::collections::HashMap;

pub struct SandboxCommand {
    pub image: String,
    pub command: String,
    pub args: Vec<String>,
    pub envs: HashMap<String, String>,
    pub cwd: String,
}
