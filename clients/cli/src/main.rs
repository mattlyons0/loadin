use std::error::Error;

use loadin_core::{destroy, init_sandbox, install, list, run, stop};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "loadin", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Install {
        /// URL to a supported format (currently only eggs)
        url: String,
        /// Disk size to allocate in mb (combined storage space for server and save files)
        #[arg(short, long, default_value = "8192")]
        disk_size: u32,
        /// Port to bind server to
        #[arg(short, long)]
        port: u16,
    },
    Run {
        /// Name of installed server (case sensitive). Use `loadin list` to view installed servers.
        name: String,
    },
    Stop {
        /// Name of installed server (case sensitive). Use `loadin list` to view installed servers.
        name: String,
    },
    Destroy {
        /// Name of installed server (case sensitive). Use `loadin list` to view installed servers.
        name: String,

        /// Delete all saved data. This is not reversible!!
        #[arg(long)]
        delete: bool,
    },
    List {},
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    init_sandbox().await?;

    match &cli.command {
        Commands::Install {
            url,
            disk_size,
            port,
        } => {
            install(url, *disk_size, *port, client).await?;
        }
        Commands::Run { name } => {
            run(name).await?;
        }
        Commands::Stop { name } => {
            stop(name).await?;
        }
        Commands::Destroy { name, delete } => {
            destroy(name, delete).await?;
        }
        Commands::List {} => {
            list().await?;
        }
    }

    Ok(())
}
