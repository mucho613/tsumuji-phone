use clap::{Parser, Subcommand};
use std::thread;
use std::time::{Duration, Instant};
use telephony_core::{
    AccountConfig, CallPhase, EngineEvent, RegistrationState, StateMachine, TelephonyEngine,
};
use telephony_pjsip::PjsipEngine;

#[cfg(target_os = "windows")]
const KEYRING_SERVICE: &str = "app.hikari-phone.local";
#[cfg(target_os = "windows")]
const KEYRING_ACCOUNT: &str = "sip-account";
#[cfg(target_os = "windows")]
const BLOCKED_NUMBERS: [&str; 3] = ["110", "118", "119"];

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
    /// Place a time-limited call to a numeric extension (Windows only).
    #[cfg(target_os = "windows")]
    CallTest {
        #[arg(long)]
        server: String,
        #[arg(long)]
        local_address: String,
        #[arg(long)]
        user_id: String,
        #[arg(long, value_parser = parse_destination)]
        destination: String,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u16).range(1..=300))]
        seconds: u16,
    },
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
        #[cfg(target_os = "windows")]
        Command::CallTest {
            server,
            local_address,
            user_id,
            destination,
            seconds,
        } => run_call_test(server, local_address, user_id, &destination, seconds)?,
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn parse_destination(value: &str) -> Result<String, String> {
    if !(1..=32).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("destination must contain 1 to 32 ASCII digits".into());
    }
    if BLOCKED_NUMBERS.contains(&value) {
        return Err("emergency and special-service numbers are blocked".into());
    }
    Ok(value.to_owned())
}

#[cfg(target_os = "windows")]
fn run_call_test(
    server: String,
    local_address: String,
    user_id: String,
    destination: &str,
    seconds: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let password = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.get_password()?;
    let config = AccountConfig {
        server,
        local_address,
        sip_port: 5060,
        rtp_port_start: 40_000,
        rtp_port_end: 40_009,
        user_id,
        password,
    };
    let mut engine = PjsipEngine::default();
    engine.start(&config)?;

    let result = (|| {
        wait_for_registration(&engine, Duration::from_secs(15))?;
        println!("registered; calling extension {destination}");
        engine.dial(destination)?;
        wait_for_active_call(&engine, Duration::from_secs(30))?;
        println!("call active; audio test window is {seconds} seconds");

        let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
        while Instant::now() < deadline {
            if engine.snapshot().call.is_none() {
                println!("remote side ended the call");
                return Ok(());
            }
            thread::sleep(Duration::from_millis(200));
        }

        if engine.snapshot().call.is_some() {
            engine.hang_up()?;
            println!("audio test window complete; call ended locally");
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })();

    let stop_result = engine.stop();
    result?;
    stop_result?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn wait_for_registration(
    engine: &PjsipEngine,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let snapshot = engine.snapshot();
        if snapshot.registration == RegistrationState::Registered {
            return Ok(());
        }
        if let Some(error) = snapshot.last_error {
            return Err(error.into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("registration timed out".into())
}

#[cfg(target_os = "windows")]
fn wait_for_active_call(
    engine: &PjsipEngine,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let snapshot = engine.snapshot();
        if snapshot.call.as_ref().map(|call| call.phase) == Some(CallPhase::Active) {
            return Ok(());
        }
        if snapshot.call.is_none() {
            return Err(snapshot
                .last_call_reason
                .unwrap_or_else(|| "call ended before it became active".into())
                .into());
        }
        if let Some(error) = snapshot.last_error {
            return Err(error.into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("call was not answered within 30 seconds".into())
}
