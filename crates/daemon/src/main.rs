use std::error::Error;
use std::path::PathBuf;

use beankey_daemon::{DaemonConfig, DaemonServer, Engine, ServerError};

fn main() {
    if let Err(error) = run() {
        eprintln!("beankey-daemon: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let config_path = parse_config_argument()?;
    let config = DaemonConfig::load(config_path)?;
    let engine = Engine::open_with_llama(
        &config.dictionary,
        &config.model,
        &config.llama_backend_directory,
    )?;
    let server = match DaemonServer::bind_from_environment(engine, &config.runtime_socket) {
        Ok(server) => server,
        Err(ServerError::AlreadyRunning) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    server.run()?;
    Ok(())
}

fn parse_config_argument() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let Some(flag) = arguments.next() else {
        return Err("usage: beankey-daemon --config PATH".into());
    };
    if flag != "--config" {
        return Err("usage: beankey-daemon --config PATH".into());
    }
    let Some(path) = arguments.next() else {
        return Err("--config requires a path".into());
    };
    if arguments.next().is_some() {
        return Err("unexpected daemon arguments".into());
    }
    Ok(path.into())
}
