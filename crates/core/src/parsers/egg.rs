use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
};

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::sandbox::sandbox_command::SandboxCommand;

#[derive(Debug, PartialEq, Eq)]
pub enum StopAction {
    Signal(i32),
    Command(String),
}

#[derive(Serialize, Deserialize)]
struct EggMeta {
    version: String,
}

#[derive(Serialize, Deserialize)]
struct EggConfig {
    files: String,
    startup: String,
    logs: String,
    stop: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct EggScript {
    script: String,
    container: String,
    entrypoint: String,
}

#[derive(Serialize, Deserialize)]
struct EggScripts {
    installation: EggScript,
}

#[derive(Serialize, Deserialize)]
struct EggVariable {
    name: String,
    description: Option<String>,
    env_variable: String,
    default_value: String,
    user_viewable: Option<bool>,
    user_editable: Option<bool>,
    rules: Option<String>,
    field_type: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Egg {
    pub name: String,
    meta: EggMeta,
    docker_images: BTreeMap<String, String>,
    startup: String,
    config: EggConfig,
    scripts: EggScripts,
    variables: Vec<EggVariable>,
}

impl Egg {
    pub async fn fetch_from_url(
        url: &str,
        server_port: u16,
        client: Client,
    ) -> Result<Egg, Box<dyn Error>> {
        let mut egg: Egg = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        egg.variables.retain(|v| v.env_variable != "SERVER_PORT");
        egg.variables.push(EggVariable {
            name: "Server Port".into(),
            description: Some("Primary port to bind the server to".into()),
            env_variable: "SERVER_PORT".into(),
            default_value: server_port.to_string(),
            user_viewable: Some(true),
            user_editable: Some(true),
            rules: Some("required|numeric".into()),
            field_type: Some("text".into()),
        });

        Ok(egg)
    }

    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes.as_ref())
    }

    pub fn get_install_command(&self) -> SandboxCommand {
        SandboxCommand {
            image: self.scripts.installation.container.clone(),
            command: self.scripts.installation.entrypoint.clone(),
            args: vec![
                "-c".into(),
                self.scripts.installation.script.replace("\r\n", "\n"),
            ],
            envs: self.get_default_envs(),
            cwd: "/mnt/server".into(),
        }
    }

    pub fn get_run_command(&self) -> SandboxCommand {
        SandboxCommand {
            image: self.docker_images.values().next().unwrap().clone(), // TODO: More deterministic default & chooser
            command: "/entrypoint.sh".into(),
            args: Vec::new(),
            envs: self.get_default_envs(),
            cwd: "/home/container".into(),
        }
    }

    pub fn get_stop_action(&self) -> StopAction {
        let Some(stop) = self.config.stop.clone().filter(|s| !s.is_empty()) else {
            return StopAction::Signal(15);
        };

        if let Some(signal) = stop.strip_prefix('^') {
            return match signal.to_ascii_uppercase().as_str() {
                "C" | "SIGINT" => StopAction::Signal(2),
                "SIGABRT" => StopAction::Signal(6),
                "SIGTERM" => StopAction::Signal(15),
                _ => StopAction::Signal(9),
            };
        }

        StopAction::Command(stop.to_string())
    }

    pub fn get_server_port(&self) -> u16 {
        self.variables
            .iter()
            .find_map(|v| {
                if v.env_variable == "SERVER_PORT" {
                    return Some(v.default_value.parse());
                }
                None
            })
            .unwrap()
            .unwrap()
    }

    fn get_default_envs(&self) -> HashMap<String, String> {
        let mut envs = self.variables.iter().fold(
            HashMap::with_capacity(self.variables.len()),
            |mut acc, item| {
                acc.insert(item.env_variable.clone(), item.default_value.clone());
                acc
            },
        );
        envs.insert("STARTUP".into(), self.startup.clone());

        envs
    }
}
