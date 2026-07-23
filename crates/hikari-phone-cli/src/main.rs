use clap::{Parser, Subcommand};
use telephony_core::{EngineEvent, RegistrationState, StateMachine};

#[derive(Debug, Parser)]
#[command(name = "hikari-phone-cli", about = "Hikari Phone diagnostic utility")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print compile-time backend information without exposing credentials.
    Backend,
    /// Exercise the platform-independent call state machine.
    StateDemo,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Backend => {
            println!("backend: {}", telephony_pjsip::backend_description());
            println!("codec: PCMU/8000");
            println!("SIP transport: UDP/IPv4");
            println!("RTP ports: 40000-40009");
        }
        Command::StateDemo => {
            let mut state = StateMachine::default();
            state.apply(EngineEvent::RegistrationChanged(
                RegistrationState::Registered,
            ))?;
            state.apply(EngineEvent::OutgoingCall {
                id: 1,
                peer: "3".into(),
            })?;
            state.apply(EngineEvent::CallActive { id: 1 })?;
            println!("{:#?}", state.snapshot());
            state.apply(EngineEvent::CallEnded {
                id: 1,
                reason: "diagnostic complete".into(),
            })?;
            println!("{:#?}", state.snapshot());
        }
    }
    Ok(())
}
