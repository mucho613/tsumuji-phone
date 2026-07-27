use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tsumuji_push_gateway::{issue_pairing_code, serve, GatewayState};

#[derive(Debug, Parser)]
#[command(name = "tsumuji-push-gateway", about = "Tsumuji Phone push gateway")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the tailnet pairing API and the loopback-only wake API.
    Serve {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        service_account: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8787")]
        public_listen: SocketAddr,
        #[arg(long, default_value = "127.0.0.1:8788")]
        internal_listen: SocketAddr,
    },
    /// Issue a one-time pairing code for a SIP user.
    PairingCode {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        sip_user: String,
        #[arg(long, default_value_t = 600)]
        ttl_seconds: u64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Serve {
            database,
            service_account,
            public_listen,
            internal_listen,
        } => {
            let state = GatewayState::new(database, Some(&service_account))?;
            serve(state, public_listen, internal_listen).await?;
        }
        Command::PairingCode {
            database,
            sip_user,
            ttl_seconds,
        } => {
            let code = issue_pairing_code(&database, &sip_user, Duration::from_secs(ttl_seconds))?;
            println!("{code}");
        }
    }
    Ok(())
}
