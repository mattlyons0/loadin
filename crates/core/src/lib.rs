mod parsers;
mod sandbox;

use std::error::Error;
use std::io::{self, Write};
use std::net::Ipv4Addr;

use microsandbox::{
    ExecEvent, NetworkPolicy, NetworkProfile, Sandbox, Volume, VolumeHandle, config::GlobalConfig,
    setup::ensure_runtime,
};
use reqwest::Client;

use crate::parsers::egg::{Egg, StopAction};

pub async fn init_sandbox() -> Result<(), Box<dyn Error>> {
    ensure_runtime(&GlobalConfig::default(), Default::default()).await?;

    Ok(())
}

pub async fn install(
    url: &str,
    disk_size: u32,
    server_port: u16,
    client: Client,
) -> Result<(), Box<dyn Error>> {
    let egg = Egg::fetch_from_url(url, server_port, client).await?;
    let serverfiles = init_installer(&egg, disk_size).await?;
    init_runtime(&egg, serverfiles).await?.stop().await?;

    println!("{} successfully installed", egg.name);

    Ok(())
}

pub async fn run(name: &str) -> Result<(), Box<dyn Error>> {
    match Volume::get(&volume_name(name)).await {
        Ok(vol) => vol,
        Err(_) => {
            return Err("Gameserver not found. Install it with `loadin install`".into());
        }
    };

    let sbx = match Sandbox::get(name).await {
        Ok(s) => s.start_detached().await?,
        Err(_) => return Err("Gameserver not found. Install it with `loadin install`".into()),
    };

    let egg = Egg::from_bytes(&sbx.fs().read("/home/container/egg.json").await?)?;
    let run_command = egg.get_run_command();

    println!(
        "Running runtime command: {} {} {}",
        run_command
            .envs
            .iter()
            .map(|e| format!("{}={}", e.0, e.1))
            .collect::<Vec<_>>()
            .join(" "),
        run_command.command,
        run_command.args.join(" ")
    );

    let mut run_handle = sbx
        .exec_stream_with(run_command.command, |e| {
            e.args(run_command.args)
                .envs(run_command.envs)
                .cwd(run_command.cwd)
                .stdin_pipe()
        })
        .await?;

    let stdin = run_handle.take_stdin();
    let mut shutdown = false;

    loop {
        tokio::select! {
            event = run_handle.recv() => match event {
                Some(ExecEvent::Stdout(bytes)) => {
                    io::stdout().write_all(&bytes)?;
                    io::stdout().flush()?;
                }
                Some(ExecEvent::Stderr(bytes)) => {
                    io::stderr().write_all(&bytes)?;
                    io::stderr().flush()?;
                }
                Some(ExecEvent::Exited { code }) => {
                    println!("\nServer exited with code {code}");
                    break;
                }
                Some(ExecEvent::Failed(err)) => return Err(format!("Exec failed: {err:?}").into()),
                Some(ExecEvent::Started { pid: _ }) => {},
                Some(ExecEvent::StdinError(_)) => {},
                None => break,
            },
            _ = tokio::signal::ctrl_c() => {
                if shutdown {
                    eprintln!("Shutdown signal received, forcing shutdown...");
                    run_handle.kill().await?;
                    break;
                }

                shutdown = true;
                eprintln!("Shutting down server...");
                let res = match egg.get_stop_action() {
                    StopAction::Signal(sig) => run_handle.signal(sig).await,
                    StopAction::Command(cmd) => match stdin.as_ref() {
                        Some(sink) => sink.write(format!("{cmd}\n")).await,
                        None => run_handle.signal(15).await,
                    },
                };
                if let Err(e) = res {
                    eprintln!("Shutdown attempt failed: {e}");
                }
            }
        }
    }

    sbx.stop().await?;

    Ok(())
}

pub async fn stop(name: &str) -> Result<(), Box<dyn Error>> {
    Sandbox::get(name).await?.stop().await?;

    println!("Stopped {}", name);

    Ok(())
}

pub async fn destroy(name: &str, delete: &bool) -> Result<(), Box<dyn Error>> {
    if let Ok(sbx) = Sandbox::get(name).await {
        sbx.destroy().await?;
        println!("Destroyed {}", name);
    }

    if let Ok(sbx) = Sandbox::get(&format!("{}-installer", name)).await {
        sbx.destroy().await?;
        println!("Destroyed {} installer", name);
    }

    if *delete && let Ok(vol) = Volume::get(&volume_name(name)).await {
        vol.remove().await?;
        println!("Deleted all {} data", name);
    }

    Ok(())
}

pub async fn list() -> Result<(), Box<dyn Error>> {
    println!("Servers:");

    Sandbox::list()
        .await?
        .sandboxes
        .iter()
        .for_each(|sbx| println!("{}: {:?}", sbx.name(), sbx.local().unwrap().status));

    Ok(())
}

async fn init_installer(egg: &Egg, disk_size: u32) -> Result<VolumeHandle, Box<dyn Error>> {
    let serverfiles = match Volume::get(&volume_name(&egg.name)).await {
        Ok(vol) => vol,
        Err(_) => {
            Volume::builder(volume_name(&egg.name))
                .disk()
                .size(disk_size)
                .create()
                .await?;
            Volume::get(&volume_name(&egg.name)).await?
        }
    };

    let install_command = egg.get_install_command();

    println!("Pulling installer image: {}", install_command.image);
    let sbx = Sandbox::builder(format!("{}-installer", egg.name))
        .image(install_command.image)
        .volume("/mnt/server", |v| v.named(serverfiles.name()))
        .replace()
        .create()
        .await?;

    println!(
        "Running install command: {} {} {}",
        install_command
            .envs
            .iter()
            .map(|e| format!("{}={}", e.0, e.1))
            .collect::<Vec<_>>()
            .join(" "),
        install_command.command,
        install_command.args.join(" ")
    );

    let egg_json = serde_json::to_vec(&egg)?;
    sbx.fs()
        .write(&format!("{}/egg.json", install_command.cwd), &egg_json)
        .await?;

    let install_output = sbx
        .exec_with(install_command.command, |e| {
            e.args(install_command.args)
                .envs(install_command.envs)
                .cwd(install_command.cwd)
        })
        .await?;
    println!(
        "Install exited with code {}\n\nStdout: {}\n\nStderr: {}",
        install_output.status().code,
        install_output.stdout()?,
        install_output.stderr()?
    );

    sbx.destroy().await?;

    if install_output.status().success {
        return Ok(serverfiles);
    }
    Err(format!("Install exited with code {}", install_output.status().code).into())
}

async fn init_runtime(egg: &Egg, serverfiles: VolumeHandle) -> Result<Sandbox, Box<dyn Error>> {
    let run_command = egg.get_run_command();
    let server_port = egg.get_server_port();

    println!("Pulling runtime image: {}", run_command.image);
    let sbx = Sandbox::builder(&egg.name)
        .image(run_command.image)
        .volume("/home/container", |v| v.named(serverfiles.name()))
        .network(|n| n.policy(NetworkPolicy::from_profiles([NetworkProfile::Public])))
        .port_bind(Ipv4Addr::UNSPECIFIED.into(), server_port, server_port)
        .port_udp_bind(Ipv4Addr::UNSPECIFIED.into(), server_port, server_port)
        .replace()
        .create()
        .await?;

    Ok(sbx)
}

fn volume_name(egg_name: &str) -> String {
    format!("{}-server-files", egg_name)
}
